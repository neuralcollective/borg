use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::AppState;

const MAX_LOGIN_ATTEMPTS: u32 = 5;
const LOGIN_WINDOW_SECS: u64 = 300;
const SSO_STATE_EXPIRY_SECS: i64 = 600;
pub const WORKSPACE_HEADER: &str = "x-workspace-id";
pub const DEFAULT_CLOUDFLARE_ACCESS_EMAIL_HEADER: &str = "cf-access-authenticated-user-email";

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
    pub sub: i64, // user id
    pub username: String,
    pub is_admin: bool,
    pub exp: usize, // expiry (unix timestamp)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct SsoStateClaims {
    pub provider: String,
    pub exp: usize,
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
    .unwrap_or_else(|e| {
        tracing::error!("JWT encode failed: {e}");
        String::new()
    })
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

fn create_sso_state(provider: &str, secret: &str) -> String {
    let exp = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::seconds(SSO_STATE_EXPIRY_SECS))
        .unwrap_or_else(chrono::Utc::now)
        .timestamp() as usize;
    let claims = SsoStateClaims {
        provider: provider.to_string(),
        exp,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap_or_else(|e| {
        tracing::error!("SSO state encode failed: {e}");
        String::new()
    })
}

fn verify_sso_state(token: &str, secret: &str) -> Option<SsoStateClaims> {
    decode::<SsoStateClaims>(
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
    pub default_workspace_id: i64,
}

impl AuthUser {
    pub fn system_admin() -> Self {
        Self {
            id: 0,
            username: "admin".to_string(),
            is_admin: true,
            default_workspace_id: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceContext {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub role: String,
    pub is_default: bool,
}

fn auth_mode_is_cloudflare_access(mode: &str) -> bool {
    mode.eq_ignore_ascii_case("cloudflare_access")
}

fn extract_email_header(headers: &HeaderMap, header_name: &str) -> Option<String> {
    let header_name = if header_name.trim().is_empty() {
        DEFAULT_CLOUDFLARE_ACCESS_EMAIL_HEADER
    } else {
        header_name
    };
    headers
        .get(header_name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

// Extract bearer token from Authorization header
fn extract_bearer(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

pub fn resolve_auth_user_from_headers(
    headers: &HeaderMap,
    secret: &str,
    api_token: &str,
    auth_disabled: bool,
    auth_mode: &str,
    cloudflare_access_email_header: &str,
) -> Option<AuthUser> {
    if auth_disabled {
        return Some(AuthUser::system_admin());
    }
    if auth_mode_is_cloudflare_access(auth_mode) {
        if let Some(token) = extract_bearer(headers) {
            if token == api_token {
                return Some(AuthUser::system_admin());
            }
        }
        if let Some(email) = extract_email_header(headers, cloudflare_access_email_header) {
            return Some(AuthUser {
                id: 0,
                username: email,
                is_admin: false,
                default_workspace_id: 0,
            });
        }
        return None;
    }
    let token = extract_bearer(headers)?;
    if let Some(claims) = verify_jwt(token, secret) {
        return Some(AuthUser {
            id: claims.sub,
            username: claims.username,
            is_admin: claims.is_admin,
            default_workspace_id: 0,
        });
    }
    if token == api_token {
        return Some(AuthUser::system_admin());
    }
    None
}

fn external_email_is_admin(config: &borg_core::config::Config, email: &str) -> bool {
    config
        .cloudflare_admin_emails
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(email))
}

fn provision_external_user(state: &AppState, email: &str) -> Result<AuthUser, Response> {
    let has_admins = state.db.count_admin_users().unwrap_or(0) > 0;
    let desired_admin = external_email_is_admin(&state.config, email) || !has_admins;
    let existing = state.db.get_user_by_username(email).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("user lookup failed: {e}")})),
        )
            .into_response()
    })?;

    let user_id = if let Some((id, _, _, _, is_admin)) = existing {
        if desired_admin && !is_admin {
            state.db.set_user_admin(id, true).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("failed to promote admin: {e}")})),
                )
                    .into_response()
            })?;
        }
        id
    } else {
        let password_hash = hash_password(&generate_token()).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("password setup failed: {e}")})),
            )
                .into_response()
        })?;
        state
            .db
            .create_user(email, email, &password_hash, desired_admin)
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("user create failed: {e}")})),
                )
                    .into_response()
            })?
    };

    let is_admin = state
        .db
        .get_user_by_id(user_id)
        .ok()
        .flatten()
        .map(|(_, _, _, is_admin)| is_admin)
        .unwrap_or(desired_admin);

    if is_admin {
        if let Err(e) = state.db.ensure_admin_workspace_memberships(user_id) {
            tracing::warn!(
                user_id,
                email,
                "failed to sync admin workspace memberships: {e}"
            );
        }
        if let Err(e) = state.db.set_preferred_admin_workspace(user_id) {
            tracing::warn!(
                user_id,
                email,
                "failed to set preferred admin workspace: {e}"
            );
        }
    }

    let default_workspace_id = state
        .db
        .get_user_default_workspace_id(user_id)
        .ok()
        .flatten()
        .unwrap_or(0);
    Ok(AuthUser {
        id: user_id,
        username: email.to_string(),
        is_admin,
        default_workspace_id,
    })
}

