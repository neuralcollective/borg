use std::sync::Arc;

use axum::{
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::internal;
use crate::AppState;

#[derive(Deserialize)]
pub(crate) struct UpdateKnowledgeBody {
    pub description: Option<String>,
    pub inline: Option<bool>,
    pub tags: Option<String>,
    pub category: Option<String>,
    pub jurisdiction: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ListKnowledgeQuery {
    #[serde(default = "super::projects::default_project_file_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub jurisdiction: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct TemplatesQuery {
    category: Option<String>,
    jurisdiction: Option<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct AddKnowledgeRepoBody {
    pub url: String,
    pub name: Option<String>,
}

pub(crate) fn safe_knowledge_path(
    data_dir: &str,
    workspace_id: Option<i64>,
    file_name: &str,
) -> Option<std::path::PathBuf> {
    let base = std::path::Path::new(file_name).file_name()?.to_str()?;
    let workspace_id = workspace_id?;
    let knowledge_root = std::path::Path::new(data_dir).join("knowledge");
    let scoped_dir = knowledge_root
        .join("workspaces")
        .join(workspace_id.to_string());
    let scoped = scoped_dir.join(base);
    scoped.starts_with(&scoped_dir).then_some(scoped)
}

pub(crate) async fn list_knowledge(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Query(q): Query<ListKnowledgeQuery>,
) -> Result<Json<Value>, StatusCode> {
    let (files, total) = state
        .db
        .list_knowledge_file_page_in_workspace(
            workspace.id,
            Some(&q.q),
            q.category.as_deref(),
            q.jurisdiction.as_deref(),
            q.limit,
            q.offset,
        )
        .map_err(internal)?;
    let limit = q.limit.clamp(1, 200);
    let offset = q.offset.max(0);
    Ok(Json(json!({
        "files": files,
        "total": total,
        "offset": offset,
        "limit": limit,
        "has_more": offset + (files.len() as i64) < total,
        "total_bytes": state.db.total_knowledge_file_bytes_in_workspace(workspace.id).map_err(internal)?,
    })))
}

pub(crate) async fn upload_knowledge(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    mut multipart: Multipart,
) -> Result<Json<Value>, StatusCode> {
    const MAX_KNOWLEDGE_FILE_BYTES: i64 = 50 * 1024 * 1024;
    let max_knowledge_total_bytes = state.config.knowledge_max_bytes.max(1);

    let knowledge_dir = format!(
        "{}/knowledge/workspaces/{}",
        state.config.data_dir, workspace.id
    );
    std::fs::create_dir_all(&knowledge_dir).map_err(internal)?;

    let mut file_name = String::new();
    let mut description = String::new();
    let mut inline = false;
    let mut category = String::new();
    let mut file_bytes: Vec<u8> = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    {
        match field.name() {
            Some("file") => {
                if let Some(name) = field.file_name() {
                    file_name = super::projects::sanitize_upload_name(name);
                }
                file_bytes = field
                    .bytes()
                    .await
                    .map_err(|_| StatusCode::BAD_REQUEST)?
                    .to_vec();
            },
            Some("description") => {
                description = field.text().await.unwrap_or_default();
            },
            Some("inline") => {
                let v = field.text().await.unwrap_or_default();
                inline = v == "true" || v == "1";
            },
            Some("category") => {
                category = field.text().await.unwrap_or_default();
            },
            _ => {},
        }
    }

    if file_name.is_empty() || file_bytes.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let file_size = file_bytes.len() as i64;
    if file_size > MAX_KNOWLEDGE_FILE_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }

    let total_bytes = state
        .db
        .total_knowledge_file_bytes_in_workspace(workspace.id)
        .map_err(internal)?;
    if total_bytes + file_size > max_knowledge_total_bytes {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }

    let dest = format!("{knowledge_dir}/{file_name}");
    if std::path::Path::new(&dest).exists() {
        return Err(StatusCode::CONFLICT);
    }
    std::fs::write(&dest, &file_bytes).map_err(internal)?;

    let id = state
        .db
        .insert_knowledge_file(
            workspace.id,
            &file_name,
            &description,
            file_bytes.len() as i64,
            inline,
        )
        .map_err(internal)?;
    if !category.is_empty() {
        let _ = state.db.update_knowledge_file_in_workspace(
            workspace.id,
            id,
            None,
            None,
            None,
            Some(&category),
            None,
        );
    }

    tracing::info!(
        target: "instrumentation.storage",
        message = "knowledge file uploaded",
        user_id = user.id,
        username = user.username.as_str(),
        knowledge_id = id,
        size_bytes = file_size,
        inline = inline,
        category = category.as_str(),
    );

    Ok(Json(json!({ "id": id, "file_name": file_name })))
}

pub(crate) async fn update_knowledge(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateKnowledgeBody>,
) -> Result<Json<Value>, StatusCode> {
    state
        .db
        .update_knowledge_file_in_workspace(
            workspace.id,
            id,
            body.description.as_deref(),
            body.inline,
            body.tags.as_deref(),
            body.category.as_deref(),
            body.jurisdiction.as_deref(),
        )
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn delete_knowledge(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    if let Ok(Some(file)) = state.db.get_knowledge_file_in_workspace(workspace.id, id) {
        if let Some(safe_path) =
            safe_knowledge_path(&state.config.data_dir, Some(workspace.id), &file.file_name)
        {
            let _ = std::fs::remove_file(&safe_path);
        }
    }
    state
        .db
        .delete_knowledge_file_in_workspace(workspace.id, id)
        .map_err(internal)?;
    tracing::info!(
        target: "instrumentation.storage",
        message = "knowledge file deleted",
        user_id = user.id,
        username = user.username.as_str(),
        knowledge_id = id,
    );
    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn delete_all_knowledge(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    let files = state
        .db
        .list_knowledge_files_in_workspace(workspace.id)
        .map_err(internal)?;
    for file in &files {
        if let Some(safe_path) =
            safe_knowledge_path(&state.config.data_dir, Some(workspace.id), &file.file_name)
        {
            let _ = std::fs::remove_file(&safe_path);
        }
    }
    let deleted = state
        .db
        .delete_all_knowledge_files_in_workspace(workspace.id)
        .map_err(internal)?;
    tracing::info!(
        target: "instrumentation.storage",
        message = "knowledge files deleted",
        user_id = user.id,
        username = user.username.as_str(),
        deleted = deleted,
    );
    Ok(Json(json!({ "ok": true, "deleted": deleted })))
}

pub(crate) async fn list_templates(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Query(q): Query<TemplatesQuery>,
) -> Result<Json<Value>, StatusCode> {
    let templates = state
        .db
        .list_templates_in_workspace(
            workspace.id,
            q.category.as_deref(),
            q.jurisdiction.as_deref(),
        )
        .map_err(internal)?;
    Ok(Json(json!(templates)))
}

// --- Shared inner: content download ---

fn knowledge_file_path(data_dir: &str, workspace_id: i64, user_id: Option<i64>, file_name: &str) -> String {
    match user_id {
        Some(uid) => format!("{}/knowledge/workspaces/{}/users/{}/{}", data_dir, workspace_id, uid, file_name),
        None => safe_knowledge_path(data_dir, Some(workspace_id), file_name)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
    }
}

async fn inner_get_knowledge_content(
    file_name: &str,
    path: &str,
) -> Result<axum::response::Response, StatusCode> {
    use axum::response::IntoResponse;
    let bytes = std::fs::read(path).map_err(|_| StatusCode::NOT_FOUND)?;
    let disp = format!("attachment; filename=\"{}\"", file_name.replace('"', "_"));
    Ok((
        axum::http::StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (axum::http::header::CONTENT_DISPOSITION, disp),
        ],
        bytes,
    )
        .into_response())
}

pub(crate) async fn get_knowledge_content(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, StatusCode> {
    let file = state
        .db
        .get_knowledge_file_in_workspace(workspace.id, id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let path = safe_knowledge_path(&state.config.data_dir, Some(workspace.id), &file.file_name)
        .ok_or(StatusCode::BAD_REQUEST)?;
    inner_get_knowledge_content(&file.file_name, path.to_str().unwrap_or_default()).await
}

pub(crate) async fn get_user_knowledge_content(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, StatusCode> {
    let file = state
        .db
        .get_user_knowledge_file(workspace.id, user.id, id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let path = knowledge_file_path(&state.config.data_dir, workspace.id, Some(user.id), &file.file_name);
    inner_get_knowledge_content(&file.file_name, &path).await
}

pub(crate) async fn list_user_knowledge(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Query(q): Query<ListKnowledgeQuery>,
) -> Result<Json<Value>, StatusCode> {
    let (files, total) = state
        .db
        .list_user_knowledge_page(workspace.id, user.id, Some(&q.q), q.limit, q.offset)
        .map_err(internal)?;
    let limit = q.limit.clamp(1, 200);
    let offset = q.offset.max(0);
    Ok(Json(json!({
        "files": files,
        "total": total,
        "offset": offset,
        "limit": limit,
        "has_more": offset + (files.len() as i64) < total,
        "total_bytes": state.db.total_user_knowledge_bytes(workspace.id, user.id).map_err(internal)?,
    })))
}

pub(crate) async fn upload_user_knowledge(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    mut multipart: Multipart,
) -> Result<Json<Value>, StatusCode> {
    const MAX_KNOWLEDGE_FILE_BYTES: i64 = 50 * 1024 * 1024;
    let max_knowledge_total_bytes = state.config.knowledge_max_bytes.max(1);

    let knowledge_dir = format!(
        "{}/knowledge/workspaces/{}/users/{}",
        state.config.data_dir, workspace.id, user.id
    );
    std::fs::create_dir_all(&knowledge_dir).map_err(internal)?;

    let mut file_name = String::new();
    let mut description = String::new();
    let mut inline = false;
    let mut file_bytes: Vec<u8> = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    {
        match field.name() {
            Some("file") => {
                if let Some(name) = field.file_name() {
                    file_name = super::projects::sanitize_upload_name(name);
                }
                file_bytes = field
                    .bytes()
                    .await
                    .map_err(|_| StatusCode::BAD_REQUEST)?
                    .to_vec();
            },
            Some("description") => {
                description = field.text().await.unwrap_or_default();
            },
            Some("inline") => {
                let v = field.text().await.unwrap_or_default();
                inline = v == "true" || v == "1";
            },
            _ => {},
        }
    }

    if file_name.is_empty() || file_bytes.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let file_size = file_bytes.len() as i64;
    if file_size > MAX_KNOWLEDGE_FILE_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }

    let total_bytes = state
        .db
        .total_user_knowledge_bytes(workspace.id, user.id)
        .map_err(internal)?;
    if total_bytes + file_size > max_knowledge_total_bytes {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }

    let dest = format!("{knowledge_dir}/{file_name}");
    if std::path::Path::new(&dest).exists() {
        return Err(StatusCode::CONFLICT);
    }
    std::fs::write(&dest, &file_bytes).map_err(internal)?;

    let id = state
        .db
        .insert_knowledge_file_for_user(
            workspace.id,
            Some(user.id),
            &file_name,
            &description,
            file_bytes.len() as i64,
            inline,
        )
        .map_err(internal)?;

    tracing::info!(
        target: "instrumentation.storage",
        message = "user knowledge file uploaded",
        user_id = user.id,
        knowledge_id = id,
        size_bytes = file_size,
    );

    Ok(Json(json!({ "id": id, "file_name": file_name })))
}

pub(crate) async fn delete_user_knowledge(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    if let Ok(Some(file)) = state.db.get_user_knowledge_file(workspace.id, user.id, id) {
        let path = knowledge_file_path(&state.config.data_dir, workspace.id, Some(user.id), &file.file_name);
        let _ = std::fs::remove_file(&path);
    }
    state
        .db
        .delete_user_knowledge_file(workspace.id, user.id, id)
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn delete_all_user_knowledge(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    let files = state
        .db
        .list_user_knowledge_files(workspace.id, user.id)
        .map_err(internal)?;
    for file in &files {
        let path = knowledge_file_path(&state.config.data_dir, workspace.id, Some(user.id), &file.file_name);
        let _ = std::fs::remove_file(&path);
    }
    let deleted = state
        .db
        .delete_all_user_knowledge_files(workspace.id, user.id)
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true, "deleted": deleted })))
}

// --- Shared inner: repo handlers ---

async fn inner_list_knowledge_repos(
    state: &Arc<AppState>,
    workspace_id: i64,
    user_id: Option<i64>,
) -> Result<Json<Value>, StatusCode> {
    let repos = state.db.list_knowledge_repos(workspace_id, user_id).map_err(internal)?;
    Ok(Json(json!({ "repos": repos })))
}

async fn inner_add_knowledge_repo(
    state: Arc<AppState>,
    workspace_id: i64,
    user_id: Option<i64>,
    body: AddKnowledgeRepoBody,
) -> Result<Json<Value>, StatusCode> {
    let url = body.url.trim().to_string();
    if url.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let name = body.name.unwrap_or_default();
    let name = if name.trim().is_empty() {
        url.trim_end_matches('/').rsplit('/').next().unwrap_or("repo").trim_end_matches(".git").to_string()
    } else {
        name.trim().to_string()
    };
    let id = state.db.insert_knowledge_repo(workspace_id, user_id, &url, &name).map_err(internal)?;
    let data_dir = state.config.data_dir.clone();
    let db = Arc::clone(&state.db);
    tokio::spawn(async move {
        clone_knowledge_repo(id, &url, &data_dir, &db).await;
    });
    let repos = state.db.list_knowledge_repos(workspace_id, user_id).map_err(internal)?;
    Ok(Json(json!({ "repos": repos })))
}

pub(crate) async fn list_knowledge_repos(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    inner_list_knowledge_repos(&state, workspace.id, None).await
}

pub(crate) async fn add_knowledge_repo(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Json(body): Json<AddKnowledgeRepoBody>,
) -> Result<Json<Value>, StatusCode> {
    inner_add_knowledge_repo(state, workspace.id, None, body).await
}

pub(crate) async fn delete_knowledge_repo_handler(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let local_path = state.db.delete_knowledge_repo(id, workspace.id).map_err(internal)?;
    if !local_path.is_empty() {
        let _ = tokio::fs::remove_dir_all(&local_path).await;
    }
    inner_list_knowledge_repos(&state, workspace.id, None).await
}

pub(crate) async fn list_user_knowledge_repos(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    inner_list_knowledge_repos(&state, workspace.id, Some(user.id)).await
}

pub(crate) async fn add_user_knowledge_repo(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Json(body): Json<AddKnowledgeRepoBody>,
) -> Result<Json<Value>, StatusCode> {
    inner_add_knowledge_repo(state, workspace.id, Some(user.id), body).await
}

pub(crate) async fn delete_user_knowledge_repo_handler(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let repos = state.db.list_knowledge_repos(workspace.id, Some(user.id)).map_err(internal)?;
    if !repos.iter().any(|r| r.id == id) {
        return Err(StatusCode::NOT_FOUND);
    }
    let local_path = state.db.delete_knowledge_repo(id, workspace.id).map_err(internal)?;
    if !local_path.is_empty() {
        let _ = tokio::fs::remove_dir_all(&local_path).await;
    }
    inner_list_knowledge_repos(&state, workspace.id, Some(user.id)).await
}

fn inject_git_token(url: &str, username: &str, token: &str) -> String {
    if token.is_empty() { return url.to_string(); }
    for prefix in &["https://", "http://"] {
        if let Some(rest) = url.strip_prefix(prefix) {
            return format!("{}{}:{}@{}", prefix, username, token, rest);
        }
    }
    url.to_string()
}

fn git_token_for_url(url: &str, settings: &std::collections::HashMap<String, String>) -> (String, String) {
    if url.contains("github.com") {
        ("x-access-token".into(), settings.get("github_token").cloned().unwrap_or_default())
    } else if url.contains("gitlab.com") || url.contains("gitlab.") {
        ("oauth2".into(), settings.get("gitlab_token").cloned().unwrap_or_default())
    } else if url.contains("codeberg.org") {
        ("oauth2".into(), settings.get("codeberg_token").cloned().unwrap_or_default())
    } else {
        (String::new(), String::new())
    }
}

pub(crate) async fn clone_knowledge_repo(id: i64, url: &str, data_dir: &str, db: &Arc<borg_core::db::Db>) {
    let repos = db.list_all_knowledge_repos().unwrap_or_default();
    let effective_url = if let Some(repo) = repos.iter().find(|r| r.id == id) {
        if let Some(uid) = repo.user_id {
            let settings = db.get_all_user_settings(uid).unwrap_or_default();
            let (username, token) = git_token_for_url(url, &settings);
            inject_git_token(url, &username, &token)
        } else {
            url.to_string()
        }
    } else {
        url.to_string()
    };

    let dest = format!("{}/knowledge-repos/{}", data_dir, id);
    let _ = std::fs::create_dir_all(&dest);
    let result = if std::path::Path::new(&dest).join(".git").exists() {
        tokio::process::Command::new("git")
            .args(["-C", &dest, "pull", "--ff-only", "--quiet"])
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .await
    } else {
        tokio::process::Command::new("git")
            .args(["clone", "--depth=1", "--quiet", &effective_url, &dest])
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .await
    };
    match result {
        Ok(out) if out.status.success() => {
            let _ = db.update_knowledge_repo_status(id, "ready", &dest, "");
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr).to_string();
            let _ = db.update_knowledge_repo_status(id, "error", "", &err);
        }
        Err(e) => {
            let _ = db.update_knowledge_repo_status(id, "error", "", &e.to_string());
        }
    }
}
