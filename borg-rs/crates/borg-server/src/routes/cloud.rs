use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use borg_core::db::Db;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{internal, require_project_access};
use crate::{ingestion::IngestionQueue, AppState};

#[derive(Deserialize)]
pub(crate) struct CloudAuthQuery {
    pub project_id: i64,
}

#[derive(Deserialize)]
pub(crate) struct CloudCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CloudBrowseQuery {
    pub folder_id: Option<String>,
    pub cursor: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CloudImportBody {
    pub files: Vec<CloudImportFile>,
    #[serde(default)]
    pub privileged: bool,
}

#[derive(Deserialize)]
pub(crate) struct CloudImportFile {
    pub id: String,
    pub name: String,
    pub size: Option<i64>,
}

pub(crate) fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'?' | b'&' | b'#' | b' ' | b'%' | b'+' => {
                out.push_str(&format!("%{b:02X}"));
            },
            _ => out.push(b as char),
        }
    }
    out
}

pub(crate) fn percent_encode_allow_slash(s: &str, allow_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            },
            b'/' if allow_slash => out.push('/'),
            _ => {
                out.push('%');
                out.push(
                    char::from_digit((b >> 4) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
                out.push(
                    char::from_digit((b & 0xf) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
            },
        }
    }
    out
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let combined = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((combined >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((combined >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((combined >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(combined & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn cloud_callback_url(config: &borg_core::config::Config, provider: &str) -> String {
    let base = config.get_base_url();
    format!("{base}/api/cloud/{provider}/callback")
}

fn is_privileged_upload_allowed(state: &AppState, project_id: i64) -> bool {
    state.db.is_session_privileged(project_id).unwrap_or(false)
}

fn guess_mime(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "doc" => "application/msword",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xls" => "application/vnd.ms-excel",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "xml" => "application/xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
    .to_string()
}

pub(crate) async fn cloud_auth_init(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Query(q): Query<CloudAuthQuery>,
) -> Result<axum::response::Response, StatusCode> {
    let public_url = state
        .db
        .get_config("public_url")
        .map_err(internal)?
        .unwrap_or_default();
    if public_url.trim().is_empty() {
        return Ok(axum::response::Redirect::temporary(
            "/#/projects?cloud_error=missing_public_url",
        )
        .into_response());
    }

    let client_id = match provider.as_str() {
        "dropbox" => state.db.get_config("dropbox_client_id").map_err(internal)?,
        "google_drive" => state.db.get_config("google_client_id").map_err(internal)?,
        "onedrive" => state.db.get_config("ms_client_id").map_err(internal)?,
        _ => return Err(StatusCode::NOT_FOUND),
    };
    let client_id = client_id.unwrap_or_else(|| {
        tracing::warn!("cloud: no client_id configured for {provider}");
        String::new()
    });
    if client_id.trim().is_empty() {
        return Ok(axum::response::Redirect::temporary(&format!(
            "/#/projects?cloud_error=missing_credentials&provider={provider}"
        ))
        .into_response());
    }

    let state_json =
        serde_json::json!({ "project_id": q.project_id, "provider": provider }).to_string();
    let encoded_state = base64_encode(state_json.as_bytes());
    let redirect_uri = cloud_callback_url(&state.config, &provider);

    let auth_url = match provider.as_str() {
        "dropbox" => format!(
            "https://www.dropbox.com/oauth2/authorize?client_id={client_id}\
             &redirect_uri={}&response_type=code&token_access_type=offline&state={encoded_state}",
            percent_encode(&redirect_uri)
        ),
        "google_drive" => format!(
            "https://accounts.google.com/o/oauth2/v2/auth?client_id={client_id}\
             &redirect_uri={}&response_type=code\
             &scope=https://www.googleapis.com/auth/drive.readonly\
             &access_type=offline&prompt=consent&state={encoded_state}",
            percent_encode(&redirect_uri)
        ),
        "onedrive" => format!(
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize?client_id={client_id}\
             &redirect_uri={}&response_type=code\
             &scope=files.read%20offline_access&state={encoded_state}",
            percent_encode(&redirect_uri)
        ),
        _ => return Err(StatusCode::NOT_FOUND),
    };

    Ok(axum::response::Redirect::temporary(&auth_url).into_response())
}

pub(crate) async fn cloud_auth_callback(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Query(q): Query<CloudCallbackQuery>,
) -> Result<axum::response::Response, StatusCode> {
    use axum::response::IntoResponse;
    if let Some(err) = q.error {
        tracing::warn!("cloud OAuth error for {provider}: {err}");
        return Ok(axum::response::Redirect::temporary(&format!(
            "/#/projects?cloud_error=access_denied&provider={provider}"
        ))
        .into_response());
    }
    let code = q.code.ok_or(StatusCode::BAD_REQUEST)?;
    let state_raw = q.state.ok_or(StatusCode::BAD_REQUEST)?;
    let state_bytes =
        super::utils::base64_decode(&state_raw).map_err(|_| StatusCode::BAD_REQUEST)?;
    let state_val: serde_json::Value =
        serde_json::from_slice(&state_bytes).map_err(|_| StatusCode::BAD_REQUEST)?;
    let project_id = state_val["project_id"]
        .as_i64()
        .ok_or(StatusCode::BAD_REQUEST)?;

    let client_id = match provider.as_str() {
        "dropbox" => state.db.get_config("dropbox_client_id").map_err(internal)?,
        "google_drive" => state.db.get_config("google_client_id").map_err(internal)?,
        "onedrive" => state.db.get_config("ms_client_id").map_err(internal)?,
        _ => return Err(StatusCode::NOT_FOUND),
    }
    .ok_or(StatusCode::BAD_REQUEST)?;
    let client_secret = match provider.as_str() {
        "dropbox" => state
            .db
            .get_config("dropbox_client_secret")
            .map_err(internal)?,
        "google_drive" => state
            .db
            .get_config("google_client_secret")
            .map_err(internal)?,
        "onedrive" => state.db.get_config("ms_client_secret").map_err(internal)?,
        _ => return Err(StatusCode::NOT_FOUND),
    }
    .ok_or(StatusCode::BAD_REQUEST)?;

    let redirect_uri = cloud_callback_url(&state.config, &provider);
    let token_url = match provider.as_str() {
        "dropbox" => "https://api.dropboxapi.com/oauth2/token",
        "google_drive" => "https://oauth2.googleapis.com/token",
        "onedrive" => "https://login.microsoftonline.com/common/oauth2/v2.0/token",
        _ => return Err(StatusCode::NOT_FOUND),
    };

    let client = reqwest::Client::new();
    let params = [
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &redirect_uri),
        ("client_id", &client_id),
        ("client_secret", &client_secret),
    ];
    let resp = client
        .post(token_url)
        .form(&params)
        .send()
        .await
        .map_err(internal)?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        tracing::error!("cloud token exchange failed for {provider}: {body}");
        return Ok(axum::response::Redirect::temporary(&format!(
            "/#/projects?cloud_error=token_exchange&provider={provider}"
        ))
        .into_response());
    }
    let token_json: serde_json::Value = resp.json().await.map_err(internal)?;
    let access_token = token_json["access_token"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let refresh_token = token_json["refresh_token"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let expires_in = token_json["expires_in"].as_i64().unwrap_or(3600);
    let expiry = (chrono::Utc::now() + chrono::Duration::seconds(expires_in)).to_rfc3339();

    let (account_email, account_id) =
        fetch_cloud_account_info(&client, &provider, &access_token).await;

    let existing = state
        .db
        .list_cloud_connections(project_id)
        .map_err(internal)?;
    if let Some(conn) = existing
        .iter()
        .find(|c| c.provider == provider && c.account_id == account_id)
    {
        state
            .db
            .update_cloud_connection_tokens(conn.id, &access_token, &refresh_token, &expiry)
            .map_err(internal)?;
    } else {
        state
            .db
            .insert_cloud_connection(
                project_id,
                &provider,
                &access_token,
                &refresh_token,
                &expiry,
                &account_email,
                &account_id,
            )
            .map_err(internal)?;
    }

    Ok(axum::response::Redirect::temporary(&format!(
        "/#/projects?cloud_connected={provider}&project_id={project_id}"
    ))
    .into_response())
}

async fn fetch_cloud_account_info(
    client: &reqwest::Client,
    provider: &str,
    access_token: &str,
) -> (String, String) {
    match provider {
        "dropbox" => {
            let resp = client
                .post("https://api.dropboxapi.com/2/users/get_current_account")
                .header("Authorization", format!("Bearer {access_token}"))
                .header("Content-Type", "")
                .body("")
                .send()
                .await;
            if let Ok(r) = resp {
                if let Ok(v) = r.json::<serde_json::Value>().await {
                    let email = v["email"].as_str().unwrap_or("").to_string();
                    let id = v["account_id"].as_str().unwrap_or("").to_string();
                    return (email, id);
                }
            }
        },
        "google_drive" => {
            let resp = client
                .get("https://www.googleapis.com/oauth2/v2/userinfo")
                .bearer_auth(access_token)
                .send()
                .await;
            if let Ok(r) = resp {
                if let Ok(v) = r.json::<serde_json::Value>().await {
                    let email = v["email"].as_str().unwrap_or("").to_string();
                    let id = v["id"].as_str().unwrap_or("").to_string();
                    return (email, id);
                }
            }
        },
        "onedrive" => {
            let resp = client
                .get("https://graph.microsoft.com/v1.0/me")
                .bearer_auth(access_token)
                .send()
                .await;
            if let Ok(r) = resp {
                if let Ok(v) = r.json::<serde_json::Value>().await {
                    let email = v["mail"]
                        .as_str()
                        .or_else(|| v["userPrincipalName"].as_str())
                        .unwrap_or("")
                        .to_string();
                    let id = v["id"].as_str().unwrap_or("").to_string();
                    return (email, id);
                }
            }
        },
        _ => {},
    }
    (String::new(), String::new())
}

async fn refresh_cloud_token_if_needed(
    db: &Db,
    conn: &borg_core::db::CloudConnection,
    config: &borg_core::config::Config,
) -> String {
    let expires_soon = chrono::DateTime::parse_from_rfc3339(&conn.token_expiry)
        .map(|exp| exp.signed_duration_since(chrono::Utc::now()).num_seconds() < 300)
        .unwrap_or(true);
    if !expires_soon {
        return conn.access_token.clone();
    }
    if conn.refresh_token.is_empty() {
        return conn.access_token.clone();
    }
    let (client_id_key, client_secret_key, token_url) = match conn.provider.as_str() {
        "dropbox" => (
            "dropbox_client_id",
            "dropbox_client_secret",
            "https://api.dropboxapi.com/oauth2/token",
        ),
        "google_drive" => (
            "google_client_id",
            "google_client_secret",
            "https://oauth2.googleapis.com/token",
        ),
        "onedrive" => (
            "ms_client_id",
            "ms_client_secret",
            "https://login.microsoftonline.com/common/oauth2/v2.0/token",
        ),
        _ => return conn.access_token.clone(),
    };
    let client_id = db
        .get_config(client_id_key)
        .ok()
        .flatten()
        .unwrap_or_default();
    let client_secret = db
        .get_config(client_secret_key)
        .ok()
        .flatten()
        .unwrap_or_default();
    let _ = config;
    let client = reqwest::Client::new();
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", &conn.refresh_token),
        ("client_id", &client_id),
        ("client_secret", &client_secret),
    ];
    if let Ok(resp) = client.post(token_url).form(&params).send().await {
        if let Ok(v) = resp.json::<serde_json::Value>().await {
            let new_access = v["access_token"].as_str().unwrap_or("").to_string();
            if !new_access.is_empty() {
                let new_refresh = v["refresh_token"]
                    .as_str()
                    .unwrap_or(&conn.refresh_token)
                    .to_string();
                let expires_in = v["expires_in"].as_i64().unwrap_or(3600);
                let expiry =
                    (chrono::Utc::now() + chrono::Duration::seconds(expires_in)).to_rfc3339();
                let _ =
                    db.update_cloud_connection_tokens(conn.id, &new_access, &new_refresh, &expiry);
                return new_access;
            }
        }
    }
    conn.access_token.clone()
}

pub(crate) async fn list_cloud_connections(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    let conns = state.db.list_cloud_connections(id).map_err(internal)?;
    let out: Vec<Value> = conns
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "provider": c.provider,
                "account_email": c.account_email,
                "connected_at": c.created_at,
            })
        })
        .collect();
    Ok(Json(json!(out)))
}

pub(crate) async fn delete_cloud_connection(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path((id, conn_id)): Path<(i64, i64)>,
) -> Result<StatusCode, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    let conn = state
        .db
        .get_cloud_connection(conn_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if conn.project_id != id {
        return Err(StatusCode::NOT_FOUND);
    }
    state
        .db
        .delete_cloud_connection(conn_id)
        .map_err(internal)?;
    Ok(StatusCode::OK)
}

pub(crate) async fn browse_cloud_files(
    State(state): State<Arc<AppState>>,
    Path((id, conn_id)): Path<(i64, i64)>,
    Query(q): Query<CloudBrowseQuery>,
) -> Result<Json<Value>, StatusCode> {
    let conn = state
        .db
        .get_cloud_connection(conn_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if conn.project_id != id {
        return Err(StatusCode::NOT_FOUND);
    }
    let token = refresh_cloud_token_if_needed(&state.db, &conn, &state.config).await;
    let client = reqwest::Client::new();
    let result = match conn.provider.as_str() {
        "dropbox" => {
            browse_dropbox(&client, &token, q.folder_id.as_deref(), q.cursor.as_deref()).await
        },
        "google_drive" => {
            browse_google_drive(&client, &token, q.folder_id.as_deref(), q.cursor.as_deref()).await
        },
        "onedrive" => {
            browse_onedrive(&client, &token, q.folder_id.as_deref(), q.cursor.as_deref()).await
        },
        _ => return Err(StatusCode::NOT_FOUND),
    };
    result.map(Json).map_err(|e| {
        tracing::error!("cloud browse error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

async fn browse_dropbox(
    client: &reqwest::Client,
    token: &str,
    folder_path: Option<&str>,
    cursor: Option<&str>,
) -> anyhow::Result<Value> {
    let (url, body) = if let Some(cur) = cursor {
        (
            "https://api.dropboxapi.com/2/files/list_folder/continue".to_string(),
            serde_json::json!({ "cursor": cur }).to_string(),
        )
    } else {
        ("https://api.dropboxapi.com/2/files/list_folder".to_string(),
         serde_json::json!({ "path": folder_path.unwrap_or(""), "recursive": false, "limit": 200 }).to_string())
    };
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;
    let entries: Vec<Value> = resp["entries"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|e| {
            let is_folder = e[".tag"].as_str() == Some("folder");
            json!({
                "id": e["id"].as_str().unwrap_or(""),
                "name": e["name"].as_str().unwrap_or(""),
                "type": if is_folder { "folder" } else { "file" },
                "size": e["size"].as_i64().unwrap_or(0),
                "modified": e["server_modified"].as_str().unwrap_or(""),
                "path": e["path_display"].as_str().unwrap_or(""),
                "mime_type": e["media_info"]["metadata"]["mime_type"].as_str().unwrap_or(""),
            })
        })
        .collect();
    Ok(json!({
        "items": entries,
        "cursor": resp["cursor"].as_str(),
        "has_more": resp["has_more"].as_bool().unwrap_or(false),
        "folder_id": folder_path.unwrap_or(""),
    }))
}

async fn browse_google_drive(
    client: &reqwest::Client,
    token: &str,
    folder_id: Option<&str>,
    cursor: Option<&str>,
) -> anyhow::Result<Value> {
    let parent = folder_id.unwrap_or("root");
    let q = format!("'{}' in parents and trashed = false", parent);
    let mut req = client
        .get("https://www.googleapis.com/drive/v3/files")
        .bearer_auth(token)
        .query(&[
            ("q", q.as_str()),
            (
                "fields",
                "files(id,name,mimeType,size,modifiedTime,parents),nextPageToken",
            ),
            ("pageSize", "200"),
        ]);
    if let Some(page_token) = cursor {
        req = req.query(&[("pageToken", page_token)]);
    }
    let resp = req.send().await?.json::<serde_json::Value>().await?;
    let items: Vec<Value> = resp["files"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|f| {
            let mime = f["mimeType"].as_str().unwrap_or("");
            let is_folder = mime == "application/vnd.google-apps.folder";
            json!({
                "id": f["id"].as_str().unwrap_or(""),
                "name": f["name"].as_str().unwrap_or(""),
                "type": if is_folder { "folder" } else { "file" },
                "size": f["size"].as_str().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0),
                "modified": f["modifiedTime"].as_str().unwrap_or(""),
                "mime_type": mime,
            })
        })
        .collect();
    Ok(json!({
        "items": items,
        "next_page_token": resp["nextPageToken"].as_str(),
        "has_more": resp["nextPageToken"].is_string(),
        "folder_id": parent,
    }))
}

async fn browse_onedrive(
    client: &reqwest::Client,
    token: &str,
    folder_id: Option<&str>,
    cursor: Option<&str>,
) -> anyhow::Result<Value> {
    let req = if let Some(next_link) = cursor {
        client.get(next_link).bearer_auth(token)
    } else {
        let url = match folder_id {
            Some(id) => format!("https://graph.microsoft.com/v1.0/me/drive/items/{id}/children"),
            None => "https://graph.microsoft.com/v1.0/me/drive/root/children".to_string(),
        };
        client.get(&url).bearer_auth(token).query(&[
            ("$top", "200"),
            (
                "$select",
                "id,name,file,folder,size,lastModifiedDateTime,@microsoft.graph.downloadUrl",
            ),
        ])
    };
    let resp = req.send().await?.json::<serde_json::Value>().await?;
    let items: Vec<Value> = resp["value"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|f| {
            let is_folder = f["folder"].is_object();
            json!({
                "id": f["id"].as_str().unwrap_or(""),
                "name": f["name"].as_str().unwrap_or(""),
                "type": if is_folder { "folder" } else { "file" },
                "size": f["size"].as_i64().unwrap_or(0),
                "modified": f["lastModifiedDateTime"].as_str().unwrap_or(""),
                "mime_type": f["file"]["mimeType"].as_str().unwrap_or(""),
            })
        })
        .collect();
    Ok(json!({
        "items": items,
        "next_page_token": resp["@odata.nextLink"].as_str(),
        "has_more": resp["@odata.nextLink"].is_string(),
        "folder_id": folder_id.unwrap_or("root"),
    }))
}

pub(crate) async fn import_cloud_files(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path((id, conn_id)): Path<(i64, i64)>,
    Json(body): Json<CloudImportBody>,
) -> Result<Json<Value>, StatusCode> {
    let max_import_batch_files = state.config.cloud_import_max_batch_files.max(1) as usize;
    let max_project_bytes = state.config.project_max_bytes.max(1);
    if body.files.len() > max_import_batch_files {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let conn = state
        .db
        .get_cloud_connection(conn_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if conn.project_id != id {
        return Err(StatusCode::NOT_FOUND);
    }
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    if body.privileged && !is_privileged_upload_allowed(state.as_ref(), id) {
        return Err(StatusCode::FORBIDDEN);
    }

    let token = refresh_cloud_token_if_needed(&state.db, &conn, &state.config).await;
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(internal)?;
    let mut imported: Vec<Value> = Vec::new();
    let mut total_bytes = state.db.total_project_file_bytes(id).map_err(internal)?;

    for file in &body.files {
        let estimated = file.size.unwrap_or(0);
        if total_bytes + estimated > max_project_bytes {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        let bytes = match conn.provider.as_str() {
            "dropbox" => download_dropbox_file(&client, &token, &file.id).await,
            "google_drive" => download_google_file(&client, &token, &file.id).await,
            "onedrive" => download_onedrive_file(&client, &token, &file.id).await,
            _ => Err(anyhow::anyhow!("unknown provider")),
        };
        let bytes = match bytes {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("failed to download cloud file {}: {e}", file.name);
                continue;
            },
        };
        if bytes.is_empty() {
            continue;
        }
        let file_size = bytes.len() as i64;
        if total_bytes + file_size > max_project_bytes {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        let safe_name = super::projects::sanitize_upload_name(&file.name);
        let source_path = super::projects::sanitize_upload_relative_path(&file.name);
        let content_hash = super::utils::sha256_hex_bytes(&bytes);
        if state
            .db
            .find_project_file_by_hash(id, &content_hash)
            .map_err(internal)?
            .is_some()
        {
            if body.privileged {
                let _ = state.db.set_session_privileged(id);
            }
            continue;
        }
        let unique_name = format!(
            "{}_{}_cloud_{}",
            Utc::now().timestamp_millis(),
            super::utils::rand_suffix(),
            safe_name
        );
        let stored_path = state
            .file_storage
            .put_project_file(id, &unique_name, &bytes)
            .await
            .map_err(internal)?;

        let mime = guess_mime(&file.name);
        let file_id = state
            .db
            .insert_project_file(
                id,
                &safe_name,
                &source_path,
                &stored_path,
                &mime,
                file_size,
                &content_hash,
                body.privileged,
            )
            .map_err(internal)?;
        if let Err(e) = state
            .ingestion_queue
            .enqueue_project_file(id, file_id, &safe_name, &stored_path, &mime, file_size)
            .await
        {
            tracing::warn!("failed to enqueue cloud-imported file ingest: {e}");
        }
        total_bytes += file_size;
        imported.push(json!({ "id": file_id, "file_name": safe_name, "size_bytes": file_size }));

        if matches!(state.ingestion_queue.as_ref(), IngestionQueue::Disabled) {
            let db2 = state.db.clone();
            let search = state.search.clone();
            let embed_reg = Arc::clone(&state.embed_registry);
            let mime2 = mime.clone();
            let bytes2 = bytes.clone();
            let proj_id = id;
            let fname = safe_name.clone();
            let source_path2 = source_path.clone();
            let privileged = body.privileged;
            tokio::spawn(async move {
                if let Ok(text) =
                    crate::ingestion::extract_text_from_bytes(&fname, &mime2, &bytes2).await
                {
                    if !text.is_empty() {
                        let _ = db2.update_project_file_text(file_id, &text);
                        let _ = db2.fts_index_document(proj_id, 0, &source_path2, &fname, &text);
                        if let Some(search) = &search {
                            super::projects::chunk_embed_and_index(
                                search,
                                embed_reg.default_client(),
                                proj_id,
                                file_id,
                                &source_path2,
                                &fname,
                                &text,
                                privileged,
                                &mime2,
                            )
                            .await;
                        }
                    }
                }
            });
        }
    }

    Ok(Json(json!({ "imported": imported })))
}

async fn download_dropbox_file(
    client: &reqwest::Client,
    token: &str,
    path: &str,
) -> anyhow::Result<Vec<u8>> {
    let mut last_err = String::new();
    for attempt in 0..3 {
        let arg = serde_json::json!({ "path": path }).to_string();
        match client
            .post("https://content.dropboxapi.com/2/files/download")
            .header("Authorization", format!("Bearer {token}"))
            .header("Dropbox-API-Arg", &arg)
            .header("Content-Type", "")
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => return Ok(resp.bytes().await?.to_vec()),
            Ok(resp) => last_err = format!("Dropbox download failed: {}", resp.status()),
            Err(e) => last_err = e.to_string(),
        }
        if attempt < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt + 1) as u64)).await;
        }
    }
    anyhow::bail!("{last_err}")
}

async fn download_google_file(
    client: &reqwest::Client,
    token: &str,
    file_id: &str,
) -> anyhow::Result<Vec<u8>> {
    let mut last_err = String::new();
    for attempt in 0..3 {
        match client
            .get(format!(
                "https://www.googleapis.com/drive/v3/files/{file_id}"
            ))
            .bearer_auth(token)
            .query(&[("alt", "media")])
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => return Ok(resp.bytes().await?.to_vec()),
            Ok(resp) => last_err = format!("Google Drive download failed: {}", resp.status()),
            Err(e) => last_err = e.to_string(),
        }
        if attempt < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt + 1) as u64)).await;
        }
    }
    anyhow::bail!("{last_err}")
}

async fn download_onedrive_file(
    client: &reqwest::Client,
    token: &str,
    item_id: &str,
) -> anyhow::Result<Vec<u8>> {
    let mut last_err = String::new();
    for attempt in 0..3 {
        match client
            .get(format!(
                "https://graph.microsoft.com/v1.0/me/drive/items/{item_id}/content"
            ))
            .bearer_auth(token)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => return Ok(resp.bytes().await?.to_vec()),
            Ok(resp) => last_err = format!("OneDrive download failed: {}", resp.status()),
            Err(e) => last_err = e.to_string(),
        }
        if attempt < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt + 1) as u64)).await;
        }
    }
    anyhow::bail!("{last_err}")
}

