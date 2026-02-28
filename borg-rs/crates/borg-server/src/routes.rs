use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        Json,
    },
};
use borg_core::{
    config::Config,
    db::{Db, LegacyEvent, ProjectFileRow, ProjectRow, TaskMessage, TaskOutput},
    modes::all_modes,
    pipeline::PipelineEvent,
    types::{PhaseConfig, PhaseContext, PipelineMode, RepoConfig, Task},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex as TokioMutex};
use tokio_stream::{
    wrappers::{BroadcastStream, UnboundedReceiverStream},
    StreamExt,
};

use crate::AppState;

// ── Error helper ──────────────────────────────────────────────────────────

pub(crate) fn internal(e: impl std::fmt::Display) -> StatusCode {
    tracing::error!("internal error: {e}");
    StatusCode::INTERNAL_SERVER_ERROR
}

// ── Request body types ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct CreateTaskBody {
    pub title: String,
    pub description: Option<String>,
    pub mode: Option<String>,
    pub repo: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CreateMessageBody {
    pub role: String,
    pub content: String,
}

#[derive(Deserialize)]
pub(crate) struct FocusBody {
    pub text: String,
}

#[derive(Deserialize)]
pub(crate) struct RepoQuery {
    pub repo: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct TasksQuery {
    pub repo: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct EventsQuery {
    pub category: Option<String>,
    pub level: Option<String>,
    pub since: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct ChatMessagesQuery {
    pub thread: String,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct ChatPostBody {
    pub text: String,
    pub sender: Option<String>,
    pub thread: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CreateProjectBody {
    pub name: String,
    pub mode: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ProjectFilesQuery {
    pub limit: Option<i64>,
}

// ── Serializable wrappers ─────────────────────────────────────────────────

#[derive(Serialize)]
pub(crate) struct TaskOutputJson {
    pub id: i64,
    pub task_id: i64,
    pub phase: String,
    pub output: String,
    pub exit_code: i64,
    pub created_at: String,
}

impl From<TaskOutput> for TaskOutputJson {
    fn from(o: TaskOutput) -> Self {
        Self {
            id: o.id,
            task_id: o.task_id,
            phase: o.phase,
            output: o.output,
            exit_code: o.exit_code,
            created_at: o.created_at.to_rfc3339(),
        }
    }
}

#[derive(Serialize)]
pub(crate) struct TaskMessageJson {
    pub id: i64,
    pub task_id: i64,
    pub role: String,
    pub content: String,
    pub created_at: String,
    pub delivered_phase: Option<String>,
}

impl From<TaskMessage> for TaskMessageJson {
    fn from(m: TaskMessage) -> Self {
        Self {
            id: m.id,
            task_id: m.task_id,
            role: m.role,
            content: m.content,
            created_at: m.created_at.to_rfc3339(),
            delivered_phase: m.delivered_phase,
        }
    }
}

#[derive(Serialize)]
pub(crate) struct ProjectJson {
    pub id: i64,
    pub name: String,
    pub mode: String,
    pub created_at: String,
}

impl From<ProjectRow> for ProjectJson {
    fn from(p: ProjectRow) -> Self {
        Self {
            id: p.id,
            name: p.name,
            mode: p.mode,
            created_at: p.created_at.to_rfc3339(),
        }
    }
}

#[derive(Serialize)]
pub(crate) struct ProjectFileJson {
    pub id: i64,
    pub project_id: i64,
    pub file_name: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub created_at: String,
}

impl From<ProjectFileRow> for ProjectFileJson {
    fn from(f: ProjectFileRow) -> Self {
        Self {
            id: f.id,
            project_id: f.project_id,
            file_name: f.file_name,
            mime_type: f.mime_type,
            size_bytes: f.size_bytes,
            created_at: f.created_at.to_rfc3339(),
        }
    }
}

// ── Settings constants ────────────────────────────────────────────────────

pub(crate) const SETTINGS_KEYS: &[&str] = &[
    "continuous_mode",
    "release_interval_mins",
    "pipeline_max_backlog",
    "agent_timeout_s",
    "pipeline_seed_cooldown_s",
    "pipeline_tick_s",
    "model",
    "container_memory_mb",
    "assistant_name",
    "pipeline_max_agents",
    "pipeline_agent_cooldown_s",
    "proposal_promote_threshold",
    "backend",
    "git_claude_coauthor",
    "git_user_coauthor",
];

pub(crate) const SETTINGS_DEFAULTS: &[(&str, &str)] = &[
    ("continuous_mode", "false"),
    ("release_interval_mins", "180"),
    ("pipeline_max_backlog", "5"),
    ("agent_timeout_s", "600"),
    ("pipeline_seed_cooldown_s", "3600"),
    ("pipeline_tick_s", "30"),
    ("model", "claude-sonnet-4-6"),
    ("container_memory_mb", "2048"),
    ("assistant_name", "Borg"),
    ("pipeline_max_agents", "3"),
    ("pipeline_agent_cooldown_s", "120"),
    ("proposal_promote_threshold", "70"),
    ("backend", "claude"),
    ("git_claude_coauthor", "false"),
    ("git_user_coauthor", ""),
];

// ── Shared helper functions ───────────────────────────────────────────────

fn sanitize_chat_key(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn sanitize_upload_name(name: &str) -> String {
    let base = std::path::Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("upload.bin");
    let mut out = String::with_capacity(base.len());
    for c in base.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('.').trim();
    if trimmed.is_empty() {
        "upload.bin".to_string()
    } else {
        trimmed.to_string()
    }
}

fn project_chat_key(project_id: i64) -> String {
    format!("project:{project_id}")
}

fn rand_suffix() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos as u64) & 0xFFFFF
}

fn get_custom_modes(db: &Db) -> Vec<PipelineMode> {
    let raw = match db.get_config("custom_modes") {
        Ok(Some(v)) => v,
        _ => return Vec::new(),
    };
    serde_json::from_str::<Vec<PipelineMode>>(&raw).unwrap_or_default()
}

fn save_custom_modes(db: &Db, modes: &[PipelineMode]) -> Result<(), StatusCode> {
    let serialized = serde_json::to_string(modes).map_err(internal)?;
    db.set_config("custom_modes", &serialized)
        .map_err(internal)?;
    Ok(())
}

fn valid_mode_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn parse_project_chat_key(chat_key: &str) -> Option<i64> {
    chat_key.strip_prefix("project:")?.parse::<i64>().ok()
}

fn build_project_context(project: &ProjectRow, files: &[ProjectFileRow]) -> String {
    if files.is_empty() {
        return String::new();
    }

    const MAX_CONTEXT_BYTES: usize = 120_000;
    const MAX_FILE_PREVIEW_BYTES: usize = 12_000;
    let mut remaining = MAX_CONTEXT_BYTES;

    let mut context = format!(
        "Project context:\nProject: {} (mode: {})\nFiles: {}\n\n",
        project.name,
        project.mode,
        files.len()
    );
    if context.len() >= remaining {
        return context;
    }
    remaining -= context.len();

    for file in files {
        if remaining < 256 {
            break;
        }
        let header = format!(
            "--- FILE: {} ({} bytes, {}) ---\n",
            file.file_name, file.size_bytes, file.mime_type
        );
        if header.len() >= remaining {
            break;
        }
        context.push_str(&header);
        remaining -= header.len();

        let preview_budget = remaining.min(MAX_FILE_PREVIEW_BYTES);
        let preview = match std::fs::read(&file.stored_path) {
            Ok(raw) => {
                let clipped = &raw[..raw.len().min(preview_budget)];
                String::from_utf8_lossy(clipped).to_string()
            },
            Err(_) => "[file unavailable]\n".to_string(),
        };
        let preview = preview.replace('\0', "");
        if preview.len() > remaining {
            context.push_str(&preview[..remaining]);
            break;
        } else {
            context.push_str(&preview);
            remaining -= preview.len();
        }

        if remaining >= 2 {
            context.push('\n');
            context.push('\n');
            remaining -= 2;
        }
    }

    context
}

/// Run claude as a conversational chat agent with session continuity.
/// `sessions` maps chat_key → claude session_id for resume.
pub(crate) async fn run_chat_agent(
    chat_key: &str,
    sender_name: &str,
    messages: &[String],
    sessions: &Arc<TokioMutex<HashMap<String, String>>>,
    config: &Config,
    db: &Arc<Db>,
    chat_event_tx: &broadcast::Sender<String>,
) -> anyhow::Result<String> {
    let session_dir = format!(
        "{}/sessions/chat-{}",
        config.data_dir,
        sanitize_chat_key(chat_key)
    );
    std::fs::create_dir_all(&session_dir)?;

    // Store each incoming message
    let ts_secs = Utc::now().timestamp();
    for (i, msg) in messages.iter().enumerate() {
        let msg_id = format!("{}-{}-{}", chat_key, ts_secs, i);
        let _ = db.insert_chat_message(
            &msg_id,
            chat_key,
            Some(sender_name),
            Some(sender_name),
            msg,
            false,
            false,
        );
        let event = json!({
            "role": "user",
            "sender": sender_name,
            "text": msg,
            "ts": ts_secs,
            "thread": chat_key,
        })
        .to_string();
        let _ = chat_event_tx.send(event);
    }

    let prompt = if messages.len() == 1 {
        format!("{} says: {}", sender_name, messages[0])
    } else {
        let joined: Vec<String> = messages.iter().map(|m| format!("- {m}")).collect();
        format!("{} says:\n{}", sender_name, joined.join("\n"))
    };
    let prompt = if let Some(project_id) = parse_project_chat_key(chat_key) {
        match db.get_project(project_id) {
            Ok(Some(project)) => {
                let files = db.list_project_files(project_id).unwrap_or_default();
                let ctx = build_project_context(&project, &files);
                if ctx.is_empty() {
                    prompt
                } else {
                    format!("{ctx}\n\nUser request:\n{prompt}")
                }
            },
            _ => prompt,
        }
    } else {
        prompt
    };

    let mut args = vec![
        "--model".to_string(),
        config.model.clone(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--allowedTools".to_string(),
        "none".to_string(),
        "--max-turns".to_string(),
        "10".to_string(),
    ];

    let session_id = sessions.lock().await.get(chat_key).cloned();
    if let Some(ref sid) = session_id {
        args.push("--resume".to_string());
        args.push(sid.clone());
    }

    args.push("--print".to_string());
    args.push(prompt);

    let out = tokio::process::Command::new("claude")
        .args(&args)
        .current_dir(&session_dir)
        .env("HOME", &session_dir)
        .env("ANTHROPIC_API_KEY", &config.oauth_token)
        .env("CLAUDE_CODE_OAUTH_TOKEN", &config.oauth_token)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .await?;

    let raw = String::from_utf8_lossy(&out.stdout).into_owned();
    let (text, new_session_id) = borg_agent::event::parse_stream(&raw);

    if let Some(sid) = new_session_id {
        sessions.lock().await.insert(chat_key.to_string(), sid);
    }

    // Store bot response
    if !text.is_empty() {
        let reply_ts = Utc::now().timestamp();
        let reply_id = format!("{}-bot-{}", chat_key, reply_ts);
        let _ = db.insert_chat_message(
            &reply_id,
            chat_key,
            Some("borg"),
            Some("borg"),
            &text,
            true,
            true,
        );
        let event = json!({
            "role": "assistant",
            "sender": "borg",
            "text": &text,
            "ts": reply_ts,
            "thread": chat_key,
        })
        .to_string();
        let _ = chat_event_tx.send(event);
    }

    Ok(text)
}

/// Build a release binary and replace the running process via execve.
/// Returns true only if execve was invoked (this process should be replaced).
pub(crate) async fn rebuild_and_exec(repo_path: &str) -> bool {
    let build_dir = format!("{repo_path}/borg-rs");
    let build = tokio::process::Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(&build_dir)
        .status()
        .await;
    match build {
        Ok(s) if s.success() => {
            tracing::info!("Build done, restarting");
            let bin = format!("{repo_path}/borg-rs/target/release/borg-server");
            use std::os::unix::process::CommandExt;
            let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
            let err = std::process::Command::new(&bin).args(&args[1..]).exec();
            tracing::error!("execve failed: {err}");
            false
        },
        Ok(_) => {
            tracing::error!("Release build failed");
            false
        },
        Err(e) => {
            tracing::error!("Failed to run cargo: {e}");
            false
        },
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────

pub(crate) async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

// Projects

pub(crate) async fn list_projects(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let projects = state.db.list_projects().map_err(internal)?;
    let out: Vec<ProjectJson> = projects.into_iter().map(ProjectJson::from).collect();
    Ok(Json(json!(out)))
}

pub(crate) async fn create_project(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateProjectBody>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mode = body.mode.unwrap_or_else(|| "general".to_string());
    let id = state.db.insert_project(name, &mode).map_err(internal)?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub(crate) async fn list_project_files(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    if state.db.get_project(id).map_err(internal)?.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let files = state.db.list_project_files(id).map_err(internal)?;
    let out: Vec<ProjectFileJson> = files.into_iter().map(ProjectFileJson::from).collect();
    Ok(Json(json!(out)))
}

pub(crate) async fn upload_project_files(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    mut multipart: Multipart,
) -> Result<Json<Value>, StatusCode> {
    const MAX_PROJECT_BYTES: i64 = 100 * 1024 * 1024;
    if state.db.get_project(id).map_err(internal)?.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }

    let mut total_bytes = state.db.total_project_file_bytes(id).map_err(internal)?;
    let mut uploaded: Vec<ProjectFileJson> = Vec::new();
    let files_dir = format!("{}/projects/{}/files", state.config.data_dir, id);
    tokio::fs::create_dir_all(&files_dir)
        .await
        .map_err(internal)?;

    while let Some(field) = multipart.next_field().await.map_err(internal)? {
        let raw_name = field
            .file_name()
            .map(std::string::ToString::to_string)
            .unwrap_or_else(|| "upload.bin".to_string());
        let file_name = sanitize_upload_name(&raw_name);
        let mime_type = field
            .content_type()
            .map(std::string::ToString::to_string)
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let bytes = field.bytes().await.map_err(internal)?;
        let file_size = bytes.len() as i64;
        if file_size == 0 {
            continue;
        }
        if total_bytes + file_size > MAX_PROJECT_BYTES {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }

        let unique_name = format!(
            "{}_{}_{}",
            Utc::now().timestamp_millis(),
            rand_suffix(),
            file_name
        );
        let stored_path = format!("{}/{}", files_dir, unique_name);
        tokio::fs::write(&stored_path, &bytes)
            .await
            .map_err(internal)?;

        let file_id = state
            .db
            .insert_project_file(id, &file_name, &stored_path, &mime_type, file_size)
            .map_err(internal)?;
        total_bytes += file_size;

        let inserted = state
            .db
            .list_project_files(id)
            .map_err(internal)?
            .into_iter()
            .find(|f| f.id == file_id);
        if let Some(row) = inserted {
            uploaded.push(ProjectFileJson::from(row));
        }
    }

    Ok(Json(json!({ "uploaded": uploaded })))
}

// Tasks

pub(crate) async fn list_tasks(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TasksQuery>,
) -> Result<Json<Value>, StatusCode> {
    let tasks = state
        .db
        .list_all_tasks(q.repo.as_deref())
        .map_err(internal)?;
    Ok(Json(json!(tasks)))
}

pub(crate) async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    match state.db.get_task_with_outputs(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some((task, outputs)) => {
            let outputs_json: Vec<TaskOutputJson> =
                outputs.into_iter().map(TaskOutputJson::from).collect();
            let mut v = serde_json::to_value(&task).map_err(internal)?;
            if let Some(obj) = v.as_object_mut() {
                obj.insert(
                    "outputs".into(),
                    serde_json::to_value(outputs_json).map_err(internal)?,
                );
            }
            Ok(Json(v))
        },
    }
}

pub(crate) async fn create_task(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateTaskBody>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let repo = body
        .repo
        .or_else(|| std::env::var("PIPELINE_REPO").ok())
        .unwrap_or_default();
    let mode = body.mode.unwrap_or_else(|| "sweborg".into());
    let task = Task {
        id: 0,
        title: body.title,
        description: body.description.unwrap_or_default(),
        repo_path: repo,
        branch: String::new(),
        status: "backlog".into(),
        attempt: 0,
        max_attempts: 5,
        last_error: String::new(),
        created_by: "api".into(),
        notify_chat: String::new(),
        created_at: Utc::now(),
        session_id: String::new(),
        mode,
        backend: String::new(),
    };
    let id = state.db.insert_task(&task).map_err(internal)?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub(crate) async fn retry_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    match state.db.get_task(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some(_) => {
            state
                .db
                .update_task_status(id, "backlog", None)
                .map_err(internal)?;
            Ok(StatusCode::OK)
        },
    }
}

// Task messages

pub(crate) async fn get_task_messages(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    match state.db.get_task(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some(_) => {
            let messages = state.db.get_task_messages(id).map_err(internal)?;
            let messages_json: Vec<TaskMessageJson> =
                messages.into_iter().map(TaskMessageJson::from).collect();
            Ok(Json(json!({ "messages": messages_json })))
        },
    }
}

pub(crate) async fn post_task_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<CreateMessageBody>,
) -> Result<StatusCode, StatusCode> {
    match state.db.get_task(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some(_) => {
            state
                .db
                .insert_task_message(id, &body.role, &body.content)
                .map_err(internal)?;
            let _ = state.pipeline_event_tx.send(PipelineEvent::Output {
                task_id: Some(id),
                message: body.content.clone(),
            });
            Ok(StatusCode::CREATED)
        },
    }
}

// Queue

pub(crate) async fn list_queue(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let entries = state.db.list_queue().map_err(internal)?;
    Ok(Json(json!(entries)))
}

// Status

pub(crate) async fn get_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let uptime_s = state.start_time.elapsed().as_secs();

    let repos = state.db.list_repos().map_err(internal)?;
    let primary_repo = std::env::var("PIPELINE_REPO").unwrap_or_default();
    let watched_repos: Vec<Value> = repos
        .iter()
        .map(|r| {
            json!({
                "path": r.path,
                "test_cmd": r.test_cmd,
                "is_self": r.path == primary_repo,
                "auto_merge": r.auto_merge,
                "mode": r.mode,
            })
        })
        .collect();

    let (active, merged, failed, total) = state.db.task_stats().map_err(internal)?;

    let model = state
        .db
        .get_config("model")
        .map_err(internal)?
        .unwrap_or_else(|| "claude-sonnet-4-6".into());

    let release_interval_mins: i64 = state
        .db
        .get_config("release_interval_mins")
        .map_err(internal)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(180);

    let continuous_mode: bool = state
        .db
        .get_config("continuous_mode")
        .map_err(internal)?
        .map(|v| v == "true")
        .unwrap_or(false);

    let assistant_name = state
        .db
        .get_config("assistant_name")
        .map_err(internal)?
        .unwrap_or_else(|| "Borg".into());

    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_s": uptime_s,
        "model": model,
        "watched_repos": watched_repos,
        "release_interval_mins": release_interval_mins,
        "continuous_mode": continuous_mode,
        "assistant_name": assistant_name,
        "active_tasks": active,
        "merged_tasks": merged,
        "failed_tasks": failed,
        "total_tasks": total,
        "dispatched_agents": 0,
    })))
}

// Proposals

pub(crate) async fn list_proposals(
    State(state): State<Arc<AppState>>,
    Query(q): Query<RepoQuery>,
) -> Result<Json<Value>, StatusCode> {
    let proposals = state
        .db
        .list_all_proposals(q.repo.as_deref())
        .map_err(internal)?;
    Ok(Json(json!(proposals)))
}

pub(crate) async fn approve_proposal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let proposal = state
        .db
        .get_proposal(id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;

    state
        .db
        .update_proposal_status(id, "approved")
        .map_err(internal)?;

    let task = Task {
        id: 0,
        title: proposal.title.clone(),
        description: proposal.description.clone(),
        repo_path: proposal.repo_path.clone(),
        branch: String::new(),
        status: "backlog".into(),
        attempt: 0,
        max_attempts: 5,
        last_error: String::new(),
        created_by: "proposal".into(),
        notify_chat: String::new(),
        created_at: Utc::now(),
        session_id: String::new(),
        mode: "sweborg".into(),
        backend: String::new(),
    };
    let task_id = state.db.insert_task(&task).map_err(internal)?;
    Ok(Json(json!({ "task_id": task_id })))
}

pub(crate) async fn dismiss_proposal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    match state.db.get_proposal(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some(_) => {
            state
                .db
                .update_proposal_status(id, "dismissed")
                .map_err(internal)?;
            Ok(StatusCode::OK)
        },
    }
}