// Paths exempt from bearer auth entirely.
fn is_exempt(path: &str) -> bool {
    path == "/api/health"
        || path == "/api/auth/login"
        || path == "/api/auth/setup"
        || path == "/api/auth/status"
        || path.starts_with("/api/auth/sso/")
        || path == "/api/email/inbound"
        || path.starts_with("/api/public/")
        || !path.starts_with("/api/")
}

// Sync admin workspace memberships at most once per 60s per user.
fn sync_admin_memberships_if_stale(state: &AppState, user_id: i64) {
    use std::{collections::HashMap, sync::Mutex, time::Instant};
    static LAST_SYNC: Mutex<Option<HashMap<i64, Instant>>> = Mutex::new(None);
    let mut guard = LAST_SYNC.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    let now = Instant::now();
    if let Some(last) = map.get(&user_id) {
        if now.duration_since(*last).as_secs() < 60 {
            return;
        }
    }
    map.insert(user_id, now);
    drop(guard);
    if let Err(e) = state.db.ensure_admin_workspace_memberships(user_id) {
        tracing::warn!(user_id, "failed to sync admin workspace memberships: {e}");
    }
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

    if state.config.disable_auth {
        request.extensions_mut().insert(AuthUser::system_admin());
        return next.run(request).await;
    }

    if auth_mode_is_cloudflare_access(&state.config.auth_mode) {
        if let Some(token) = extract_bearer(request.headers()) {
            if token == state.api_token {
                request.extensions_mut().insert(AuthUser::system_admin());
                return next.run(request).await;
            }
        }
        let Some(email) = extract_email_header(
            request.headers(),
            &state.config.cloudflare_access_email_header,
        ) else {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "missing Cloudflare Access identity"})),
            )
                .into_response();
        };
        match provision_external_user(state.as_ref(), &email) {
            Ok(user) => {
                request.extensions_mut().insert(user);
                return next.run(request).await;
            },
            Err(response) => return response,
        }
    }

    // Try JWT first
    if let Some(token) = extract_bearer(request.headers()) {
        if let Some(claims) = verify_jwt(token, &state.jwt_secret) {
            if claims.is_admin {
                sync_admin_memberships_if_stale(&state, claims.sub);
            }
            let default_workspace_id = state
                .db
                .get_user_default_workspace_id(claims.sub)
                .ok()
                .flatten()
                .unwrap_or(0);
            request.extensions_mut().insert(AuthUser {
                id: claims.sub,
                username: claims.username,
                is_admin: claims.is_admin,
                default_workspace_id,
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

fn requested_workspace_id(headers: &HeaderMap) -> Option<i64> {
    headers
        .get(WORKSPACE_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|id| *id > 0)
}

pub async fn workspace_middleware(
    State(state): State<Arc<AppState>>,
    mut request: axum::extract::Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    if is_exempt(&path) {
        return next.run(request).await;
    }

    let Some(user) = request.extensions().get::<AuthUser>().cloned() else {
        return next.run(request).await;
    };

    let requested_id = requested_workspace_id(request.headers());
    let workspace = if user.id == 0 {
        let candidate = if let Some(id) = requested_id {
            state.db.get_workspace(id).ok().flatten()
        } else {
            state.db.get_system_workspace().ok().flatten()
        };
        match candidate {
            Some(workspace) => WorkspaceContext {
                id: workspace.id,
                name: workspace.name,
                kind: workspace.kind,
                role: "admin".to_string(),
                is_default: requested_id.is_none(),
            },
            None => return next.run(request).await,
        }
    } else {
        let workspace_id = requested_id.unwrap_or(user.default_workspace_id);
        if workspace_id <= 0 {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "no workspace available"})),
            )
                .into_response();
        }
        let membership = match state
            .db
            .get_user_workspace_membership(user.id, workspace_id)
        {
            Ok(Some(m)) => m,
            Ok(None) => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({"error": "workspace access denied"})),
                )
                    .into_response();
            },
            Err(e) => {
                tracing::error!("workspace resolution failed: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "workspace resolution failed"})),
                )
                    .into_response();
            },
        };
        WorkspaceContext {
            id: membership.workspace_id,
            name: membership.name,
            kind: membership.kind,
            role: membership.role,
            is_default: membership.is_default,
        }
    };

    request.extensions_mut().insert(workspace);
    next.run(request).await
}