#[cfg(test)]
mod percent_encode_tests {
    use super::percent_encode;

    #[test]
    fn percent_encode_safe_chars_unchanged() {
        assert_eq!(percent_encode("src/main.rs"), "src/main.rs");
        assert_eq!(
            percent_encode("refs/heads/my-branch"),
            "refs/heads/my-branch"
        );
        assert_eq!(percent_encode("abc123_.-~"), "abc123_.-~");
    }

    #[test]
    fn percent_encode_question_mark() {
        assert_eq!(percent_encode("file?raw=1"), "file%3Fraw=1");
    }

    #[test]
    fn percent_encode_ampersand() {
        assert_eq!(percent_encode("a&b"), "a%26b");
    }

    #[test]
    fn percent_encode_hash() {
        assert_eq!(percent_encode("file#section"), "file%23section");
    }

    #[test]
    fn percent_encode_space() {
        assert_eq!(percent_encode("my file.txt"), "my%20file.txt");
    }

    #[test]
    fn percent_encode_percent() {
        assert_eq!(percent_encode("50%off"), "50%25off");
    }

    #[test]
    fn percent_encode_plus() {
        assert_eq!(percent_encode("a+b"), "a%2Bb");
    }

    #[test]
    fn percent_encode_url_construction() {
        let path = "file?raw=1";
        let ref_name = "branch&extra=1";
        let url = format!(
            "repos/owner/repo/contents/{}?ref={}",
            percent_encode(path),
            percent_encode(ref_name)
        );
        assert_eq!(
            url,
            "repos/owner/repo/contents/file%3Fraw=1?ref=branch%26extra=1"
        );
    }

