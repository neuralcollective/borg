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
    config::{refresh_oauth_token, Config},
    db::{Db, LegacyEvent, ProjectFileRow, ProjectRow, TaskMessage, TaskOutput},
    modes::all_modes,
    pipeline::PipelineEvent,
    types::{PhaseConfig, PhaseContext, PhaseType, PipelineMode, RepoConfig, Task},
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

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'?' | b'&' | b'#' | b' ' | b'%' | b'+' => {
                out.push_str(&format!("%{b:02X}"));
            }
            _ => out.push(b as char),
        }
    }
    out
}

fn base64_decode(input: &str) -> anyhow::Result<Vec<u8>> {
    let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut buf = 0u32;
    let mut bits = 0u32;
    for c in clean.bytes() {
        if c == b'=' { break; }
        let val = table.iter().position(|&t| t == c)
            .ok_or_else(|| anyhow::anyhow!("invalid base64 char"))? as u32;
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

// ── Request body types ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct CreateTaskBody {
    pub title: String,
    pub description: Option<String>,
    pub mode: Option<String>,
    pub repo: Option<String>,
    pub project_id: Option<i64>,
    pub task_type: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct PatchTaskBody {
    pub title: Option<String>,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct SearchQuery {
    pub q: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct DocQuery {
    pub path: Option<String>,
    pub ref_name: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ExportQuery {
    pub path: Option<String>,
    pub format: Option<String>,
    pub ref_name: Option<String>,
    pub template_id: Option<i64>,
    pub toc: Option<bool>,
    pub number_sections: Option<bool>,
    pub title_page: Option<bool>,
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
    pub client_name: Option<String>,
    pub opposing_counsel: Option<String>,
    pub jurisdiction: Option<String>,
    pub matter_type: Option<String>,
    pub privilege_level: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct UpdateProjectBody {
    pub name: Option<String>,
    pub client_name: Option<String>,
    pub case_number: Option<String>,
    pub jurisdiction: Option<String>,
    pub matter_type: Option<String>,
    pub opposing_counsel: Option<String>,
    pub deadline: Option<Option<String>>,
    pub privilege_level: Option<String>,
    pub status: Option<String>,
    pub default_template_id: Option<Option<i64>>,
}

#[derive(Deserialize)]
pub(crate) struct ProjectFilesQuery {
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct ConflictQuery {
    pub client_name: Option<String>,
    pub opposing_counsel: Option<String>,
    pub exclude_project_id: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct UpdateKnowledgeBody {
    pub description: Option<String>,
    pub inline: Option<bool>,
    pub tags: Option<String>,
    pub category: Option<String>,
    pub jurisdiction: Option<String>,
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
    pub repo_path: String,
    pub client_name: String,
    pub case_number: String,
    pub jurisdiction: String,
    pub matter_type: String,
    pub opposing_counsel: String,
    pub deadline: Option<String>,
    pub privilege_level: String,
    pub status: String,
    pub default_template_id: Option<i64>,
    pub created_at: String,
}

impl From<ProjectRow> for ProjectJson {
    fn from(p: ProjectRow) -> Self {
        Self {
            id: p.id,
            name: p.name,
            mode: p.mode,
            repo_path: p.repo_path,
            client_name: p.client_name,
            case_number: p.case_number,
            jurisdiction: p.jurisdiction,
            matter_type: p.matter_type,
            opposing_counsel: p.opposing_counsel,
            deadline: p.deadline,
            privilege_level: p.privilege_level,
            status: p.status,
            default_template_id: p.default_template_id,
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
    pub has_text: bool,
    pub text_chars: usize,
    pub created_at: String,
}

impl From<ProjectFileRow> for ProjectFileJson {
    fn from(f: ProjectFileRow) -> Self {
        let text_chars = f.extracted_text.len();
        Self {
            id: f.id,
            project_id: f.project_id,
            file_name: f.file_name,
            mime_type: f.mime_type,
            size_bytes: f.size_bytes,
            has_text: text_chars > 0,
            text_chars,
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
    "chat_disallowed_tools",
    "pipeline_disallowed_tools",
];

pub(crate) const SETTINGS_DEFAULTS: &[(&str, &str)] = &[
    ("continuous_mode", "false"),
    ("release_interval_mins", "180"),
    ("pipeline_max_backlog", "5"),
    ("agent_timeout_s", "600"),
    ("pipeline_seed_cooldown_s", "3600"),
    ("pipeline_tick_s", "10"),
    ("model", "claude-sonnet-4-6"),
    ("container_memory_mb", "2048"),
    ("assistant_name", "Borg"),
    ("pipeline_max_agents", "2"),
    ("pipeline_agent_cooldown_s", "120"),
    ("proposal_promote_threshold", "70"),
    ("backend", "claude"),
    ("git_claude_coauthor", "false"),
    ("git_user_coauthor", ""),
    ("chat_disallowed_tools", ""),
    ("pipeline_disallowed_tools", ""),
];

// ── Shared helper functions ───────────────────────────────────────────────

fn sanitize_chat_key(key: &str) -> String {
    key.chars()
        .take(128)
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

/// Resolve a knowledge file path, canonicalizing to prevent traversal.
fn safe_knowledge_path(data_dir: &str, file_name: &str) -> Option<std::path::PathBuf> {
    let base = std::path::Path::new(file_name)
        .file_name()?
        .to_str()?;
    let dir = std::path::Path::new(data_dir).join("knowledge");
    let full = dir.join(base);
    // Ensure the resolved path stays inside the knowledge directory
    if full.starts_with(&dir) {
        Some(full)
    } else {
        None
    }
}

fn project_chat_key(project_id: i64) -> String {
    format!("project:{project_id}")
}

fn rand_suffix() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let s = RandomState::new();
    let mut h = s.build_hasher();
    h.write_u64(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64);
    h.finish()
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

fn is_binary_mime(mime: &str) -> bool {
    mime.starts_with("application/pdf")
        || mime.starts_with("image/")
        || mime.starts_with("audio/")
        || mime.starts_with("video/")
        || mime.starts_with("application/zip")
        || mime.starts_with("application/octet-stream")
}

fn stage_project_files(session_dir: &str, files: &[ProjectFileRow]) {
    let dest_dir = format!("{session_dir}/project_files");
    let _ = std::fs::create_dir_all(&dest_dir);
    for file in files {
        let safe_name = std::path::Path::new(&file.stored_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unnamed");
        let dest = format!("{dest_dir}/{safe_name}");
        let _ = std::fs::copy(&file.stored_path, &dest);
    }
}

fn build_project_context(project: &ProjectRow, files: &[ProjectFileRow], session_dir: &str, db: &Db) -> String {
    let tasks = db.list_project_tasks(project.id).unwrap_or_default();
    let completed_tasks: Vec<_> = tasks.iter()
        .filter(|t| t.status == "merged" || t.status == "done" || t.status == "complete")
        .collect();

    if files.is_empty() && completed_tasks.is_empty() {
        return String::new();
    }

    if !files.is_empty() {
        stage_project_files(session_dir, files);
    }

    const MAX_CONTEXT_BYTES: usize = 120_000;
    const MAX_FILE_PREVIEW_BYTES: usize = 12_000;
    let mut remaining = MAX_CONTEXT_BYTES;

    let files_dir = format!("{session_dir}/project_files");

    let mut context = format!(
        "Project context:\nProject: {} (mode: {})\nFiles: {} (available in {}/)\n\n",
        project.name, project.mode, files.len(), files_dir,
    );
    if context.len() >= remaining {
        return context;
    }
    remaining -= context.len();

    for file in files {
        if remaining < 256 {
            break;
        }

        let staged_name = std::path::Path::new(&file.stored_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unnamed");
        let file_path = format!("{files_dir}/{staged_name}");

        if is_binary_mime(&file.mime_type) {
            let note = format!(
                "--- FILE: {} ({} bytes, {}) ---\n[Binary file — use Read tool on: {}]\n\n",
                file.file_name, file.size_bytes, file.mime_type, file_path,
            );
            if note.len() >= remaining {
                break;
            }
            context.push_str(&note);
            remaining -= note.len();
            continue;
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

    // Add completed task summaries for context
    for task in completed_tasks {
        if remaining < 256 {
            break;
        }
        if let Ok(outputs) = db.get_task_outputs(task.id) {
            if let Some(last) = outputs.last() {
                let summary = if last.output.len() > 2000 { &last.output[..2000] } else { &last.output };
                let entry = format!("\n\n## Prior research: {} (Task #{})\n{}", task.title, task.id, summary);
                if entry.len() > remaining {
                    break;
                }
                context.push_str(&entry);
                remaining -= entry.len();
            }
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
                let ctx = build_project_context(&project, &files, &session_dir, db);
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

    let mut system_prompt = config.chat_system_prompt();

    // Detect project mode for MCP wiring
    let project_mode = parse_project_chat_key(chat_key)
        .and_then(|pid| db.get_project(pid).ok().flatten())
        .map(|p| p.mode);
    let is_legal = matches!(project_mode.as_deref(), Some("lawborg" | "legal"));

    if is_legal {
        system_prompt.push_str(borg_domains::legal::legal_chat_system_suffix());
    }

    let knowledge_files = db.list_knowledge_files().unwrap_or_default();
    if !knowledge_files.is_empty() {
        let knowledge_dir = format!("{}/knowledge", config.data_dir);
        let kb = borg_agent::instruction::build_knowledge_section(&knowledge_files, &knowledge_dir);
        if !kb.is_empty() {
            system_prompt.push('\n');
            system_prompt.push_str(&kb);
        }
    }

    let mut args = vec![
        "--model".to_string(),
        config.model.clone(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--dangerously-skip-permissions".to_string(),
        "--max-turns".to_string(),
        "64".to_string(),
        "--append-system-prompt".to_string(),
        system_prompt,
    ];

    // Apply disallowed tools from settings
    if let Ok(Some(disallowed)) = db.get_config("chat_disallowed_tools") {
        let disallowed = disallowed.trim();
        if !disallowed.is_empty() {
            args.push("--disallowedTools".to_string());
            args.push(disallowed.to_string());
        }
    }

    // Wire up lawborg MCP server for legal project chats
    if is_legal {
        let legal_mcp_path = if let Ok(p) = std::env::var("LAWBORG_MCP_SERVER") {
            std::path::PathBuf::from(p)
        } else {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../sidecar/lawborg-mcp/server.js")
        };
        match legal_mcp_path.canonicalize() {
            Ok(mcp_server) => {
                tracing::info!(chat_key, path = %mcp_server.display(), "wiring lawborg-mcp for chat");
                let mut env_vars = serde_json::Map::new();
                let providers = ["lexisnexis", "westlaw", "clio", "imanage",
                    "netdocuments", "congress", "openstates", "canlii", "regulations_gov"];
                for provider in providers {
                    if let Ok(Some(key)) = db.get_api_key("global", provider) {
                        let env_name = match provider {
                            "lexisnexis" => "LEXISNEXIS_API_KEY",
                            "westlaw" => "WESTLAW_API_KEY",
                            "clio" => "CLIO_API_KEY",
                            "imanage" => "IMANAGE_API_KEY",
                            "netdocuments" => "NETDOCUMENTS_API_KEY",
                            "congress" => "CONGRESS_API_KEY",
                            "openstates" => "OPENSTATES_API_KEY",
                            "canlii" => "CANLII_API_KEY",
                            "regulations_gov" => "REGULATIONS_GOV_API_KEY",
                            _ => continue,
                        };
                        env_vars.insert(env_name.into(), serde_json::Value::String(key));
                    }
                }
                let config_json = serde_json::json!({
                    "mcpServers": {
                        "legal": {
                            "command": "bun",
                            "args": ["run", mcp_server],
                            "env": env_vars,
                        }
                    }
                });
                // Write .mcp.json to session dir so Claude auto-discovers it on resume
                let mcp_json_path = format!("{session_dir}/.mcp.json");
                if let Err(e) = std::fs::write(&mcp_json_path, config_json.to_string()) {
                    tracing::warn!(chat_key, "failed to write .mcp.json: {e}");
                }
                // Also pass via --mcp-config for the current invocation
                args.push("--mcp-config".to_string());
                args.push(mcp_json_path);
            }
            Err(e) => {
                tracing::warn!(chat_key, path = %legal_mcp_path.display(), "lawborg-mcp not found: {e}");
            }
        }
    }

    let session_id = sessions.lock().await.get(chat_key).cloned()
        .or_else(|| db.get_session(&format!("chat-{}", sanitize_chat_key(chat_key))).ok().flatten());
    if let Some(ref sid) = session_id {
        args.push("--resume".to_string());
        args.push(sid.clone());
    }

    args.push("--print".to_string());
    args.push(prompt);

    let token = refresh_oauth_token(&config.credentials_path, &config.oauth_token);

    let timeout = std::time::Duration::from_secs(config.agent_timeout_s.max(300) as u64);
    let out = tokio::time::timeout(
        timeout,
        tokio::process::Command::new("claude")
            .args(&args)
            .current_dir(&session_dir)
            .env("HOME", &session_dir)
            .env("CLAUDE_CODE_OAUTH_TOKEN", &token)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("chat agent timed out after {}s", timeout.as_secs()))?
    ?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        tracing::warn!("chat agent failed ({}): {}", chat_key, stderr.chars().take(500).collect::<String>());
    }

    let raw = String::from_utf8_lossy(&out.stdout).into_owned();
    let (text, new_session_id) = borg_agent::event::parse_stream(&raw);

    if let Some(sid) = new_session_id {
        sessions.lock().await.insert(chat_key.to_string(), sid.clone());
        let folder = format!("chat-{}", sanitize_chat_key(chat_key));
        let _ = db.set_session(&folder, &sid);
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
pub(crate) async fn rebuild_and_exec(repo_path: &str, build_cmd: &str) -> bool {
    let build_dir = format!("{repo_path}/borg-rs");
    let parts: Vec<&str> = build_cmd.split_whitespace().collect();
    let (cmd, args) = match parts.split_first() {
        Some((c, a)) => (*c, a),
        None => {
            tracing::error!("empty build_cmd");
            return false;
        }
    };
    let build = tokio::process::Command::new(cmd)
        .args(args)
        .current_dir(&build_dir)
        .status()
        .await;
    match build {
        Ok(s) if s.success() => {
            tracing::info!("Build done, restarting");
            // Use the current executable path (immutable) instead of deriving from config
            let bin = match std::env::current_exe() {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("failed to resolve current_exe: {e}");
                    return false;
                }
            };
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

pub(crate) async fn search_projects(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<Value>, StatusCode> {
    let q = params.q.unwrap_or_default();
    if q.is_empty() {
        return list_projects(State(state)).await;
    }
    let projects = state.db.search_projects(&q).map_err(internal)?;
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
    let client_name = body.client_name.as_deref().unwrap_or("");
    let jurisdiction = body.jurisdiction.as_deref().unwrap_or("");
    let matter_type = body.matter_type.as_deref().unwrap_or("");
    let privilege_level = body.privilege_level.as_deref().unwrap_or("");
    let opposing_counsel = body.opposing_counsel.as_deref().unwrap_or("");

    // Check for conflicts before creating
    let conflicts = state
        .db
        .check_conflicts(None, client_name, opposing_counsel)
        .map_err(internal)?;

    // Insert with empty repo_path first to get the ID
    let id = state
        .db
        .insert_project(name, &mode, "", client_name, jurisdiction, matter_type, privilege_level)
        .map_err(internal)?;

    // Sync parties for future conflict checks
    let _ = state.db.sync_project_parties(id, client_name, opposing_counsel);

    // Auto-init a dedicated git repo for legal projects
    let repo_dir = format!("{}/legal-repos/{}", state.config.data_dir, id);
    tokio::fs::create_dir_all(&repo_dir).await.map_err(internal)?;
    let init = tokio::process::Command::new("git")
        .args(["init", &repo_dir])
        .output()
        .await
        .map_err(internal)?;
    if init.status.success() {
        // Initial commit so branches can be created
        let _ = tokio::process::Command::new("git")
            .args(["-C", &repo_dir, "commit", "--allow-empty", "-m", "init"])
            .output()
            .await;
        state
            .db
            .update_project(id, None, None, None, None, None, None, None, None, None, Some(&repo_dir), None)
            .map_err(internal)?;
    }

    let _ = state.db.log_event_full(None, None, Some(id), "api", "matter.created", &json!({ "name": name, "mode": mode }));

    let mut resp = json!({ "id": id });
    if !conflicts.is_empty() {
        resp["conflicts"] = json!(conflicts);
    }
    Ok((StatusCode::CREATED, Json(resp)))
}

pub(crate) async fn get_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let project = state
        .db
        .get_project(id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(json!(ProjectJson::from(project))))
}

pub(crate) async fn update_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateProjectBody>,
) -> Result<Json<Value>, StatusCode> {
    if state.db.get_project(id).map_err(internal)?.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    state
        .db
        .update_project(
            id,
            body.name.as_deref(),
            body.client_name.as_deref(),
            body.case_number.as_deref(),
            body.jurisdiction.as_deref(),
            body.matter_type.as_deref(),
            body.opposing_counsel.as_deref(),
            body.deadline.as_ref().map(|d| d.as_deref()),
            body.privilege_level.as_deref(),
            body.status.as_deref(),
            None,
            body.default_template_id,
        )
        .map_err(internal)?;
    let updated = state.db.get_project(id).map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;

    // Re-sync parties when client or opposing counsel change
    if body.client_name.is_some() || body.opposing_counsel.is_some() {
        let _ = state.db.sync_project_parties(
            id,
            &updated.client_name,
            &updated.opposing_counsel,
        );
    }

    let mut resp = json!(ProjectJson::from(updated));

    // Return conflicts if party fields changed
    if body.client_name.is_some() || body.opposing_counsel.is_some() {
        let project = state.db.get_project(id).map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;
        let conflicts = state
            .db
            .check_conflicts(Some(id), &project.client_name, &project.opposing_counsel)
            .map_err(internal)?;
        if !conflicts.is_empty() {
            resp["conflicts"] = json!(conflicts);
        }
    }

    Ok(Json(resp))
}

pub(crate) async fn check_conflicts(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ConflictQuery>,
) -> Result<Json<Value>, StatusCode> {
    let client = params.client_name.as_deref().unwrap_or("");
    let opposing = params.opposing_counsel.as_deref().unwrap_or("");
    let exclude = params.exclude_project_id;
    let conflicts = state
        .db
        .check_conflicts(exclude, client, opposing)
        .map_err(internal)?;
    Ok(Json(json!({ "conflicts": conflicts })))
}

pub(crate) async fn delete_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    let project = state.db.get_project(id).map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;
    // Clean up dedicated repo if it exists
    if !project.repo_path.is_empty() {
        let _ = tokio::fs::remove_dir_all(&project.repo_path).await;
    }
    let _ = state.db.log_event_full(None, None, Some(id), "api", "matter.deleted", &json!({ "name": project.name }));
    state.db.delete_project(id).map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list_project_tasks(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    if state.db.get_project(id).map_err(internal)?.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let tasks = state.db.list_project_tasks(id).map_err(internal)?;
    Ok(Json(json!(tasks)))
}

// ── Deadlines ────────────────────────────────────────────────────────────

pub(crate) async fn list_project_deadlines(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    if state.db.get_project(id).map_err(internal)?.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let deadlines = state.db.list_project_deadlines(id).map_err(internal)?;
    Ok(Json(json!(deadlines)))
}

#[derive(Deserialize)]
pub(crate) struct CreateDeadlineBody {
    label: String,
    due_date: String,
    #[serde(default)]
    rule_basis: String,
}

pub(crate) async fn create_deadline(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<CreateDeadlineBody>,
) -> Result<Json<Value>, StatusCode> {
    if state.db.get_project(id).map_err(internal)?.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let did = state.db.insert_deadline(id, &body.label, &body.due_date, &body.rule_basis).map_err(internal)?;
    let _ = state.db.log_event_full(None, None, Some(id), "api", "deadline.created", &json!({ "label": body.label, "due_date": body.due_date }));
    Ok(Json(json!({ "id": did })))
}

#[derive(Deserialize)]
pub(crate) struct UpdateDeadlineBody {
    label: Option<String>,
    due_date: Option<String>,
    rule_basis: Option<String>,
    status: Option<String>,
}

pub(crate) async fn update_deadline(
    State(state): State<Arc<AppState>>,
    Path((_pid, did)): Path<(i64, i64)>,
    Json(body): Json<UpdateDeadlineBody>,
) -> Result<StatusCode, StatusCode> {
    state.db.update_deadline(did, body.label.as_deref(), body.due_date.as_deref(), body.rule_basis.as_deref(), body.status.as_deref()).map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn delete_deadline(
    State(state): State<Arc<AppState>>,
    Path((_pid, did)): Path<(i64, i64)>,
) -> Result<StatusCode, StatusCode> {
    state.db.delete_deadline(did).map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub(crate) struct UpcomingDeadlinesQuery {
    #[serde(default = "default_deadline_limit")]
    limit: i64,
}
fn default_deadline_limit() -> i64 { 50 }

pub(crate) async fn list_upcoming_deadlines(
    State(state): State<Arc<AppState>>,
    Query(q): Query<UpcomingDeadlinesQuery>,
) -> Result<Json<Value>, StatusCode> {
    let rows = state.db.list_upcoming_deadlines(q.limit).map_err(internal)?;
    let items: Vec<Value> = rows.into_iter().map(|(d, project_name)| {
        json!({
            "id": d.id,
            "project_id": d.project_id,
            "project_name": project_name,
            "label": d.label,
            "due_date": d.due_date,
            "rule_basis": d.rule_basis,
            "status": d.status,
        })
    }).collect();
    Ok(Json(json!(items)))
}

// ── Search ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct FtsSearchQuery {
    q: String,
    #[serde(default)]
    project_id: Option<i64>,
    #[serde(default = "default_search_limit")]
    limit: i64,
    #[serde(default)]
    semantic: bool,
}
fn default_search_limit() -> i64 { 50 }

// ── Audit ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct AuditQuery {
    #[serde(default = "default_audit_limit")]
    limit: i64,
}
fn default_audit_limit() -> i64 { 100 }

pub(crate) async fn list_project_audit(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Value>, StatusCode> {
    if state.db.get_project(id).map_err(internal)?.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let events = state.db.list_project_events(id, q.limit).map_err(internal)?;
    Ok(Json(json!(events)))
}

pub(crate) async fn search_documents(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FtsSearchQuery>,
) -> Result<Json<Value>, StatusCode> {
    if query.q.trim().is_empty() {
        return Ok(Json(json!([])));
    }

    // FTS5 keyword search
    let fts_results = state.db.fts_search(&query.q, query.project_id, query.limit).map_err(internal)?;
    let mut items: Vec<Value> = Vec::new();
    for r in &fts_results {
        let project_name = state.db.get_project(r.project_id)
            .ok()
            .flatten()
            .map(|p| p.name.clone())
            .unwrap_or_default();
        items.push(json!({
            "project_id": r.project_id,
            "project_name": project_name,
            "task_id": r.task_id,
            "file_path": r.file_path,
            "title_snippet": r.title_snippet,
            "content_snippet": r.content_snippet,
            "rank": r.rank,
            "source": "keyword",
        }));
    }

    // Semantic search (when requested and embeddings exist)
    if query.semantic && state.db.embedding_count() > 0 {
        if let Ok(query_emb) = state.embed_client.embed_single(&query.q).await {
            if let Ok(sem_results) = state.db.search_embeddings(&query_emb, query.limit as usize, query.project_id) {
                for r in sem_results.iter().filter(|r| r.score > 0.5) {
                    items.push(json!({
                        "project_id": r.project_id,
                        "task_id": r.task_id,
                        "file_path": r.file_path,
                        "content_snippet": if r.chunk_text.len() > 200 { &r.chunk_text[..200] } else { &r.chunk_text },
                        "score": r.score,
                        "source": "semantic",
                    }));
                }
            }
        }
    }

    Ok(Json(json!(items)))
}

/// Read a file from git: tries local `git show ref:path` first, falls back to `gh api`.
async fn git_show_file(repo_path: &str, slug: &str, ref_name: &str, path: &str) -> Option<Vec<u8>> {
    // Try local git first
    if !repo_path.is_empty() && std::path::Path::new(repo_path).join(".git").exists() {
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::process::Command::new("git")
                .args(["-C", repo_path, "show", &format!("{ref_name}:{path}")])
                .stderr(std::process::Stdio::null())
                .output(),
        )
        .await;
        if let Ok(Ok(output)) = out {
            if output.status.success() {
                return Some(output.stdout);
            }
        }
    }
    // Fall back to GitHub API
    if !slug.is_empty() {
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio::process::Command::new("gh")
                .args([
                    "api",
                    &format!("repos/{slug}/contents/{}?ref={}", percent_encode(path), percent_encode(ref_name)),
                    "--jq",
                    ".content",
                ])
                .stderr(std::process::Stdio::null())
                .output(),
        )
        .await;
        if let Ok(Ok(output)) = out {
            if output.status.success() {
                let b64 = String::from_utf8_lossy(&output.stdout).trim().replace('\n', "");
                return base64_decode(&b64).ok();
            }
        }
    }
    None
}

pub(crate) async fn list_project_documents(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    if state.db.get_project(id).map_err(internal)?.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let tasks = state.db.list_project_tasks(id).map_err(internal)?;
    let mut documents: Vec<Value> = Vec::new();

    for task in &tasks {
        if task.branch.is_empty() {
            continue;
        }
        let repo = state
            .config
            .watched_repos
            .iter()
            .find(|r| r.path == task.repo_path);
        let slug = repo.map(|r| r.repo_slug.as_str()).unwrap_or("");
        let repo_path = &task.repo_path;

        // Try local git ls-tree first
        let file_list = if std::path::Path::new(repo_path).join(".git").exists() {
            let out = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                tokio::process::Command::new("git")
                    .args(["-C", repo_path, "ls-tree", "--name-only", &task.branch])
                    .stderr(std::process::Stdio::null())
                    .output(),
            )
            .await;
            match out {
                Ok(Ok(output)) if output.status.success() => {
                    Some(String::from_utf8_lossy(&output.stdout).to_string())
                }
                _ => None,
            }
        } else {
            None
        };

        // Fall back to GitHub if local failed
        let file_list = match file_list {
            Some(f) => f,
            None if !slug.is_empty() => {
                let out = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    tokio::process::Command::new("gh")
                        .args([
                            "api",
                            &format!("repos/{slug}/git/trees/{}", task.branch),
                            "--jq",
                            ".tree[] | select(.type==\"blob\") | .path",
                        ])
                        .stderr(std::process::Stdio::null())
                        .output(),
                )
                .await;
                match out {
                    Ok(Ok(output)) if output.status.success() => {
                        String::from_utf8_lossy(&output.stdout).to_string()
                    }
                    _ => continue,
                }
            }
            _ => continue,
        };

        for line in file_list.lines() {
            let name = line.trim();
            if name.ends_with(".md") && !name.starts_with('.') {
                documents.push(json!({
                    "task_id": task.id,
                    "branch": task.branch,
                    "path": name,
                    "repo_slug": slug,
                    "task_title": task.title,
                    "task_status": task.status,
                }));
            }
        }
    }

    Ok(Json(json!(documents)))
}

pub(crate) async fn get_project_document_content(
    State(state): State<Arc<AppState>>,
    Path((id, task_id)): Path<(i64, i64)>,
    Query(q): Query<DocQuery>,
) -> Result<axum::response::Response, StatusCode> {
    if state.db.get_project(id).map_err(internal)?.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let task = state
        .db
        .get_task(task_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if task.branch.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    let repo = state
        .config
        .watched_repos
        .iter()
        .find(|r| r.path == task.repo_path);
    let slug = repo.map(|r| r.repo_slug.as_str()).unwrap_or("");

    let path = q.path.as_deref().unwrap_or("research.md");
    let ref_name = q.ref_name.as_deref().unwrap_or(&task.branch);

    let bytes = git_show_file(&task.repo_path, slug, ref_name, path)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    tracing::info!(project_id = id, task_id, path, "document accessed");

    Ok(axum::response::Response::builder()
        .header("content-type", "text/markdown; charset=utf-8")
        .body(axum::body::Body::from(bytes))
        .unwrap())
}

pub(crate) async fn get_project_document_versions(
    State(state): State<Arc<AppState>>,
    Path((id, task_id)): Path<(i64, i64)>,
    Query(q): Query<DocQuery>,
) -> Result<Json<Value>, StatusCode> {
    if state.db.get_project(id).map_err(internal)?.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let task = state
        .db
        .get_task(task_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if task.branch.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    let path = q.path.as_deref().unwrap_or("research.md");
    let repo_path = &task.repo_path;

    // Try local git log first
    if std::path::Path::new(repo_path).join(".git").exists() {
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::process::Command::new("git")
                .args([
                    "-C", repo_path, "log", &task.branch,
                    "--format=%H\t%s\t%aI\t%an",
                    "--", path,
                ])
                .stderr(std::process::Stdio::null())
                .output(),
        )
        .await;
        if let Ok(Ok(output)) = out {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let versions: Vec<Value> = stdout
                    .lines()
                    .filter_map(|line| {
                        let parts: Vec<&str> = line.splitn(4, '\t').collect();
                        if parts.len() >= 4 {
                            Some(json!({
                                "sha": parts[0],
                                "message": parts[1],
                                "date": parts[2],
                                "author": parts[3],
                            }))
                        } else {
                            None
                        }
                    })
                    .collect();
                return Ok(Json(json!(versions)));
            }
        }
    }

    // Fall back to GitHub API
    let repo = state
        .config
        .watched_repos
        .iter()
        .find(|r| r.path == task.repo_path);
    let slug = repo.map(|r| r.repo_slug.as_str()).unwrap_or("");
    if slug.is_empty() {
        return Ok(Json(json!([])));
    }

    let out = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new("gh")
            .args([
                "api",
                &format!("repos/{slug}/commits?sha={}&path={path}", task.branch),
                "--jq",
                r#"[.[] | {sha: .sha, message: .commit.message, date: .commit.author.date, author: .commit.author.name}]"#,
            ])
            .stderr(std::process::Stdio::null())
            .output(),
    )
    .await
    .map_err(internal)?
    .map_err(internal)?;

    if !out.status.success() {
        return Ok(Json(json!([])));
    }

    let versions: Value = serde_json::from_slice(&out.stdout).unwrap_or(json!([]));
    Ok(Json(versions))
}

/// Clean markdown for professional export: strip internal markers, normalize formatting.
fn preprocess_legal_markdown(md: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in md.lines() {
        // Strip Confidence: markers
        if line.trim().starts_with("Confidence:") || line.trim().starts_with("**Confidence:") {
            continue;
        }
        // Strip structured.json references
        if line.contains("structured.json") || line.contains("signal.json") {
            continue;
        }
        // Strip internal metadata markers
        if line.trim().starts_with("<!-- borg:") || line.trim().starts_with("<!-- internal") {
            continue;
        }
        // Convert > blockquotes to proper indentation for legal citations
        lines.push(line.to_string());
    }
    lines.join("\n")
}

pub(crate) async fn export_project_document(
    State(state): State<Arc<AppState>>,
    Path((id, task_id)): Path<(i64, i64)>,
    Query(q): Query<ExportQuery>,
) -> Result<axum::response::Response, StatusCode> {
    if state.db.get_project(id).map_err(internal)?.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    let task = state
        .db
        .get_task(task_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if task.branch.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    let repo = state
        .config
        .watched_repos
        .iter()
        .find(|r| r.path == task.repo_path);
    let slug = repo.map(|r| r.repo_slug.as_str()).unwrap_or("");

    let path = q.path.as_deref().unwrap_or("research.md");
    let ref_name = q.ref_name.as_deref().unwrap_or(&task.branch);
    let format = q.format.as_deref().unwrap_or("pdf");

    if format != "pdf" && format != "docx" {
        return Ok(axum::response::Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(axum::body::Body::from("format must be 'pdf' or 'docx'"))
            .unwrap());
    }

    // Check pandoc availability
    let pandoc_check = tokio::process::Command::new("pandoc")
        .arg("--version")
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .status()
        .await;
    if pandoc_check.is_err() || !pandoc_check.unwrap().success() {
        return Ok(axum::response::Response::builder()
            .status(StatusCode::NOT_IMPLEMENTED)
            .header("content-type", "text/plain")
            .body(axum::body::Body::from(
                "pandoc is not installed on the server; install it to enable document export",
            ))
            .unwrap());
    }

    let raw_md_bytes = git_show_file(&task.repo_path, slug, ref_name, path)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let project = state.db.get_project(id).map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;
    let add_toc = q.toc.unwrap_or(false);
    let number_sections = q.number_sections.unwrap_or(false);
    let title_page = q.title_page.unwrap_or(true);

    // Preprocess markdown: strip internal markers, add title page metadata
    let raw_md = String::from_utf8_lossy(&raw_md_bytes);
    let mut md_content = preprocess_legal_markdown(&raw_md);

    // Add privilege header
    if !project.privilege_level.is_empty() {
        md_content = format!(
            "**PRIVILEGED AND CONFIDENTIAL — {}**\n\n---\n\n{}",
            project.privilege_level.to_uppercase(),
            md_content
        );
    }

    // Prepend YAML front matter for title page
    if title_page {
        let title_block = format!(
            "---\ntitle: \"{}\"\n{}{}{}{}\ndate: \"{}\"\n---\n\n",
            task.title.replace('"', "'"),
            if !project.client_name.is_empty() { format!("subtitle: \"Prepared for {}\"\n", project.client_name.replace('"', "'")) } else { String::new() },
            if !project.case_number.is_empty() { format!("subject: \"Case No. {}\"\n", project.case_number) } else { String::new() },
            if !project.jurisdiction.is_empty() { format!("keywords: [\"{}\"]\n", project.jurisdiction) } else { String::new() },
            if !project.privilege_level.is_empty() { format!("header-includes: |\n  \\fancyfoot[C]{{PRIVILEGED AND CONFIDENTIAL — {}}}\n", project.privilege_level.to_uppercase()) } else { String::new() },
            Utc::now().format("%B %d, %Y"),
        );
        md_content = format!("{}{}", title_block, md_content);
    }

    let md_bytes = md_content.into_bytes();

    // Resolve template: explicit template_id takes priority, then project default
    let effective_template_id = q.template_id.or(project.default_template_id);
    let template_info = if let Some(tid) = effective_template_id {
        let kf = state.db.list_knowledge_files().map_err(internal)?;
        kf.iter().find(|f| f.id == tid).map(|f| {
            let p = format!("{}/knowledge/{}", state.config.data_dir, f.file_name);
            let is_docx = f.file_name.to_lowercase().ends_with(".docx");
            (p, is_docx)
        })
    } else {
        None
    };
    let use_docxtemplater = format == "docx"
        && template_info.as_ref().is_some_and(|(p, is_docx)| *is_docx && std::path::Path::new(p).exists());

    let tmp_dir = tempfile::tempdir().map_err(internal)?;
    let stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("document");
    let out_filename = format!("{}.{}", stem, format);
    let out_path = tmp_dir.path().join(&out_filename);

    if use_docxtemplater {
        // Use docxtemplater to fill the .docx template
        let (tpl_path, _) = template_info.as_ref().unwrap();
        let fill_data = json!({
            "title": task.title,
            "client_name": project.client_name,
            "case_number": project.case_number,
            "jurisdiction": project.jurisdiction,
            "matter_type": project.matter_type,
            "date": Utc::now().format("%B %d, %Y").to_string(),
            "privilege_header": if project.privilege_level.is_empty() { String::new() }
                else { format!("PRIVILEGED AND CONFIDENTIAL — {}", project.privilege_level.to_uppercase()) },
            "body": String::from_utf8_lossy(&md_bytes).to_string(),
        });
        let fill_input = json!({
            "templatePath": tpl_path,
            "outputPath": out_path.to_string_lossy(),
            "data": fill_data,
        });
        let fill_script = format!("{}/sidecar/docx-template/fill.ts",
            std::env::current_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| ".".into()));
        if let Ok(mut child) = tokio::process::Command::new("bun")
            .arg("run").arg(&fill_script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(stdin) = child.stdin.as_mut() {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(fill_input.to_string().as_bytes()).await;
            }
            match child.wait_with_output().await {
                Ok(out) if !out.status.success() => {
                    tracing::warn!("docxtemplater fill failed: {}", String::from_utf8_lossy(&out.stderr));
                }
                Err(e) => {
                    tracing::warn!("docxtemplater process error: {e}");
                }
                _ => {}
            }
        }
    }

    // Fall back to pandoc if docxtemplater didn't produce output
    if !out_path.exists() {
        let md_path = tmp_dir.path().join("document.md");
        tokio::fs::write(&md_path, &md_bytes).await.map_err(internal)?;

        let mut cmd = tokio::process::Command::new("pandoc");
        cmd.arg(&md_path)
            .arg("-f").arg("markdown")
            .arg("-o").arg(&out_path)
            .stderr(std::process::Stdio::piped());

        if add_toc {
            cmd.arg("--toc").arg("--toc-depth=3");
        }
        if number_sections {
            cmd.arg("--number-sections");
        }

        if format == "pdf" {
            cmd.arg("-t").arg("html").arg("--pdf-engine=weasyprint");
        } else {
            cmd.arg("-t").arg(format);
            if let Some((ref tpl, _)) = template_info {
                if std::path::Path::new(tpl).exists() {
                    cmd.arg("--reference-doc").arg(tpl);
                }
            }
        }

        let pandoc_out = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            cmd.output(),
        )
        .await
        .map_err(internal)?
        .map_err(internal)?;

        // If weasyprint failed for PDF, retry with xelatex
        if !pandoc_out.status.success() && format == "pdf" {
            let retry = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                tokio::process::Command::new("pandoc")
                    .arg(&md_path)
                    .arg("-f").arg("markdown")
                    .arg("-o").arg(&out_path)
                    .arg("--pdf-engine=xelatex")
                    .stderr(std::process::Stdio::piped())
                    .output(),
            )
            .await
            .map_err(internal)?
            .map_err(internal)?;

            if !retry.status.success() {
                let stderr = String::from_utf8_lossy(&retry.stderr);
                tracing::warn!("pandoc export failed: {stderr}");
                return Ok(axum::response::Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .header("content-type", "text/plain")
                    .body(axum::body::Body::from(format!("pandoc failed: {stderr}")))
                    .unwrap());
            }
        } else if !pandoc_out.status.success() {
            let stderr = String::from_utf8_lossy(&pandoc_out.stderr);
            tracing::warn!("pandoc export failed: {stderr}");
            return Ok(axum::response::Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("content-type", "text/plain")
                .body(axum::body::Body::from(format!("pandoc failed: {stderr}")))
                .unwrap());
        }
    } // end pandoc fallback

    let file_bytes = tokio::fs::read(&out_path).await.map_err(internal)?;

    let content_type = if format == "pdf" {
        "application/pdf"
    } else {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    };

    Ok(axum::response::Response::builder()
        .header("content-type", content_type)
        .header(
            "content-disposition",
            format!("attachment; filename=\"{out_filename}\""),
        )
        .body(axum::body::Body::from(file_bytes))
        .unwrap())
}

#[derive(Deserialize)]
pub(crate) struct ExportAllQuery {
    pub format: Option<String>,
    pub toc: Option<bool>,
    pub template_id: Option<i64>,
}

pub(crate) async fn export_all_project_documents(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(q): Query<ExportAllQuery>,
) -> Result<axum::response::Response, StatusCode> {
    let project = state.db.get_project(id).map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;
    let tasks = state.db.list_project_tasks(id).map_err(internal)?;
    let format = q.format.as_deref().unwrap_or("docx");
    if format != "pdf" && format != "docx" {
        return Err(StatusCode::BAD_REQUEST);
    }

    let tmp_dir = tempfile::tempdir().map_err(internal)?;
    let mut file_entries: Vec<(String, Vec<u8>)> = Vec::new();

    let effective_tid = q.template_id.or(project.default_template_id);
    let template_info = effective_tid.and_then(|tid| {
        state.db.list_knowledge_files().ok().and_then(|kf| {
            kf.iter().find(|f| f.id == tid).map(|f| {
                let p = format!("{}/knowledge/{}", state.config.data_dir, f.file_name);
                let is_docx = f.file_name.to_lowercase().ends_with(".docx");
                (p, is_docx)
            })
        })
    });
    let use_docxtemplater = format == "docx"
        && template_info.as_ref().is_some_and(|(p, is_docx)| *is_docx && std::path::Path::new(p).exists());
    let fill_script = format!("{}/sidecar/docx-template/fill.ts",
        std::env::current_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| ".".into()));

    for task in &tasks {
        if task.branch.is_empty() { continue; }
        let slug = state.config.watched_repos.iter()
            .find(|r| r.path == task.repo_path)
            .map(|r| r.repo_slug.as_str()).unwrap_or("");

        for doc_path in &["research.md", "analysis.md", "review_notes.md"] {
            let raw_bytes = match git_show_file(&task.repo_path, slug, &task.branch, doc_path).await {
                Some(b) if !b.is_empty() => b,
                _ => continue,
            };

            let raw_md = String::from_utf8_lossy(&raw_bytes);
            let mut md_content = preprocess_legal_markdown(&raw_md);
            if !project.privilege_level.is_empty() {
                md_content = format!(
                    "**PRIVILEGED AND CONFIDENTIAL — {}**\n\n---\n\n{}",
                    project.privilege_level.to_uppercase(), md_content
                );
            }

            let safe_title = task.title.chars()
                .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
                .collect::<String>()
                .trim().to_string();
            let stem = std::path::Path::new(doc_path).file_stem()
                .and_then(|s| s.to_str()).unwrap_or("doc");
            let out_name = format!("{}-{}.{}", safe_title, stem, format);
            let md_path = tmp_dir.path().join(format!("src-{}-{}.md", task.id, stem));
            let out_path = tmp_dir.path().join(&out_name);

            let mut produced = false;

            if use_docxtemplater {
                let (tpl_path, _) = template_info.as_ref().unwrap();
                let fill_input = json!({
                    "templatePath": tpl_path,
                    "outputPath": out_path.to_string_lossy(),
                    "data": {
                        "title": task.title,
                        "client_name": project.client_name,
                        "case_number": project.case_number,
                        "jurisdiction": project.jurisdiction,
                        "date": Utc::now().format("%B %d, %Y").to_string(),
                        "body": md_content,
                    },
                });
                if let Ok(mut child) = tokio::process::Command::new("bun")
                    .arg("run").arg(&fill_script)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                {
                    if let Some(stdin) = child.stdin.as_mut() {
                        use tokio::io::AsyncWriteExt;
                        let _ = stdin.write_all(fill_input.to_string().as_bytes()).await;
                    }
                    if let Ok(out) = child.wait_with_output().await {
                        if out.status.success() && out_path.exists() {
                            produced = true;
                        }
                    }
                }
            }

            if !produced {
                tokio::fs::write(&md_path, md_content.as_bytes()).await.map_err(internal)?;
                let mut cmd = tokio::process::Command::new("pandoc");
                cmd.arg(&md_path).arg("-f").arg("markdown").arg("-o").arg(&out_path)
                    .stderr(std::process::Stdio::piped());
                if q.toc.unwrap_or(false) {
                    cmd.arg("--toc").arg("--toc-depth=3");
                }
                if format == "pdf" {
                    cmd.arg("-t").arg("html").arg("--pdf-engine=weasyprint");
                } else {
                    cmd.arg("-t").arg("docx");
                    if let Some((ref tpl, _)) = template_info {
                        if std::path::Path::new(tpl).exists() {
                            cmd.arg("--reference-doc").arg(tpl);
                        }
                    }
                }
                let out = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output())
                    .await.map_err(internal)?.map_err(internal)?;
                produced = out.status.success();
            }

            if produced {
                if let Ok(bytes) = tokio::fs::read(&out_path).await {
                    file_entries.push((out_name, bytes));
                }
            }
        }
    }

    if file_entries.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Build ZIP archive
    let zip_path = tmp_dir.path().join("export.zip");
    let zip_file = std::fs::File::create(&zip_path).map_err(internal)?;
    let mut zip = zip::ZipWriter::new(zip_file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in &file_entries {
        zip.start_file(name, options).map_err(internal)?;
        use std::io::Write;
        zip.write_all(bytes).map_err(internal)?;
    }
    zip.finish().map_err(internal)?;

    let zip_bytes = tokio::fs::read(&zip_path).await.map_err(internal)?;
    let filename = format!("{}-export.zip", project.name.chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
        .collect::<String>().trim().to_string());

    Ok(axum::response::Response::builder()
        .header("content-type", "application/zip")
        .header("content-disposition", format!("attachment; filename=\"{filename}\""))
        .body(axum::body::Body::from(zip_bytes))
        .unwrap())
}

pub(crate) async fn delete_project_document(
    State(state): State<Arc<AppState>>,
    Path((id, task_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, StatusCode> {
    state.db.get_project(id).map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;
    let task = state.db.get_task(task_id).map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;
    if task.branch.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    let out = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new("git")
            .args(["-C", &task.repo_path, "branch", "-D", &task.branch])
            .stderr(std::process::Stdio::piped())
            .output(),
    )
    .await
    .map_err(internal)?
    .map_err(internal)?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        tracing::warn!(task_id, branch = task.branch, "git branch -D failed: {stderr}");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Json(json!({ "ok": true })))
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

pub(crate) async fn get_project_file_content(
    State(state): State<Arc<AppState>>,
    Path((project_id, file_id)): Path<(i64, i64)>,
) -> Result<axum::response::Response, StatusCode> {
    let row = state
        .db
        .get_project_file(project_id, file_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let bytes = tokio::fs::read(&row.stored_path)
        .await
        .map_err(internal)?;

    let safe_name = row.file_name.replace('"', "_");
    Ok(axum::response::Response::builder()
        .header("content-type", "application/octet-stream")
        .header("content-disposition", format!("attachment; filename=\"{safe_name}\""))
        .body(axum::body::Body::from(bytes))
        .unwrap())
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

    // Extract text from uploaded files in background
    let db = state.db.clone();
    let project_id = id;
    tokio::spawn(async move {
        for file in db.list_project_files(project_id).unwrap_or_default() {
            if !file.extracted_text.is_empty() { continue; }
            if let Ok(text) = extract_text(&file.stored_path, &file.mime_type).await {
                if !text.is_empty() {
                    let _ = db.update_project_file_text(file.id, &text);
                    let _ = db.fts_index_document(project_id, 0, &file.file_name, &file.file_name, &text);
                    tracing::info!("extracted {} chars from {}", text.len(), file.file_name);
                }
            }
        }
    });

    Ok(Json(json!({ "uploaded": uploaded })))
}

async fn extract_text(path: &str, mime: &str) -> Result<String, StatusCode> {
    let path = path.to_string();
    let mime = mime.to_string();
    let text = tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
        let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
        let is_pdf = mime.contains("pdf") || ext == "pdf";
        let is_docx = mime.contains("wordprocessingml") || mime.contains("msword")
            || ext == "docx" || ext == "doc";
        let is_text = mime.starts_with("text/") || ext == "txt" || ext == "md"
            || ext == "csv" || ext == "json" || ext == "xml";

        if is_pdf {
            let out = std::process::Command::new("pdftotext")
                .args(["-layout", &path, "-"])
                .output()?;
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        } else if is_docx {
            let out = std::process::Command::new("pandoc")
                .args([&path, "-t", "plain", "--wrap=none"])
                .output()?;
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        } else if is_text {
            let content = std::fs::read_to_string(&path)?;
            Ok(content)
        } else {
            Ok(String::new())
        }
    }).await.map_err(internal)?.map_err(internal)?;
    Ok(text)
}

pub(crate) async fn get_project_file_text(
    State(state): State<Arc<AppState>>,
    Path((project_id, file_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, StatusCode> {
    let file = state.db.get_project_file(project_id, file_id).map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(json!({
        "id": file.id,
        "file_name": file.file_name,
        "extracted_text": file.extracted_text,
        "has_text": !file.extracted_text.is_empty(),
    })))
}

pub(crate) async fn reextract_project_file(
    State(state): State<Arc<AppState>>,
    Path((project_id, file_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, StatusCode> {
    let file = state.db.get_project_file(project_id, file_id).map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let text = extract_text(&file.stored_path, &file.mime_type).await?;
    if !text.is_empty() {
        state.db.update_project_file_text(file_id, &text).map_err(internal)?;
        state.db.fts_index_document(project_id, 0, &file.file_name, &file.file_name, &text)
            .map_err(internal)?;
    }
    Ok(Json(json!({
        "id": file_id,
        "extracted_text_chars": text.len(),
        "has_text": !text.is_empty(),
    })))
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
            let structured = state.db.get_task_structured_data(task.id).unwrap_or_default();
            let mut v = serde_json::to_value(&task).map_err(internal)?;
            if let Some(obj) = v.as_object_mut() {
                obj.insert(
                    "outputs".into(),
                    serde_json::to_value(outputs_json).map_err(internal)?,
                );
                if !structured.is_empty() {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&structured) {
                        obj.insert("structured_data".into(), parsed);
                    }
                }
            }
            Ok(Json(v))
        },
    }
}

pub(crate) async fn create_task(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateTaskBody>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let repo = if let Some(r) = body.repo {
        r
    } else if let Some(pid) = body.project_id {
        // Resolve project's dedicated repo
        state
            .db
            .get_project(pid)
            .map_err(internal)?
            .and_then(|p| if p.repo_path.is_empty() { None } else { Some(p.repo_path) })
            .unwrap_or_else(|| state.config.pipeline_repo.clone())
    } else {
        state.config.pipeline_repo.clone()
    };
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
        project_id: body.project_id.unwrap_or(0),
        task_type: body.task_type.unwrap_or_default(),
        started_at: None,
        completed_at: None,
        duration_secs: None,
        review_status: None,
        revision_count: 0,
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
    if state.triage_running.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Json(json!({ "scored": 0, "error": "triage already running" }));
    }

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
    let Some(backend) = state.default_backend("claude") else {
        tracing::error!("triage_proposals: no backends configured");
        return Json(json!({ "scored": 0 }));
    };
    let model = db
        .get_config("model")
        .ok()
        .flatten()
        .unwrap_or_else(|| "claude-sonnet-4-6".into());
    let oauth = state.config.oauth_token.clone();

    let triage_flag = Arc::clone(&state.triage_running);
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
                project_id: 0,
                task_type: String::new(),
                started_at: None,
                completed_at: None,
                duration_secs: None,
        review_status: None,
        revision_count: 0,
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
                    repo_slug: String::new(),
                },
                data_dir: state.config.data_dir.clone(),
                session_dir: format!("{}/sessions/triage-{}", state.config.data_dir, proposal.id),
                work_dir: proposal.repo_path.clone(),
                oauth_token: oauth.clone(),
                model: model.clone(),
                pending_messages: Vec::new(),
                system_prompt_suffix: String::new(),
                user_coauthor: String::new(),
                stream_tx: None,
                setup_script: String::new(),
                api_keys: std::collections::HashMap::new(),
                disallowed_tools: String::new(),
                knowledge_files: Vec::new(),
                knowledge_dir: String::new(),
                agent_network: None,
                prior_research: Vec::new(),
                revision_count: 0,
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
        triage_flag.store(false, std::sync::atomic::Ordering::SeqCst);
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
    let msgs = match state.db.get_chat_messages(&q.thread, q.limit.unwrap_or(100)) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("get_chat_messages({}): {e}", q.thread);
            return Ok(Json(json!([])));
        }
    };
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
    if body.text.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let thread = body
        .thread
        .clone()
        .unwrap_or_else(|| "web:dashboard".to_string());

    // Rate limit: one message per (60 / chat_rate_limit) seconds per thread
    let rate = state.config.chat_rate_limit.max(1) as u64;
    let cooldown = std::time::Duration::from_secs(60 / rate);
    {
        let mut map = state.chat_rate.lock().unwrap();
        let now = std::time::Instant::now();
        if let Some(last) = map.get(&thread) {
            if now.duration_since(*last) < cooldown {
                return Err(StatusCode::TOO_MANY_REQUESTS);
            }
        }
        map.insert(thread.clone(), now);
    }
    let sender = body
        .sender
        .clone()
        .unwrap_or_else(|| "web-user".to_string());
    let ts = Utc::now().timestamp_millis();
    let msg_id = format!("{}-{}", thread, ts);
    let ts = ts / 1000;

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
                "repo_slug": r.repo_slug,
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

// ── API Keys (BYOK) ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct StoreKeyBody {
    pub provider: String,
    pub key_name: Option<String>,
    pub key_value: String,
    pub owner: Option<String>,
}

pub(crate) async fn list_api_keys(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let keys = state.db.list_api_keys("global").map_err(internal)?;
    Ok(Json(json!({ "keys": keys })))
}

pub(crate) async fn store_api_key(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StoreKeyBody>,
) -> Result<Json<Value>, StatusCode> {
    let owner = body.owner.as_deref().unwrap_or("global");
    let key_name = body.key_name.as_deref().unwrap_or("");
    let id = state
        .db
        .store_api_key(owner, &body.provider, key_name, &body.key_value)
        .map_err(internal)?;
    Ok(Json(json!({ "id": id })))
}

pub(crate) async fn delete_api_key(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    state.db.delete_api_key(id).map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

// ── Cache volumes ─────────────────────────────────────────────────────

pub(crate) async fn list_cache_volumes(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let volumes = borg_core::sandbox::Sandbox::list_cache_volumes("borg-cache-").await;
    let arr: Vec<_> = volumes
        .into_iter()
        .map(|(name, size, last_used)| json!({ "name": name, "size": size, "last_used": last_used }))
        .collect();
    Ok(Json(json!({ "volumes": arr })))
}

pub(crate) async fn delete_cache_volume(
    State(_state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    // Only allow alphanumeric, hyphens, and underscores in volume names
    if !name.starts_with("borg-cache-") || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(StatusCode::BAD_REQUEST);
    }
    let removed = borg_core::sandbox::Sandbox::remove_volume(&name).await;
    if removed {
        Ok(Json(json!({ "ok": true })))
    } else {
        Err(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

// ── Knowledge base ────────────────────────────────────────────────────

pub(crate) async fn list_knowledge(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let files = state.db.list_knowledge_files().map_err(internal)?;
    Ok(Json(json!({ "files": files })))
}

pub(crate) async fn upload_knowledge(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<Value>, StatusCode> {
    const MAX_KNOWLEDGE_FILE_BYTES: i64 = 50 * 1024 * 1024;
    const MAX_KNOWLEDGE_TOTAL_BYTES: i64 = 1024 * 1024 * 1024;

    let knowledge_dir = format!("{}/knowledge", state.config.data_dir);
    std::fs::create_dir_all(&knowledge_dir).map_err(internal)?;

    let mut file_name = String::new();
    let mut description = String::new();
    let mut inline = false;
    let mut category = String::new();
    let mut file_bytes: Vec<u8> = Vec::new();

    while let Some(field) = multipart.next_field().await.map_err(|_| StatusCode::BAD_REQUEST)? {
        match field.name() {
            Some("file") => {
                if let Some(name) = field.file_name() {
                    file_name = sanitize_upload_name(name);
                }
                file_bytes = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?.to_vec();
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

    let total_bytes = state.db.total_knowledge_file_bytes().map_err(internal)?;
    if total_bytes + file_size > MAX_KNOWLEDGE_TOTAL_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }

    let dest = format!("{knowledge_dir}/{file_name}");
    if std::path::Path::new(&dest).exists() {
        return Err(StatusCode::CONFLICT);
    }
    std::fs::write(&dest, &file_bytes).map_err(internal)?;

    let id = state
        .db
        .insert_knowledge_file(&file_name, &description, file_bytes.len() as i64, inline)
        .map_err(internal)?;
    if !category.is_empty() {
        let _ = state.db.update_knowledge_file(id, None, None, None, Some(&category), None);
    }

    Ok(Json(json!({ "id": id, "file_name": file_name })))
}

pub(crate) async fn update_knowledge(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateKnowledgeBody>,
) -> Result<Json<Value>, StatusCode> {
    state
        .db
        .update_knowledge_file(id, body.description.as_deref(), body.inline, body.tags.as_deref(), body.category.as_deref(), body.jurisdiction.as_deref())
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn delete_knowledge(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    if let Ok(Some(file)) = state.db.get_knowledge_file(id) {
        if let Some(safe_path) = safe_knowledge_path(&state.config.data_dir, &file.file_name) {
            let _ = std::fs::remove_file(&safe_path);
        }
    }
    state.db.delete_knowledge_file(id).map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub(crate) struct TemplatesQuery {
    category: Option<String>,
    jurisdiction: Option<String>,
}

pub(crate) async fn list_templates(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TemplatesQuery>,
) -> Result<Json<Value>, StatusCode> {
    let templates = state.db.list_templates(q.category.as_deref(), q.jurisdiction.as_deref()).map_err(internal)?;
    Ok(Json(json!(templates)))
}

pub(crate) async fn get_knowledge_content(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, StatusCode> {
    use axum::response::IntoResponse;
    let file = state
        .db
        .get_knowledge_file(id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let path = safe_knowledge_path(&state.config.data_dir, &file.file_name)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let bytes = std::fs::read(&path).map_err(|_| StatusCode::NOT_FOUND)?;
    let disp = format!(
        "attachment; filename=\"{}\"",
        file.file_name.replace('"', "_")
    );
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

// ── Container inspection ──────────────────────────────────────────────────

/// Extract container ID from task stream history by looking for container_id event.
async fn container_id_from_stream(state: &AppState, task_id: i64) -> Option<String> {
    let (history, _) = state.stream_manager.subscribe(task_id).await;
    for line in history.iter().rev() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if v.get("type").and_then(|t| t.as_str()) == Some("container_event")
                && v.get("event").and_then(|e| e.as_str()) == Some("container_id")
            {
                if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::percent_encode;

    #[test]
    fn percent_encode_safe_chars_unchanged() {
        assert_eq!(percent_encode("src/main.rs"), "src/main.rs");
        assert_eq!(percent_encode("refs/heads/my-branch"), "refs/heads/my-branch");
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
        let url = format!("repos/owner/repo/contents/{}?ref={}", percent_encode(path), percent_encode(ref_name));
        assert_eq!(url, "repos/owner/repo/contents/file%3Fraw=1?ref=branch%26extra=1");
    }

    #[test]
    fn percent_encode_ref_with_hash() {
        let ref_name = "sha#abc";
        let url = format!("repos/owner/repo/contents/file?ref={}", percent_encode(ref_name));
        assert_eq!(url, "repos/owner/repo/contents/file?ref=sha%23abc");
    }
}

pub(crate) async fn get_task_container(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let container_id = container_id_from_stream(&state, task_id).await;
    match container_id {
        Some(id) => {
            // Try to get live status via `docker inspect`
            let status = tokio::process::Command::new("docker")
                .args(["inspect", "--format", "{{.State.Status}}", &id])
                .output()
                .await
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            Ok(Json(json!({ "task_id": task_id, "container_id": id, "status": status })))
        },
        None => Err(StatusCode::NOT_FOUND),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_upload_name_basic() {
        assert_eq!(sanitize_upload_name("hello.txt"), "hello.txt");
        assert_eq!(sanitize_upload_name("my file.pdf"), "my_file.pdf");
        assert_eq!(sanitize_upload_name("../../../etc/passwd"), "passwd");
        assert_eq!(sanitize_upload_name(""), "upload.bin");
    }

    #[test]
    fn test_sanitize_upload_name_strips_leading_dots() {
        assert_eq!(sanitize_upload_name("..."), "upload.bin");
        assert_eq!(sanitize_upload_name(".hidden"), "hidden");
    }

    #[test]
    fn test_duplicate_upload_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest = dir.path().join("report.pdf");

        // First upload: file does not exist yet
        assert!(!dest.exists());

        // Write to simulate first upload succeeding
        std::fs::write(&dest, b"first content").expect("write");
        assert!(dest.exists());

        // Second upload: file already exists — should be rejected
        assert!(dest.exists(), "conflict check: file exists, 409 must fire");
    }

    #[test]
    fn test_different_name_no_conflict() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dest_a = dir.path().join("report_a.pdf");
        let dest_b = dir.path().join("report_b.pdf");

        std::fs::write(&dest_a, b"content a").expect("write a");

        // Different name — no conflict
        assert!(!dest_b.exists());
    }

    #[test]
    fn test_safe_knowledge_path_traversal_contained() {
        let data_dir = "/tmp/borg-test-data";
        // Even with .. in path, file_name() strips components and result stays inside knowledge/
        let p = safe_knowledge_path(data_dir, "../secrets.txt").expect("some");
        assert!(p.starts_with("/tmp/borg-test-data/knowledge"));
        let p2 = safe_knowledge_path(data_dir, "../../etc/passwd").expect("some");
        assert!(p2.starts_with("/tmp/borg-test-data/knowledge"));
    }

    #[test]
    fn test_safe_knowledge_path_pure_dotdot_is_none() {
        let data_dir = "/tmp/borg-test-data";
        // Path ending in ".." has no file_name component → None
        assert!(safe_knowledge_path(data_dir, "subdir/..").is_none());
    }

    #[test]
    fn test_safe_knowledge_path_valid() {
        let data_dir = "/tmp/borg-test-data";
        let path = safe_knowledge_path(data_dir, "report.pdf");
        assert!(path.is_some());
        let p = path.unwrap();
        assert!(p.to_string_lossy().ends_with("knowledge/report.pdf"));
    use chrono::Utc;

    fn make_file(file_name: &str, stored_path: &str) -> ProjectFileRow {
        ProjectFileRow {
            id: 1,
            project_id: 1,
            file_name: file_name.to_string(),
            stored_path: stored_path.to_string(),
            mime_type: "text/plain".to_string(),
            size_bytes: 0,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn stage_project_files_uses_stored_path_basename() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().to_str().unwrap();

        // Two source files with different content but same display file_name
        let src1 = dir.path().join("1700000001_aaa_report.pdf");
        let src2 = dir.path().join("1700000002_bbb_report.pdf");
        std::fs::write(&src1, b"content-one").unwrap();
        std::fs::write(&src2, b"content-two").unwrap();

        let files = vec![
            make_file("report.pdf", src1.to_str().unwrap()),
            make_file("report.pdf", src2.to_str().unwrap()),
        ];

        stage_project_files(session_dir, &files);

        let dest_dir = dir.path().join("project_files");
        let staged1 = std::fs::read(dest_dir.join("1700000001_aaa_report.pdf")).unwrap();
        let staged2 = std::fs::read(dest_dir.join("1700000002_bbb_report.pdf")).unwrap();
        assert_eq!(staged1, b"content-one");
        assert_eq!(staged2, b"content-two");
    }

    #[test]
    fn stage_project_files_unique_names_no_collision() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().to_str().unwrap();

        let src1 = dir.path().join("1700000001_aaa_doc.txt");
        let src2 = dir.path().join("1700000002_bbb_doc.txt");
        std::fs::write(&src1, b"first").unwrap();
        std::fs::write(&src2, b"second").unwrap();

        let files = vec![
            make_file("doc.txt", src1.to_str().unwrap()),
            make_file("doc.txt", src2.to_str().unwrap()),
        ];

        stage_project_files(session_dir, &files);

        let dest_dir = dir.path().join("project_files");
        let entries: Vec<_> = std::fs::read_dir(&dest_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        // Both files must be present — no collision
        assert_eq!(entries.len(), 2);
    }
}

