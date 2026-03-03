use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use rand::Rng;
use serde_json::json;
use subtle::ConstantTimeEq;

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

    let valid = provided
        .map(|t| t.as_bytes().ct_eq(state.api_token.as_bytes()).into())
        .unwrap_or(false);

    if valid {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response()
    }
}

// GET /api/auth/token — returns the token to any caller that can reach the
// dashboard. The token protects against rogue local processes (e.g. a
// compromised container), not against someone who already has HTTP access to
// the dashboard. If the dashboard page loads, the caller is authorized.
pub async fn get_token(
    State(state): State<Arc<AppState>>,
) -> Response {
    Json(json!({"token": state.api_token})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_token_is_64_hex_chars() {
        let t = generate_token();
        assert_eq!(t.len(), 64);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_token_produces_unique_values() {
        let a = generate_token();
        let b = generate_token();
        assert_ne!(a, b);
    }

    #[test]
    fn is_exempt_health() {
        assert!(is_exempt("/api/health"));
    }

    #[test]
    fn is_exempt_auth_token() {
        assert!(is_exempt("/api/auth/token"));
    }

    #[test]
    fn is_exempt_non_api_paths() {
        assert!(is_exempt("/"));
        assert!(is_exempt("/index.html"));
        assert!(is_exempt("/assets/app.js"));
    }

    #[test]
    fn is_exempt_protected_api() {
        assert!(!is_exempt("/api/tasks"));
        assert!(!is_exempt("/api/status"));
    }

    #[test]
    fn ct_eq_matching_tokens() {
        let token = "abc123";
        let result: bool = token.as_bytes().ct_eq(token.as_bytes()).into();
        assert!(result);
    }

    #[test]
    fn ct_eq_mismatched_tokens() {
        let result: bool = "abc123".as_bytes().ct_eq("xyz789".as_bytes()).into();
        assert!(!result);
    }

    #[test]
    fn ct_eq_prefix_not_accepted() {
        // A prefix of the real token must not be accepted.
        let token = "abc123def";
        let prefix = "abc123";
        let result: bool = prefix.as_bytes().ct_eq(token.as_bytes()).into();
        assert!(!result);
    }

    #[test]
    fn ct_eq_empty_vs_nonempty() {
        let result: bool = "".as_bytes().ct_eq("token".as_bytes()).into();
        assert!(!result);
    }
}
