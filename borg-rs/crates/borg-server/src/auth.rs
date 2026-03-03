use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{ConnectInfo, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use rand::Rng;
use serde_json::json;

use crate::AppState;

pub fn generate_token() -> String {
    rand::thread_rng()
        .gen::<[u8; 32]>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// Paths exempt from bearer auth entirely.
fn is_exempt(path: &str) -> bool {
    path == "/api/health" || path == "/api/auth/token" || !path.starts_with("/api/")
}

pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let path = request.uri().path();

    if is_exempt(path) {
        return next.run(request).await;
    }

    // Check Authorization: Bearer header first
    let header_token = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    // For SSE endpoints EventSource can't set headers — allow ?token= query param
    let query_token_buf = if header_token.is_none() {
        request.uri().query().and_then(|q| {
            q.split('&').find_map(|kv| {
                let mut parts = kv.splitn(2, '=');
                let k = parts.next()?;
                let v = parts.next()?;
                if k == "token" { Some(v.to_string()) } else { None }
            })
        })
    } else {
        None
    };

    let provided = header_token.or(query_token_buf.as_deref());

    if provided == Some(state.api_token.as_str()) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response()
    }
}

// GET /api/auth/token — only reachable from loopback so agent containers on
// borg-agent-net cannot obtain the master token by calling this endpoint.
pub async fn get_token(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> Response {
    if !addr.ip().is_loopback() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "token endpoint is localhost-only"})),
        )
            .into_response();
    }
    Json(json!({"token": state.api_token})).into_response()
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    fn is_loopback(addr: SocketAddr) -> bool {
        addr.ip().is_loopback()
    }

    #[test]
    fn localhost_ipv4_is_allowed() {
        assert!(is_loopback(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1234)));
    }

    #[test]
    fn localhost_ipv6_is_allowed() {
        assert!(is_loopback(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 1234)));
    }

    #[test]
    fn docker_bridge_ip_is_rejected() {
        // Typical Docker bridge gateway — agent containers appear from this range
        let docker_ip: IpAddr = "172.17.0.1".parse().unwrap();
        assert!(!is_loopback(SocketAddr::new(docker_ip, 1234)));
    }

    #[test]
    fn private_network_ip_is_rejected() {
        let private_ip: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(!is_loopback(SocketAddr::new(private_ip, 1234)));
    }

    #[test]
    fn public_ip_is_rejected() {
        let public_ip: IpAddr = "203.0.113.5".parse().unwrap();
        assert!(!is_loopback(SocketAddr::new(public_ip, 1234)));
    }
}
