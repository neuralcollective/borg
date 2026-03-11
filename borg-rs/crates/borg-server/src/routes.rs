use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use axum::{
    body::Bytes,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Json,
    },
};
use borg_core::{
    config::{refresh_oauth_token, Config},
    db::{
        Db, LegacyEvent, ProjectFileMetaRow, ProjectFilePageCursor, ProjectFileRow, ProjectRow,
        ProjectTaskCounts, TaskMessage, TaskOutput,
    },
    linked_credentials::{PROVIDER_CLAUDE, PROVIDER_OPENAI},
    types::{PhaseConfig, PhaseContext, RepoConfig, Task},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt},
    sync::{broadcast, Mutex as TokioMutex},
};
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};

use crate::{
    ingestion::{detect_doc_type, extract_text_from_bytes, IngestionQueue},
    storage::FileStorage,
    vespa::ChunkMetadata,
    AppState,
};

pub(crate) mod tasks;
pub(crate) use tasks::*;

pub(crate) mod linked_credentials;
pub(crate) use linked_credentials::*;

pub(crate) use crate::routes_modes::{
    delete_custom_mode, get_full_modes, get_modes, list_custom_modes, upsert_custom_mode,
};

// ── Error helper ──────────────────────────────────────────────────────────

pub(crate) fn internal(e: impl std::fmt::Debug + std::fmt::Display) -> StatusCode {
    tracing::error!("internal error: {e:#}");
    tracing::debug!("internal error detail: {e:?}");
    StatusCode::INTERNAL_SERVER_ERROR
}

pub(crate) fn require_project_access(
    state: &AppState,
    workspace: &crate::auth::WorkspaceContext,
    project_id: i64,
) -> Result<ProjectRow, StatusCode> {
    state
        .db
        .get_project_in_workspace(workspace.id, project_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)
}

pub(crate) fn require_task_access(
    state: &AppState,
    workspace: &crate::auth::WorkspaceContext,
    task_id: i64,
) -> Result<Task, StatusCode> {
    state
        .db
        .get_task_in_workspace(workspace.id, task_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)
}

