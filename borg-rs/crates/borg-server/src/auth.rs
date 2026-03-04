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

fn verify_token(headers: &axum::http::HeaderMap, expected: &str) -> bool {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        == Some(expected)
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

    if verify_token(request.headers(), &state.api_token) {
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
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn is_exempt_health() {
        assert!(is_exempt("/api/health"));
    }

    #[test]
    fn is_exempt_auth_token() {
        assert!(is_exempt("/api/auth/token"));
    }

    #[test]
    fn is_exempt_static_assets() {
        assert!(is_exempt("/"));
        assert!(is_exempt("/index.html"));
        assert!(is_exempt("/static/main.js"));
    }

    #[test]
    fn not_exempt_api_paths() {
        assert!(!is_exempt("/api/tasks"));
        assert!(!is_exempt("/api/logs"));
        assert!(!is_exempt("/api/tasks/1/stream"));
        assert!(!is_exempt("/api/chat/events"));
    }

    #[test]
    fn generate_token_is_64_hex_chars() {
        let token = generate_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_tokens_are_unique() {
        assert_ne!(generate_token(), generate_token());
    }

    #[test]
    fn verify_token_accepts_valid_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret123"),
        );
        assert!(verify_token(&headers, "secret123"));
    }

    #[test]
    fn verify_token_rejects_wrong_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrongtoken"),
        );
        assert!(!verify_token(&headers, "secret123"));
    }

    #[test]
    fn verify_token_rejects_missing_header() {
        let headers = HeaderMap::new();
        assert!(!verify_token(&headers, "secret123"));
    }

    #[test]
    fn verify_token_rejects_query_param_only() {
        // Tokens in query strings are not accepted — Authorization header required.
        let headers = HeaderMap::new();
        assert!(!verify_token(&headers, "secret123"));
    }

    #[test]
    fn verify_token_rejects_malformed_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("secret123"),
        );
        assert!(!verify_token(&headers, "secret123"));
    }

    #[test]
    fn verify_token_rejects_basic_auth() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Basic secret123"),
        );
        assert!(!verify_token(&headers, "secret123"));
    }
}
