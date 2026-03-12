use std::{path::PathBuf, sync::Arc};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use base64::Engine;
use borg_core::linked_credentials::{
    capture_bundle, restore_bundle, should_revalidate, validate_home, LinkedCredentialValidation,
    PROVIDER_CLAUDE, PROVIDER_OPENAI,
};
use chrono::{Duration as ChronoDuration, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    time::{sleep, Duration},
};

use super::internal;
use crate::AppState;

const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CLAUDE_AUTH_ENDPOINT: &str = "https://claude.ai/oauth/authorize";
const CLAUDE_TOKEN_ENDPOINT: &str = "https://console.anthropic.com/v1/oauth/token";
const CLAUDE_REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";
const CLAUDE_SCOPES: &[&str] = &[
    "org:create_api_key",
    "user:profile",
    "user:inference",
    "user:sessions:claude_code",
    "user:mcp_servers",
];

#[derive(Debug, Clone, Serialize)]
pub(crate) struct LinkedCredentialConnectSession {
    pub id: String,
    pub provider: String,
    pub status: String,
    pub auth_url: String,
    pub device_code: String,
    pub message: String,
    pub error: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing)]
    pub user_id: i64,
    #[serde(skip_serializing)]
    pub code_verifier: String,
}

fn normalize_provider(provider: &str) -> Option<&'static str> {
    match provider.trim().to_ascii_lowercase().as_str() {
        PROVIDER_CLAUDE => Some(PROVIDER_CLAUDE),
        PROVIDER_OPENAI => Some(PROVIDER_OPENAI),
        _ => None,
    }
}

fn auth_session_root(state: &AppState, purpose: &str, session_id: &str) -> PathBuf {
    PathBuf::from(&state.config.data_dir)
        .join("linked-auth")
        .join(purpose)
        .join(session_id)
}

fn trim_token(text: &str) -> &str {
    text.trim_matches(|c: char| {
        c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '.' | ')' | '(' | '[' | ']')
    })
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&nc) = chars.peek() {
                    chars.next();
                    if nc.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn extract_first_url(text: &str) -> Option<String> {
    let text = strip_ansi(text);
    text.split_whitespace().find_map(|token| {
        let token = trim_token(token);
        if token.starts_with("https://") || token.starts_with("http://") {
            Some(token.to_string())
        } else {
            None
        }
    })
}

fn extract_device_code(text: &str) -> Option<String> {
    let text = strip_ansi(text);
    text.split_whitespace().find_map(|token| {
        let token = trim_token(token);
        let valid = token.contains('-')
            && token.len() >= 7
            && token
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-');
        valid.then(|| token.to_string())
    })
}

// --- PKCE helpers ---

fn generate_pkce_verifier() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::thread_rng().gen();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash)
}

fn build_claude_auth_url(challenge: &str, verifier: &str) -> String {
    let scope = CLAUDE_SCOPES
        .iter()
        .map(|s| urlencoding::encode(s).into_owned())
        .collect::<Vec<_>>()
        .join("+");
    let redirect = urlencoding::encode(CLAUDE_REDIRECT_URI);
    format!(
        "{CLAUDE_AUTH_ENDPOINT}?code=true\
         &client_id={CLAUDE_CLIENT_ID}\
         &response_type=code\
         &redirect_uri={redirect}\
         &scope={scope}\
         &code_challenge={challenge}\
         &code_challenge_method=S256\
         &state={verifier}"
    )
}

/// Returns (code, optional_state) from user input.
/// Handles: full URL with ?code=, "code#state" format, or raw code.
fn extract_oauth_code(input: &str) -> Option<(String, Option<String>)> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    // URL: extract code= param
    if input.starts_with("http://") || input.starts_with("https://") {
        if let Some(query) = input.split('?').nth(1) {
            for param in query.split('&') {
                if let Some(code) = param.strip_prefix("code=") {
                    let code = code.split('&').next().unwrap_or(code);
                    if !code.is_empty() {
                        return Some((code.to_string(), None));
                    }
                }
            }
        }
        return None;
    }
    // "code#state" format from console.anthropic.com callback page
    if input.contains('#') {
        let mut parts = input.splitn(2, '#');
        let code = parts.next().unwrap_or("").to_string();
        let state = parts.next().map(|s| s.to_string());
        if !code.is_empty() {
            return Some((code, state));
        }
    }
    Some((input.to_string(), None))
}