pub(crate) async fn reopen_proposal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    match state.db.get_proposal(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some(_) => {
            state
                .db
                .update_proposal_status(id, "proposed")
                .map_err(internal)?;
            Ok(StatusCode::OK)
        },
    }
}

pub(crate) async fn triage_proposals(State(state): State<Arc<AppState>>) -> Json<Value> {
    let proposals = match state.db.list_untriaged_proposals() {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("list_untriaged_proposals: {e}");
            return Json(json!({ "scored": 0 }));
        },
    };
    let count = proposals.len();
    if count == 0 {
        return Json(json!({ "scored": 0 }));
    }

    let db = Arc::clone(&state.db);
    let backend = state.default_backend("claude");
    let model = db
        .get_config("model")
        .ok()
        .flatten()
        .unwrap_or_else(|| "claude-sonnet-4-6".into());
    let oauth = std::env::var("ANTHROPIC_API_KEY")
        .or_else(|_| std::env::var("CLAUDE_CODE_OAUTH_TOKEN"))
        .unwrap_or_default();

    tokio::spawn(async move {
        for proposal in proposals {
            let prompt = format!(
                "Score this software proposal as JSON.\n\nTitle: {}\nDescription: {}\nRationale: {}\n\nRespond ONLY with valid JSON:\n{{\"score\":0-100,\"impact\":0-100,\"feasibility\":0-100,\"risk\":0-100,\"effort\":0-100,\"reasoning\":\"...\"}}",
                proposal.title, proposal.description, proposal.rationale
            );

            let task = Task {
                id: proposal.id,
                title: format!("triage:{}", proposal.id),
                description: String::new(),
                repo_path: proposal.repo_path.clone(),
                branch: String::new(),
                status: "triage".into(),
                attempt: 0,
                max_attempts: 1,
                last_error: String::new(),
                created_by: "triage".into(),
                notify_chat: String::new(),
                created_at: chrono::Utc::now(),
                session_id: String::new(),
                mode: "sweborg".into(),
                backend: String::new(),
            };

            let phase = PhaseConfig {
                name: "triage".into(),
                label: "Triage".into(),
                instruction: prompt,
                fresh_session: true,
                allowed_tools: String::new(),
                ..Default::default()
            };

            let ctx = PhaseContext {
                task: task.clone(),
                repo_config: RepoConfig {
                    path: proposal.repo_path.clone(),
                    test_cmd: String::new(),
                    prompt_file: String::new(),
                    mode: "sweborg".into(),
                    is_self: false,
                    auto_merge: false,
                    lint_cmd: String::new(),
                    backend: String::new(),
                },
                session_dir: format!("store/sessions/triage-{}", proposal.id),
                worktree_path: proposal.repo_path.clone(),
                oauth_token: oauth.clone(),
                model: model.clone(),
                pending_messages: Vec::new(),
                system_prompt_suffix: String::new(),
                user_coauthor: String::new(),
                stream_tx: None,
                setup_script: String::new(),
            };

            tokio::fs::create_dir_all(&ctx.session_dir).await.ok();

            match backend.run_phase(&task, &phase, ctx).await {
                Ok(result) => {
                    if let Some(json_start) = result.output.find('{') {
                        if let Some(json_end) = result.output[json_start..].rfind('}') {
                            let json_str = &result.output[json_start..json_start + json_end + 1];
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                                let score = v["score"].as_i64().unwrap_or(0);
                                let impact = v["impact"].as_i64().unwrap_or(0);
                                let feasibility = v["feasibility"].as_i64().unwrap_or(0);
                                let risk = v["risk"].as_i64().unwrap_or(0);
                                let effort = v["effort"].as_i64().unwrap_or(0);
                                let reasoning = v["reasoning"].as_str().unwrap_or("").to_string();
                                if let Err(e) = db.update_proposal_triage(
                                    proposal.id,
                                    score,
                                    impact,
                                    feasibility,
                                    risk,
                                    effort,
                                    &reasoning,
                                ) {
                                    tracing::error!("update_proposal_triage #{}: {e}", proposal.id);
                                } else {
                                    tracing::info!(
                                        "triaged proposal #{}: score={score}",
                                        proposal.id
                                    );
                                }
                            }
                        }
                    }
                },
                Err(e) => tracing::error!("triage agent for proposal #{}: {e}", proposal.id),
            }
        }
    });

    Json(json!({ "scored": count }))
}