// ── Handlers ─────────────────────────────────────────────────────────────

// GET /api/auth/token — returns shared token for backward compat
pub async fn get_token(State(state): State<Arc<AppState>>) -> Response {
    if !state.config.disable_auth {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "shared token disabled"})),
        )
            .into_response();
    }
    Json(json!({"token": state.api_token})).into_response()
}

// GET /api/auth/status — whether setup is needed, and user count
pub async fn auth_status(State(state): State<Arc<AppState>>) -> Response {
    let google_configured = state
        .db
        .get_config("google_client_id")
        .ok()
        .flatten()
        .is_some_and(|v| !v.trim().is_empty())
        && state
            .db
            .get_config("google_client_secret")
            .ok()
            .flatten()
            .is_some_and(|v| !v.trim().is_empty());
    let microsoft_configured = state
        .db
        .get_config("ms_client_id")
        .ok()
        .flatten()
        .is_some_and(|v| !v.trim().is_empty())
        && state
            .db
            .get_config("ms_client_secret")
            .ok()
            .flatten()
            .is_some_and(|v| !v.trim().is_empty());
    let mut sso_providers = Vec::new();
    if google_configured {
        sso_providers.push("google");
    }
    if microsoft_configured {
        sso_providers.push("microsoft");
    }
    if state.config.disable_auth {
        return Json(json!({
            "needs_setup": false,
            "user_count": 1,
            "auth_disabled": true,
            "auth_mode": "disabled",
            "sso_providers": sso_providers,
        }))
        .into_response();
    }
    if auth_mode_is_cloudflare_access(&state.config.auth_mode) {
        let user_count = state.db.count_users().unwrap_or(0);
        return Json(json!({
            "needs_setup": false,
            "user_count": user_count,
            "auth_disabled": false,
            "auth_mode": "cloudflare_access",
            "sso_providers": sso_providers,
        }))
        .into_response();
    }
    let user_count = state.db.count_users().unwrap_or(0);
    Json(json!({
        "needs_setup": user_count == 0,
        "user_count": user_count,
        "auth_disabled": false,
        "auth_mode": "local",
        "sso_providers": sso_providers,
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
static SETUP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub async fn setup(State(state): State<Arc<AppState>>, Json(body): Json<SetupBody>) -> Response {
    if auth_mode_is_cloudflare_access(&state.config.auth_mode) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "setup disabled when AUTH_MODE=cloudflare_access"})),
        )
            .into_response();
    }
    let _guard = match SETUP_LOCK.try_lock() {
        Ok(g) => g,
        Err(_) => {
            return (
                StatusCode::CONFLICT,
                Json(json!({"error": "setup already in progress"})),
            )
                .into_response();
        },
    };

    let user_count = match state.db.count_users() {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        },
    };

    if user_count > 0 {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "setup already completed"})),
        )
            .into_response();
    }

    if body.username.trim().is_empty() || body.password.len() < 8 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "username required, password min 8 chars"})),
        )
            .into_response();
    }

    let password_hash = match hash_password(&body.password) {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        },
    };

    let display = body.display_name.as_deref().unwrap_or(&body.username);
    match state
        .db
        .create_user(&body.username, display, &password_hash, true)
    {
        Ok(id) => {
            let default_workspace_id = state
                .db
                .get_user_default_workspace_id(id)
                .ok()
                .flatten()
                .unwrap_or(0);
            let token = create_jwt(id, &body.username, true, &state.jwt_secret);
            Json(json!({
                "token": token,
                "user": { "id": id, "username": body.username, "display_name": display, "is_admin": true, "default_workspace_id": default_workspace_id }
            }))
            .into_response()
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct LoginBody {
    pub username: String,
    pub password: String,
}

// POST /api/auth/login
pub async fn login(State(state): State<Arc<AppState>>, Json(body): Json<LoginBody>) -> Response {
    if auth_mode_is_cloudflare_access(&state.config.auth_mode) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "login disabled when AUTH_MODE=cloudflare_access"})),
        )
            .into_response();
    }
    // Rate limiting
    {
        let mut attempts = state
            .login_attempts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let now = std::time::Instant::now();
        // Clean stale entries
        attempts.retain(|_, (_, t)| now.duration_since(*t).as_secs() < LOGIN_WINDOW_SECS);
        if let Some((count, _)) = attempts.get(&body.username) {
            if *count >= MAX_LOGIN_ATTEMPTS {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({"error": "too many login attempts, try again later"})),
                )
                    .into_response();
            }
        }
    }

    let user = match state.db.get_user_by_username(&body.username) {
        Ok(Some(u)) => u,
        Ok(None) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "invalid credentials"})),
            )
                .into_response();
        },
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        },
    };

    let (id, username, display_name, password_hash, is_admin) = user;

    if !verify_password(&body.password, &password_hash) {
        let mut attempts = state
            .login_attempts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let entry = attempts
            .entry(body.username.clone())
            .or_insert((0, std::time::Instant::now()));
        entry.0 += 1;
        entry.1 = std::time::Instant::now();
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid credentials"})),
        )
            .into_response();
    }

    // Clear rate limit on success
    {
        let mut attempts = state
            .login_attempts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        attempts.remove(&body.username);
    }

    let token = create_jwt(id, &username, is_admin, &state.jwt_secret);
    let default_workspace_id = state
        .db
        .get_user_default_workspace_id(id)
        .ok()
        .flatten()
        .unwrap_or(0);
    Json(json!({
        "token": token,
        "user": { "id": id, "username": username, "display_name": display_name, "is_admin": is_admin, "default_workspace_id": default_workspace_id }
    }))
    .into_response()
}