// --- Session helpers ---

async fn patch_connect_session(
    state: &AppState,
    session_id: &str,
    mutator: impl FnOnce(&mut LinkedCredentialConnectSession),
) {
    let mut sessions = state.linked_credential_sessions.lock().await;
    if let Some(session) = sessions.get_mut(session_id) {
        mutator(session);
        session.updated_at = Utc::now().to_rfc3339();
    }
}

async fn snapshot_connect_session(
    state: &AppState,
    session_id: &str,
) -> Option<LinkedCredentialConnectSession> {
    let sessions = state.linked_credential_sessions.lock().await;
    sessions.get(session_id).cloned()
}

fn connect_command(provider: &str, temp_home: &str) -> Command {
    let mut cmd = match provider {
        PROVIDER_OPENAI => {
            let mut cmd = Command::new("codex");
            cmd.args(["login", "--device-auth"]);
            cmd.env("CODEX_HOME", format!("{temp_home}/.codex"));
            cmd
        },
        _ => unreachable!(),
    };
    cmd.env("HOME", temp_home)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    cmd
}

async fn persist_validated_credential(
    state: &AppState,
    user_id: i64,
    provider: &str,
    temp_home: &str,
    validation: &LinkedCredentialValidation,
) -> anyhow::Result<()> {
    let bundle = capture_bundle(provider, &PathBuf::from(temp_home))?;
    let now = Utc::now().to_rfc3339();
    state.db.upsert_user_linked_credential(
        user_id,
        provider,
        &validation.auth_kind,
        &validation.account_email,
        &validation.account_label,
        "connected",
        &validation.expires_at,
        &now,
        &now,
        "",
        &bundle,
    )?;
    Ok(())
}

async fn revalidate_stored_credential(
    state: &AppState,
    user_id: i64,
    provider: &str,
) -> anyhow::Result<()> {
    let Some(secret) = state.db.get_user_linked_credential(user_id, provider)? else {
        return Ok(());
    };
    let session_id = crate::auth::generate_token();
    let temp_home = auth_session_root(state, "validate", &session_id);
    tokio::fs::create_dir_all(&temp_home).await?;
    restore_bundle(&secret.bundle, &temp_home)?;
    let validation = validate_home(provider, &temp_home).await?;
    let now = Utc::now().to_rfc3339();
    if validation.ok {
        let refreshed_bundle = capture_bundle(provider, &temp_home)?;
        state.db.update_user_linked_credential_state(
            user_id,
            provider,
            &validation.auth_kind,
            &validation.account_email,
            &validation.account_label,
            "connected",
            &validation.expires_at,
            &now,
            "",
            Some(&refreshed_bundle),
        )?;
    } else {
        state.db.update_user_linked_credential_state(
            user_id,
            provider,
            if validation.auth_kind.is_empty() {
                &secret.entry.auth_kind
            } else {
                &validation.auth_kind
            },
            if validation.account_email.is_empty() {
                &secret.entry.account_email
            } else {
                &validation.account_email
            },
            if validation.account_label.is_empty() {
                &secret.entry.account_label
            } else {
                &validation.account_label
            },
            "expired",
            &validation.expires_at,
            &now,
            &validation.last_error,
            None,
        )?;
    }
    let _ = tokio::fs::remove_dir_all(&temp_home).await;
    Ok(())
}

// --- OpenAI CLI connect session (unchanged) ---