// Modes

pub(crate) async fn get_modes(State(state): State<Arc<AppState>>) -> Json<Value> {
    let mut merged_modes = all_modes();
    merged_modes.extend(get_custom_modes(&state.db));
    let modes: Vec<Value> = merged_modes
        .into_iter()
        .map(|m| {
            let phases: Vec<Value> = m
                .phases
                .iter()
                .map(|p| json!({ "name": p.name, "label": p.label }))
                .collect();
            json!({
                "name": m.name,
                "label": m.label,
                "category": m.category,
                "phases": phases,
            })
        })
        .collect();
    Json(json!(modes))
}

pub(crate) async fn get_full_modes(State(state): State<Arc<AppState>>) -> Json<Value> {
    let mut merged_modes = all_modes();
    merged_modes.extend(get_custom_modes(&state.db));
    Json(json!(merged_modes))
}

pub(crate) async fn list_custom_modes(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!(get_custom_modes(&state.db)))
}

pub(crate) async fn upsert_custom_mode(
    State(state): State<Arc<AppState>>,
    Json(mode): Json<PipelineMode>,
) -> Result<Json<Value>, StatusCode> {
    let name = mode.name.trim();
    if !valid_mode_name(name) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if all_modes().iter().any(|m| m.name == name) {
        return Err(StatusCode::CONFLICT);
    }
    if mode.phases.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut custom = get_custom_modes(&state.db);
    custom.retain(|m| m.name != name);
    custom.push(mode);
    save_custom_modes(&state.db, &custom)?;
    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn delete_custom_mode(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    if all_modes().iter().any(|m| m.name == name) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut custom = get_custom_modes(&state.db);
    let before = custom.len();
    custom.retain(|m| m.name != name);
    if before == custom.len() {
        return Err(StatusCode::NOT_FOUND);
    }
    save_custom_modes(&state.db, &custom)?;
    Ok(Json(json!({ "ok": true })))
}

// Settings

pub(crate) async fn get_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let mut obj = serde_json::Map::new();
    for key in SETTINGS_KEYS {
        let val = state.db.get_config(key).map_err(internal)?;
        let default = SETTINGS_DEFAULTS
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| *v)
            .unwrap_or("");
        let s = val.as_deref().unwrap_or(default);
        let json_val = if matches!(*key, "continuous_mode" | "git_claude_coauthor") {
            json!(s == "true")
        } else if matches!(
            *key,
            "release_interval_mins"
                | "pipeline_max_backlog"
                | "agent_timeout_s"
                | "pipeline_seed_cooldown_s"
                | "pipeline_tick_s"
                | "container_memory_mb"
                | "pipeline_max_agents"
                | "pipeline_agent_cooldown_s"
                | "proposal_promote_threshold"
        ) {
            s.parse::<i64>().map(|n| json!(n)).unwrap_or(json!(s))
        } else {
            json!(s)
        };
        obj.insert(key.to_string(), json_val);
    }
    Ok(Json(Value::Object(obj)))
}