fn percent_encode(s: &str) -> String {
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

fn percent_encode_allow_slash(s: &str, allow_slash: bool) -> String {
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

async fn sha256_hex_file(path: &str) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let count = file.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_hex_file_blocking(path: &std::path::Path) -> anyhow::Result<String> {
    use std::io::Read;

    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_hex_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn base64_decode(input: &str) -> anyhow::Result<Vec<u8>> {
    let clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut buf = 0u32;
    let mut bits = 0u32;
    for c in clean.bytes() {
        if c == b'=' {
            break;
        }
        let val = table
            .iter()
            .position(|&t| t == c)
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

// Removed the second percent_encode function

// ── Request body types ────────────────────────────────────────────────────

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
pub(crate) struct RepoQuery {
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
    pub jurisdiction: Option<String>,
    pub matter_type: Option<String>,
    pub privilege_level: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CreateWorkspaceBody {
    pub name: String,
    pub kind: Option<String>,
    pub set_default: Option<bool>,
}

#[derive(Deserialize)]
pub(crate) struct AddWorkspaceMemberBody {
    pub username: String,
    pub role: Option<String>,
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
    pub session_privileged: bool,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_counts: Option<ProjectTaskCounts>,
}

impl ProjectJson {
    fn from_row(p: ProjectRow, counts: Option<ProjectTaskCounts>) -> Self {
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
            session_privileged: p.session_privileged,
            created_at: p.created_at.to_rfc3339(),
            task_counts: counts,
        }
    }
}

impl From<ProjectRow> for ProjectJson {
    fn from(p: ProjectRow) -> Self {
        Self::from_row(p, None)
    }
}

#[derive(Serialize)]
pub(crate) struct ProjectFileJson {
    pub id: i64,
    pub project_id: i64,
    pub file_name: String,
    pub source_path: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub privileged: bool,
    pub has_text: bool,
    pub text_chars: usize,
    pub created_at: String,
}

impl From<ProjectFileRow> for ProjectFileJson {
    fn from(f: ProjectFileRow) -> Self {
        let text_chars = f.extracted_text.len();
        let source_path = if f.source_path.is_empty() {
            f.file_name.clone()
        } else {
            f.source_path.clone()
        };
        Self {
            id: f.id,
            project_id: f.project_id,
            file_name: f.file_name,
            source_path,
            mime_type: f.mime_type,
            size_bytes: f.size_bytes,
            privileged: f.privileged,
            has_text: text_chars > 0,
            text_chars,
            created_at: f.created_at.to_rfc3339(),
        }
    }
}

impl From<ProjectFileMetaRow> for ProjectFileJson {
    fn from(f: ProjectFileMetaRow) -> Self {
        let source_path = if f.source_path.is_empty() {
            f.file_name.clone()
        } else {
            f.source_path.clone()
        };
        Self {
            id: f.id,
            project_id: f.project_id,
            file_name: f.file_name,
            source_path,
            mime_type: f.mime_type,
            size_bytes: f.size_bytes,
            privileged: f.privileged,
            has_text: f.has_text,
            text_chars: f.text_chars.max(0) as usize,
            created_at: f.created_at.to_rfc3339(),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct ListProjectFilesQuery {
    #[serde(default = "default_project_file_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    q: String,
    #[serde(default)]
    has_text: Option<bool>,
    #[serde(default)]
    privileged_only: Option<bool>,
}

fn default_project_file_limit() -> i64 {
    50
}

#[derive(Deserialize)]
pub(crate) struct ListKnowledgeQuery {
    #[serde(default = "default_project_file_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
    #[serde(default)]
    q: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    jurisdiction: Option<String>,
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
    "public_url",
    "dropbox_client_id",
    "dropbox_client_secret",
    "google_client_id",
    "google_client_secret",
    "ms_client_id",
    "ms_client_secret",
    "storage_backend",
    "s3_bucket",
    "s3_region",
    "s3_endpoint",
    "s3_prefix",
    "backup_backend",
    "backup_mode",
    "backup_bucket",
    "backup_region",
    "backup_endpoint",
    "backup_prefix",
    "backup_poll_interval_s",
    "project_max_bytes",
    "knowledge_max_bytes",
    "cloud_import_max_batch_files",
    "ingestion_queue_backend",
    "sqs_queue_url",
    "sqs_region",
    "search_backend",
    "vespa_url",
    "vespa_namespace",
    "vespa_document_type",
    "experimental_domains",
    "visible_categories",
    "model_override",
    "dashboard_mode",
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
    ("public_url", ""),
    ("dropbox_client_id", ""),
    ("dropbox_client_secret", ""),
    ("google_client_id", ""),
    ("google_client_secret", ""),
    ("ms_client_id", ""),
    ("ms_client_secret", ""),
    ("storage_backend", "local"),
    ("s3_bucket", ""),
    ("s3_region", "us-east-1"),
    ("s3_endpoint", ""),
    ("s3_prefix", "borg/"),
    ("backup_backend", "disabled"),
    ("backup_mode", "active_work_only"),
    ("backup_bucket", ""),
    ("backup_region", "us-east-1"),
    ("backup_endpoint", ""),
    ("backup_prefix", "borg-backups/"),
    ("backup_poll_interval_s", "300"),
    ("project_max_bytes", "214748364800"),
    ("knowledge_max_bytes", "536870912000"),
    ("cloud_import_max_batch_files", "1000"),
    ("ingestion_queue_backend", "disabled"),
    ("sqs_queue_url", ""),
    ("sqs_region", "us-east-1"),
    ("search_backend", "vespa"),
    ("vespa_url", "http://127.0.0.1:8080"),
    ("vespa_namespace", "borg"),
    ("vespa_document_type", "project_file"),
    ("experimental_domains", "false"),
    ("visible_categories", "Professional Services"),
    ("model_override", ""),
    ("dashboard_mode", "general"),
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

fn sanitize_upload_relative_path(name: &str) -> String {
    let parts = std::path::Path::new(name)
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(seg) => seg.to_str().map(sanitize_upload_name),
            _ => None,
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        sanitize_upload_name(name)
    } else {
        parts.join("/")
    }
}

/// Resolve a knowledge file path, canonicalizing to prevent traversal.
fn safe_knowledge_path(
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

fn project_chat_key(project_id: i64) -> String {
    format!("project:{project_id}")
}

fn rand_suffix() -> u64 {
    use std::{
        collections::hash_map::RandomState,
        hash::{BuildHasher, Hasher},
    };
    let s = RandomState::new();
    let mut h = s.build_hasher();
    h.write_u64(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64,
    );
    h.finish()
}

fn parse_project_chat_key(chat_key: &str) -> Option<i64> {
    chat_key.strip_prefix("project:")?.parse::<i64>().ok()
}

fn workspace_chat_prefix(workspace_id: i64) -> String {
    format!("web:workspace:{workspace_id}:")
}

fn scoped_workspace_chat_thread(workspace_id: i64, requested: &str) -> String {
    let requested = requested.trim();
    let requested = if requested.is_empty() {
        "dashboard"
    } else {
        requested
    };
    format!(
        "{}{}",
        workspace_chat_prefix(workspace_id),
        sanitize_chat_key(requested)
    )
}

fn visible_workspace_chat_thread(workspace_id: i64, chat_jid: &str) -> Option<String> {
    chat_jid
        .strip_prefix(&workspace_chat_prefix(workspace_id))
        .map(|s| s.to_string())
}

fn visible_chat_thread_for_workspace(db: &Db, workspace_id: i64, chat_jid: &str) -> Option<String> {
    if let Some(thread) = visible_workspace_chat_thread(workspace_id, chat_jid) {
        return Some(thread);
    }
    let project_id = parse_project_chat_key(chat_jid)?;
    db.get_project_in_workspace(workspace_id, project_id)
        .ok()
        .flatten()
        .map(|_| chat_jid.to_string())
}

fn is_binary_mime(mime: &str) -> bool {
    mime.starts_with("application/pdf")
        || mime.starts_with("image/")
        || mime.starts_with("audio/")
        || mime.starts_with("video/")
        || mime.starts_with("application/zip")
        || mime.starts_with("application/octet-stream")
}

fn format_compact_bytes(n: i64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else if n < 1024 * 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", n as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn truncate_chars(input: &str, max_chars: usize) -> (String, bool) {
    if input.chars().count() <= max_chars {
        return (input.to_string(), false);
    }
    let clipped = input.chars().take(max_chars).collect::<String>();
    (clipped, true)
}

fn encode_project_file_cursor(file: &ProjectFileMetaRow) -> String {
    format!(
        "{}|{}",
        file.created_at.format("%Y-%m-%d %H:%M:%S"),
        file.id
    )
}

fn decode_project_file_cursor(raw: Option<&str>) -> Option<ProjectFilePageCursor> {
    let raw = raw?.trim();
    let (created_at, id_str) = raw.rsplit_once('|')?;
    let id = id_str.parse::<i64>().ok()?;
    Some(ProjectFilePageCursor {
        created_at: created_at.to_string(),
        id,
    })
}

#[derive(Clone)]
struct ProjectContextHit {
    source_path: String,
    snippet: String,
    score: f64,
    source: &'static str,
}

#[derive(Clone)]
struct StagedProjectFile {
    file: ProjectFileRow,
    staged_path: String,
    snippet: String,
    source: &'static str,
    score: f64,
    clipped: bool,
}

async fn search_project_context_hits(
    db: &Db,
    search: Option<&crate::search::SearchClient>,
    project_id: i64,
    query: &str,
    limit: i64,
) -> Vec<ProjectContextHit> {
    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    if let Some(search) = search {
        match search.search(query, Some(project_id), limit.max(1)).await {
            Ok(results) => {
                for r in results {
                    if seen.insert(r.file_path.clone()) {
                        hits.push(ProjectContextHit {
                            source_path: r.file_path,
                            snippet: r.content_snippet,
                            score: r.score,
                            source: "search",
                        });
                    }
                }
            },
            Err(e) => {
                tracing::warn!("project context search query failed: {e}");
            },
        }
    }

    let trimmed = query.trim();
    let filename_targeted = trimmed.contains('/')
        || trimmed.contains('.')
        || trimmed.contains('_')
        || (trimmed.contains('-') && trimmed.split_whitespace().count() == 1);
    let filename_limit = if filename_targeted {
        limit
    } else {
        limit.min(6)
    };
    if (filename_targeted || hits.is_empty()) && !trimmed.is_empty() {
        if let Ok(meta_rows) = db.search_project_file_name_hits(project_id, query, filename_limit) {
            for row in meta_rows {
                let source_path = if row.source_path.is_empty() {
                    row.file_name.clone()
                } else {
                    row.source_path
                };
                if seen.insert(source_path.clone()) {
                    hits.push(ProjectContextHit {
                        source_path,
                        snippet: "Filename matched the current request.".to_string(),
                        score: 0.0,
                        source: "filename",
                    });
                }
            }
        }
    }

    hits.truncate(limit.max(1) as usize);
    hits
}

async fn stage_project_files(
    session_dir: &str,
    files: &[(ProjectFileRow, ProjectContextHit)],
    storage: &FileStorage,
) -> Vec<StagedProjectFile> {
    const MAX_STAGE_FILES: usize = 8;
    const MAX_STAGE_CHARS_PER_FILE: usize = 160_000;
    const MAX_STAGE_CHARS_TOTAL: usize = 600_000;

    let dest_dir = format!("{session_dir}/project_files");
    let _ = std::fs::remove_dir_all(&dest_dir);
    let _ = std::fs::create_dir_all(&dest_dir);
    let mut staged = Vec::new();
    let mut remaining_chars = MAX_STAGE_CHARS_TOTAL;

    for (idx, (file, hit)) in files.iter().take(MAX_STAGE_FILES).enumerate() {
        if remaining_chars < 512 {
            break;
        }

        let source_text = if !file.extracted_text.trim().is_empty() {
            file.extracted_text.replace('\0', "")
        } else if !is_binary_mime(&file.mime_type) {
            match storage.read_all(&file.stored_path).await {
                Ok(bytes) => String::from_utf8_lossy(&bytes).replace('\0', ""),
                Err(_) => String::new(),
            }
        } else {
            String::new()
        };

        if source_text.trim().is_empty() {
            continue;
        }

        let per_file_budget = MAX_STAGE_CHARS_PER_FILE.min(remaining_chars);
        let (clipped_text, clipped) = truncate_chars(&source_text, per_file_budget);
        let base_name = sanitize_upload_name(&file.file_name);
        let stage_name = if is_binary_mime(&file.mime_type) || !file.mime_type.starts_with("text/")
        {
            format!("{:02}-{}.extracted.txt", idx + 1, base_name)
        } else {
            format!("{:02}-{}", idx + 1, base_name)
        };
        let dest = format!("{dest_dir}/{stage_name}");
        if tokio::fs::write(&dest, clipped_text.as_bytes())
            .await
            .is_err()
        {
            continue;
        }
        remaining_chars = remaining_chars.saturating_sub(clipped_text.chars().count());
        staged.push(StagedProjectFile {
            file: file.clone(),
            staged_path: dest,
            snippet: hit.snippet.clone(),
            source: hit.source,
            score: hit.score,
            clipped,
        });
    }
    staged
}

async fn build_project_context(
    project: &ProjectRow,
    retrieval_query: &str,
    session_dir: &str,
    db: &Db,
    search: Option<&crate::search::SearchClient>,
    storage: &FileStorage,
) -> String {
    let stats = db.get_project_file_stats(project.id).unwrap_or_default();
    let completed_tasks = db
        .list_recent_completed_project_tasks(project.id, 3)
        .unwrap_or_default();

    if stats.total_files == 0 && completed_tasks.is_empty() {
        return String::new();
    }

    const MAX_CONTEXT_BYTES: usize = 120_000;
    let mut remaining = MAX_CONTEXT_BYTES;
    let files_dir = format!("{session_dir}/project_files");
    let raw_query = retrieval_query.trim();
    let hits = if raw_query.is_empty() {
        Vec::new()
    } else {
        search_project_context_hits(db, search, project.id, raw_query, 12).await
    };
    let mut selected = Vec::new();
    for hit in hits {
        if let Ok(Some(file)) =
            db.find_latest_project_file_by_source_path(project.id, &hit.source_path)
        {
            selected.push((file, hit));
        }
    }
    if selected.is_empty() && !project.session_privileged && stats.total_files <= 50 {
        for file in db
            .list_recent_project_files(project.id, 4, true)
            .unwrap_or_default()
        {
            let source_path = if file.source_path.is_empty() {
                file.file_name.clone()
            } else {
                file.source_path.clone()
            };
            selected.push((
                file,
                ProjectContextHit {
                    source_path,
                    snippet: "Recent project document selected as a small-corpus fallback."
                        .to_string(),
                    score: 0.0,
                    source: "recent",
                },
            ));
        }
    }
    let staged_files = stage_project_files(session_dir, &selected, storage).await;

    let mut context = format!(
        "Project context:\nProject: {} (mode: {})\nCorpus: {} files, {} extracted-text files, {} privileged files, {} total\nSession privileged: {}\nStaged working set: {} file(s) in {}/\n",
        project.name,
        project.mode,
        stats.total_files,
        stats.text_files,
        stats.privileged_files,
        format_compact_bytes(stats.total_bytes),
        if project.session_privileged { "yes" } else { "no" },
        staged_files.len(),
        files_dir,
    );
    if !project.client_name.trim().is_empty()
        || !project.jurisdiction.trim().is_empty()
        || !project.matter_type.trim().is_empty()
    {
        context.push_str(&format!(
            "Matter details: client={}, jurisdiction={}, type={}\n",
            if project.client_name.trim().is_empty() {
                "n/a"
            } else {
                project.client_name.trim()
            },
            if project.jurisdiction.trim().is_empty() {
                "n/a"
            } else {
                project.jurisdiction.trim()
            },
            if project.matter_type.trim().is_empty() {
                "n/a"
            } else {
                project.matter_type.trim()
            },
        ));
    }
    if !raw_query.is_empty() {
        context.push_str(&format!("Retrieval query: {}\n", raw_query));
    }
    context.push_str("Selection policy: only the staged working set was materialized for this request. Do not assume unstaged corpus documents were reviewed.\n");
    if project.session_privileged {
        context.push_str("Legal handling: this matter is privileged. Prefer staged matter files and existing internal knowledge only. If the staged set is insufficient, say which documents are missing instead of guessing.\n");
    }
    context.push('\n');
    if context.len() >= remaining {
        return context;
    }
    remaining -= context.len();

    if !staged_files.is_empty() {
        let heading = "Retrieved matter files:\n";
        if heading.len() < remaining {
            context.push_str(heading);
            remaining -= heading.len();
        }
    }
    for staged in &staged_files {
        if remaining < 256 {
            break;
        }
        let entry = format!(
            "- {} [{}; {}; privileged={}; score={:.3}; source={}]\n  source path: {}\n  staged at: {}\n  snippet: {}\n{}",
            staged.file.file_name,
            staged.file.mime_type,
            format_compact_bytes(staged.file.size_bytes),
            if staged.file.privileged { "yes" } else { "no" },
            staged.score,
            staged.source,
            if staged.file.source_path.is_empty() {
                staged.file.file_name.as_str()
            } else {
                staged.file.source_path.as_str()
            },
            staged.staged_path,
            staged.snippet.replace('\n', " "),
            if staged.clipped {
                "  note: staged text was clipped to keep the working set bounded.\n"
            } else {
                ""
            }
        );
        if entry.len() >= remaining {
            break;
        }
        context.push_str(&entry);
        remaining -= entry.len();
    }
    if staged_files.is_empty() && stats.total_files > 0 && remaining > 256 {
        let note = "No corpus files were auto-staged for this request. Work from project metadata and ask for a narrower document target if document-specific analysis is required.\n";
        if note.len() < remaining {
            context.push_str(note);
            remaining -= note.len();
        }
    }

    // Add completed task summaries for context
    for task in completed_tasks {
        if remaining < 256 {
            break;
        }
        if let Ok(outputs) = db.get_task_outputs(task.id) {
            if let Some(last) = outputs.last() {
                let summary = if last.output.len() > 2000 {
                    &last.output[..last.output.floor_char_boundary(2000)]
                } else {
                    &last.output
                };
                let entry = format!(
                    "\n\n## Prior research: {} (Task #{})\n{}",
                    task.title, task.id, summary
                );
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
    run_id: &str,
    sender_name: &str,
    messages: &[String],
    sessions: &Arc<TokioMutex<HashMap<String, String>>>,
    config: &Config,
    db: &Arc<Db>,
    search: Option<Arc<crate::search::SearchClient>>,
    storage: &Arc<FileStorage>,
    chat_event_tx: &broadcast::Sender<String>,
    ai_request_count: &Arc<AtomicU64>,
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
            "run_id": run_id,
        })
        .to_string();
        let _ = chat_event_tx.send(event);
    }

    let retrieval_query = messages.join("\n");
    let project_for_chat =
        parse_project_chat_key(chat_key).and_then(|pid| db.get_project(pid).ok().flatten());
    let prompt = if messages.len() == 1 {
        format!("{} says: {}", sender_name, messages[0])
    } else {
        let joined: Vec<String> = messages.iter().map(|m| format!("- {m}")).collect();
        format!("{} says:\n{}", sender_name, joined.join("\n"))
    };
    let prompt = if let Some(project) = project_for_chat.as_ref() {
        let ctx = build_project_context(
            project,
            &retrieval_query,
            &session_dir,
            db,
            search.as_deref(),
            storage,
        )
        .await;
        if ctx.is_empty() {
            prompt
        } else {
            format!("{ctx}\n\nUser request:\n{prompt}")
        }
    } else {
        prompt
    };

    let mut system_prompt = config.chat_system_prompt();

    // Detect project mode for MCP wiring
    let project_mode = project_for_chat.as_ref().map(|p| p.mode.clone());
    let is_legal = matches!(project_mode.as_deref(), Some("lawborg" | "legal"));

    if is_legal {
        system_prompt.push_str(borg_domains::legal::legal_chat_system_suffix());
    }

    let knowledge_files = project_for_chat
        .as_ref()
        .and_then(|project| {
            db.list_all_knowledge_in_workspace(
                project.workspace_id,
                Some(&retrieval_query),
                Some(project.jurisdiction.as_str()),
                80,
            )
            .ok()
        })
        .unwrap_or_default();
    if !knowledge_files.is_empty() {
        let knowledge_dir = format!(
            "{}/knowledge/workspaces/{}",
            config.data_dir,
            project_for_chat
                .as_ref()
                .map(|p| p.workspace_id)
                .unwrap_or_default()
        );
        let selected = borg_agent::instruction::select_relevant_knowledge_files(
            &knowledge_files,
            &retrieval_query,
            project_mode.as_deref(),
            project_for_chat.as_ref().map(|p| p.jurisdiction.as_str()),
            project_for_chat.as_ref().map(|p| p.id),
            24,
        );
        let kb = borg_agent::instruction::build_knowledge_section(&selected, &knowledge_dir);
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

    // Wire up MCP servers: borg-mcp always, lawborg-mcp for legal projects
    let api_url = format!("http://127.0.0.1:{}", config.web_port);
    let api_token =
        std::fs::read_to_string(format!("{}/.api-token", config.data_dir)).unwrap_or_default();

    let mut mcp_servers = serde_json::Map::new();

    // borg-mcp: document search + task management (always wired)
    let borg_mcp_path = if let Ok(p) = std::env::var("BORG_MCP_SERVER") {
        std::path::PathBuf::from(p)
    } else {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../sidecar/borg-mcp/server.js")
    };
    if let Ok(mcp_server) = borg_mcp_path.canonicalize() {
        let mut env_vars = serde_json::Map::new();
        env_vars.insert("API_BASE_URL".into(), json!(api_url));
        if !api_token.is_empty() {
            env_vars.insert("API_TOKEN".into(), json!(api_token));
        }
        env_vars.insert("CHAT_THREAD".into(), json!(chat_key));
        if let Some(ref p) = project_for_chat {
            env_vars.insert("PROJECT_ID".into(), json!(p.id.to_string()));
            env_vars.insert("PROJECT_MODE".into(), json!(&p.mode));
        }
        mcp_servers.insert(
            "borg".into(),
            json!({
                "command": "bun",
                "args": ["run", mcp_server],
                "env": env_vars,
            }),
        );
    } else {
        tracing::warn!(chat_key, path = %borg_mcp_path.display(), "borg-mcp not found");
    }

    // lawborg-mcp: external legal research tools (legal projects only)
    if is_legal {
        let legal_mcp_path = if let Ok(p) = std::env::var("LAWBORG_MCP_SERVER") {
            std::path::PathBuf::from(p)
        } else {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../sidecar/lawborg-mcp/server.js")
        };
        if let Ok(mcp_server) = legal_mcp_path.canonicalize() {
            tracing::info!(chat_key, path = %mcp_server.display(), "wiring lawborg-mcp for chat");
            let mut env_vars = serde_json::Map::new();
            let providers = [
                "lexisnexis",
                "westlaw",
                "clio",
                "imanage",
                "netdocuments",
                "congress",
                "openstates",
                "canlii",
                "regulations_gov",
            ];
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
            mcp_servers.insert(
                "legal".into(),
                json!({
                    "command": "bun",
                    "args": ["run", mcp_server],
                    "env": env_vars,
                }),
            );
        } else {
            tracing::warn!(chat_key, path = %legal_mcp_path.display(), "lawborg-mcp not found");
        }
    }

    if !mcp_servers.is_empty() {
        let config_json = json!({ "mcpServers": mcp_servers });
        let mcp_json_path = format!("{session_dir}/.mcp.json");
        if let Err(e) = std::fs::write(&mcp_json_path, config_json.to_string()) {
            tracing::warn!(chat_key, "failed to write .mcp.json: {e}");
        }
        args.push("--mcp-config".to_string());
        args.push(mcp_json_path);
    }

    let session_id = sessions.lock().await.get(chat_key).cloned().or_else(|| {
        db.get_session(&format!("chat-{}", sanitize_chat_key(chat_key)))
            .ok()
            .flatten()
    });
    if let Some(ref sid) = session_id {
        args.push("--resume".to_string());
        args.push(sid.clone());
    }

    args.push("--print".to_string());
    args.push(prompt);

    let token = refresh_oauth_token(&config.credentials_path, &config.oauth_token);

    // Write CLAUDE.md — tools handle search/tasks now, this just provides context
    if !api_token.is_empty() {
        let project_id_hint = project_for_chat
            .as_ref()
            .map(|p| {
                format!(
                    "Current project_id: {}\nCurrent project mode: {}\n",
                    p.id, p.mode
                )
            })
            .unwrap_or_default();
        let agent_claude_md = format!(
            "# Borg Chat Agent\n\n\
             {project_id_hint}\
             ## Tools\n\n\
             You have MCP tools for document search and pipeline task management:\n\
             - `search_documents` — hybrid semantic search across project documents. ALWAYS use this when the user asks about document content, especially with large document sets. Search iteratively with different queries rather than trying to read files one by one.\n\
             - `list_documents` — browse available documents by name\n\
             - `read_document` — read full text of a specific document by ID\n\
             - `create_task` — create a pipeline task for long-running async work (code changes, document generation, multi-step research). Ask clarifying questions first.\n\
             - `get_task_status` — check progress on a pipeline task\n\
             - `list_project_tasks` — see all tasks for the current project\n\n\
             ## When to search vs create a task\n\n\
             If BorgSearch returns `no_project_corpus`, ask the user to select or attach the relevant matter/project before continuing.\n\
             If a task needs exhaustive review of the attached matter corpus, set `requires_exhaustive_corpus_review=true` when creating it.\n\
             \n\
             - User asks a question about their documents → search_documents (may need multiple searches)\n\
             - User wants a document drafted, code changed, or complex multi-step work → create_task\n\
             - User asks about task status or project progress → get_task_status / list_project_tasks\n\
             - Quick factual question → answer directly\n",
        );
        let claude_md_path = format!("{session_dir}/CLAUDE.md");
        let _ = std::fs::write(&claude_md_path, &agent_claude_md);
    }

    let timeout = std::time::Duration::from_secs(config.agent_timeout_s.max(300) as u64);
    ai_request_count.fetch_add(1, Ordering::Relaxed);
    let mut child = tokio::process::Command::new("claude")
        .args(&args)
        .current_dir(&session_dir)
        .env("HOME", &session_dir)
        .env("CLAUDE_CODE_OAUTH_TOKEN", &token)
        .env("API_BASE_URL", &api_url)
        .env("API_TOKEN", &api_token)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    // Stream stdout line-by-line, forwarding NDJSON events to chat SSE
    let stdout = child.stdout.take().expect("stdout piped");
    let mut reader = tokio::io::BufReader::new(stdout).lines();
    let mut raw_lines: Vec<String> = Vec::new();
    let stream_result = tokio::time::timeout(timeout, async {
        while let Some(line) = reader.next_line().await? {
            raw_lines.push(line.clone());
            // Forward stream events so frontend can show agentic breakdown
            let stream_event = json!({
                "type": "chat_stream",
                "thread": chat_key,
                "run_id": run_id,
                "data": line,
            })
            .to_string();
            let _ = chat_event_tx.send(stream_event);
        }
        Ok::<(), anyhow::Error>(())
    })
    .await;

    let status = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait())
        .await
        .ok()
        .and_then(|r| r.ok());

    if let Err(_) = stream_result {
        let _ = child.kill().await;
        anyhow::bail!("chat agent timed out after {}s", timeout.as_secs());
    }

    if let Some(st) = status {
        if !st.success() {
            tracing::warn!("chat agent failed ({}) exit={:?}", chat_key, st.code());
        }
    }

    let raw = raw_lines.join("\n");
    let (text, new_session_id) = borg_agent::event::parse_stream(&raw);

    if let Some(sid) = new_session_id {
        sessions
            .lock()
            .await
            .insert(chat_key.to_string(), sid.clone());
        let folder = format!("chat-{}", sanitize_chat_key(chat_key));
        let _ = db.set_session(&folder, &sid);
    }

    // Store bot response with raw stream for replaying agent interactions
    if !text.is_empty() {
        let reply_ts = Utc::now().timestamp();
        let reply_id = format!("{}-bot-{}", chat_key, reply_ts);
        let stream_data = if raw.is_empty() {
            None
        } else {
            Some(raw.as_str())
        };
        let _ = db.insert_chat_message_with_stream(
            &reply_id,
            chat_key,
            Some("borg"),
            Some("borg"),
            &text,
            true,
            true,
            stream_data,
        );
        let event = json!({
            "role": "assistant",
            "sender": "borg",
            "text": &text,
            "ts": reply_ts,
            "thread": chat_key,
            "run_id": run_id,
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
        },
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
                },
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

// ── Email inbound ──────────────────────────────────────────────────────────

/// POST /api/email/inbound
/// Accepts raw RFC 2822 email or Postmark JSON. Auth via ?api_token= or X-Api-Token header.
pub(crate) async fn email_inbound(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    body: Bytes,
) -> impl IntoResponse {
    // Verify via api_token query param or X-Api-Token header
    let provided = params
        .get("api_token")
        .cloned()
        .or_else(|| {
            headers
                .get("x-api-token")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();
    if provided != state.api_token {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let ct = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let email = match borg_core::email::parse_auto(&body, &ct) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("email_inbound: parse failed: {e}");
            return (StatusCode::BAD_REQUEST, "Bad email format").into_response();
        },
    };

    if email.from.is_empty() {
        return (StatusCode::OK, "OK").into_response();
    }

    // Look up user by sender email
    let user = state.db.get_user_by_email(&email.from).ok().flatten();
    let (sender_name, _user_id) = match user {
        Some((id, _, display_name, _)) => {
            let name = if display_name.is_empty() { email.from_name.clone() } else { display_name };
            (name, Some(id))
        },
        None => {
            // Accept from unknown senders — route to global email thread
            (
                if email.from_name.is_empty() { email.from.clone() } else { email.from_name.clone() },
                None,
            )
        },
    };

    // Save attachments
    let att_dir = format!(
        "{}/attachments/email-{}",
        state.config.data_dir,
        chrono::Utc::now().timestamp_millis()
    );
    let att_paths =
        borg_core::email::save_attachments(&email.attachments, std::path::Path::new(&att_dir))
            .unwrap_or_default();

    // Build message text
    let mut agent_messages: Vec<String> = vec![format!(
        "Email from {} <{}>: {}\n\n{}",
        sender_name, email.from, email.subject, email.body
    )];
    for path in &att_paths {
        let size_kb = std::fs::metadata(path).map(|m| m.len() / 1024).unwrap_or(0);
        let filename = path.file_name().unwrap_or_default().to_string_lossy();
        agent_messages.push(format!(
            "[Attached file: {} ({}KB)] Path: {}",
            filename,
            size_kb,
            path.display()
        ));
    }

    let chat_key = format!("email:{}", email.from);
    let sessions = Arc::clone(&state.web_sessions);
    let config = Arc::clone(&state.config);
    let db = Arc::clone(&state.db);
    let search = state.search.clone();
    let storage = Arc::clone(&state.file_storage);
    let chat_tx = state.chat_event_tx.clone();
    let ai_count = Arc::clone(&state.ai_request_count);
    let from_email = email.from.clone();
    let reply_subject = format!("Re: {}", email.subject);

    tokio::spawn(async move {
        let run_id = crate::messaging_progress::new_chat_run_id();
        match run_chat_agent(
            &chat_key,
            &run_id,
            &sender_name,
            &agent_messages,
            &sessions,
            &config,
            &db,
            search,
            &storage,
            &chat_tx,
            &ai_count,
        )
        .await
        {
            Ok(reply) if !reply.is_empty() => {
                let _ = borg_core::email::send_smtp_reply(
                    &config.smtp_host,
                    config.smtp_port,
                    &config.smtp_from,
                    &config.smtp_user,
                    &config.smtp_pass,
                    &from_email,
                    &reply_subject,
                    &reply,
                )
                .await;
            },
            Ok(_) => {},
            Err(e) => tracing::warn!("email inbound agent error: {e}"),
        }
    });

    (StatusCode::OK, "OK").into_response()
}

// ── Handlers ──────────────────────────────────────────────────────────────

pub(crate) async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    let storage_result = state.file_storage.healthcheck().await;
    let search_result = if let Some(search) = &state.search {
        search.healthcheck().await
    } else {
        Ok(())
    };
    let backup = crate::backup::backup_status_snapshot(&state.db, &state.config).await;
    let ok = storage_result.is_ok() && search_result.is_ok();
    let mut search_info = serde_json::json!({
        "backend": state.search.as_ref().map(|s| s.backend_name()).unwrap_or("none"),
        "target": state.search.as_ref().map(|s| s.target()).unwrap_or_default(),
        "healthy": search_result.is_ok(),
        "error": search_result.err().map(|e| e.to_string()),
    });
    if let Some(search) = &state.search {
        let files = search.document_count("project_file").await.unwrap_or(-1);
        let chunks = search.document_count("project_chunk").await.unwrap_or(-1);
        search_info["documents"] = json!(files);
        search_info["chunks"] = json!(chunks);
    }
    Json(json!({
        "status": if ok { "ok" } else { "degraded" },
        "storage": {
            "backend": state.file_storage.backend_name(),
            "target": state.file_storage.target(),
            "healthy": storage_result.is_ok(),
            "error": storage_result.err().map(|e| e.to_string()),
        },
        "search": search_info,
        "backup": backup,
    }))
}

fn mcp_service_specs() -> [(&'static str, &'static str); 9] {
    [
        ("lexisnexis", "LexisNexis"),
        ("westlaw", "Westlaw"),
        ("clio", "Clio"),
        ("imanage", "iManage"),
        ("netdocuments", "NetDocuments"),
        ("congress", "Congress.gov"),
        ("openstates", "OpenStates"),
        ("canlii", "CanLII"),
        ("regulations_gov", "Regulations.gov"),
    ]
}

fn linked_credential_status_item(
    key: &str,
    label: &str,
    entry: Option<&borg_core::db::LinkedCredentialEntry>,
) -> Value {
    let Some(entry) = entry else {
        return mcp_status_item(
            key,
            label,
            "missing",
            format!("No linked {label} account for this user"),
            Some("user"),
            None,
        );
    };

    let expiry_suffix = if !entry.expires_at.is_empty() {
        if let Ok(exp) = chrono::DateTime::parse_from_rfc3339(&entry.expires_at) {
            let now = Utc::now();
            let until = exp.with_timezone(&Utc).signed_duration_since(now);
            if until.num_seconds() <= 0 {
                " — token expired".to_string()
            } else if until.num_hours() < 1 {
                format!(" — expires in {}m", until.num_minutes())
            } else if until.num_hours() < 24 {
                format!(" — expires in {}h", until.num_hours())
            } else {
                format!(" — expires in {}d", until.num_days())
            }
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let is_expiring_soon = entry.expires_at.parse::<chrono::DateTime<chrono::FixedOffset>>().ok()
        .is_some_and(|exp| exp.with_timezone(&Utc).signed_duration_since(Utc::now()).num_hours() < 2);

    if entry.status == "connected" && !is_expiring_soon {
        let detail = if entry.account_email.is_empty() {
            format!("Linked and validated{expiry_suffix}")
        } else {
            format!("{} linked and validated{expiry_suffix}", entry.account_email)
        };
        mcp_status_item(key, label, "verified", detail, Some("user"), Some(entry.last_validated_at.clone()))
    } else if entry.status == "connected" && is_expiring_soon {
        let detail = if entry.account_email.is_empty() {
            format!("Token expiring soon{expiry_suffix}")
        } else {
            format!("{}{expiry_suffix}", entry.account_email)
        };
        mcp_status_item(key, label, "degraded", detail, Some("user"), Some(entry.last_validated_at.clone()))
    } else {
        let detail = if !entry.last_error.is_empty() {
            format!("{}{expiry_suffix}", entry.last_error)
        } else {
            format!("Linked account needs reconnect{expiry_suffix}")
        };
        mcp_status_item(key, label, "degraded", detail, Some("user"), Some(entry.last_validated_at.clone()))
    }
}

fn mcp_status_item(
    key: &str,
    label: &str,
    status: &str,
    detail: impl Into<String>,
    source: Option<&str>,
    checked_at: Option<String>,
) -> Value {
    json!({
        "key": key,
        "label": label,
        "status": status,
        "detail": detail.into(),
        "source": source.unwrap_or(""),
        "checked_at": checked_at.unwrap_or_default(),
    })
}

pub(crate) async fn get_mcp_status(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    let linked_credentials = state
        .db
        .list_user_linked_credentials(user.id)
        .map_err(internal)?;
    let linked_by_provider: HashMap<_, _> = linked_credentials
        .into_iter()
        .map(|entry| (entry.provider.clone(), entry))
        .collect();
    let available_keys = state
        .db
        .list_api_keys(&format!("workspace:{}", workspace.id))
        .map_err(internal)?;
    let mut effective_key_by_provider = HashMap::new();
    for entry in available_keys {
        let provider = entry.provider.clone();
        let replace = effective_key_by_provider.get(&provider).is_none_or(
            |current: &borg_core::db::ApiKeyEntry| {
                current.owner == "global" && entry.owner != "global"
            },
        );
        if replace {
            effective_key_by_provider.insert(provider, entry);
        }
    }

    let search_result = if let Some(search) = &state.search {
        search.healthcheck().await
    } else {
        Ok(())
    };
    let search_backend = state
        .search
        .as_ref()
        .map(|s| s.backend_name())
        .unwrap_or("none");
    let search_target = state
        .search
        .as_ref()
        .map(|s| s.target())
        .unwrap_or_default();

    let borg_mcp_path = if let Ok(path) = std::env::var("BORG_MCP_SERVER") {
        path
    } else {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../sidecar/borg-mcp/server.js")
            .to_string_lossy()
            .to_string()
    };
    let lawborg_mcp_path = if let Ok(path) = std::env::var("LAWBORG_MCP_SERVER") {
        path
    } else {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../sidecar/lawborg-mcp/server.js")
            .to_string_lossy()
            .to_string()
    };

    let agent_access = vec![
        linked_credential_status_item("claude", "Claude Code", linked_by_provider.get(PROVIDER_CLAUDE)),
        linked_credential_status_item("openai", "Codex / ChatGPT", linked_by_provider.get(PROVIDER_OPENAI)),
    ];

    let runtime = vec![
        if std::path::Path::new(&borg_mcp_path).exists() {
            mcp_status_item(
                "borg_mcp",
                "Borg MCP",
                "verified",
                format!("Sidecar present at {borg_mcp_path}"),
                Some("filesystem"),
                Some(Utc::now().to_rfc3339()),
            )
        } else {
            mcp_status_item(
                "borg_mcp",
                "Borg MCP",
                "missing",
                format!("Sidecar missing at {borg_mcp_path}"),
                Some("filesystem"),
                None,
            )
        },
        if search_backend == "none" {
            mcp_status_item(
                "borgsearch",
                "BorgSearch Tools",
                "missing",
                "No search backend configured",
                Some("runtime"),
                None,
            )
        } else if search_result.is_ok() {
            mcp_status_item(
                "borgsearch",
                "BorgSearch Tools",
                "verified",
                format!("{search_backend} healthy at {search_target}"),
                Some("endpoint"),
                Some(Utc::now().to_rfc3339()),
            )
        } else {
            mcp_status_item(
                "borgsearch",
                "BorgSearch Tools",
                "degraded",
                search_result
                    .err()
                    .map(|err| err.to_string())
                    .unwrap_or_else(|| "Search healthcheck failed".to_string()),
                Some("endpoint"),
                Some(Utc::now().to_rfc3339()),
            )
        },
        if std::path::Path::new(&lawborg_mcp_path).exists() {
            mcp_status_item(
                "lawborg_mcp",
                "Lawborg MCP",
                "verified",
                format!("Sidecar present at {lawborg_mcp_path}"),
                Some("filesystem"),
                Some(Utc::now().to_rfc3339()),
            )
        } else {
            mcp_status_item(
                "lawborg_mcp",
                "Lawborg MCP",
                "missing",
                format!("Sidecar missing at {lawborg_mcp_path}"),
                Some("filesystem"),
                None,
            )
        },
    ];

    let services: Vec<Value> = mcp_service_specs()
        .into_iter()
        .map(|(provider, label)| {
            if let Some(entry) = effective_key_by_provider.get(provider) {
                let source = if entry.owner == "global" {
                    "global"
                } else {
                    "workspace"
                };
                mcp_status_item(
                    provider,
                    label,
                    "configured",
                    format!("Credential configured via {source} scope"),
                    Some(source),
                    Some(entry.created_at.clone()),
                )
            } else {
                mcp_status_item(
                    provider,
                    label,
                    "missing",
                    "No credential configured via workspace or global scope",
                    None,
                    None,
                )
            }
        })
        .collect();

    let mut verified = 0;
    let mut configured = 0;
    let mut degraded = 0;
    let mut missing = 0;
    for item in agent_access
        .iter()
        .chain(runtime.iter())
        .chain(services.iter())
    {
        match item
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("missing")
        {
            "verified" => verified += 1,
            "configured" => configured += 1,
            "degraded" => degraded += 1,
            _ => missing += 1,
        }
    }

    let mut service_counts: HashMap<&str, i64> = HashMap::new();
    service_counts.insert("verified", 0);
    service_counts.insert("configured", 0);
    service_counts.insert("degraded", 0);
    service_counts.insert("missing", 0);
    for item in &services {
        let status = item
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("missing");
        *service_counts.entry(status).or_insert(0) += 1;
    }

    Ok(Json(json!({
        "generated_at": Utc::now().to_rfc3339(),
        "summary": {
            "verified": verified,
            "configured": configured,
            "degraded": degraded,
            "missing": missing,
        },
        "agent_access": agent_access,
        "runtime": runtime,
        "services": services,
        "workspace": {
            "id": workspace.id,
            "name": workspace.name,
        },
        "service_rollup": {
            "verified": service_counts.get("verified").copied().unwrap_or(0),
            "configured": service_counts.get("configured").copied().unwrap_or(0),
            "degraded": service_counts.get("degraded").copied().unwrap_or(0),
            "missing": service_counts.get("missing").copied().unwrap_or(0),
        }
    })))
}

// Projects

pub(crate) async fn list_projects(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    let projects = state
        .db
        .list_projects_in_workspace(workspace.id)
        .map_err(internal)?;
    let out: Vec<ProjectJson> = projects
        .into_iter()
        .map(|p| {
            let counts = state.db.project_task_status_counts(p.id).ok();
            ProjectJson::from_row(p, counts)
        })
        .collect();
    Ok(Json(json!(out)))
}

pub(crate) async fn search_projects(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<Value>, StatusCode> {
    let q = params.q.unwrap_or_default();
    if q.is_empty() {
        return list_projects(State(state), axum::Extension(workspace)).await;
    }
    let projects = state
        .db
        .search_projects_in_workspace(workspace.id, &q)
        .map_err(internal)?;
    let out: Vec<ProjectJson> = projects
        .into_iter()
        .map(|p| {
            let counts = state.db.project_task_status_counts(p.id).ok();
            ProjectJson::from_row(p, counts)
        })
        .collect();
    Ok(Json(json!(out)))
}

pub(crate) async fn create_project(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
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
    // Insert with empty repo_path first to get the ID
    let id = state
        .db
        .insert_project(
            workspace.id,
            name,
            &mode,
            "",
            client_name,
            jurisdiction,
            matter_type,
            privilege_level,
        )
        .map_err(internal)?;

    // Auto-init a dedicated git repo for projects
    let repo_dir = format!("{}/legal-repos/{}", state.config.data_dir, id);
    tokio::fs::create_dir_all(&repo_dir)
        .await
        .map_err(internal)?;
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
            .update_project(
                id,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(&repo_dir),
                None,
            )
            .map_err(internal)?;
    }

    let _ = state.db.log_event_full(
        None,
        None,
        Some(id),
        "api",
        "matter.created",
        &json!({ "name": name, "mode": &mode }),
    );
    tracing::info!(
        target: "instrumentation.project",
        message = "project created",
        user_id = user.id,
        username = user.username.as_str(),
        project_id = id,
        mode = mode.as_str(),
        jurisdiction = jurisdiction,
        matter_type = matter_type,
        privilege_level = privilege_level,
    );

    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub(crate) async fn get_project(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let project = require_project_access(state.as_ref(), &workspace, id)?;
    let counts = state.db.project_task_status_counts(id).ok();
    Ok(Json(json!(ProjectJson::from_row(project, counts))))
}

pub(crate) async fn update_project(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateProjectBody>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
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
    let updated = require_project_access(state.as_ref(), &workspace, id)?;
    tracing::info!(
        target: "instrumentation.project",
        message = "project updated",
        user_id = user.id,
        username = user.username.as_str(),
        project_id = id,
        mode = updated.mode.as_str(),
        jurisdiction = updated.jurisdiction.as_str(),
        matter_type = updated.matter_type.as_str(),
        status = updated.status.as_str(),
    );

    Ok(Json(json!(ProjectJson::from(updated))))
}

pub(crate) async fn delete_project(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    let project = require_project_access(state.as_ref(), &workspace, id)?;
    // Clean up dedicated repo if it exists
    if !project.repo_path.is_empty() {
        let _ = tokio::fs::remove_dir_all(&project.repo_path).await;
    }
    let _ = state.db.log_event_full(
        None,
        None,
        Some(id),
        "api",
        "matter.deleted",
        &json!({ "name": &project.name }),
    );
    if let Some(search) = &state.search {
        let _ = search.delete_project_chunks(id).await;
    }
    state.db.delete_project(id).map_err(internal)?;
    tracing::info!(
        target: "instrumentation.project",
        message = "project deleted",
        user_id = user.id,
        username = user.username.as_str(),
        project_id = id,
        mode = project.mode.as_str(),
        status = project.status.as_str(),
    );
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list_project_tasks(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    let tasks = state.db.list_project_tasks(id).map_err(internal)?;
    Ok(Json(json!(tasks)))
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
fn default_search_limit() -> i64 {
    50
}

#[derive(Deserialize)]
pub(crate) struct ThemeQuery {
    #[serde(default = "default_theme_limit")]
    limit: i64,
    #[serde(default = "default_theme_min_docs")]
    min_docs: i64,
}
fn default_theme_limit() -> i64 {
    30
}
fn default_theme_min_docs() -> i64 {
    2
}

// ── Audit ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct AuditQuery {
    #[serde(default = "default_audit_limit")]
    limit: i64,
}
fn default_audit_limit() -> i64 {
    100
}

pub(crate) async fn list_project_audit(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    let events = state
        .db
        .list_project_events(id, q.limit)
        .map_err(internal)?;
    Ok(Json(json!(events)))
}

pub(crate) async fn search_documents(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Query(query): Query<FtsSearchQuery>,
) -> Result<Json<Value>, StatusCode> {
    if query.q.trim().is_empty() {
        return Ok(Json(json!([])));
    }

    if let Some(project_id) = query.project_id {
        let _project = require_project_access(state.as_ref(), &workspace, project_id)?;
    }

    let allowed_project_ids: HashSet<i64> = state
        .db
        .list_projects_in_workspace(workspace.id)
        .map_err(internal)?
        .into_iter()
        .map(|p| p.id)
        .collect();

    let mut items: Vec<Value> = Vec::new();

    // Keyword search backend: Vespa (or another configured external search provider).
    if let Some(search) = &state.search {
        match search.search(&query.q, query.project_id, query.limit).await {
            Ok(results) => {
                for r in results {
                    if !allowed_project_ids.contains(&r.project_id) {
                        continue;
                    }
                    let project_name = state
                        .db
                        .get_project_in_workspace(workspace.id, r.project_id)
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
                        "score": r.score,
                        "source": search.backend_name(),
                    }));
                }
            },
            Err(e) => {
                tracing::warn!("external search query failed: {e}");
            },
        }
    } else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    // Semantic search (when requested and embeddings exist)
    if query.semantic && state.db.embedding_count() > 0 {
        if let Ok(query_emb) = state
            .embed_registry
            .default_client()
            .embed_query(&query.q)
            .await
        {
            if let Ok(sem_results) =
                state
                    .db
                    .search_embeddings(&query_emb, query.limit as usize, query.project_id)
            {
                for r in sem_results.iter().filter(|r| r.score > 0.5) {
                    let Some(project_id) = r.project_id else {
                        continue;
                    };
                    if !allowed_project_ids.contains(&project_id) {
                        continue;
                    }
                    items.push(json!({
                        "project_id": project_id,
                        "task_id": r.task_id,
                        "file_path": r.file_path,
                        "content_snippet": if r.chunk_text.len() > 200 { &r.chunk_text[..r.chunk_text.floor_char_boundary(200)] } else { &r.chunk_text },
                        "score": r.score,
                        "source": "semantic",
                    }));
                }
            }
        }
    }

    tracing::info!(
        target: "instrumentation.search",
        message = "document search completed",
        user_id = user.id,
        username = user.username.as_str(),
        project_id = query.project_id.unwrap_or_default(),
        semantic = query.semantic,
        limit = query.limit,
        query_len = query.q.chars().count() as u64,
        result_count = items.len() as u64,
    );

    Ok(Json(json!(items)))
}

pub(crate) async fn summarize_project_themes(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Query(q): Query<ThemeQuery>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    let summary = state
        .db
        .summarize_themes(Some(id), q.limit.clamp(5, 200), q.min_docs.clamp(1, 1000))
        .map_err(internal)?;
    Ok(Json(json!(summary)))
}

pub(crate) async fn summarize_workspace_themes(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Query(q): Query<ThemeQuery>,
) -> Result<Json<Value>, StatusCode> {
    let summary = state
        .db
        .summarize_themes_for_workspace(
            workspace.id,
            q.limit.clamp(5, 200),
            q.min_docs.clamp(1, 1000),
        )
        .map_err(internal)?;
    Ok(Json(json!(summary)))
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
                    &format!(
                        "repos/{slug}/contents/{}?ref={}",
                        percent_encode_allow_slash(path, true),
                        percent_encode(ref_name)
                    ),
                    "--jq",
                    ".content",
                ])
                .stderr(std::process::Stdio::null())
                .output(),
        )
        .await;
        if let Ok(Ok(output)) = out {
            if output.status.success() {
                let b64 = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .replace('\n', "");
                return base64_decode(&b64).ok();
            }
        }
    }
    None
}

pub(crate) async fn list_project_documents(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
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
                },
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
                    },
                    _ => continue,
                }
            },
            _ => continue,
        };

        for line in file_list.lines() {
            let name = line.trim();
            if !name.is_empty() && !name.starts_with('.') {
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
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path((id, task_id)): Path<(i64, i64)>,
    Query(q): Query<DocQuery>,
) -> Result<axum::response::Response, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    let task = require_task_access(state.as_ref(), &workspace, task_id)?;
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
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path((id, task_id)): Path<(i64, i64)>,
    Query(q): Query<DocQuery>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    let task = require_task_access(state.as_ref(), &workspace, task_id)?;
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
                    "-C",
                    repo_path,
                    "log",
                    &task.branch,
                    "--format=%H\t%s\t%aI\t%an",
                    "--",
                    path,
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
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path((id, task_id)): Path<(i64, i64)>,
    Query(q): Query<ExportQuery>,
) -> Result<axum::response::Response, StatusCode> {
    let project = require_project_access(state.as_ref(), &workspace, id)?;
    let task = require_task_access(state.as_ref(), &workspace, task_id)?;
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
            if !project.client_name.is_empty() {
                format!(
                    "subtitle: \"Prepared for {}\"\n",
                    project.client_name.replace('"', "'")
                )
            } else {
                String::new()
            },
            if !project.case_number.is_empty() {
                format!("subject: \"Case No. {}\"\n", project.case_number)
            } else {
                String::new()
            },
            if !project.jurisdiction.is_empty() {
                format!("keywords: [\"{}\"]\n", project.jurisdiction)
            } else {
                String::new()
            },
            if !project.privilege_level.is_empty() {
                format!(
                    "header-includes: |\n  \\fancyfoot[C]{{PRIVILEGED AND CONFIDENTIAL — {}}}\n",
                    project.privilege_level.to_uppercase()
                )
            } else {
                String::new()
            },
            Utc::now().format("%B %d, %Y"),
        );
        md_content = format!("{}{}", title_block, md_content);
    }

    let md_bytes = md_content.into_bytes();

    // Resolve template: explicit template_id takes priority, then project default
    let effective_template_id = q.template_id.or(project.default_template_id);
    let template_info = if let Some(tid) = effective_template_id {
        state
            .db
            .get_knowledge_file_in_workspace(project.workspace_id, tid)
            .map_err(internal)?
            .map(|f| {
                let p = safe_knowledge_path(
                    &state.config.data_dir,
                    Some(project.workspace_id),
                    &f.file_name,
                )
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
                let is_docx = f.file_name.to_lowercase().ends_with(".docx");
                (p, is_docx)
            })
    } else {
        None
    };
    let use_docxtemplater = format == "docx"
        && template_info
            .as_ref()
            .is_some_and(|(p, is_docx)| *is_docx && std::path::Path::new(p).exists());

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
        let fill_script = format!(
            "{}/sidecar/docx-template/fill.ts",
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".into())
        );
        if let Ok(mut child) = tokio::process::Command::new("bun")
            .arg("run")
            .arg(&fill_script)
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
                    tracing::warn!(
                        "docxtemplater fill failed: {}",
                        String::from_utf8_lossy(&out.stderr)
                    );
                },
                Err(e) => {
                    tracing::warn!("docxtemplater process error: {e}");
                },
                _ => {},
            }
        }
    }

    // Fall back to pandoc if docxtemplater didn't produce output
    if !out_path.exists() {
        let md_path = tmp_dir.path().join("document.md");
        tokio::fs::write(&md_path, &md_bytes)
            .await
            .map_err(internal)?;

        let mut cmd = tokio::process::Command::new("pandoc");
        cmd.arg(&md_path)
            .arg("-f")
            .arg("markdown")
            .arg("-o")
            .arg(&out_path)
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

        let pandoc_out = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output())
            .await
            .map_err(internal)?
            .map_err(internal)?;

        // If weasyprint failed for PDF, retry with xelatex
        if !pandoc_out.status.success() && format == "pdf" {
            let retry = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                tokio::process::Command::new("pandoc")
                    .arg(&md_path)
                    .arg("-f")
                    .arg("markdown")
                    .arg("-o")
                    .arg(&out_path)
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
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Query(q): Query<ExportAllQuery>,
) -> Result<axum::response::Response, StatusCode> {
    let project = require_project_access(state.as_ref(), &workspace, id)?;
    let tasks = state.db.list_project_tasks(id).map_err(internal)?;
    let format = q.format.as_deref().unwrap_or("docx");
    if format != "pdf" && format != "docx" {
        return Err(StatusCode::BAD_REQUEST);
    }

    let tmp_dir = tempfile::tempdir().map_err(internal)?;
    let mut file_entries: Vec<(String, Vec<u8>)> = Vec::new();

    let effective_tid = q.template_id.or(project.default_template_id);
    let template_info = effective_tid.and_then(|tid| {
        state
            .db
            .get_knowledge_file_in_workspace(project.workspace_id, tid)
            .ok()
            .flatten()
            .map(|f| {
                let p = safe_knowledge_path(
                    &state.config.data_dir,
                    Some(project.workspace_id),
                    &f.file_name,
                )
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
                let is_docx = f.file_name.to_lowercase().ends_with(".docx");
                (p, is_docx)
            })
    });
    let use_docxtemplater = format == "docx"
        && template_info
            .as_ref()
            .is_some_and(|(p, is_docx)| *is_docx && std::path::Path::new(p).exists());
    let fill_script = format!(
        "{}/sidecar/docx-template/fill.ts",
        std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".into())
    );

    for task in &tasks {
        if task.branch.is_empty() {
            continue;
        }
        let slug = state
            .config
            .watched_repos
            .iter()
            .find(|r| r.path == task.repo_path)
            .map(|r| r.repo_slug.as_str())
            .unwrap_or("");

        for doc_path in &["research.md", "analysis.md", "review_notes.md"] {
            let raw_bytes = match git_show_file(&task.repo_path, slug, &task.branch, doc_path).await
            {
                Some(b) if !b.is_empty() => b,
                _ => continue,
            };

            let raw_md = String::from_utf8_lossy(&raw_bytes);
            let mut md_content = preprocess_legal_markdown(&raw_md);
            if !project.privilege_level.is_empty() {
                md_content = format!(
                    "**PRIVILEGED AND CONFIDENTIAL — {}**\n\n---\n\n{}",
                    project.privilege_level.to_uppercase(),
                    md_content
                );
            }

            let safe_title = task
                .title
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
                .collect::<String>()
                .trim()
                .to_string();
            let stem = std::path::Path::new(doc_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("doc");
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
                    .arg("run")
                    .arg(&fill_script)
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
                tokio::fs::write(&md_path, md_content.as_bytes())
                    .await
                    .map_err(internal)?;
                let mut cmd = tokio::process::Command::new("pandoc");
                cmd.arg(&md_path)
                    .arg("-f")
                    .arg("markdown")
                    .arg("-o")
                    .arg(&out_path)
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
                    .await
                    .map_err(internal)?
                    .map_err(internal)?;
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
    let filename = format!(
        "{}-export.zip",
        project
            .name
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
            .collect::<String>()
            .trim()
            .to_string()
    );

    Ok(axum::response::Response::builder()
        .header("content-type", "application/zip")
        .header(
            "content-disposition",
            format!("attachment; filename=\"{filename}\""),
        )
        .body(axum::body::Body::from(zip_bytes))
        .unwrap())
}

pub(crate) async fn delete_project_document(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path((id, task_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    let task = require_task_access(state.as_ref(), &workspace, task_id)?;
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
        tracing::warn!(
            task_id,
            branch = task.branch,
            "git branch -D failed: {stderr}"
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn list_project_files(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Query(q): Query<ListProjectFilesQuery>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    let cursor = decode_project_file_cursor(q.cursor.as_deref());
    let (files, total) = state
        .db
        .list_project_file_page(
            id,
            Some(&q.q),
            q.limit,
            q.offset,
            cursor.as_ref(),
            q.has_text,
            q.privileged_only,
        )
        .map_err(internal)?;
    let next_cursor = files.last().map(encode_project_file_cursor);
    let out: Vec<ProjectFileJson> = files.into_iter().map(ProjectFileJson::from).collect();
    let stats = state.db.get_project_file_stats(id).map_err(internal)?;
    let limit = q.limit.clamp(1, 200);
    let offset = q.offset.max(0);
    let has_more = offset + (out.len() as i64) < total;
    Ok(Json(json!({
        "items": out,
        "total": total,
        "offset": offset,
        "limit": limit,
        "has_more": has_more,
        "next_cursor": if has_more { next_cursor } else { None },
        "summary": stats,
    })))
}

pub(crate) async fn delete_all_project_files(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;

    let files = state.db.list_project_files(id).map_err(internal)?;
    for file in &files {
        if let Err(err) = state.file_storage.delete(&file.stored_path).await {
            tracing::warn!(
                project_id = id,
                file_id = file.id,
                "failed to delete stored file: {err}"
            );
        }
    }
    if let Some(search) = &state.search {
        let _ = search.delete_project_chunks(id).await;
    }

    let deleted = state.db.delete_all_project_files(id).map_err(internal)?;
    Ok(Json(json!({ "ok": true, "deleted": deleted })))
}

pub(crate) async fn get_project_file_content(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path((project_id, file_id)): Path<(i64, i64)>,
) -> Result<axum::response::Response, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, project_id)?;
    let row = state
        .db
        .get_project_file(project_id, file_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let bytes = state
        .file_storage
        .read_all(&row.stored_path)
        .await
        .map_err(internal)?;

    let safe_name = row.file_name.replace('"', "_");
    Ok(axum::response::Response::builder()
        .header("content-type", "application/octet-stream")
        .header(
            "content-disposition",
            format!("attachment; filename=\"{safe_name}\""),
        )
        .body(axum::body::Body::from(bytes))
        .unwrap())
}

#[derive(Deserialize)]
pub(crate) struct CreateUploadSessionBody {
    pub file_name: String,
    pub mime_type: Option<String>,
    pub file_size: i64,
    pub chunk_size: i64,
    pub total_chunks: i64,
    #[serde(default)]
    pub is_zip: bool,
    #[serde(default)]
    pub privileged: bool,
}

#[derive(Deserialize)]
pub(crate) struct UploadSessionsQuery {
    #[serde(default = "default_upload_session_limit")]
    pub limit: i64,
}

#[derive(Deserialize)]
pub(crate) struct UploadProjectFilesQuery {
    #[serde(default)]
    pub privileged: bool,
}

fn default_upload_session_limit() -> i64 {
    100
}

const MIN_UPLOAD_CHUNK_SIZE: i64 = 256 * 1024; // 256 KiB
const MAX_UPLOAD_CHUNK_SIZE: i64 = 64 * 1024 * 1024; // 64 MiB
const MAX_ACTIVE_UPLOAD_SESSIONS_PER_PROJECT: i64 = 24;

fn is_privileged_upload_allowed(state: &AppState, project_id: i64) -> bool {
    state.db.is_session_privileged(project_id).unwrap_or(false)
}

fn upload_chunks_dir(data_dir: &str, session_id: i64) -> String {
    format!("{data_dir}/uploads/sessions/{session_id}/chunks")
}

fn upload_assembled_path(data_dir: &str, session_id: i64, file_name: &str) -> String {
    format!(
        "{data_dir}/uploads/assembled/{}_{}_{}",
        session_id,
        Utc::now().timestamp_millis(),
        sanitize_upload_name(file_name)
    )
}

fn guess_mime_from_name(file_name: &str) -> String {
    let ext = file_name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "txt" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "doc" => "application/msword",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn compute_missing_chunk_ranges(
    total_chunks: i64,
    uploaded_set: &HashSet<i64>,
    max_ranges: usize,
) -> Vec<(i64, i64)> {
    let mut ranges = Vec::new();
    let mut idx = 0i64;
    while idx < total_chunks && ranges.len() < max_ranges {
        if uploaded_set.contains(&idx) {
            idx += 1;
            continue;
        }
        let start = idx;
        idx += 1;
        while idx < total_chunks && !uploaded_set.contains(&idx) {
            idx += 1;
        }
        let end = idx - 1;
        ranges.push((start, end));
    }
    ranges
}

pub(crate) async fn create_upload_session(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(project_id): Path<i64>,
    Json(body): Json<CreateUploadSessionBody>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, project_id)?;
    let active_sessions = state
        .db
        .count_active_upload_sessions(project_id)
        .map_err(internal)?;
    if active_sessions >= MAX_ACTIVE_UPLOAD_SESSIONS_PER_PROJECT {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    if body.file_size <= 0
        || body.chunk_size < MIN_UPLOAD_CHUNK_SIZE
        || body.chunk_size > MAX_UPLOAD_CHUNK_SIZE
        || body.total_chunks <= 0
        || body.total_chunks > 1_000_000
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let expected_chunks = (body.file_size + body.chunk_size - 1) / body.chunk_size;
    if body.total_chunks != expected_chunks {
        return Err(StatusCode::BAD_REQUEST);
    }
    let current_bytes = state
        .db
        .total_project_file_bytes(project_id)
        .map_err(internal)?;
    if current_bytes + body.file_size > state.config.project_max_bytes.max(1) {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let file_name = sanitize_upload_name(&body.file_name);
    if file_name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if body.privileged && !is_privileged_upload_allowed(state.as_ref(), project_id) {
        return Err(StatusCode::FORBIDDEN);
    }
    let mime_type = body
        .mime_type
        .unwrap_or_else(|| guess_mime_from_name(&file_name));
    let session_id = state
        .db
        .create_upload_session(
            project_id,
            &file_name,
            &mime_type,
            body.file_size,
            body.chunk_size,
            body.total_chunks,
            body.is_zip || file_name.to_ascii_lowercase().ends_with(".zip"),
            body.privileged,
        )
        .map_err(internal)?;
    tracing::info!(
        target: "instrumentation.storage",
        message = "upload session created",
        user_id = user.id,
        username = user.username.as_str(),
        project_id = project_id,
        session_id = session_id,
        file_size = body.file_size,
        chunk_size = body.chunk_size,
        total_chunks = body.total_chunks,
        privileged = body.privileged,
        is_zip = body.is_zip,
    );
    Ok(Json(json!({
        "session_id": session_id,
        "project_id": project_id,
        "status": "uploading",
        "file_name": file_name,
        "privileged": body.privileged,
        "total_chunks": body.total_chunks,
        "chunk_size": body.chunk_size,
    })))
}

pub(crate) async fn list_project_upload_sessions(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(project_id): Path<i64>,
    Query(q): Query<UploadSessionsQuery>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, project_id)?;
    let sessions = state
        .db
        .list_upload_sessions(Some(project_id), q.limit)
        .map_err(internal)?;
    let counts = state
        .db
        .count_upload_sessions_by_status(Some(project_id))
        .map_err(internal)?;
    Ok(Json(json!({
        "sessions": sessions,
        "counts": counts,
    })))
}

pub(crate) async fn get_upload_overview(
    State(state): State<Arc<AppState>>,
    Query(q): Query<UploadSessionsQuery>,
) -> Result<Json<Value>, StatusCode> {
    let sessions = state
        .db
        .list_upload_sessions(None, q.limit)
        .map_err(internal)?;
    let counts = state
        .db
        .count_upload_sessions_by_status(None)
        .map_err(internal)?;
    Ok(Json(json!({
        "sessions": sessions,
        "counts": counts,
        "processing_capacity": {
            "total": state.upload_processing_limit,
            "available": state.upload_processing_sem.available_permits(),
        },
    })))
}

pub(crate) async fn upload_session_chunk(
    State(state): State<Arc<AppState>>,
    Path((project_id, session_id, chunk_index)): Path<(i64, i64, i64)>,
    bytes: Bytes,
) -> Result<Json<Value>, StatusCode> {
    let session = state
        .db
        .get_upload_session(session_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if session.project_id != project_id {
        return Err(StatusCode::NOT_FOUND);
    }
    if session.status != "uploading" {
        return Err(StatusCode::CONFLICT);
    }
    if chunk_index < 0 || chunk_index >= session.total_chunks {
        return Err(StatusCode::BAD_REQUEST);
    }
    if bytes.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let expected_size = if chunk_index == session.total_chunks - 1 {
        let rem = session.file_size - (session.chunk_size * (session.total_chunks - 1));
        rem.max(1)
    } else {
        session.chunk_size
    };
    if bytes.len() as i64 != expected_size {
        return Err(StatusCode::BAD_REQUEST);
    }
    let chunks_dir = upload_chunks_dir(&state.config.data_dir, session_id);
    tokio::fs::create_dir_all(&chunks_dir)
        .await
        .map_err(internal)?;
    let chunk_path = format!("{chunks_dir}/{chunk_index}.part");
    let mut file = tokio::fs::File::create(&chunk_path)
        .await
        .map_err(internal)?;
    file.write_all(&bytes).await.map_err(internal)?;
    file.flush().await.map_err(internal)?;
    state
        .db
        .upsert_upload_chunk(session_id, chunk_index, bytes.len() as i64)
        .map_err(internal)?;
    let updated = state
        .db
        .get_upload_session(session_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(json!({
        "session_id": session_id,
        "uploaded_bytes": updated.uploaded_bytes,
        "file_size": updated.file_size,
        "status": updated.status,
    })))
}

pub(crate) async fn get_upload_session_status(
    State(state): State<Arc<AppState>>,
    Path((project_id, session_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, StatusCode> {
    let session = state
        .db
        .get_upload_session(session_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if session.project_id != project_id {
        return Err(StatusCode::NOT_FOUND);
    }
    let uploaded_chunks = state
        .db
        .list_uploaded_chunks(session_id)
        .map_err(internal)?;
    let uploaded_set: HashSet<i64> = uploaded_chunks.iter().copied().collect();
    let uploaded_count = uploaded_set.len() as i64;
    let mut next_missing = None;
    for idx in 0..session.total_chunks {
        if !uploaded_set.contains(&idx) {
            next_missing = Some(idx);
            break;
        }
    }
    let missing_ranges = compute_missing_chunk_ranges(session.total_chunks, &uploaded_set, 128);
    Ok(Json(json!({
        "session": session,
        "uploaded_chunks": uploaded_count,
        "total_chunks": session.total_chunks,
        "missing_chunks": (session.total_chunks - uploaded_count).max(0),
        "next_missing_chunk": next_missing,
        "missing_ranges": missing_ranges,
    })))
}

async fn process_completed_upload_session(
    state: Arc<AppState>,
    project_id: i64,
    session_id: i64,
    assembled_path: String,
) {
    let session = match state.db.get_upload_session(session_id) {
        Ok(Some(s)) => s,
        _ => return,
    };
    let result = if session.is_zip {
        process_uploaded_zip(
            state.clone(),
            project_id,
            session_id,
            &assembled_path,
            session.privileged,
        )
        .await
    } else {
        process_uploaded_single_file(
            state.clone(),
            project_id,
            session_id,
            &session.file_name,
            &session.mime_type,
            &assembled_path,
            session.privileged,
        )
        .await
    };
    match result {
        Ok(stored_path) => {
            let _ =
                state
                    .db
                    .set_upload_session_state(session_id, "done", Some(&stored_path), Some(""));
            tracing::info!(
                target: "instrumentation.storage",
                message = "upload session processed",
                project_id = project_id,
                session_id = session_id,
                status = "done",
                privileged = session.privileged,
                is_zip = session.is_zip,
                file_size = session.file_size,
            );
            let _ = tokio::fs::remove_file(&assembled_path).await;
            let chunks_dir = upload_chunks_dir(&state.config.data_dir, session_id);
            let _ = tokio::fs::remove_dir_all(chunks_dir).await;
        },
        Err(e) => {
            let _ = state.db.set_upload_session_state(
                session_id,
                "failed",
                Some(&assembled_path),
                Some(&e.to_string()),
            );
            tracing::warn!(
                target: "instrumentation.storage",
                message = "upload session processed",
                project_id = project_id,
                session_id = session_id,
                status = "failed",
                privileged = session.privileged,
                is_zip = session.is_zip,
                file_size = session.file_size,
                error = e.to_string(),
            );
        },
    }
}

pub(crate) async fn retry_upload_session(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Path((project_id, session_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, StatusCode> {
    let session = state
        .db
        .get_upload_session(session_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if session.project_id != project_id {
        return Err(StatusCode::NOT_FOUND);
    }
    if session.status != "failed" {
        return Err(StatusCode::CONFLICT);
    }
    if session.stored_path.trim().is_empty() {
        return Err(StatusCode::CONFLICT);
    }
    state
        .db
        .set_upload_session_state(
            session_id,
            "processing",
            Some(&session.stored_path),
            Some(""),
        )
        .map_err(internal)?;

    let state_cloned = state.clone();
    let sem = Arc::clone(&state.upload_processing_sem);
    let assembled_path = session.stored_path.clone();
    tokio::spawn(async move {
        let Ok(_permit) = sem.acquire_owned().await else {
            let _ = state_cloned.db.set_upload_session_state(
                session_id,
                "failed",
                Some(&assembled_path),
                Some("upload processing semaphore unavailable"),
            );
            return;
        };
        process_completed_upload_session(state_cloned, project_id, session_id, assembled_path)
            .await;
    });

    tracing::info!(
        target: "instrumentation.storage",
        message = "upload session retried",
        user_id = user.id,
        username = user.username.as_str(),
        project_id = project_id,
        session_id = session_id,
        file_size = session.file_size,
        privileged = session.privileged,
        is_zip = session.is_zip,
    );

    Ok(Json(json!({
        "session_id": session_id,
        "status": "processing",
    })))
}

async fn process_uploaded_single_file(
    state: Arc<AppState>,
    project_id: i64,
    _session_id: i64,
    file_name: &str,
    mime_type: &str,
    assembled_path: &str,
    privileged: bool,
) -> anyhow::Result<String> {
    let content_hash = sha256_hex_file(assembled_path).await?;
    if let Some(existing) = state
        .db
        .find_project_file_by_hash(project_id, &content_hash)?
    {
        if privileged {
            let _ = state.db.set_session_privileged(project_id);
        }
        return Ok(existing.stored_path);
    }
    let safe_name = sanitize_upload_name(file_name);
    let source_path = sanitize_upload_relative_path(file_name);
    let unique_name = format!(
        "{}_{}_{}",
        Utc::now().timestamp_millis(),
        rand_suffix(),
        safe_name
    );
    let stored_path = state
        .file_storage
        .put_project_file_from_path(project_id, &unique_name, assembled_path)
        .await?;
    let size_bytes = tokio::fs::metadata(assembled_path).await?.len() as i64;
    let file_id = state.db.insert_project_file(
        project_id,
        &safe_name,
        &source_path,
        &stored_path,
        mime_type,
        size_bytes,
        &content_hash,
        privileged,
    )?;
    if let Err(e) = state
        .ingestion_queue
        .enqueue_project_file(
            project_id,
            file_id,
            file_name,
            &stored_path,
            mime_type,
            size_bytes,
        )
        .await
    {
        tracing::warn!("failed to enqueue uploaded file ingest: {e}");
    }
    if matches!(state.ingestion_queue.as_ref(), IngestionQueue::Disabled) {
        if let Ok(Some(row)) = state.db.get_project_file(project_id, file_id) {
            extract_and_index_project_file(state.as_ref(), &row).await;
        }
    }
    Ok(stored_path)
}

async fn process_uploaded_zip(
    state: Arc<AppState>,
    project_id: i64,
    session_id: i64,
    assembled_path: &str,
    privileged: bool,
) -> anyhow::Result<String> {
    let assembled_path_owned = assembled_path.to_string();
    let state_for_zip = state.clone();
    let (imported, deduped, pending_rows) =
        tokio::task::spawn_blocking(move || -> anyhow::Result<(i64, i64, Vec<ProjectFileRow>)> {
            let handle = tokio::runtime::Handle::current();
            let tmp_dir = std::path::Path::new(&state_for_zip.config.data_dir).join("uploads/tmp");
            std::fs::create_dir_all(&tmp_dir)?;
            let file = std::fs::File::open(&assembled_path_owned)?;
            let mut archive = zip::ZipArchive::new(file)?;
            let mut imported = 0i64;
            let mut deduped = 0i64;
            let queue_disabled = matches!(
                state_for_zip.ingestion_queue.as_ref(),
                IngestionQueue::Disabled
            );
            let mut pending_rows: Vec<ProjectFileRow> = Vec::new();

            for idx in 0..archive.len() {
                let mut entry = archive.by_index(idx)?;
                if entry.is_dir() {
                    continue;
                }

                let raw_name = entry.name().to_string();
                let fallback = format!("file-{idx}");
                let source_path = sanitize_upload_relative_path(&raw_name);
                let leaf = std::path::Path::new(&source_path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&fallback);
                let file_name = sanitize_upload_name(leaf);
                if file_name.is_empty() {
                    continue;
                }

                let mut tmp = tempfile::NamedTempFile::new_in(&tmp_dir)?;
                std::io::copy(&mut entry, &mut tmp)?;
                let tmp_path = tmp.path().to_path_buf();
                let content_hash = sha256_hex_file_blocking(&tmp_path)?;
                if state_for_zip
                    .db
                    .find_project_file_by_hash(project_id, &content_hash)?
                    .is_some()
                {
                    if privileged {
                        let _ = state_for_zip.db.set_session_privileged(project_id);
                    }
                    deduped += 1;
                    continue;
                }

                let unique_name = format!(
                    "{}_{}_{}",
                    Utc::now().timestamp_millis(),
                    rand_suffix(),
                    sanitize_upload_name(&file_name)
                );
                let tmp_path_str = tmp_path.to_string_lossy().to_string();
                let stored_path =
                    handle.block_on(state_for_zip.file_storage.put_project_file_from_path(
                        project_id,
                        &unique_name,
                        &tmp_path_str,
                    ))?;
                let size_bytes = tmp.as_file().metadata()?.len() as i64;
                let mime_type = guess_mime_from_name(&file_name);
                let file_id = state_for_zip.db.insert_project_file(
                    project_id,
                    &file_name,
                    &source_path,
                    &stored_path,
                    &mime_type,
                    size_bytes,
                    &content_hash,
                    privileged,
                )?;
                if let Err(e) = handle.block_on(state_for_zip.ingestion_queue.enqueue_project_file(
                    project_id,
                    file_id,
                    &file_name,
                    &stored_path,
                    &mime_type,
                    size_bytes,
                )) {
                    tracing::warn!("failed to enqueue zip file ingest: {e}");
                }
                if queue_disabled {
                    if let Ok(Some(row)) = state_for_zip.db.get_project_file(project_id, file_id) {
                        pending_rows.push(row);
                    }
                }
                imported += 1;
            }
            Ok((imported, deduped, pending_rows))
        })
        .await
        .map_err(|e| anyhow::anyhow!("zip extraction join error: {e}"))??;

    // Process extracted files concurrently (embedding + Vespa indexing)
    if !pending_rows.is_empty() {
        let state2 = state.clone();
        tokio::spawn(async move {
            process_files_concurrently(&state2, pending_rows).await;
        });
    }
    Ok(format!(
        "zip://session/{session_id}/imported/{imported}/deduped/{deduped}"
    ))
}

pub(crate) async fn complete_upload_session(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Path((project_id, session_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, StatusCode> {
    let session = state
        .db
        .get_upload_session(session_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if session.project_id != project_id {
        return Err(StatusCode::NOT_FOUND);
    }
    if session.status != "uploading" {
        return Err(StatusCode::CONFLICT);
    }
    let uploaded = state
        .db
        .list_uploaded_chunks(session_id)
        .map_err(internal)?;
    let uploaded_set: HashSet<i64> = uploaded.into_iter().collect();
    for idx in 0..session.total_chunks {
        if !uploaded_set.contains(&idx) {
            return Err(StatusCode::CONFLICT);
        }
    }

    let assembled_dir = format!("{}/uploads/assembled", state.config.data_dir);
    tokio::fs::create_dir_all(&assembled_dir)
        .await
        .map_err(internal)?;
    let assembled_path =
        upload_assembled_path(&state.config.data_dir, session_id, &session.file_name);
    let mut assembled_file = tokio::fs::File::create(&assembled_path)
        .await
        .map_err(internal)?;
    let chunks_dir = upload_chunks_dir(&state.config.data_dir, session_id);
    for idx in 0..session.total_chunks {
        let chunk_path = format!("{chunks_dir}/{idx}.part");
        let mut chunk_file = tokio::fs::File::open(&chunk_path).await.map_err(internal)?;
        tokio::io::copy(&mut chunk_file, &mut assembled_file)
            .await
            .map_err(internal)?;
    }
    assembled_file.flush().await.map_err(internal)?;
    state
        .db
        .set_upload_session_state(session_id, "processing", Some(&assembled_path), Some(""))
        .map_err(internal)?;

    let state_cloned = state.clone();
    let sem = Arc::clone(&state.upload_processing_sem);
    tokio::spawn(async move {
        let Ok(_permit) = sem.acquire_owned().await else {
            let _ = state_cloned.db.set_upload_session_state(
                session_id,
                "failed",
                None,
                Some("upload processing semaphore unavailable"),
            );
            return;
        };
        process_completed_upload_session(state_cloned, project_id, session_id, assembled_path)
            .await;
    });

    tracing::info!(
        target: "instrumentation.storage",
        message = "upload session completed",
        user_id = user.id,
        username = user.username.as_str(),
        project_id = project_id,
        session_id = session_id,
        total_chunks = session.total_chunks,
        file_size = session.file_size,
        privileged = session.privileged,
        is_zip = session.is_zip,
    );

    Ok(Json(json!({
        "session_id": session_id,
        "status": "processing",
    })))
}

pub(crate) async fn upload_project_files(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Query(q): Query<UploadProjectFilesQuery>,
    mut multipart: Multipart,
) -> Result<Json<Value>, StatusCode> {
    let max_project_bytes = state.config.project_max_bytes.max(1);
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    if q.privileged && !is_privileged_upload_allowed(state.as_ref(), id) {
        return Err(StatusCode::FORBIDDEN);
    }

    let mut total_bytes = state.db.total_project_file_bytes(id).map_err(internal)?;
    let mut uploaded: Vec<ProjectFileJson> = Vec::new();
    let mut deduped = 0i64;
    let mut uploaded_rows: Vec<ProjectFileRow> = Vec::new();
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
        let content_hash = sha256_hex_bytes(&bytes);
        if state
            .db
            .find_project_file_by_hash(id, &content_hash)
            .map_err(internal)?
            .is_some()
        {
            if q.privileged {
                let _ = state.db.set_session_privileged(id);
            }
            deduped += 1;
            continue;
        }
        if total_bytes + file_size > max_project_bytes {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }

        let unique_name = format!(
            "{}_{}_{}",
            Utc::now().timestamp_millis(),
            rand_suffix(),
            file_name
        );
        let stored_path = state
            .file_storage
            .put_project_file(id, &unique_name, &bytes)
            .await
            .map_err(internal)?;

        let file_id = state
            .db
            .insert_project_file(
                id,
                &file_name,
                &file_name,
                &stored_path,
                &mime_type,
                file_size,
                &content_hash,
                q.privileged,
            )
            .map_err(internal)?;
        if let Err(e) = state
            .ingestion_queue
            .enqueue_project_file(id, file_id, &file_name, &stored_path, &mime_type, file_size)
            .await
        {
            tracing::warn!("failed to enqueue project file ingest: {e}");
        }
        total_bytes += file_size;

        let inserted = state
            .db
            .get_project_file(id, file_id)
            .map_err(internal)?
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        uploaded_rows.push(inserted.clone());
        uploaded.push(ProjectFileJson::from(inserted));
    }

    if matches!(state.ingestion_queue.as_ref(), IngestionQueue::Disabled)
        && !uploaded_rows.is_empty()
    {
        let state2 = Arc::clone(&state);
        let rows = uploaded_rows;
        tokio::spawn(async move {
            process_files_concurrently(&state2, rows).await;
        });
    }

    let uploaded_bytes: i64 = uploaded.iter().map(|file| file.size_bytes).sum();
    tracing::info!(
        target: "instrumentation.storage",
        message = "project files uploaded",
        user_id = user.id,
        username = user.username.as_str(),
        project_id = id,
        uploaded_count = uploaded.len() as u64,
        uploaded_bytes = uploaded_bytes,
        deduped = deduped,
        privileged = q.privileged,
    );

    Ok(Json(json!({ "uploaded": uploaded, "deduped": deduped })))
}

async fn chunk_embed_and_index(
    search: &crate::search::SearchClient,
    embed_client: &borg_core::knowledge::EmbeddingClient,
    project_id: i64,
    file_id: i64,
    file_path: &str,
    title: &str,
    text: &str,
    privileged: bool,
    mime_type: &str,
) {
    let _ = search.delete_file_chunks(project_id, file_id).await;
    let chunks_text = borg_core::knowledge::chunk_text(text);
    if chunks_text.is_empty() {
        return;
    }
    let metadata = ChunkMetadata {
        doc_type: detect_doc_type(title, mime_type, text),
        jurisdiction: crate::ingestion::detect_jurisdiction(text),
        privileged,
        mime_type: mime_type.to_string(),
    };
    let chunks_with_embeddings =
        crate::ingestion::batch_embed_chunks(&chunks_text, Some(embed_client), embed_client.dim())
            .await;
    if let Err(e) = search
        .index_chunks(
            project_id,
            file_id,
            file_path,
            title,
            &chunks_with_embeddings,
            &metadata,
        )
        .await
    {
        tracing::warn!("chunk index failed for file {file_id}: {e}");
    }
}

/// Process files with cross-file batch embedding to minimize API calls.
/// Extracts text from all files, batches their chunks into large embedding calls,
/// then indexes to Vespa concurrently.
async fn process_files_concurrently(state: &Arc<AppState>, files: Vec<ProjectFileRow>) {
    // Phase 1: Extract text from all files concurrently
    struct ExtractedFile {
        row: ProjectFileRow,
        text: String,
        source_path: String,
    }
    let mut extracted: Vec<ExtractedFile> = Vec::with_capacity(files.len());
    let mut set = tokio::task::JoinSet::new();
    let sem = Arc::new(tokio::sync::Semaphore::new(20));

    for file in files {
        let state = Arc::clone(state);
        let sem = Arc::clone(&sem);
        set.spawn(async move {
            let _permit = sem.acquire().await;
            if !file.extracted_text.is_empty() {
                return None;
            }
            let source_path = if file.source_path.is_empty() {
                file.file_name.clone()
            } else {
                file.source_path.clone()
            };
            let bytes = state.file_storage.read_all(&file.stored_path).await.ok()?;
            let text = extract_text_from_bytes(&file.file_name, &file.mime_type, &bytes)
                .await
                .ok()?;
            if text.is_empty() {
                return None;
            }
            let _ = state.db.update_project_file_text(file.id, &text);
            let _ = state.db.fts_index_document(
                file.project_id,
                0,
                &source_path,
                &file.file_name,
                &text,
            );
            Some(ExtractedFile {
                row: file,
                text,
                source_path,
            })
        });
    }
    while let Some(result) = set.join_next().await {
        if let Ok(Some(ef)) = result {
            extracted.push(ef);
        }
    }

    let Some(search) = &state.search else {
        return;
    };
    let embed_client = state.embed_registry.default_client();

    // Phase 2: Chunk all files and collect a flat list with file ownership
    let mut file_chunk_ranges: Vec<(usize, usize)> = Vec::with_capacity(extracted.len());
    let mut all_chunk_texts: Vec<String> = Vec::new();

    for ef in &extracted {
        let chunks = borg_core::knowledge::chunk_text(&ef.text);
        let start = all_chunk_texts.len();
        all_chunk_texts.extend(chunks);
        file_chunk_ranges.push((start, all_chunk_texts.len()));
    }

    if all_chunk_texts.is_empty() {
        return;
    }

    tracing::info!(
        "batch embedding {} chunks across {} files",
        all_chunk_texts.len(),
        extracted.len()
    );

    // Phase 3: Batch embed ALL chunks across files (128 per API call)
    let embeddings = crate::ingestion::batch_embed_chunks(
        &all_chunk_texts,
        Some(embed_client),
        embed_client.dim(),
    )
    .await;

    // Phase 4: Reassemble per-file chunks with embeddings and index to Vespa concurrently
    let mut index_set = tokio::task::JoinSet::new();
    let index_sem = Arc::new(tokio::sync::Semaphore::new(20));
    for (file_idx, ef) in extracted.into_iter().enumerate() {
        let (start, end) = file_chunk_ranges[file_idx];
        let chunks_with_embeddings: Vec<(String, Vec<f32>)> = embeddings[start..end].to_vec();
        if chunks_with_embeddings.is_empty() {
            continue;
        }
        let search = search.clone();
        let sem = Arc::clone(&index_sem);
        let metadata = ChunkMetadata {
            doc_type: detect_doc_type(&ef.row.file_name, &ef.row.mime_type, &ef.text),
            jurisdiction: crate::ingestion::detect_jurisdiction(&ef.text),
            privileged: ef.row.privileged,
            mime_type: ef.row.mime_type.clone(),
        };
        index_set.spawn(async move {
            let _permit = sem.acquire().await;
            let _ = search
                .delete_file_chunks(ef.row.project_id, ef.row.id)
                .await;
            if let Err(e) = search
                .index_chunks(
                    ef.row.project_id,
                    ef.row.id,
                    &ef.source_path,
                    &ef.row.file_name,
                    &chunks_with_embeddings,
                    &metadata,
                )
                .await
            {
                tracing::warn!("batch index failed for file {}: {e}", ef.row.id);
            }
        });
    }
    while index_set.join_next().await.is_some() {}

    tracing::info!(
        "batch processed {} chunks across {} files",
        all_chunk_texts.len(),
        file_chunk_ranges.len()
    );
}

async fn extract_and_index_project_file(state: &AppState, file: &ProjectFileRow) {
    if !file.extracted_text.is_empty() {
        return;
    }
    let source_path = if file.source_path.is_empty() {
        file.file_name.as_str()
    } else {
        file.source_path.as_str()
    };
    let bytes = match state.file_storage.read_all(&file.stored_path).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("project file read failed for {}: {e}", file.file_name);
            return;
        },
    };
    let text = match extract_text_from_bytes(&file.file_name, &file.mime_type, &bytes).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("project file extract failed for {}: {e}", file.file_name);
            return;
        },
    };
    if text.is_empty() {
        return;
    }
    if let Err(e) = state.db.update_project_file_text(file.id, &text) {
        tracing::warn!(
            "project file text update failed for {}: {e}",
            file.file_name
        );
        return;
    }
    if let Err(e) =
        state
            .db
            .fts_index_document(file.project_id, 0, source_path, &file.file_name, &text)
    {
        tracing::warn!("project file fts index failed for {}: {e}", file.file_name);
        return;
    }
    if let Some(search) = &state.search {
        chunk_embed_and_index(
            search,
            state.embed_registry.default_client(),
            file.project_id,
            file.id,
            source_path,
            &file.file_name,
            &text,
            file.privileged,
            &file.mime_type,
        )
        .await;
    }
    tracing::info!("extracted {} chars from {}", text.len(), file.file_name);
}

pub(crate) async fn get_project_file_text(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path((project_id, file_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, project_id)?;
    let file = state
        .db
        .get_project_file(project_id, file_id)
        .map_err(internal)?
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
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path((project_id, file_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, project_id)?;
    let file = state
        .db
        .get_project_file(project_id, file_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let bytes = state
        .file_storage
        .read_all(&file.stored_path)
        .await
        .map_err(internal)?;
    let text = extract_text_from_bytes(&file.file_name, &file.mime_type, &bytes)
        .await
        .map_err(internal)?;
    if !text.is_empty() {
        state
            .db
            .update_project_file_text(file_id, &text)
            .map_err(internal)?;
        state
            .db
            .fts_index_document(project_id, 0, &file.file_name, &file.file_name, &text)
            .map_err(internal)?;
        if let Some(search) = &state.search {
            chunk_embed_and_index(
                search,
                state.embed_registry.default_client(),
                project_id,
                file_id,
                &file.file_name,
                &file.file_name,
                &text,
                file.privileged,
                &file.mime_type,
            )
            .await;
        }
    }
    Ok(Json(json!({
        "id": file_id,
        "extracted_text_chars": text.len(),
        "has_text": !text.is_empty(),
    })))
}

pub(crate) async fn list_queue(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let entries = state.db.list_queue().map_err(internal)?;
    Ok(Json(json!(entries)))
}

pub(crate) async fn get_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let uptime_s = state.start_time.elapsed().as_secs();
    let now = chrono::Utc::now().timestamp();

    let watched_repos: Vec<Value> = state
        .config
        .watched_repos
        .iter()
        .map(|r| {
            json!({
                "path": r.path,
                "test_cmd": r.test_cmd,
                "is_self": r.is_self,
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

    let rebase_count = state
        .db
        .count_tasks_with_status("rebase")
        .map_err(internal)?;
    let queued_count = state
        .db
        .count_queue_with_status("queued")
        .map_err(internal)?
        + state
            .db
            .count_queue_with_status("merging")
            .map_err(internal)?;
    let last_merge_ts = state.db.get_ts("last_release_ts");
    let no_merge_mins = if last_merge_ts > 0 {
        ((now - last_merge_ts).max(0)) / 60
    } else {
        0
    };
    let rebase_backlog_alert = rebase_count >= 50;
    let no_merge_alert = queued_count > 0 && last_merge_ts > 0 && (now - last_merge_ts) >= 60 * 60;
    let guardrail_alert = rebase_backlog_alert || no_merge_alert;
    let ai_requests = state.ai_request_count.load(Ordering::Relaxed);
    state.db.set_ts("ai_request_count", ai_requests as i64);
    let backup = crate::backup::backup_status_snapshot(&state.db, &state.config).await;

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
        "ai_requests": ai_requests,
        "failed_tasks": failed,
        "total_tasks": total,
        "dispatched_agents": 0,
        "guardrail_alert": guardrail_alert,
        "guardrail_rebase_count": rebase_count,
        "guardrail_queued_count": queued_count,
        "guardrail_no_merge_mins": no_merge_mins,
        "storage": {
            "backend": state.file_storage.backend_name(),
            "target": state.file_storage.target(),
        },
        "search": {
            "backend": state.search.as_ref().map(|s| s.backend_name()).unwrap_or("none"),
            "target": state.search.as_ref().map(|s| s.target()).unwrap_or_default(),
        },
        "backup": backup,
    })))
}

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
        updated_at: Utc::now(),
        session_id: String::new(),
        mode: state
            .config
            .watched_repos
            .iter()
            .find(|r| r.path == proposal.repo_path)
            .map(|r| r.mode.clone())
            .unwrap_or_else(|| "sweborg".into()),
        backend: String::new(),
        workspace_id: 0,
        project_id: 0,
        task_type: String::new(),
        requires_exhaustive_corpus_review: false,
        started_at: None,
        completed_at: None,
        duration_secs: None,
        review_status: None,
        revision_count: 0,
        chat_thread: String::new(),
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
    if state
        .triage_running
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
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
                updated_at: chrono::Utc::now(),
                session_id: String::new(),
                mode: "sweborg".into(),
                backend: String::new(),
                workspace_id: 0,
                project_id: 0,
                task_type: String::new(),
                requires_exhaustive_corpus_review: false,
                started_at: None,
                completed_at: None,
                duration_secs: None,
                review_status: None,
                revision_count: 0,
                chat_thread: String::new(),
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
                phase_attempt: task.attempt,
                phase_gate_token: format!(
                    "triage:{}:{}",
                    task.id,
                    chrono::Utc::now()
                        .timestamp_nanos_opt()
                        .unwrap_or_else(|| chrono::Utc::now().timestamp_micros() * 1_000)
                ),
                system_prompt_suffix: String::new(),
                user_coauthor: String::new(),
                stream_tx: None,
                setup_script: String::new(),
                api_keys: std::collections::HashMap::new(),
                disallowed_tools: String::new(),
                knowledge_files: Vec::new(),
                knowledge_dir: String::new(),
                knowledge_repo_paths: Vec::new(),
                agent_network: None,
                prior_research: Vec::new(),
                revision_count: 0,
                experimental_domains: state.config.experimental_domains,
                isolated: true, // Triage is always secure/isolated
                borg_api_url: format!("http://127.0.0.1:{}", state.config.web_port),
                borg_api_token: state.api_token.clone(),
                chat_context: Vec::new(),
                github_token: state.config.github_token.clone(),
            };

            tokio::fs::create_dir_all(&ctx.session_dir).await.ok();

            state.ai_request_count.fetch_add(1, Ordering::Relaxed);
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

// Settings

pub(crate) async fn get_settings(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
) -> Result<Json<Value>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let mut obj = serde_json::Map::new();
    for key in SETTINGS_KEYS {
        let val = state.db.get_config(key).map_err(internal)?;
        let default = SETTINGS_DEFAULTS
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| *v)
            .unwrap_or("");
        let s = val.as_deref().unwrap_or(default);
        let json_val = if matches!(
            *key,
            "continuous_mode" | "git_claude_coauthor" | "experimental_domains"
        ) {
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
                | "project_max_bytes"
                | "knowledge_max_bytes"
                | "cloud_import_max_batch_files"
                | "backup_poll_interval_s"
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
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
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

// ── User management (admin-only) ────────────────────────────────────────

pub(crate) async fn list_users(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
) -> Result<Json<Value>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let users = state.db.list_users().map_err(internal)?;
    let arr: Vec<Value> = users
        .into_iter()
        .map(|(id, username, display_name, is_admin, created_at)| {
            json!({ "id": id, "username": username, "display_name": display_name, "is_admin": is_admin, "created_at": created_at })
        })
        .collect();
    Ok(Json(json!(arr)))
}

#[derive(Deserialize)]
pub(crate) struct CreateUserBody {
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
    pub is_admin: Option<bool>,
}

pub(crate) async fn create_user(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Json(body): Json<CreateUserBody>,
) -> Result<Json<Value>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    if body.username.trim().is_empty() || body.password.len() < 4 {
        return Ok(Json(
            json!({"error": "username required, password min 4 chars"}),
        ));
    }
    let hash = crate::auth::hash_password(&body.password).map_err(|e| {
        tracing::error!("hash_password: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let display = body.display_name.as_deref().unwrap_or(&body.username);
    let is_admin = body.is_admin.unwrap_or(false);
    let id = state
        .db
        .create_user(&body.username, display, &hash, is_admin)
        .map_err(internal)?;
    Ok(Json(
        json!({ "id": id, "username": body.username, "display_name": display, "is_admin": is_admin }),
    ))
}

pub(crate) async fn delete_user(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    if id == user.id {
        return Ok(Json(json!({"error": "cannot delete yourself"})));
    }
    state.db.delete_user(id).map_err(internal)?;
    Ok(Json(json!({ "deleted": id })))
}

#[derive(Deserialize)]
pub(crate) struct ChangePasswordBody {
    pub password: String,
}

pub(crate) async fn change_password(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Path(id): Path<i64>,
    Json(body): Json<ChangePasswordBody>,
) -> Result<Json<Value>, StatusCode> {
    if !user.is_admin && user.id != id {
        return Err(StatusCode::FORBIDDEN);
    }
    if body.password.len() < 4 {
        return Ok(Json(json!({"error": "password min 4 chars"})));
    }
    let hash = crate::auth::hash_password(&body.password).map_err(|e| {
        tracing::error!("hash_password: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    state.db.update_user_password(id, &hash).map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

fn workspace_role_can_manage(role: &str) -> bool {
    matches!(role, "owner" | "admin")
}

pub(crate) async fn list_workspaces(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    if user.id == 0 {
        return Ok(Json(json!({
            "workspaces": [{
                "workspace_id": workspace.id,
                "name": workspace.name,
                "slug": "",
                "kind": workspace.kind,
                "role": workspace.role,
                "is_default": workspace.is_default,
                "created_at": "",
            }],
            "default_workspace_id": workspace.id,
        })));
    }
    let workspaces = state.db.list_user_workspaces(user.id).map_err(internal)?;
    Ok(Json(json!({
        "workspaces": workspaces,
        "default_workspace_id": user.default_workspace_id,
    })))
}

pub(crate) async fn create_workspace(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Json(body): Json<CreateWorkspaceBody>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    if user.id == 0 {
        return Err(StatusCode::FORBIDDEN);
    }
    let name = body.name.trim();
    if name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let kind = body.kind.as_deref().unwrap_or("shared");
    if !matches!(kind, "shared" | "org") {
        return Err(StatusCode::BAD_REQUEST);
    }
    let workspace_id = state
        .db
        .create_workspace(name, kind, Some(user.id))
        .map_err(internal)?;
    state
        .db
        .add_workspace_member(workspace_id, user.id, "owner")
        .map_err(internal)?;
    if body.set_default.unwrap_or(false) {
        state
            .db
            .set_user_default_workspace_id(user.id, workspace_id)
            .map_err(internal)?;
    }
    let workspace = state
        .db
        .get_workspace(workspace_id)
        .map_err(internal)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "workspace": workspace,
            "default_workspace_id": if body.set_default.unwrap_or(false) { workspace_id } else { user.default_workspace_id },
        })),
    ))
}

pub(crate) async fn select_workspace(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    if user.id == 0 {
        return Err(StatusCode::FORBIDDEN);
    }
    let membership = state
        .db
        .get_user_workspace_membership(user.id, id)
        .map_err(internal)?
        .ok_or(StatusCode::FORBIDDEN)?;
    state
        .db
        .set_user_default_workspace_id(user.id, id)
        .map_err(internal)?;
    Ok(Json(json!({
        "ok": true,
        "workspace_id": membership.workspace_id,
    })))
}

pub(crate) async fn add_workspace_member(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Path(id): Path<i64>,
    Json(body): Json<AddWorkspaceMemberBody>,
) -> Result<Json<Value>, StatusCode> {
    let membership = state
        .db
        .get_user_workspace_membership(user.id, id)
        .map_err(internal)?
        .ok_or(StatusCode::FORBIDDEN)?;
    if !user.is_admin && !workspace_role_can_manage(&membership.role) {
        return Err(StatusCode::FORBIDDEN);
    }
    let role = body.role.as_deref().unwrap_or("member");
    if !matches!(role, "owner" | "admin" | "member" | "viewer") {
        return Err(StatusCode::BAD_REQUEST);
    }
    let target = state
        .db
        .get_user_by_username(body.username.trim())
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    state
        .db
        .add_workspace_member(id, target.0, role)
        .map_err(internal)?;
    Ok(Json(json!({
        "ok": true,
        "workspace_id": id,
        "user_id": target.0,
        "role": role,
    })))
}

// ── Per-user settings ────────────────────────────────────────────────────

const USER_SETTINGS_KEYS: &[&str] = &[
    "model",
    "backend",
    "github_token",
    "gitlab_token",
    "codeberg_token",
    "telegram_bot_token",
    "telegram_bot_username",
    "contact_email",
    "discord_bot_token",
    "discord_bot_username",
    "dashboard_mode",
];
/// Keys that cannot be set via the generic PUT endpoint (use dedicated routes).
const USER_SETTINGS_PROTECTED: &[&str] = &[
    "telegram_bot_token",
    "telegram_bot_username",
    "discord_bot_token",
    "discord_bot_username",
];

pub(crate) async fn get_user_settings(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
) -> Result<Json<Value>, StatusCode> {
    let settings = state.db.get_all_user_settings(user.id).map_err(internal)?;

    // Check for global model override
    let model_override = state.db.get_config("model_override").map_err(internal)?;
    let has_override = model_override.as_ref().map_or(false, |v| !v.is_empty());

    let mut obj = serde_json::Map::new();
    for key in USER_SETTINGS_KEYS {
        let val = settings.get(*key).cloned().unwrap_or_default();
        match *key {
            "github_token" => {
                obj.insert("github_token_set".to_string(), json!(!val.is_empty()));
            },
            "gitlab_token" => {
                obj.insert("gitlab_token_set".to_string(), json!(!val.is_empty()));
            },
            "codeberg_token" => {
                obj.insert("codeberg_token_set".to_string(), json!(!val.is_empty()));
            },
            "telegram_bot_token" => {
                // Exposed via dedicated telegram-bot endpoints, not here
            },
            _ => {
                obj.insert(key.to_string(), json!(val));
            },
        }
    }
    obj.insert(
        "model_override".to_string(),
        json!(model_override.unwrap_or_default()),
    );
    obj.insert("model_override_active".to_string(), json!(has_override));

    // Telegram bot status
    let tg_username = settings.get("telegram_bot_username").cloned().unwrap_or_default();
    let tg_connected = !settings
        .get("telegram_bot_token")
        .map(|t| t.is_empty())
        .unwrap_or(true);
    obj.insert("telegram_bot_connected".to_string(), json!(tg_connected));
    obj.insert("telegram_bot_username".to_string(), json!(tg_username));

    // Discord bot status
    let dc_username = settings.get("discord_bot_username").cloned().unwrap_or_default();
    let dc_connected = !settings
        .get("discord_bot_token")
        .map(|t| t.is_empty())
        .unwrap_or(true);
    obj.insert("discord_bot_connected".to_string(), json!(dc_connected));
    obj.insert("discord_bot_username".to_string(), json!(dc_username));

    Ok(Json(Value::Object(obj)))
}

pub(crate) async fn put_user_settings(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let map = body.as_object().ok_or(StatusCode::BAD_REQUEST)?;
    let mut updated = 0usize;
    for (key, val) in map {
        if !USER_SETTINGS_KEYS.contains(&key.as_str())
            || USER_SETTINGS_PROTECTED.contains(&key.as_str())
        {
            continue;
        }
        let s = match val {
            Value::String(s) => s.clone(),
            _ => continue,
        };
        if s.is_empty() {
            state
                .db
                .delete_user_setting(user.id, key)
                .map_err(internal)?;
        } else {
            state
                .db
                .set_user_setting(user.id, key, &s)
                .map_err(internal)?;
        }
        updated += 1;
    }
    Ok(Json(json!({ "updated": updated })))
}

// ── Per-user Telegram bot ─────────────────────────────────────────────

pub(crate) async fn connect_telegram_bot(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let token = body["token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or(StatusCode::BAD_REQUEST)?;

    // Validate the token by calling getMe
    let client = reqwest::Client::new();
    let resp: Value = client
        .get(format!("https://api.telegram.org/bot{token}/getMe"))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        .json()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    let username = resp["result"]["username"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?
        .to_string();

    state
        .db
        .set_user_setting(user.id, "telegram_bot_token", token)
        .map_err(internal)?;
    state
        .db
        .set_user_setting(user.id, "telegram_bot_username", &username)
        .map_err(internal)?;

    tracing::info!(
        user_id = user.id,
        bot = %username,
        "user connected telegram bot"
    );

    Ok(Json(json!({
        "ok": true,
        "bot_username": username,
    })))
}

pub(crate) async fn disconnect_telegram_bot(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
) -> Result<Json<Value>, StatusCode> {
    state
        .db
        .delete_user_setting(user.id, "telegram_bot_token")
        .map_err(internal)?;
    state
        .db
        .delete_user_setting(user.id, "telegram_bot_username")
        .map_err(internal)?;

    tracing::info!(user_id = user.id, "user disconnected telegram bot");

    Ok(Json(json!({ "ok": true })))
}

// ── Per-user Discord bot ──────────────────────────────────────────────

pub(crate) async fn connect_discord_bot(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let token = body["token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or(StatusCode::BAD_REQUEST)?;

    // Validate the token by calling Discord's /users/@me
    let client = reqwest::Client::new();
    let resp: Value = client
        .get("https://discord.com/api/v10/users/@me")
        .header("Authorization", format!("Bot {token}"))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        .json()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    let username = resp["username"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?
        .to_string();

    state
        .db
        .set_user_setting(user.id, "discord_bot_token", token)
        .map_err(internal)?;
    state
        .db
        .set_user_setting(user.id, "discord_bot_username", &username)
        .map_err(internal)?;

    tracing::info!(
        user_id = user.id,
        bot = %username,
        "user connected discord bot"
    );

    Ok(Json(json!({
        "ok": true,
        "bot_username": username,
    })))
}

pub(crate) async fn disconnect_discord_bot(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
) -> Result<Json<Value>, StatusCode> {
    state
        .db
        .delete_user_setting(user.id, "discord_bot_token")
        .map_err(internal)?;
    state
        .db
        .delete_user_setting(user.id, "discord_bot_username")
        .map_err(internal)?;

    tracing::info!(user_id = user.id, "user disconnected discord bot");

    Ok(Json(json!({ "ok": true })))
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
                Err(e) => {
                    tracing::debug!("log SSE broadcast closed: {e}");
                    break;
                },
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
                    Err(e) => {
                        tracing::debug!("task SSE broadcast closed: {e}");
                        break;
                    },
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
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut live_rx = state.chat_event_tx.subscribe();
    let db = Arc::clone(&state.db);
    tokio::spawn(async move {
        loop {
            match live_rx.recv().await {
                Ok(line) => {
                    let Some(filtered) =
                        serde_json::from_str::<Value>(&line)
                            .ok()
                            .and_then(|mut payload| {
                                let thread = payload.get("thread")?.as_str()?;
                                let visible = visible_chat_thread_for_workspace(
                                    db.as_ref(),
                                    workspace.id,
                                    thread,
                                )?;
                                if let Some(obj) = payload.as_object_mut() {
                                    obj.insert("thread".into(), Value::String(visible));
                                }
                                serde_json::to_string(&payload).ok()
                            })
                    else {
                        continue;
                    };
                    if tx.send(filtered).is_err() {
                        return;
                    }
                },
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("chat SSE client lagged by {n} events, continuing");
                    continue;
                },
                Err(e) => {
                    tracing::debug!("chat SSE broadcast closed: {e}");
                    break;
                },
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

pub(crate) async fn get_chat_threads(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    let threads = state.db.get_chat_threads().map_err(internal)?;
    let v: Vec<Value> = threads
        .into_iter()
        .filter_map(|(jid, count, last_ts)| {
            visible_chat_thread_for_workspace(state.db.as_ref(), workspace.id, &jid).map(
                |visible_id| json!({ "id": visible_id, "message_count": count, "last_ts": last_ts }),
            )
        })
        .collect();
    Ok(Json(json!(v)))
}

pub(crate) async fn get_chat_messages(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Query(q): Query<ChatMessagesQuery>,
) -> Result<Json<Value>, StatusCode> {
    let actual_thread = if parse_project_chat_key(&q.thread).is_some() {
        let project_id = parse_project_chat_key(&q.thread).ok_or(StatusCode::BAD_REQUEST)?;
        require_project_access(state.as_ref(), &workspace, project_id)?;
        q.thread.clone()
    } else {
        scoped_workspace_chat_thread(workspace.id, &q.thread)
    };
    let msgs = match state
        .db
        .get_chat_messages(&actual_thread, q.limit.unwrap_or(100))
    {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("get_chat_messages({}): {e}", actual_thread);
            return Ok(Json(json!([])));
        },
    };
    let v: Vec<Value> = msgs
        .iter()
        .map(|m| {
            let mut obj = json!({
                "role": if m.is_from_me { "assistant" } else { "user" },
                "sender": m.sender_name,
                "text": m.content,
                "ts": m.timestamp,
                "thread": visible_chat_thread_for_workspace(state.db.as_ref(), workspace.id, &m.chat_jid)
                    .unwrap_or_else(|| q.thread.clone()),
            });
            if let Some(ref rs) = m.raw_stream {
                obj["raw_stream"] = json!(rs);
            }
            obj
        })
        .collect();
    Ok(Json(json!(v)))
}

pub(crate) async fn get_project_chat_messages(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Query(q): Query<ProjectFilesQuery>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    let thread = project_chat_key(id);
    let msgs = state
        .db
        .get_chat_messages(&thread, q.limit.unwrap_or(200))
        .map_err(internal)?;
    let v: Vec<Value> = msgs
        .iter()
        .map(|m| {
            let mut obj = json!({
                "role": if m.is_from_me { "assistant" } else { "user" },
                "sender": m.sender_name,
                "text": m.content,
                "ts": m.timestamp,
                "thread": m.chat_jid,
            });
            if let Some(ref rs) = m.raw_stream {
                obj["raw_stream"] = json!(rs);
            }
            obj
        })
        .collect();
    Ok(Json(json!(v)))
}

pub(crate) async fn post_project_chat(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Json(body): Json<ChatPostBody>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    let thread = project_chat_key(id);
    let sender = body
        .sender
        .clone()
        .unwrap_or_else(|| "web-user".to_string());
    tracing::info!(
        target: "instrumentation.chat",
        message = "project chat submitted",
        user_id = user.id,
        username = user.username.as_str(),
        project_id = id,
        thread = thread.as_str(),
        sender = sender.as_str(),
        text_len = body.text.chars().count() as u64,
    );

    let state2 = Arc::clone(&state);
    let thread2 = thread.clone();
    let sender2 = sender.clone();
    let text2 = body.text.clone();
    tokio::spawn(async move {
        let run_id = crate::messaging_progress::new_chat_run_id();
        match run_chat_agent(
            &thread2,
            &run_id,
            &sender2,
            &[text2],
            &state2.web_sessions,
            &state2.config,
            &state2.db,
            state2.search.clone(),
            &state2.file_storage,
            &state2.chat_event_tx,
            &state2.ai_request_count,
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
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Json(body): Json<ChatPostBody>,
) -> Result<Json<Value>, StatusCode> {
    if body.text.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let requested_thread = body
        .thread
        .clone()
        .unwrap_or_else(|| "dashboard".to_string());
    if parse_project_chat_key(&requested_thread).is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let thread = scoped_workspace_chat_thread(workspace.id, &requested_thread);

    // Rate limit: one message per (60 / chat_rate_limit) seconds per thread
    let rate = state.config.chat_rate_limit.max(1) as u64;
    let cooldown = std::time::Duration::from_secs(60 / rate);
    {
        let mut map = state.chat_rate.lock().unwrap_or_else(|e| e.into_inner());
        let now = std::time::Instant::now();
        // Evict stale entries to prevent unbounded growth
        if map.len() > 1000 {
            map.retain(|_, last| now.duration_since(*last) < cooldown * 10);
        }
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
    tracing::info!(
        target: "instrumentation.chat",
        message = "chat submitted",
        user_id = user.id,
        username = user.username.as_str(),
        thread = thread.as_str(),
        sender = sender.as_str(),
        text_len = body.text.chars().count() as u64,
    );

    // Run agent async — message storage + SSE broadcast handled by run_chat_agent
    let state2 = Arc::clone(&state);
    let thread2 = thread.clone();
    let sender2 = sender.clone();
    let text2 = body.text.clone();
    tokio::spawn(async move {
        let run_id = crate::messaging_progress::new_chat_run_id();
        match run_chat_agent(
            &thread2,
            &run_id,
            &sender2,
            &[text2],
            &state2.web_sessions,
            &state2.config,
            &state2.db,
            state2.search.clone(),
            &state2.file_storage,
            &state2.chat_event_tx,
            &state2.ai_request_count,
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
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    match state
        .db
        .get_task_in_workspace(workspace.id, id)
        .map_err(internal)?
    {
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
    #[serde(rename = "owner")]
    pub _owner: Option<String>,
}

pub(crate) async fn list_api_keys(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    let keys = state
        .db
        .list_workspace_api_keys(workspace.id)
        .map_err(internal)?;
    Ok(Json(json!({ "keys": keys })))
}

pub(crate) async fn store_api_key(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Json(body): Json<StoreKeyBody>,
) -> Result<Json<Value>, StatusCode> {
    let key_name = body.key_name.as_deref().unwrap_or("");
    let id = state
        .db
        .store_workspace_api_key(workspace.id, &body.provider, key_name, &body.key_value)
        .map_err(internal)?;
    Ok(Json(json!({ "id": id })))
}

pub(crate) async fn delete_api_key(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    state
        .db
        .delete_workspace_api_key(workspace.id, id)
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

// ── Cache volumes ─────────────────────────────────────────────────────

pub(crate) async fn list_cache_volumes(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let volumes = borg_core::sandbox::Sandbox::list_cache_volumes("borg-cache-").await;
    let arr: Vec<_> = volumes
        .into_iter()
        .map(
            |(name, size, last_used)| json!({ "name": name, "size": size, "last_used": last_used }),
        )
        .collect();
    Ok(Json(json!({ "volumes": arr })))
}

pub(crate) async fn delete_cache_volume(
    State(_state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    // Only allow alphanumeric, hyphens, and underscores in volume names
    if !name.starts_with("borg-cache-")
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
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
                    file_name = sanitize_upload_name(name);
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

#[derive(Deserialize)]
pub(crate) struct TemplatesQuery {
    category: Option<String>,
    jurisdiction: Option<String>,
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

pub(crate) async fn get_knowledge_content(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, StatusCode> {
    use axum::response::IntoResponse;
    let file = state
        .db
        .get_knowledge_file_in_workspace(workspace.id, id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let path = safe_knowledge_path(&state.config.data_dir, Some(workspace.id), &file.file_name)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let bytes = std::fs::read(&path).map_err(|_| StatusCode::NOT_FOUND)?;
    let disp = format!(
        "attachment; filename=\"{}\"",
        file.file_name.replace('"', "_")
    );
    Ok((
        axum::http::StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/octet-stream".to_string(),
            ),
            (axum::http::header::CONTENT_DISPOSITION, disp),
        ],
        bytes,
    )
        .into_response())
}

// ── User knowledge ("My Knowledge") ───────────────────────────────────────

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
                    file_name = sanitize_upload_name(name);
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
        let path = format!(
            "{}/knowledge/workspaces/{}/users/{}/{}",
            state.config.data_dir, workspace.id, user.id, file.file_name
        );
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
        let path = format!(
            "{}/knowledge/workspaces/{}/users/{}/{}",
            state.config.data_dir, workspace.id, user.id, file.file_name
        );
        let _ = std::fs::remove_file(&path);
    }
    let deleted = state
        .db
        .delete_all_user_knowledge_files(workspace.id, user.id)
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true, "deleted": deleted })))
}

pub(crate) async fn get_user_knowledge_content(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, StatusCode> {
    use axum::response::IntoResponse;
    let file = state
        .db
        .get_user_knowledge_file(workspace.id, user.id, id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let path = format!(
        "{}/knowledge/workspaces/{}/users/{}/{}",
        state.config.data_dir, workspace.id, user.id, file.file_name
    );
    let bytes = std::fs::read(&path).map_err(|_| StatusCode::NOT_FOUND)?;
    let disp = format!(
        "attachment; filename=\"{}\"",
        file.file_name.replace('"', "_")
    );
    Ok((
        axum::http::StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/octet-stream".to_string(),
            ),
            (axum::http::header::CONTENT_DISPOSITION, disp),
        ],
        bytes,
    )
        .into_response())
}

// ── Knowledge Repos ───────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub(crate) struct AddKnowledgeRepoBody {
    pub url: String,
    pub name: Option<String>,
}

pub(crate) async fn list_knowledge_repos(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    let repos = state.db.list_knowledge_repos(workspace.id, None).map_err(internal)?;
    Ok(Json(json!({ "repos": repos })))
}

pub(crate) async fn add_knowledge_repo(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Json(body): Json<AddKnowledgeRepoBody>,
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
    let id = state.db.insert_knowledge_repo(workspace.id, None, &url, &name).map_err(internal)?;
    let data_dir = state.config.data_dir.clone();
    let db = Arc::clone(&state.db);
    tokio::spawn(async move {
        clone_knowledge_repo(id, &url, &data_dir, &db).await;
    });
    let repos = state.db.list_knowledge_repos(workspace.id, None).map_err(internal)?;
    Ok(Json(json!({ "repos": repos })))
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
    let repos = state.db.list_knowledge_repos(workspace.id, None).map_err(internal)?;
    Ok(Json(json!({ "repos": repos })))
}

pub(crate) async fn list_user_knowledge_repos(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    let repos = state.db.list_knowledge_repos(workspace.id, Some(user.id)).map_err(internal)?;
    Ok(Json(json!({ "repos": repos })))
}

pub(crate) async fn add_user_knowledge_repo(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Json(body): Json<AddKnowledgeRepoBody>,
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
    let id = state.db.insert_knowledge_repo(workspace.id, Some(user.id), &url, &name).map_err(internal)?;
    let data_dir = state.config.data_dir.clone();
    let db = Arc::clone(&state.db);
    tokio::spawn(async move {
        clone_knowledge_repo(id, &url, &data_dir, &db).await;
    });
    let repos = state.db.list_knowledge_repos(workspace.id, Some(user.id)).map_err(internal)?;
    Ok(Json(json!({ "repos": repos })))
}

pub(crate) async fn delete_user_knowledge_repo_handler(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    // Verify the repo belongs to this user before deleting
    let repos = state.db.list_knowledge_repos(workspace.id, Some(user.id)).map_err(internal)?;
    if !repos.iter().any(|r| r.id == id) {
        return Err(StatusCode::NOT_FOUND);
    }
    let local_path = state.db.delete_knowledge_repo(id, workspace.id).map_err(internal)?;
    if !local_path.is_empty() {
        let _ = tokio::fs::remove_dir_all(&local_path).await;
    }
    let repos = state.db.list_knowledge_repos(workspace.id, Some(user.id)).map_err(internal)?;
    Ok(Json(json!({ "repos": repos })))
}

/// Inject a PAT into a git HTTPS URL using the provider's required format.
/// GitHub: https://x-access-token:TOKEN@github.com/...
/// GitLab/Codeberg: https://oauth2:TOKEN@host/...
fn inject_git_token(url: &str, username: &str, token: &str) -> String {
    if token.is_empty() { return url.to_string(); }
    for prefix in &["https://", "http://"] {
        if let Some(rest) = url.strip_prefix(prefix) {
            return format!("{}{}:{}@{}", prefix, username, token, rest);
        }
    }
    url.to_string()
}

/// Returns (git_username, token) for the URL based on stored user settings.
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
    // Look up credentials: for user-scoped repos use the owner's stored tokens
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

// ── Cloud storage ─────────────────────────────────────────────────────────

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

/// GET /api/cloud/:provider/auth?project_id=X
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

/// GET /api/cloud/:provider/callback?code=X&state=Y
pub(crate) async fn cloud_auth_callback(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Query(q): Query<CloudCallbackQuery>,
) -> Result<axum::response::Response, StatusCode> {
    if let Some(err) = q.error {
        tracing::warn!("cloud OAuth error for {provider}: {err}");
        return Ok(axum::response::Redirect::temporary(&format!(
            "/#/projects?cloud_error=access_denied&provider={provider}"
        ))
        .into_response());
    }
    let code = q.code.ok_or(StatusCode::BAD_REQUEST)?;
    let state_raw = q.state.ok_or(StatusCode::BAD_REQUEST)?;
    let state_bytes = base64_decode(&state_raw).map_err(|_| StatusCode::BAD_REQUEST)?;
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

    // Fetch account info
    let (account_email, account_id) =
        fetch_cloud_account_info(&client, &provider, &access_token).await;

    // Check if this account is already connected to this project
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
    // Check if token expires within 5 minutes
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
    let _ = config; // unused but kept for future use
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

/// GET /api/projects/:id/cloud
pub(crate) async fn list_cloud_connections(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    let conns = state.db.list_cloud_connections(id).map_err(internal)?;
    // Don't expose tokens
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

/// DELETE /api/projects/:id/cloud/:conn_id
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

/// GET /api/projects/:id/cloud/:conn_id/browse
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

/// POST /api/projects/:id/cloud/:conn_id/import
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
        let safe_name = sanitize_upload_name(&file.name);
        let source_path = sanitize_upload_relative_path(&file.name);
        let content_hash = sha256_hex_bytes(&bytes);
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
            rand_suffix(),
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
                if let Ok(text) = extract_text_from_bytes(&fname, &mime2, &bytes2).await {
                    if !text.is_empty() {
                        let _ = db2.update_project_file_text(file_id, &text);
                        let _ = db2.fts_index_document(proj_id, 0, &source_path2, &fname, &text);
                        if let Some(search) = &search {
                            chunk_embed_and_index(
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
            Ok(Json(
                json!({ "task_id": task_id, "container_id": id, "status": status }),
            ))
        },
        None => Err(StatusCode::NOT_FOUND),
    }
}

// ── BorgSearch reindex ────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct ReindexQuery {
    project_id: Option<i64>,
}

pub(crate) async fn borgsearch_reindex(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ReindexQuery>,
) -> Result<Json<Value>, StatusCode> {
    let search = state
        .search
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .clone();
    let db = state.db.clone();

    let project_ids: Vec<i64> = if let Some(pid) = query.project_id {
        vec![pid]
    } else {
        db.list_projects()
            .map_err(internal)?
            .into_iter()
            .map(|p| p.id)
            .collect()
    };

    let total_projects = project_ids.len();
    let embed_reg = Arc::clone(&state.embed_registry);
    tokio::spawn(async move {
        let mut total_files = 0usize;
        let mut total_chunks = 0usize;
        for pid in &project_ids {
            let project_mode = db
                .get_project(*pid)
                .ok()
                .flatten()
                .map(|p| p.mode)
                .unwrap_or_default();
            let embed = embed_reg.client_for_mode(&project_mode);
            let files = match db.list_project_files(*pid) {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!("reindex: failed to list files for project {pid}: {e}");
                    continue;
                },
            };
            for file in &files {
                if file.extracted_text.is_empty() {
                    continue;
                }
                let _ = search.delete_file_chunks(*pid, file.id).await;
                let chunks_text = borg_core::knowledge::chunk_text(&file.extracted_text);
                if chunks_text.is_empty() {
                    continue;
                }
                let metadata = crate::vespa::ChunkMetadata {
                    doc_type: crate::ingestion::detect_doc_type(
                        &file.file_name,
                        &file.mime_type,
                        &file.extracted_text,
                    ),
                    jurisdiction: String::new(),
                    privileged: file.privileged,
                    mime_type: file.mime_type.clone(),
                };
                let mut chunks_with_embeddings: Vec<(String, Vec<f32>)> = Vec::new();
                for chunk in &chunks_text {
                    match embed.embed_document(chunk).await {
                        Ok(emb) => chunks_with_embeddings.push((chunk.clone(), emb)),
                        Err(_) => {
                            chunks_with_embeddings.push((chunk.clone(), embed.zero_embedding()))
                        },
                    }
                }
                total_chunks += chunks_with_embeddings.len();
                if let Err(e) = search
                    .index_chunks(
                        *pid,
                        file.id,
                        &file.file_name,
                        &file.file_name,
                        &chunks_with_embeddings,
                        &metadata,
                    )
                    .await
                {
                    tracing::warn!("reindex: chunk indexing failed for file {}: {e}", file.id);
                }
                total_files += 1;
            }
        }
        tracing::info!(
            "reindex complete: {total_projects} projects, {total_files} files, {total_chunks} chunks"
        );
    });

    Ok(Json(json!({
        "status": "started",
        "projects": total_projects,
    })))
}

// ── Agent API (LLM-friendly endpoints for on-demand search) ──────────

#[derive(Deserialize)]
pub(crate) struct FacetsQuery {
    project_id: i64,
}

pub(crate) async fn borgsearch_facets(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FacetsQuery>,
) -> Result<Json<Value>, StatusCode> {
    let search = state
        .search
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let doc_types = search
        .facet_counts(query.project_id, "doc_type")
        .await
        .unwrap_or_default();
    let jurisdictions = search
        .facet_counts(query.project_id, "jurisdiction")
        .await
        .unwrap_or_default();
    Ok(Json(json!({
        "doc_types": doc_types.into_iter().map(|(v, c)| json!({"value": v, "count": c})).collect::<Vec<_>>(),
        "jurisdictions": jurisdictions.into_iter().map(|(v, c)| json!({"value": v, "count": c})).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub(crate) struct AgentSearchQuery {
    q: String,
    #[serde(default)]
    project_id: Option<i64>,
    #[serde(default = "default_agent_search_limit")]
    limit: i64,
    #[serde(default)]
    doc_type: Option<String>,
    #[serde(default)]
    jurisdiction: Option<String>,
    #[serde(default)]
    privileged_only: bool,
    #[serde(default)]
    model: Option<String>,
    /// Comma-separated terms to exclude from results (NOT filter)
    #[serde(default)]
    exclude: Option<String>,
}
fn default_agent_search_limit() -> i64 {
    20
}

pub(crate) async fn agent_search(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Query(query): Query<AgentSearchQuery>,
) -> Result<String, StatusCode> {
    if query.q.trim().is_empty() {
        return Ok("No query provided.".to_string());
    }

    let limit = query.limit.clamp(1, 100);
    let exclude_terms: Vec<String> = query
        .exclude
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let filters = crate::vespa::ChunkFilters {
        doc_type: query.doc_type.clone(),
        jurisdiction: query.jurisdiction.clone(),
        privileged_only: query.privileged_only,
        exclude_terms,
    };

    // Try chunk-level hybrid search first
    if let Some(search) = &state.search {
        let embed_client = match query.model.as_deref() {
            Some(m) => state.embed_registry.client(m),
            None => state.embed_registry.default_client(),
        };
        let query_emb = embed_client.embed_query(&query.q).await.ok();
        let emb_ref = query_emb.as_deref();

        match search
            .search_chunks(&query.q, emb_ref, query.project_id, &filters, limit)
            .await
        {
            Ok(hits) if !hits.is_empty() => {
                // Deduplicate: keep best chunk per file
                let mut seen_files: HashSet<i64> = HashSet::new();
                let hits: Vec<_> = hits
                    .into_iter()
                    .filter(|h| seen_files.insert(h.file_id))
                    .collect();

                let mut out = format!("Search results for: {}\n", query.q);
                if let Some(dt) = &query.doc_type {
                    out.push_str(&format!("Filter: doc_type={}\n", dt));
                }
                if let Some(j) = &query.jurisdiction {
                    out.push_str(&format!("Filter: jurisdiction={}\n", j));
                }
                out.push('\n');
                for (i, hit) in hits.iter().enumerate() {
                    out.push_str(&format!(
                        "--- Result {} (score: {:.3}, type: {}) ---\nFile: {} [id={}, chunk={}]\n{}\n\n",
                        i + 1,
                        hit.score,
                        if hit.doc_type.is_empty() { "unknown" } else { &hit.doc_type },
                        hit.file_path,
                        hit.file_id,
                        hit.chunk_index,
                        hit.content,
                    ));
                }
                tracing::info!(
                    target: "instrumentation.search",
                    message = "agent search completed",
                    user_id = user.id,
                    username = user.username.as_str(),
                    project_id = query.project_id,
                    limit = limit,
                    query_len = query.q.chars().count() as u64,
                    result_count = hits.len() as u64,
                    doc_type = query.doc_type.as_deref().unwrap_or(""),
                    jurisdiction = query.jurisdiction.as_deref().unwrap_or(""),
                    privileged_only = query.privileged_only,
                    source = "chunk_hybrid",
                );
                return Ok(out);
            },
            Ok(_) => {},
            Err(e) => {
                tracing::warn!("chunk search failed, falling back: {e}");
            },
        }
    }

    // Fallback: old whole-document search + semantic
    let mut results: Vec<(String, String, f64, String)> = Vec::new();

    if let Some(search) = &state.search {
        if let Ok(hits) = search.search(&query.q, query.project_id, limit).await {
            for r in hits {
                let snippet = if !r.content_snippet.is_empty() {
                    r.content_snippet.clone()
                } else {
                    r.title_snippet.clone()
                };
                results.push((
                    r.file_path,
                    snippet,
                    r.score,
                    search.backend_name().to_string(),
                ));
            }
        }
    }

    if state.db.embedding_count() > 0 {
        let fallback_ec = match query.model.as_deref() {
            Some(m) => state.embed_registry.client(m),
            None => state.embed_registry.default_client(),
        };
        if let Ok(query_emb) = fallback_ec.embed_query(&query.q).await {
            if let Ok(sem) =
                state
                    .db
                    .search_embeddings(&query_emb, limit as usize, query.project_id)
            {
                for r in sem.iter().filter(|r| r.score > 0.5) {
                    let already = results.iter().any(|(p, _, _, _)| *p == r.file_path);
                    if !already {
                        let snippet = if r.chunk_text.len() > 300 {
                            format!(
                                "{}...",
                                &r.chunk_text[..r.chunk_text.floor_char_boundary(300)]
                            )
                        } else {
                            r.chunk_text.clone()
                        };
                        results.push((
                            r.file_path.clone(),
                            snippet,
                            r.score.into(),
                            "semantic".to_string(),
                        ));
                    }
                }
            }
        }
    }

    if results.is_empty() {
        tracing::info!(
            target: "instrumentation.search",
            message = "agent search completed",
            user_id = user.id,
            username = user.username.as_str(),
            project_id = query.project_id,
            limit = limit,
            query_len = query.q.chars().count() as u64,
            result_count = 0u64,
            doc_type = query.doc_type.as_deref().unwrap_or(""),
            jurisdiction = query.jurisdiction.as_deref().unwrap_or(""),
            privileged_only = query.privileged_only,
            source = "fallback",
        );
        return Ok(format!("No results found for: {}", query.q));
    }

    results.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit as usize);

    let mut out = format!("Search results for: {}\n\n", query.q);
    for (i, (path, snippet, score, source)) in results.iter().enumerate() {
        out.push_str(&format!(
            "--- Result {} (score: {:.3}, source: {}) ---\nFile: {}\n{}\n\n",
            i + 1,
            score,
            source,
            path,
            snippet
        ));
    }
    tracing::info!(
        target: "instrumentation.search",
        message = "agent search completed",
        user_id = user.id,
        username = user.username.as_str(),
        project_id = query.project_id,
        limit = limit,
        query_len = query.q.chars().count() as u64,
        result_count = results.len() as u64,
        doc_type = query.doc_type.as_deref().unwrap_or(""),
        jurisdiction = query.jurisdiction.as_deref().unwrap_or(""),
        privileged_only = query.privileged_only,
        source = "fallback",
    );
    Ok(out)
}

#[derive(Deserialize)]
pub(crate) struct AgentFileQuery {
    project_id: i64,
}

pub(crate) async fn agent_get_file(
    State(state): State<Arc<AppState>>,
    Path(file_id): Path<i64>,
    Query(query): Query<AgentFileQuery>,
) -> Result<String, StatusCode> {
    let file = state
        .db
        .get_project_file(query.project_id, file_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let text = if !file.extracted_text.trim().is_empty() {
        file.extracted_text.clone()
    } else if !is_binary_mime(&file.mime_type) {
        match state.file_storage.read_all(&file.stored_path).await {
            Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
            Err(_) => return Err(StatusCode::NOT_FOUND),
        }
    } else {
        return Ok(format!(
            "File: {}\nType: {}\nSize: {} bytes\n\n(Binary file — no text content available)",
            file.file_name, file.mime_type, file.size_bytes
        ));
    };

    let mut out = format!(
        "File: {}\nPath: {}\nType: {}\nSize: {} bytes\n\n",
        file.file_name, file.source_path, file.mime_type, file.size_bytes
    );
    out.push_str(&text);
    Ok(out)
}

#[derive(Deserialize)]
pub(crate) struct AgentFilesQuery {
    project_id: i64,
    #[serde(default)]
    q: Option<String>,
    #[serde(default = "default_agent_files_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}
fn default_agent_files_limit() -> i64 {
    50
}

pub(crate) async fn agent_list_files(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AgentFilesQuery>,
) -> Result<String, StatusCode> {
    let (files, total) = state
        .db
        .list_project_file_page(
            query.project_id,
            query.q.as_deref(),
            query.limit.clamp(1, 200),
            query.offset.max(0),
            None,
            Some(true), // only files with extracted text
            None,
        )
        .map_err(internal)?;

    if files.is_empty() {
        return Ok(format!(
            "No files found for project {} (total: {}).",
            query.project_id, total
        ));
    }

    let mut out = format!(
        "Project files (showing {}-{} of {}):\n\n",
        query.offset + 1,
        query.offset + files.len() as i64,
        total
    );
    for f in &files {
        out.push_str(&format!(
            "  [id={}] {} ({}, {} bytes)\n",
            f.id, f.source_path, f.mime_type, f.size_bytes
        ));
    }
    out.push_str(&format!(
        "\nUse /api/borgsearch/file/<id>?project_id={} to read a file's content.",
        query.project_id
    ));
    Ok(out)
}

#[derive(Deserialize)]
pub(crate) struct CoverageQuery {
    q: String,
    project_id: i64,
    #[serde(default = "default_coverage_limit")]
    limit: i64,
    #[serde(default)]
    doc_type: Option<String>,
    #[serde(default)]
    model: Option<String>,
}
fn default_coverage_limit() -> i64 {
    100
}

pub(crate) async fn agent_coverage(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CoverageQuery>,
) -> Result<String, StatusCode> {
    // Get all project files
    let (all_files, total) = state
        .db
        .list_project_file_page(query.project_id, None, 10000, 0, None, Some(true), None)
        .map_err(internal)?;

    if all_files.is_empty() {
        return Ok(format!("No files found for project {}.", query.project_id));
    }

    // Search for matching documents
    let limit = query.limit.clamp(1, 500);
    let filters = crate::vespa::ChunkFilters {
        doc_type: query.doc_type.clone(),
        ..Default::default()
    };

    let mut matched_file_ids: HashSet<i64> = HashSet::new();

    if let Some(search) = &state.search {
        let embed_client = match query.model.as_deref() {
            Some(m) => state.embed_registry.client(m),
            None => state.embed_registry.default_client(),
        };
        let query_emb = embed_client.embed_query(&query.q).await.ok();
        let emb_ref = query_emb.as_deref();

        if let Ok(hits) = search
            .search_chunks(&query.q, emb_ref, Some(query.project_id), &filters, limit)
            .await
        {
            for h in &hits {
                matched_file_ids.insert(h.file_id);
            }
        }
    }

    // Also check embedding DB fallback
    if state.db.embedding_count() > 0 {
        let ec = match query.model.as_deref() {
            Some(m) => state.embed_registry.client(m),
            None => state.embed_registry.default_client(),
        };
        if let Ok(emb) = ec.embed_query(&query.q).await {
            if let Ok(sem) = state
                .db
                .search_embeddings(&emb, 500, Some(query.project_id))
            {
                for r in sem.iter().filter(|r| r.score > 0.4) {
                    if let Some(f) = all_files.iter().find(|f| f.source_path == r.file_path) {
                        matched_file_ids.insert(f.id);
                    }
                }
            }
        }
    }

    let mut matched = Vec::new();
    let mut unmatched = Vec::new();
    for f in &all_files {
        if matched_file_ids.contains(&f.id) {
            matched.push(f);
        } else {
            unmatched.push(f);
        }
    }

    let pct = if total > 0 {
        (matched.len() as f64 / total as f64 * 100.0).round() as i64
    } else {
        0
    };

    let mut out =
        format!(
        "## Coverage Report: \"{}\"\n\nTotal documents: {}\nMatched: {} ({}%)\nNot matched: {}\n\n",
        query.q, total, matched.len(), pct, unmatched.len()
    );

    if !unmatched.is_empty() {
        out.push_str("### Documents NOT matching query:\n\n");
        for f in &unmatched {
            out.push_str(&format!(
                "  [id={}] {} ({}, {} bytes)\n",
                f.id, f.source_path, f.mime_type, f.size_bytes
            ));
        }
        out.push('\n');
    }

    if !matched.is_empty() {
        out.push_str("### Documents matching query:\n\n");
        for f in &matched {
            out.push_str(&format!(
                "  [id={}] {} ({}, {} bytes)\n",
                f.id, f.source_path, f.mime_type, f.size_bytes
            ));
        }
    }

    Ok(out)
}

// Admin / debugging endpoints

#[derive(Deserialize)]
pub(crate) struct ConversationDumpQuery {
    thread: String,
    #[serde(default = "default_conv_limit")]
    limit: i64,
}
fn default_conv_limit() -> i64 {
    200
}

pub(crate) async fn admin_conversation_dump(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ConversationDumpQuery>,
) -> Result<Json<Value>, StatusCode> {
    let msgs = state
        .db
        .get_chat_messages(&query.thread, query.limit)
        .map_err(internal)?;

    let result: Vec<Value> = msgs
        .iter()
        .map(|m| {
            let mut obj = json!({
                "role": if m.is_from_me { "assistant" } else { "user" },
                "sender": m.sender_name,
                "content": m.content,
                "ts": m.timestamp,
            });

            if let Some(ref rs) = m.raw_stream {
                // Parse raw NDJSON into structured events
                let mut events: Vec<Value> = Vec::new();
                for line in rs.split('\n') {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(parsed) = serde_json::from_str::<Value>(line) {
                        let event_type = parsed.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match event_type {
                            "assistant" => {
                                if let Some(msg) = parsed.get("message") {
                                    if let Some(content) = msg.get("content") {
                                        if let Some(blocks) = content.as_array() {
                                            for block in blocks {
                                                let btype = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                                match btype {
                                                    "text" => {
                                                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                                            events.push(json!({"type": "text", "content": text}));
                                                        }
                                                    }
                                                    "tool_use" => {
                                                        events.push(json!({
                                                            "type": "tool_call",
                                                            "tool": block.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                                                            "input": block.get("input"),
                                                        }));
                                                    }
                                                    "thinking" => {
                                                        if let Some(text) = block.get("thinking").and_then(|t| t.as_str()) {
                                                            let preview = if text.len() > 200 { &text[..200] } else { text };
                                                            events.push(json!({"type": "thinking", "content": preview}));
                                                        }
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            "tool_result" | "tool" => {
                                let content = parsed.get("content")
                                    .or_else(|| parsed.get("output"))
                                    .or_else(|| parsed.get("result"));
                                let text = match content {
                                    Some(Value::String(s)) => {
                                        if s.len() > 500 { format!("{}...", &s[..500]) } else { s.clone() }
                                    }
                                    Some(Value::Array(arr)) => {
                                        arr.iter()
                                            .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                                            .collect::<Vec<_>>()
                                            .join("\n")
                                    }
                                    Some(v) => {
                                        let s = v.to_string();
                                        if s.len() > 500 { format!("{}...", &s[..500]) } else { s }
                                    }
                                    None => String::new(),
                                };
                                events.push(json!({
                                    "type": "tool_result",
                                    "tool": parsed.get("tool_name").or_else(|| parsed.get("name"))
                                        .and_then(|n| n.as_str()).unwrap_or(""),
                                    "output": text,
                                }));
                            }
                            "result" => {
                                if let Some(r) = parsed.get("result").and_then(|r| r.as_str()) {
                                    events.push(json!({"type": "result", "content": r}));
                                }
                            }
                            "system" => {
                                if let Some(sub) = parsed.get("subtype").and_then(|s| s.as_str()) {
                                    if sub == "init" {
                                        let model = parsed.get("model").and_then(|m| m.as_str()).unwrap_or("?");
                                        let mcp = parsed.get("mcp_servers");
                                        events.push(json!({"type": "system_init", "model": model, "mcp_servers": mcp}));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                obj["events"] = json!(events);
                obj["raw_stream_lines"] = json!(rs.split('\n').filter(|l| !l.trim().is_empty()).count());
            }

            obj
        })
        .collect();

    Ok(Json(json!({
        "thread": query.thread,
        "message_count": result.len(),
        "messages": result,
    })))
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
        // & must be encoded so it doesn't inject a second query parameter
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