async fn run_connect_session(
    state: Arc<AppState>,
    session_id: String,
    user_id: i64,
    provider: String,
    temp_home: String,
) {
    let spawn = connect_command(&provider, &temp_home).spawn();
    let mut child = match spawn {
        Ok(child) => child,
        Err(err) => {
            patch_connect_session(&state, &session_id, |session| {
                session.status = "failed".to_string();
                session.error = format!("failed to start {provider} login: {err}");
            })
            .await;
            return;
        },
    };

    if let Some(stdin) = child.stdin.take() {
        state
            .linked_credential_stdins
            .lock()
            .await
            .insert(session_id.clone(), stdin);
    }

    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            patch_connect_session(&state, &session_id, |session| {
                session.status = "failed".to_string();
                session.error = format!("{provider} login missing stdout");
            })
            .await;
            return;
        },
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            patch_connect_session(&state, &session_id, |session| {
                session.status = "failed".to_string();
                session.error = format!("{provider} login missing stderr");
            })
            .await;
            return;
        },
    };

    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut stderr_reader = BufReader::new(stderr).lines();
    let mut stdout_done = false;
    let mut stderr_done = false;
    let mut transcript = Vec::new();

    while !stdout_done || !stderr_done {
        tokio::select! {
            line = stdout_reader.next_line(), if !stdout_done => {
                match line {
                    Ok(Some(line)) => {
                        let line = line.trim().to_string();
                        if !line.is_empty() {
                            transcript.push(line.clone());
                            patch_connect_session(&state, &session_id, |session| {
                                if session.auth_url.is_empty() {
                                    if let Some(url) = extract_first_url(&line) {
                                        session.auth_url = url;
                                    }
                                }
                                if session.device_code.is_empty() {
                                    if let Some(code) = extract_device_code(&line) {
                                        session.device_code = code;
                                    }
                                }
                                session.message = line.clone();
                            }).await;
                        }
                    }
                    Ok(None) => stdout_done = true,
                    Err(err) => {
                        transcript.push(format!("stdout read error: {err}"));
                        stdout_done = true;
                    }
                }
            }
            line = stderr_reader.next_line(), if !stderr_done => {
                match line {
                    Ok(Some(line)) => {
                        let line = line.trim().to_string();
                        if !line.is_empty() {
                            transcript.push(line.clone());
                            patch_connect_session(&state, &session_id, |session| {
                                if session.auth_url.is_empty() {
                                    if let Some(url) = extract_first_url(&line) {
                                        session.auth_url = url;
                                    }
                                }
                                if session.device_code.is_empty() {
                                    if let Some(code) = extract_device_code(&line) {
                                        session.device_code = code;
                                    }
                                }
                                if session.message.is_empty() {
                                    session.message = line.clone();
                                }
                            }).await;
                        }
                    }
                    Ok(None) => stderr_done = true,
                    Err(err) => {
                        transcript.push(format!("stderr read error: {err}"));
                        stderr_done = true;
                    }
                }
            }
        }
    }

    let wait_result = child.wait().await;
    let transcript_text = transcript.join("\n");
    match wait_result {
        Ok(status) if status.success() => {
            match validate_home(&provider, &PathBuf::from(&temp_home)).await {
                Ok(validation) if validation.ok => {
                    match persist_validated_credential(
                        &state,
                        user_id,
                        &provider,
                        &temp_home,
                        &validation,
                    )
                    .await
                    {
                        Ok(()) => {
                            patch_connect_session(&state, &session_id, |session| {
                                session.status = "connected".to_string();
                                session.message =
                                    "Credential linked and ready for agent runs".to_string();
                                session.error.clear();
                            })
                            .await;
                        },
                        Err(err) => {
                            patch_connect_session(&state, &session_id, |session| {
                                session.status = "failed".to_string();
                                session.error = format!("failed to save linked credential: {err}");
                            })
                            .await;
                        },
                    }
                },
                Ok(validation) => {
                    patch_connect_session(&state, &session_id, |session| {
                        session.status = "failed".to_string();
                        session.error = if validation.last_error.is_empty() {
                            "login completed but no valid credential was stored".to_string()
                        } else {
                            validation.last_error
                        };
                    })
                    .await;
                },
                Err(err) => {
                    patch_connect_session(&state, &session_id, |session| {
                        session.status = "failed".to_string();
                        session.error = format!("credential validation failed: {err}");
                    })
                    .await;
                },
            }
        },
        Ok(status) => {
            patch_connect_session(&state, &session_id, |session| {
                session.status = "failed".to_string();
                session.error = if transcript_text.is_empty() {
                    format!(
                        "{provider} login exited with {}",
                        status.code().unwrap_or(-1)
                    )
                } else {
                    transcript_text.clone()
                };
            })
            .await;
        },
        Err(err) => {
            patch_connect_session(&state, &session_id, |session| {
                session.status = "failed".to_string();
                session.error = format!("failed waiting for {provider} login: {err}");
            })
            .await;
        },
    }

    state
        .linked_credential_stdins
        .lock()
        .await
        .remove(&session_id);
    let _ = tokio::fs::remove_dir_all(&temp_home).await;
}