pub(crate) async fn put_settings(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let map = body.as_object().ok_or(StatusCode::BAD_REQUEST)?;
    let mut updated = 0usize;
    for (key, val) in map {
        if !SETTINGS_KEYS.contains(&key.as_str()) {
            continue;
        }
        let s = match val {
            Value::String(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            _ => continue,
        };
        state.db.set_config(key, &s).map_err(internal)?;
        updated += 1;
    }
    Ok(Json(json!({ "updated": updated })))
}

// Focus

pub(crate) async fn get_focus(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let text = state
        .db
        .get_config("focus")
        .map_err(internal)?
        .unwrap_or_default();
    let active = !text.is_empty();
    Ok(Json(json!({ "text": text, "active": active })))
}

pub(crate) async fn post_focus(
    State(state): State<Arc<AppState>>,
    Json(body): Json<FocusBody>,
) -> Result<StatusCode, StatusCode> {
    state.db.set_config("focus", &body.text).map_err(internal)?;
    Ok(StatusCode::OK)
}

pub(crate) async fn delete_focus(
    State(state): State<Arc<AppState>>,
) -> Result<StatusCode, StatusCode> {
    state.db.set_config("focus", "").map_err(internal)?;
    Ok(StatusCode::OK)
}

// SSE logs — replays ring buffer history then streams live events

pub(crate) async fn sse_logs(
    State(state): State<Arc<AppState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    // Subscribe before snapshotting ring to avoid race
    let live_rx = state.log_tx.subscribe();
    let history: Vec<String> = state
        .log_ring
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect();
    tokio::spawn(async move {
        for line in history {
            if tx.send(line).is_err() {
                return;
            }
        }
        let mut live_rx = live_rx;
        loop {
            match live_rx.recv().await {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        return;
                    }
                },
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
    let stream = UnboundedReceiverStream::new(rx)
        .map(|data| Ok::<_, std::convert::Infallible>(Event::default().data(data)));
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    )
}

// Task stream SSE

pub(crate) async fn sse_task_stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let (history, live_rx) = state.stream_manager.subscribe(id).await;

        let history = if history.is_empty() && live_rx.is_none() {
            // No in-memory stream — serve stored raw_stream from DB
            let mut lines = Vec::new();
            if let Ok(outputs) = state.db.get_task_outputs(id) {
                for output in outputs {
                    for line in output.raw_stream.lines() {
                        if !line.is_empty() {
                            lines.push(line.to_string());
                        }
                    }
                }
            }
            if !lines.is_empty() {
                lines.push(r#"{"type":"stream_end"}"#.to_string());
            }
            lines
        } else {
            history
        };

        for line in history {
            if tx.send(line).is_err() {
                return;
            }
        }

        if let Some(mut live_rx) = live_rx {
            loop {
                match live_rx.recv().await {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            return;
                        }
                    },
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        }
    });
    let stream = UnboundedReceiverStream::new(rx)
        .map(|data| Ok::<_, std::convert::Infallible>(Event::default().data(data)));
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    )
}

