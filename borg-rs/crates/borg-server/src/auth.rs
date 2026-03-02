use std::sync::Arc;

use axum::{
    extract::State,
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

#[cfg(test)]
mod tests {
    use super::is_exempt;

    #[test]
    fn health_is_exempt() {
        assert!(is_exempt("/api/health"));
    }

    #[test]
    fn auth_token_is_exempt() {
        assert!(is_exempt("/api/auth/token"));
    }

    #[test]
    fn non_api_path_is_exempt() {
        assert!(is_exempt("/dashboard"));
        assert!(is_exempt("/"));
        assert!(is_exempt("/static/app.js"));
    }

    #[test]
    fn api_tasks_not_exempt() {
        assert!(!is_exempt("/api/tasks"));
    }

    #[test]
    fn api_settings_not_exempt() {
        assert!(!is_exempt("/api/settings"));
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