// --- Claude PKCE token exchange ---

async fn claude_pkce_exchange(
    state: &AppState,
    session_id: &str,
    user_id: i64,
    code_verifier: &str,
    auth_code: &str,
    oauth_state: Option<&str>,
) {
    tracing::info!("claude PKCE exchange starting for session={session_id}");
    let mut body = json!({
        "grant_type": "authorization_code",
        "code": auth_code,
        "redirect_uri": CLAUDE_REDIRECT_URI,
        "client_id": CLAUDE_CLIENT_ID,
        "code_verifier": code_verifier,
    });
    if let Some(st) = oauth_state {
        body["state"] = Value::String(st.to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let resp = match client.post(CLAUDE_TOKEN_ENDPOINT).json(&body).send().await {
        Ok(r) => r,
        Err(err) => {
            patch_connect_session(state, session_id, |s| {
                s.status = "failed".to_string();
                s.error = format!("Token exchange request failed: {err}");
            })
            .await;
            return;
        },
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!("Claude token exchange failed: {status} {body}");
        patch_connect_session(state, session_id, |s| {
            s.status = "failed".to_string();
            s.error = format!("Token exchange failed ({status}): {body}");
        })
        .await;
        return;
    }

    let token_data: Value = match resp.json().await {
        Ok(v) => v,
        Err(err) => {
            patch_connect_session(state, session_id, |s| {
                s.status = "failed".to_string();
                s.error = format!("Failed to parse token response: {err}");
            })
            .await;
            return;
        },
    };

    let access_token = token_data["access_token"].as_str().unwrap_or("");
    let refresh_token = token_data["refresh_token"].as_str().unwrap_or("");
    let expires_in = token_data["expires_in"].as_i64().unwrap_or(3600);
    let account_email = token_data["account"]
        .as_object()
        .and_then(|a| a.get("email_address").or(a.get("email")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let account_label = token_data["organization"]
        .as_object()
        .and_then(|o| o.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if access_token.is_empty() {
        patch_connect_session(state, session_id, |s| {
            s.status = "failed".to_string();
            s.error = "No access token in response".to_string();
        })
        .await;
        return;
    }

    let scopes: Vec<String> = token_data["scope"]
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    let scopes = if scopes.is_empty() {
        CLAUDE_SCOPES.iter().map(|s| s.to_string()).collect()
    } else {
        scopes
    };

    let expires_at_ms = Utc::now().timestamp_millis() + expires_in * 1000;

    let credentials = json!({
        "claudeAiOauth": {
            "accessToken": access_token,
            "refreshToken": refresh_token,
            "expiresAt": expires_at_ms,
            "scopes": scopes,
        }
    });

    let temp_home = auth_session_root(state, "connect", session_id);
    let creds_dir = temp_home.join(".claude");
    if let Err(err) = tokio::fs::create_dir_all(&creds_dir).await {
        patch_connect_session(state, session_id, |s| {
            s.status = "failed".to_string();
            s.error = format!("Failed to create credentials dir: {err}");
        })
        .await;
        return;
    }

    let creds_path = creds_dir.join(".credentials.json");
    if let Err(err) = tokio::fs::write(
        &creds_path,
        serde_json::to_string_pretty(&credentials).unwrap(),
    )
    .await
    {
        patch_connect_session(state, session_id, |s| {
            s.status = "failed".to_string();
            s.error = format!("Failed to write credentials: {err}");
        })
        .await;
        return;
    }

    let temp_home_str = temp_home.to_string_lossy().to_string();
    match validate_home(PROVIDER_CLAUDE, &temp_home).await {
        Ok(validation) if validation.ok => {
            match persist_validated_credential(
                state,
                user_id,
                PROVIDER_CLAUDE,
                &temp_home_str,
                &validation,
            )
            .await
            {
                Ok(()) => {
                    patch_connect_session(state, session_id, |s| {
                        s.status = "connected".to_string();
                        s.message = "Claude credential linked successfully".to_string();
                        s.error.clear();
                    })
                    .await;
                },
                Err(err) => {
                    patch_connect_session(state, session_id, |s| {
                        s.status = "failed".to_string();
                        s.error = format!("Failed to save credential: {err}");
                    })
                    .await;
                },
            }
        },
        Ok(validation) => {
            // Validation failed but token exchange worked — still persist with what we have
            tracing::warn!(
                "Claude PKCE: token exchange ok but validation failed: {}",
                validation.last_error
            );
            // Try persisting anyway since we have valid tokens
            let fallback_validation = LinkedCredentialValidation {
                ok: true,
                auth_kind: "claude_code_session".to_string(),
                account_email: account_email.clone(),
                account_label: account_label.clone(),
                expires_at: chrono::DateTime::from_timestamp_millis(expires_at_ms)
                    .map(|ts| ts.to_rfc3339())
                    .unwrap_or_default(),
                last_error: String::new(),
            };
            match persist_validated_credential(
                state,
                user_id,
                PROVIDER_CLAUDE,
                &temp_home_str,
                &fallback_validation,
            )
            .await
            {
                Ok(()) => {
                    patch_connect_session(state, session_id, |s| {
                        s.status = "connected".to_string();
                        s.message = "Claude credential linked (validation pending)".to_string();
                        s.error.clear();
                    })
                    .await;
                },
                Err(err) => {
                    patch_connect_session(state, session_id, |s| {
                        s.status = "failed".to_string();
                        s.error = format!("Failed to save credential: {err}");
                    })
                    .await;
                },
            }
        },
        Err(err) => {
            tracing::warn!("Claude PKCE: validation error: {err}");
            // Still try to persist — we have valid tokens from the exchange
            let fallback_validation = LinkedCredentialValidation {
                ok: true,
                auth_kind: "claude_code_session".to_string(),
                account_email: account_email.clone(),
                account_label: account_label.clone(),
                expires_at: chrono::DateTime::from_timestamp_millis(expires_at_ms)
                    .map(|ts| ts.to_rfc3339())
                    .unwrap_or_default(),
                last_error: String::new(),
            };
            match persist_validated_credential(
                state,
                user_id,
                PROVIDER_CLAUDE,
                &temp_home_str,
                &fallback_validation,
            )
            .await
            {
                Ok(()) => {
                    patch_connect_session(state, session_id, |s| {
                        s.status = "connected".to_string();
                        s.message = "Claude credential linked".to_string();
                        s.error.clear();
                    })
                    .await;
                },
                Err(err) => {
                    patch_connect_session(state, session_id, |s| {
                        s.status = "failed".to_string();
                        s.error = format!("Failed to save credential: {err}");
                    })
                    .await;
                },
            }
        },
    }

    let _ = tokio::fs::remove_dir_all(&temp_home).await;
}

// --- Maintenance ---

pub(crate) fn spawn_linked_credential_maintenance(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15 * 60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let entries = match state.db.list_all_linked_credentials() {
                Ok(entries) => entries,
                Err(err) => {
                    tracing::warn!("linked credential sweep failed to list credentials: {err}");
                    continue;
                },
            };
            for entry in entries {
                if entry.status == "connected"
                    && should_revalidate(&entry.last_validated_at, &entry.expires_at)
                {
                    if let Err(err) =
                        revalidate_stored_credential(&state, entry.user_id, &entry.provider).await
                    {
                        tracing::warn!(
                            user_id = entry.user_id,
                            provider = entry.provider.as_str(),
                            "linked credential revalidation failed: {err}"
                        );
                    }
                }
            }
            let cutoff = Utc::now() - ChronoDuration::hours(1);
            let mut sessions = state.linked_credential_sessions.lock().await;
            sessions.retain(|_, session| {
                chrono::DateTime::parse_from_rfc3339(&session.updated_at)
                    .map(|ts| ts.with_timezone(&Utc) >= cutoff)
                    .unwrap_or(false)
            });
        }
    });
}

// --- Route handlers ---

pub(crate) async fn list_user_linked_credentials(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
) -> Result<Json<Value>, StatusCode> {
    let credentials = state
        .db
        .list_user_linked_credentials(user.id)
        .map_err(internal)?;
    Ok(Json(json!({ "credentials": credentials })))
}

pub(crate) async fn start_linked_credential_connect(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Path(provider): Path<String>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let provider = normalize_provider(&provider).ok_or(StatusCode::NOT_FOUND)?;
    let session_id = crate::auth::generate_token();
    let temp_home = auth_session_root(state.as_ref(), "connect", &session_id);
    tokio::fs::create_dir_all(&temp_home)
        .await
        .map_err(internal)?;
    let now = Utc::now().to_rfc3339();

    if provider == PROVIDER_CLAUDE {
        let code_verifier = generate_pkce_verifier();
        let challenge = pkce_challenge(&code_verifier);
        let auth_url = build_claude_auth_url(&challenge, &code_verifier);

        let session = LinkedCredentialConnectSession {
            id: session_id.clone(),
            provider: provider.to_string(),
            status: "pending".to_string(),
            auth_url,
            device_code: String::new(),
            message: "Open the link to authorize your Claude account".to_string(),
            error: String::new(),
            created_at: now.clone(),
            updated_at: now,
            user_id: user.id,
            code_verifier,
        };

        state
            .linked_credential_sessions
            .lock()
            .await
            .insert(session_id, session.clone());

        return Ok((StatusCode::ACCEPTED, Json(json!(session))));
    }

    // OpenAI: spawn CLI
    {
        let mut sessions = state.linked_credential_sessions.lock().await;
        sessions.insert(
            session_id.clone(),
            LinkedCredentialConnectSession {
                id: session_id.clone(),
                provider: provider.to_string(),
                status: "pending".to_string(),
                auth_url: String::new(),
                device_code: String::new(),
                message: "Waiting for provider login instructions".to_string(),
                error: String::new(),
                created_at: now.clone(),
                updated_at: now,
                user_id: user.id,
                code_verifier: String::new(),
            },
        );
    }

    tokio::spawn(run_connect_session(
        Arc::clone(&state),
        session_id.clone(),
        user.id,
        provider.to_string(),
        temp_home.to_string_lossy().to_string(),
    ));

    for _ in 0..30 {
        if let Some(session) = snapshot_connect_session(&state, &session_id).await {
            if !session.auth_url.is_empty() || session.status != "pending" {
                return Ok((StatusCode::ACCEPTED, Json(json!(session))));
            }
        }
        sleep(Duration::from_millis(100)).await;
    }

    let session = snapshot_connect_session(&state, &session_id)
        .await
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((StatusCode::ACCEPTED, Json(json!(session))))
}

pub(crate) async fn get_linked_credential_connect_session(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let session = snapshot_connect_session(&state, &id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    if session.user_id != user.id {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(json!(session)))
}

pub(crate) async fn submit_credential_connect_code(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<Value>, StatusCode> {
    let session = snapshot_connect_session(&state, &id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    if session.user_id != user.id {
        return Err(StatusCode::NOT_FOUND);
    }
    let code = body["code"].as_str().unwrap_or("").trim().to_string();
    if code.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    if session.provider == PROVIDER_CLAUDE {
        let (auth_code, state_from_input) =
            extract_oauth_code(&code).ok_or(StatusCode::BAD_REQUEST)?;
        let code_verifier = {
            let sessions = state.linked_credential_sessions.lock().await;
            sessions
                .get(&id)
                .map(|s| s.code_verifier.clone())
                .unwrap_or_default()
        };
        if code_verifier.is_empty() {
            return Err(StatusCode::GONE);
        }

        patch_connect_session(&state, &id, |s| {
            s.message = "Exchanging authorization code...".to_string();
        })
        .await;

        let state_clone = Arc::clone(&state);
        let id_clone = id.clone();
        tokio::spawn(async move {
            claude_pkce_exchange(
                &state_clone,
                &id_clone,
                user.id,
                &code_verifier,
                &auth_code,
                state_from_input.as_deref(),
            )
            .await;
        });

        return Ok(Json(json!({ "ok": true })));
    }

    // OpenAI: write to CLI stdin
    let mut stdins = state.linked_credential_stdins.lock().await;
    if let Some(stdin) = stdins.get_mut(&id) {
        let line = format!("{code}\n");
        stdin.write_all(line.as_bytes()).await.map_err(internal)?;
        stdin.flush().await.map_err(internal)?;
    } else {
        return Err(StatusCode::GONE);
    }
    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn delete_user_linked_credential(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Path(provider): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let provider = normalize_provider(&provider).ok_or(StatusCode::NOT_FOUND)?;
    state
        .db
        .delete_user_linked_credential(user.id, provider)
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::{extract_device_code, extract_first_url, extract_oauth_code, pkce_challenge};

    #[test]
    fn extracts_provider_urls() {
        assert_eq!(
            extract_first_url("visit: https://claude.ai/oauth/authorize?x=1"),
            Some("https://claude.ai/oauth/authorize?x=1".to_string())
        );
    }

    #[test]
    fn extracts_url_with_ansi_codes() {
        assert_eq!(
            extract_first_url("   \x1b[94mhttps://auth.openai.com/codex/device\x1b[0m"),
            Some("https://auth.openai.com/codex/device".to_string())
        );
    }

    #[test]
    fn extracts_device_code_tokens() {
        assert_eq!(
            extract_device_code("Enter this one-time code O078-ZFUYD"),
            Some("O078-ZFUYD".to_string())
        );
    }

    #[test]
    fn extracts_device_code_with_ansi_codes() {
        assert_eq!(
            extract_device_code("   \x1b[94mRO4V-NTCJL\x1b[0m"),
            Some("RO4V-NTCJL".to_string())
        );
    }

    #[test]
    fn extracts_oauth_code_from_url() {
        assert_eq!(
            extract_oauth_code(
                "https://console.anthropic.com/oauth/code/callback?code=abc123&state=xyz"
            ),
            Some(("abc123".to_string(), None))
        );
        assert_eq!(
            extract_oauth_code("http://localhost:44603/?code=def456&state=xyz"),
            Some(("def456".to_string(), None))
        );
        assert_eq!(
            extract_oauth_code("raw-code-value"),
            Some(("raw-code-value".to_string(), None))
        );
        assert_eq!(extract_oauth_code(""), None);
    }

    #[test]
    fn extracts_code_and_state_from_hash_format() {
        assert_eq!(
            extract_oauth_code("i3wyjs56jkt#2996ef2f4b976e99ef31"),
            Some((
                "i3wyjs56jkt".to_string(),
                Some("2996ef2f4b976e99ef31".to_string())
            ))
        );
        assert_eq!(
            extract_oauth_code("plaincode"),
            Some(("plaincode".to_string(), None))
        );
    }

    #[test]
    fn pkce_challenge_is_base64url() {
        let verifier = "test-verifier-string";
        let challenge = pkce_challenge(verifier);
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
        assert!(!challenge.contains('='));
        assert!(!challenge.is_empty());
    }
}