// Release

pub(crate) async fn post_release(State(state): State<Arc<AppState>>) -> Json<Value> {
    state
        .force_restart
        .store(true, std::sync::atomic::Ordering::Relaxed);
    tracing::info!("Force restart requested via /api/release");
    Json(json!({ "ok": true }))
}

// Events

pub(crate) async fn get_events(
    State(state): State<Arc<AppState>>,
    Query(q): Query<EventsQuery>,
) -> Result<Json<Value>, StatusCode> {
    let events: Vec<LegacyEvent> = state
        .db
        .get_events_filtered(
            q.category.as_deref(),
            q.level.as_deref(),
            q.since,
            q.limit.unwrap_or(100),
        )
        .map_err(internal)?;
    Ok(Json(json!(events)))
}

// Chat

pub(crate) async fn sse_chat_events(
    State(state): State<Arc<AppState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = state.chat_event_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| match msg {
        Ok(data) => Some(Ok(Event::default().data(data))),
        _ => None,
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    )
}

pub(crate) async fn get_chat_threads(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let threads = state.db.get_chat_threads().map_err(internal)?;
    let v: Vec<Value> = threads
        .into_iter()
        .map(|(jid, count, last_ts)| json!({ "id": jid, "message_count": count, "last_ts": last_ts }))
        .collect();
    Ok(Json(json!(v)))
}