// GET /api/auth/me — return current user info
pub async fn get_me(request: axum::extract::Request) -> Response {
    let user = request.extensions().get::<AuthUser>().cloned();
    let workspace = request.extensions().get::<WorkspaceContext>().cloned();
    match user {
        Some(u) => Json(json!({
            "id": u.id,
            "username": u.username,
            "display_name": u.username,
            "is_admin": u.is_admin,
            "default_workspace_id": u.default_workspace_id,
            "workspace": workspace.as_ref().map(|w| json!({
                "id": w.id,
                "name": w.name,
                "kind": w.kind,
                "role": w.role,
                "is_default": w.is_default,
            })),
        }))
        .into_response(),
        None => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
pub struct SsoCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

fn sso_redirect_uri(config: &borg_core::config::Config, provider: &str) -> String {
    format!("{}/api/auth/sso/{provider}/callback", config.get_base_url())
}

fn sso_client_credentials(state: &AppState, provider: &str) -> Result<(String, String), Response> {
    let (id_key, secret_key) = match provider {
        "google" => ("google_client_id", "google_client_secret"),
        "microsoft" => ("ms_client_id", "ms_client_secret"),
        _ => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(json!({"error": "unknown provider"})),
            )
                .into_response());
        },
    };
    let client_id = state
        .db
        .get_config(id_key)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("config lookup failed: {e}")})),
            )
                .into_response()
        })?
        .unwrap_or_default();
    let client_secret = state
        .db
        .get_config(secret_key)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("config lookup failed: {e}")})),
            )
                .into_response()
        })?
        .unwrap_or_default();
    if client_id.trim().is_empty() || client_secret.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "provider credentials not configured"})),
        )
            .into_response());
    }
    Ok((client_id, client_secret))
}