    #[test]
    fn percent_encode_ref_with_hash() {
        let ref_name = "sha#abc";
        let url = format!(
            "repos/owner/repo/contents/file?ref={}",
            percent_encode(ref_name)
        );
        assert_eq!(url, "repos/owner/repo/contents/file?ref=sha%23abc");
    }
}

#[cfg(test)]
mod tests {
    use super::percent_encode_allow_slash;

    #[test]
    fn percent_encode_unreserved_passthrough() {
        assert_eq!(percent_encode_allow_slash("main", false), "main");
        assert_eq!(
            percent_encode_allow_slash("feature/my-branch", true),
            "feature/my-branch"
        );
        assert_eq!(percent_encode_allow_slash("v1.0.0~3", false), "v1.0.0~3");
    }

    #[test]
    fn percent_encode_ampersand_in_ref() {
        assert_eq!(
            percent_encode_allow_slash("bad&ref=injected", false),
            "bad%26ref%3Dinjected"
        );
    }

    #[test]
    fn percent_encode_hash_and_question_mark() {
        assert_eq!(
            percent_encode_allow_slash("ref#fragment", false),
            "ref%23fragment"
        );
        assert_eq!(
            percent_encode_allow_slash("ref?foo=1", false),
            "ref%3Ffoo%3D1"
        );
    }

    #[test]
    fn percent_encode_slash_in_path_allowed() {
        assert_eq!(
            percent_encode_allow_slash("docs/spec.md", true),
            "docs/spec.md"
        );
    }

    #[test]
    fn percent_encode_slash_in_query_encoded() {
        assert_eq!(percent_encode_allow_slash("a/b", false), "a%2Fb");
    }

    #[test]
    fn percent_encode_space_and_plus() {
        assert_eq!(
            percent_encode_allow_slash("my branch", false),
            "my%20branch"
        );
        assert_eq!(percent_encode_allow_slash("a+b", false), "a%2Bb");
    }

    #[test]
    fn percent_encode_path_with_special_chars() {
        assert_eq!(
            percent_encode_allow_slash("docs/file#top", true),
            "docs/file%23top"
        );
        assert_eq!(
            percent_encode_allow_slash("docs/file?q=1", true),
            "docs/file%3Fq%3D1"
        );
    }
}