pub(crate) async fn get_chat_messages(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ChatMessagesQuery>,
) -> Result<Json<Value>, StatusCode> {
    let msgs = state
        .db
        .get_chat_messages(&q.thread, q.limit.unwrap_or(100))
        .map_err(internal)?;
    let v: Vec<Value> = msgs
        .iter()
        .map(|m| {
            json!({
                "role": if m.is_from_me { "assistant" } else { "user" },
                "sender": m.sender_name,
                "text": m.content,
                "ts": m.timestamp,
                "thread": m.chat_jid,
            })
        })
        .collect();
    Ok(Json(json!(v)))
}

pub(crate) async fn get_project_chat_messages(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(q): Query<ProjectFilesQuery>,
) -> Result<Json<Value>, StatusCode> {
    if state.db.get_project(id).map_err(internal)?.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let thread = project_chat_key(id);
    let msgs = state
        .db
        .get_chat_messages(&thread, q.limit.unwrap_or(200))
        .map_err(internal)?;
    let v: Vec<Value> = msgs
        .iter()
        .map(|m| {
            json!({
                "role": if m.is_from_me { "assistant" } else { "user" },
                "sender": m.sender_name,
                "text": m.content,
                "ts": m.timestamp,
                "thread": m.chat_jid,
            })
        })
        .collect();
    Ok(Json(json!(v)))
}