fn sso_error_redirect(message: &str) -> Response {
    axum::response::Redirect::temporary(&format!("/#auth_error={message}")).into_response()
}

pub async fn sso_start(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
) -> Response {
    if state.config.disable_auth {
        return sso_error_redirect("sso_disabled_when_auth_is_disabled");
    }
    if auth_mode_is_cloudflare_access(&state.config.auth_mode) {
        return sso_error_redirect("sso_disabled_when_auth_mode_is_cloudflare_access");
    }
    let Ok((client_id, _)) = sso_client_credentials(state.as_ref(), &provider) else {
        return sso_error_redirect("missing_provider_credentials");
    };
    let redirect_uri = sso_redirect_uri(&state.config, &provider);
    let state_token = create_sso_state(&provider, &state.jwt_secret);
    if state_token.is_empty() {
        return sso_error_redirect("state_encode_failed");
    }
    let auth_url = match provider.as_str() {
        "google" => format!(
            "https://accounts.google.com/o/oauth2/v2/auth?client_id={client_id}\
             &redirect_uri={redirect}&response_type=code\
             &scope=openid%20email%20profile&prompt=select_account&state={state}",
            redirect = urlencoding::encode(&redirect_uri),
            state = urlencoding::encode(&state_token),
        ),
        "microsoft" => format!(
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize?client_id={client_id}\
             &redirect_uri={redirect}&response_type=code\
             &scope=openid%20profile%20email%20offline_access%20User.Read\
             &prompt=select_account&state={state}",
            redirect = urlencoding::encode(&redirect_uri),
            state = urlencoding::encode(&state_token),
        ),
        _ => return sso_error_redirect("unknown_provider"),
    };
    axum::response::Redirect::temporary(&auth_url).into_response()
}

