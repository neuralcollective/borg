use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::AppState;

// ── Token generation ─────────────────────────────────────────────────────

pub fn generate_token() -> String {
    rand::thread_rng()
        .gen::<[u8; 32]>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ── JWT ──────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JwtClaims {
    pub sub: i64,        // user id
    pub username: String,
    pub is_admin: bool,
    pub exp: usize,      // expiry (unix timestamp)
}

pub fn create_jwt(user_id: i64, username: &str, is_admin: bool, secret: &str) -> String {
    let exp = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::days(30))
        .unwrap_or_else(chrono::Utc::now)
        .timestamp() as usize;
    let claims = JwtClaims {
        sub: user_id,
        username: username.to_string(),
        is_admin,
        exp,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap_or_default()
}

pub fn verify_jwt(token: &str, secret: &str) -> Option<JwtClaims> {
    decode::<JwtClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .ok()
    .map(|data| data.claims)
}

// ── Password hashing ─────────────────────────────────────────────────────

pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    use argon2::{
        password_hash::{rand_core::OsRng, SaltString},
        Argon2, PasswordHasher,
    };
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(password.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    use argon2::{Argon2, PasswordHash, PasswordVerifier};
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

// ── Auth user (extracted from request) ───────────────────────────────────

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: i64,
    pub username: String,
    pub is_admin: bool,
}

impl AuthUser {
    pub fn system_admin() -> Self {
        Self {
            id: 0,
            username: "admin".to_string(),
            is_admin: true,
        }
    }
}

// Extract bearer token from Authorization header
fn extract_bearer(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

// Paths exempt from bearer auth entirely.
fn is_exempt(path: &str) -> bool {
    path == "/api/health"
        || path == "/api/auth/token"
        || path == "/api/auth/login"
        || path == "/api/auth/setup"
        || path == "/api/auth/status"
        || !path.starts_with("/api/")
}

fn verify_token(headers: &axum::http::HeaderMap, expected: &str) -> bool {
    extract_bearer(headers) == Some(expected)
}

// ── Middleware ────────────────────────────────────────────────────────────

pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut request: axum::extract::Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();

    if is_exempt(&path) {
        return next.run(request).await;
    }

    // Try JWT first
    if let Some(token) = extract_bearer(request.headers()) {
        if let Some(claims) = verify_jwt(token, &state.jwt_secret) {
            request.extensions_mut().insert(AuthUser {
                id: claims.sub,
                username: claims.username,
                is_admin: claims.is_admin,
            });
            return next.run(request).await;
        }

        // Fall back to shared API token (sidecar, CLI, etc.) — treated as admin
        if token == state.api_token {
            request.extensions_mut().insert(AuthUser::system_admin());
            return next.run(request).await;
        }
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "unauthorized"})),
    )
        .into_response()
}

// ── Handlers ─────────────────────────────────────────────────────────────

// GET /api/auth/token — returns shared token for backward compat
pub async fn get_token(State(state): State<Arc<AppState>>) -> Response {
    Json(json!({"token": state.api_token})).into_response()
}

// GET /api/auth/status — whether setup is needed, and user count
pub async fn auth_status(State(state): State<Arc<AppState>>) -> Response {
    let user_count = state.db.count_users().unwrap_or(0);
    Json(json!({
        "needs_setup": user_count == 0,
        "user_count": user_count,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct SetupBody {
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
}

// POST /api/auth/setup — create first admin user (only when no users exist)
pub async fn setup(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SetupBody>,
) -> Response {
    let user_count = match state.db.count_users() {
        Ok(c) => c,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response();
        }
    };

    if user_count > 0 {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "setup already completed"})),
        )
            .into_response();
    }

    if body.username.trim().is_empty() || body.password.len() < 4 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "username required, password min 4 chars"})),
        )
            .into_response();
    }

    let password_hash = match hash_password(&body.password) {
        Ok(h) => h,
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response();
        }
    };

    let display = body.display_name.as_deref().unwrap_or(&body.username);
    match state.db.create_user(&body.username, display, &password_hash, true) {
        Ok(id) => {
            let token = create_jwt(id, &body.username, true, &state.jwt_secret);
            Json(json!({
                "token": token,
                "user": { "id": id, "username": body.username, "display_name": display, "is_admin": true }
            }))
            .into_response()
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct LoginBody {
    pub username: String,
    pub password: String,
}

// POST /api/auth/login
pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginBody>,
) -> Response {
    let user = match state.db.get_user_by_username(&body.username) {
        Ok(Some(u)) => u,
        Ok(None) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "invalid credentials"})),
            )
                .into_response();
        }
        Err(e) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response();
        }
    };

    let (id, username, display_name, password_hash, is_admin) = user;

    if !verify_password(&body.password, &password_hash) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid credentials"})),
        )
            .into_response();
    }

    let token = create_jwt(id, &username, is_admin, &state.jwt_secret);
    Json(json!({
        "token": token,
        "user": { "id": id, "username": username, "display_name": display_name, "is_admin": is_admin }
    }))
    .into_response()
}

// GET /api/auth/me — return current user info
pub async fn get_me(request: axum::extract::Request) -> Response {
    let user = request.extensions().get::<AuthUser>().cloned();
    match user {
        Some(u) => Json(json!({
            "id": u.id,
            "username": u.username,
            "is_admin": u.is_admin,
        }))
        .into_response(),
        None => (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"}))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};

    use super::*;

    #[test]
    fn is_exempt_health() {
        assert!(is_exempt("/api/health"));
    }

    #[test]
    fn is_exempt_auth_token() {
        assert!(is_exempt("/api/auth/token"));
    }

    #[test]
    fn is_exempt_auth_login() {
        assert!(is_exempt("/api/auth/login"));
    }

    #[test]
    fn is_exempt_auth_setup() {
        assert!(is_exempt("/api/auth/setup"));
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

    #[test]
    fn jwt_roundtrip() {
        let secret = "test_secret_key";
        let token = create_jwt(42, "testuser", true, secret);
        let claims = verify_jwt(&token, secret).expect("should verify");
        assert_eq!(claims.sub, 42);
        assert_eq!(claims.username, "testuser");
        assert!(claims.is_admin);
    }

    #[test]
    fn jwt_wrong_secret_fails() {
        let token = create_jwt(1, "u", false, "secret1");
        assert!(verify_jwt(&token, "secret2").is_none());
    }

    #[test]
    fn password_hash_roundtrip() {
        let hash = hash_password("mypassword").expect("should hash");
        assert!(verify_password("mypassword", &hash));
        assert!(!verify_password("wrongpassword", &hash));
    }
}