pub(crate) async fn post_project_chat(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<ChatPostBody>,
) -> Result<Json<Value>, StatusCode> {
    if state.db.get_project(id).map_err(internal)?.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let thread = project_chat_key(id);
    let sender = body
        .sender
        .clone()
        .unwrap_or_else(|| "web-user".to_string());

    let state2 = Arc::clone(&state);
    let thread2 = thread.clone();
    let sender2 = sender.clone();
    let text2 = body.text.clone();
    tokio::spawn(async move {
        match run_chat_agent(
            &thread2,
            &sender2,
            &[text2],
            &state2.web_sessions,
            &state2.config,
            &state2.db,
            &state2.chat_event_tx,
        )
        .await
        {
            Ok(_) => {},
            Err(e) => tracing::warn!("project chat agent error: {e}"),
        }
    });

    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn post_chat(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ChatPostBody>,
) -> Result<Json<Value>, StatusCode> {
    let thread = body
        .thread
        .clone()
        .unwrap_or_else(|| "web:dashboard".to_string());
    let sender = body
        .sender
        .clone()
        .unwrap_or_else(|| "web-user".to_string());
    let ts = Utc::now().timestamp();
    let msg_id = format!("{}-{}", thread, ts);

    state
        .db
        .insert_chat_message(
            &msg_id,
            &thread,
            Some(&sender),
            Some(&sender),
            &body.text,
            false,
            false,
        )
        .map_err(internal)?;

    let event = json!({
        "role": "user",
        "sender": &sender,
        "text": &body.text,
        "ts": ts,
        "thread": &thread,
    })
    .to_string();
    let _ = state.chat_event_tx.send(event);

    // Run agent async — sessions shared via AppState.web_sessions
    let state2 = Arc::clone(&state);
    let thread2 = thread.clone();
    let sender2 = sender.clone();
    let text2 = body.text.clone();
    tokio::spawn(async move {
        match run_chat_agent(
            &thread2,
            &sender2,
            &[text2],
            &state2.web_sessions,
            &state2.config,
            &state2.db,
            &state2.chat_event_tx,
        )
        .await
        {
            Ok(_) => {},
            Err(e) => tracing::warn!("web chat agent error: {e}"),
        }
    });

    Ok(Json(json!({ "ok": true })))
}

// Backend overrides

pub(crate) async fn put_task_backend(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<Value>, StatusCode> {
    let backend = body["backend"].as_str().unwrap_or("").to_string();
    state
        .db
        .update_task_backend(id, &backend)
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn list_repos_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let repos = state.db.list_repos().map_err(internal)?;
    let arr: Vec<_> = repos
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "path": r.path,
                "name": r.name,
                "mode": r.mode,
                "backend": r.backend,
                "test_cmd": r.test_cmd,
                "auto_merge": r.auto_merge,
            })
        })
        .collect();
    Ok(Json(json!(arr)))
}

pub(crate) async fn put_repo_backend(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<Value>, StatusCode> {
    let backend = body["backend"].as_str().unwrap_or("").to_string();
    state
        .db
        .update_repo_backend(id, &backend)
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn get_task_outputs_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    match state.db.get_task(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some(_) => {
            let outputs = state.db.get_task_outputs(id).map_err(internal)?;
            let outputs_json: Vec<TaskOutputJson> =
                outputs.into_iter().map(TaskOutputJson::from).collect();
            Ok(Json(json!({ "outputs": outputs_json })))
        },
    }
}