pub async fn sso_callback(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Query(query): Query<SsoCallbackQuery>,
) -> Response {
    if let Some(error) = query.error {
        return sso_error_redirect(&error);
    }
    let Some(code) = query.code else {
        return sso_error_redirect("missing_code");
    };
    let Some(state_token) = query.state else {
        return sso_error_redirect("missing_state");
    };
    let Some(claims) = verify_sso_state(&state_token, &state.jwt_secret) else {
        return sso_error_redirect("invalid_state");
    };
    if claims.provider != provider {
        return sso_error_redirect("provider_mismatch");
    }
    let Ok((client_id, client_secret)) = sso_client_credentials(state.as_ref(), &provider) else {
        return sso_error_redirect("missing_provider_credentials");
    };
    let redirect_uri = sso_redirect_uri(&state.config, &provider);
    let token_url = match provider.as_str() {
        "google" => "https://oauth2.googleapis.com/token",
        "microsoft" => "https://login.microsoftonline.com/common/oauth2/v2.0/token",
        _ => return sso_error_redirect("unknown_provider"),
    };
    let client = reqwest::Client::new();
    let token_resp = match client
        .post(token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
        ])
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(err) => {
            tracing::error!("sso token exchange failed: {err}");
            return sso_error_redirect("token_exchange_failed");
        },
    };
    if !token_resp.status().is_success() {
        let body = token_resp.text().await.unwrap_or_default();
        tracing::error!("sso token exchange failed for {provider}: {body}");
        return sso_error_redirect("token_exchange_failed");
    }
    let token_json: serde_json::Value = match token_resp.json().await {
        Ok(json) => json,
        Err(err) => {
            tracing::error!("sso token parse failed: {err}");
            return sso_error_redirect("token_parse_failed");
        },
    };
    let Some(access_token) = token_json["access_token"].as_str() else {
        return sso_error_redirect("missing_access_token");
    };
    let user_info_resp = match provider.as_str() {
        "google" => {
            client
                .get("https://openidconnect.googleapis.com/v1/userinfo")
                .bearer_auth(access_token)
                .send()
                .await
        },
        "microsoft" => {
            client
                .get("https://graph.microsoft.com/v1.0/me?$select=id,mail,userPrincipalName")
                .bearer_auth(access_token)
                .send()
                .await
        },
        _ => return sso_error_redirect("unknown_provider"),
    };
    let user_info_resp = match user_info_resp {
        Ok(resp) => resp,
        Err(err) => {
            tracing::error!("sso userinfo request failed: {err}");
            return sso_error_redirect("userinfo_failed");
        },
    };
    if !user_info_resp.status().is_success() {
        let body = user_info_resp.text().await.unwrap_or_default();
        tracing::error!("sso userinfo failed for {provider}: {body}");
        return sso_error_redirect("userinfo_failed");
    }
    let user_info: serde_json::Value = match user_info_resp.json().await {
        Ok(json) => json,
        Err(err) => {
            tracing::error!("sso userinfo parse failed: {err}");
            return sso_error_redirect("userinfo_parse_failed");
        },
    };
    let email = match provider.as_str() {
        "google" => user_info["email"].as_str().unwrap_or("").trim().to_string(),
        "microsoft" => user_info["mail"]
            .as_str()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| user_info["userPrincipalName"].as_str())
            .unwrap_or("")
            .trim()
            .to_string(),
        _ => String::new(),
    };
    if email.is_empty() {
        return sso_error_redirect("missing_email");
    }
    let allowed_domains = state
        .db
        .get_config("sso_allowed_domains")
        .ok()
        .flatten()
        .unwrap_or_default();
    if !allowed_domains.trim().is_empty() {
        let domain = email.rsplit('@').next().unwrap_or("");
        let allowed = allowed_domains
            .split(',')
            .any(|d| d.trim().eq_ignore_ascii_case(domain));
        if !allowed {
            return sso_error_redirect("email_domain_not_allowed");
        }
    }
    let user = match provision_external_user(state.as_ref(), &email) {
        Ok(user) => user,
        Err(resp) => return resp,
    };
    let token = create_jwt(user.id, &user.username, user.is_admin, &state.jwt_secret);
    axum::response::Redirect::temporary(&format!(
        "/#auth_token={}&auth_provider={provider}",
        urlencoding::encode(&token)
    ))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_exempt_health() {
        assert!(is_exempt("/api/health"));
    }

    #[test]
    fn auth_token_requires_auth() {
        assert!(!is_exempt("/api/auth/token"));
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
    fn is_exempt_public_share_paths() {
        assert!(is_exempt("/api/public/projects/abc123"));
        assert!(is_exempt("/api/public/projects/abc123/tasks"));
        assert!(is_exempt("/api/public/projects/abc123/documents"));
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
    fn cloudflare_access_header_extracts_email() {
        let mut headers = HeaderMap::new();
        headers.insert(
            DEFAULT_CLOUDFLARE_ACCESS_EMAIL_HEADER,
            "user@example.com".parse().unwrap(),
        );
        let user = resolve_auth_user_from_headers(
            &headers,
            "secret",
            "api-token",
            false,
            "cloudflare_access",
            DEFAULT_CLOUDFLARE_ACCESS_EMAIL_HEADER,
        )
        .expect("cloudflare user should resolve");
        assert_eq!(user.username, "user@example.com");
    }

    #[test]
    fn password_hash_roundtrip() {
        let hash = hash_password("mypassword").expect("should hash");
        assert!(verify_password("mypassword", &hash));
        assert!(!verify_password("wrongpassword", &hash));
    }
}
