use std::{
    collections::HashSet,
    path::{Component, Path as FsPath},
    sync::Arc,
};

use axum::{
    body::Bytes,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::Json,
};
use borg_core::db::{
    Db, ProjectFileMetaRow, ProjectFilePageCursor, ProjectFileRow, ProjectRow, ProjectTaskCounts,
    TaskMessage, TaskOutput,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

use super::{internal, require_project_access, require_task_access};
use crate::{
    ingestion::{detect_doc_type, extract_text_from_bytes, IngestionQueue},
    storage::FileStorage,
    vespa::ChunkMetadata,
    AppState,
};

// ── Types ─────────────────────────────────────────────────────────────────

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
pub(crate) struct CreateProjectBody {
    pub name: String,
    pub mode: Option<String>,
    pub client_name: Option<String>,
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

pub(crate) fn default_project_file_limit() -> i64 {
    50
}

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

#[derive(Deserialize)]
pub(crate) struct AuditQuery {
    #[serde(default = "default_audit_limit")]
    limit: i64,
}
fn default_audit_limit() -> i64 {
    100
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

#[derive(Deserialize)]
pub(crate) struct ExportAllQuery {
    pub format: Option<String>,
    pub toc: Option<bool>,
    pub template_id: Option<i64>,
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

// ── Helper types ──────────────────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct ProjectContextHit {
    pub source_path: String,
    pub snippet: String,
    pub score: f64,
    pub source: &'static str,
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

// ── Upload constants ──────────────────────────────────────────────────────

const MIN_UPLOAD_CHUNK_SIZE: i64 = 256 * 1024;
const MAX_UPLOAD_CHUNK_SIZE: i64 = 64 * 1024 * 1024;
const MAX_ACTIVE_UPLOAD_SESSIONS_PER_PROJECT: i64 = 24;

// ── Utility functions ─────────────────────────────────────────────────────

pub(crate) fn sanitize_upload_name(name: &str) -> String {
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

pub(crate) fn sanitize_upload_relative_path(name: &str) -> String {
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

pub(crate) fn is_binary_mime(mime: &str) -> bool {
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

fn project_context_search_query(raw_query: &str) -> String {
    const MAX_QUERY_CHARS: usize = 1_200;

    let mut in_code_fence = false;
    let mut cleaned_lines = Vec::new();

    for line in raw_query.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_fence = !in_code_fence;
            continue;
        }
        if in_code_fence || trimmed.is_empty() || trimmed.starts_with("===") {
            continue;
        }
        cleaned_lines.push(trimmed);
    }

    let normalized = cleaned_lines.join(" ");
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let (truncated, _) = truncate_chars(&normalized, MAX_QUERY_CHARS);
    truncated
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

pub(crate) fn is_privileged_upload_allowed(state: &AppState, project_id: i64) -> bool {
    state.db.is_session_privileged(project_id).unwrap_or(false)
}

pub(crate) fn upload_chunks_dir(data_dir: &str, session_id: i64) -> String {
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

pub(crate) fn guess_mime_from_name(file_name: &str) -> String {
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

fn preprocess_legal_markdown(md: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in md.lines() {
        if line.trim().starts_with("Confidence:") || line.trim().starts_with("**Confidence:") {
            continue;
        }
        if line.contains("structured.json") || line.contains("signal.json") {
            continue;
        }
        if line.trim().starts_with("<!-- borg:") || line.trim().starts_with("<!-- internal") {
            continue;
        }
        lines.push(line.to_string());
    }
    lines.join("\n")
}

/// Read a file from git: tries local `git show ref:path` first, falls back to `gh api`.
async fn git_show_file(repo_path: &str, slug: &str, ref_name: &str, path: &str) -> Option<Vec<u8>> {
    if !repo_path.is_empty() && FsPath::new(repo_path).join(".git").exists() {
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
    if !slug.is_empty() {
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio::process::Command::new("gh")
                .args([
                    "api",
                    &format!(
                        "repos/{slug}/contents/{}?ref={}",
                        super::cloud::percent_encode_allow_slash(path, true),
                        super::cloud::percent_encode(ref_name)
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
                return super::utils::base64_decode(&b64).ok();
            }
        }
    }
    None
}

fn can_read_worktree_ref(ref_name: Option<&str>, task_branch: &str) -> bool {
    match ref_name {
        None => true,
        Some(ref_name) => ref_name == task_branch || ref_name == "worktree",
    }
}

fn is_safe_repo_relative_path(path: &str) -> bool {
    let candidate = FsPath::new(path);
    !candidate.is_absolute()
        && candidate
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

async fn read_worktree_file(repo_path: &str, path: &str) -> Option<Vec<u8>> {
    if repo_path.is_empty() || !is_safe_repo_relative_path(path) {
        return None;
    }
    let file_path = FsPath::new(repo_path).join(path);
    if !file_path.is_file() {
        return None;
    }
    tokio::fs::read(file_path).await.ok()
}

async fn list_worktree_files(repo_path: &str) -> Option<Vec<String>> {
    if repo_path.is_empty() || !FsPath::new(repo_path).join(".git").exists() {
        return None;
    }

    let out = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new("git")
            .args([
                "-C",
                repo_path,
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
            ])
            .stderr(std::process::Stdio::null())
            .output(),
    )
    .await;

    let output = match out {
        Ok(Ok(output)) if output.status.success() => output,
        _ => return None,
    };

    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let path = String::from_utf8_lossy(raw).to_string();
        if !is_safe_repo_relative_path(&path) {
            continue;
        }
        let absolute = FsPath::new(repo_path).join(&path);
        if absolute.is_file() && seen.insert(path.clone()) {
            files.push(path);
        }
    }
    files.sort();
    Some(files)
}

// ── Context building ──────────────────────────────────────────────────────

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

// ── Workspace colocation ──────────────────────────────────────────────────

#[allow(dead_code)]
pub(crate) struct ColocationResult {
    pub documents_dir: String,
    pub total_files: usize,
    pub linked: usize,
    pub written: usize,
    pub skipped: usize,
    pub repo_names: Vec<String>,
    pub has_project_repo: bool,
}

const MAX_COLOCATE_FILES: usize = 5000;
const MAX_COLOCATE_FILE_SIZE: i64 = 100 * 1024 * 1024;

fn safe_dest_path(base: &str, rel: &str) -> Option<String> {
    let p = FsPath::new(rel);
    for component in p.components() {
        if matches!(component, Component::ParentDir) {
            return None;
        }
    }
    Some(format!("{base}/{rel}"))
}

pub(crate) async fn colocate_project_workspace(
    project: &ProjectRow,
    session_dir: &str,
    db: &Db,
    storage: &FileStorage,
    knowledge_repos: &[borg_core::db::KnowledgeRepo],
) -> ColocationResult {
    let documents_dir = format!("{session_dir}/documents");
    let repos_dir = format!("{session_dir}/repos");
    let repo_link = format!("{session_dir}/repo");

    let all_files = db.list_project_files(project.id).unwrap_or_default();
    let total_files = all_files.len();
    let mut linked = 0usize;
    let mut written = 0usize;
    let mut skipped = 0usize;

    if total_files > MAX_COLOCATE_FILES {
        tracing::warn!(
            project_id = project.id,
            total_files,
            "skipping colocation: too many files"
        );
        return ColocationResult {
            documents_dir,
            total_files,
            linked: 0,
            written: 0,
            skipped: total_files,
            repo_names: Vec::new(),
            has_project_repo: false,
        };
    }

    // Clean sweep for idempotency
    let _ = std::fs::remove_dir_all(&documents_dir);
    let _ = std::fs::create_dir_all(&documents_dir);

    let is_local = storage.is_local();
    let mut seen_paths = HashSet::new();

    for file in &all_files {
        if file.size_bytes > MAX_COLOCATE_FILE_SIZE {
            skipped += 1;
            continue;
        }

        let rel = if file.source_path.is_empty() {
            file.file_name.clone()
        } else {
            file.source_path.clone()
        };

        if !seen_paths.insert(rel.clone()) {
            skipped += 1;
            continue;
        }

        let Some(dest) = safe_dest_path(&documents_dir, &rel) else {
            skipped += 1;
            continue;
        };

        // Create parent directories
        if let Some(parent) = FsPath::new(&dest).parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        if is_local && FsPath::new(&file.stored_path).exists() {
            // Symlink the original file
            if std::os::unix::fs::symlink(&file.stored_path, &dest).is_ok() {
                linked += 1;
            } else {
                skipped += 1;
                continue;
            }
            // For binary files with extracted text, write a companion .txt
            if is_binary_mime(&file.mime_type) && !file.extracted_text.trim().is_empty() {
                let txt_dest = format!("{dest}.txt");
                let _ = std::fs::write(&txt_dest, &file.extracted_text);
            }
        } else if !file.extracted_text.trim().is_empty() {
            // S3 or missing local file: write extracted text
            let txt_dest = if is_binary_mime(&file.mime_type) {
                format!("{dest}.txt")
            } else {
                dest.clone()
            };
            if std::fs::write(&txt_dest, &file.extracted_text).is_ok() {
                written += 1;
            } else {
                skipped += 1;
            }
        } else {
            skipped += 1;
        }
    }

    // Symlink knowledge repos
    let _ = std::fs::remove_dir_all(&repos_dir);
    let mut repo_names = Vec::new();
    let ready_repos: Vec<_> = knowledge_repos
        .iter()
        .filter(|r| r.status == "ready" && !r.local_path.is_empty())
        .collect();
    if !ready_repos.is_empty() {
        let _ = std::fs::create_dir_all(&repos_dir);
        for repo in &ready_repos {
            let name = if repo.name.is_empty() {
                format!("repo-{}", repo.id)
            } else {
                repo.name.replace('/', "-")
            };
            let link = format!("{repos_dir}/{name}");
            if FsPath::new(&repo.local_path).exists() {
                let _ = std::os::unix::fs::symlink(&repo.local_path, &link);
                repo_names.push(name);
            }
        }
    }

    // Symlink project repo
    let _ = std::fs::remove_file(&repo_link);
    let has_project_repo =
        !project.repo_path.is_empty() && FsPath::new(&project.repo_path).exists();
    if has_project_repo {
        let _ = std::os::unix::fs::symlink(&project.repo_path, &repo_link);
    }

    tracing::info!(
        project_id = project.id,
        total_files,
        linked,
        written,
        skipped,
        repos = repo_names.len(),
        "colocated project workspace"
    );

    ColocationResult {
        documents_dir,
        total_files,
        linked,
        written,
        skipped,
        repo_names,
        has_project_repo,
    }
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

pub(crate) async fn build_project_context(
    project: &ProjectRow,
    retrieval_query: &str,
    session_dir: &str,
    db: &Db,
    search: Option<&crate::search::SearchClient>,
    storage: &FileStorage,
    colocation: Option<&ColocationResult>,
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
    let raw_query = project_context_search_query(retrieval_query);
    let raw_query = raw_query.trim();
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

    let coloc_line = if let Some(c) = colocation {
        let colocated = c.linked + c.written;
        if colocated > 0 {
            format!(
                "Workspace: {} files colocated in documents/, {} search-matched files pre-loaded below\n",
                colocated,
                staged_files.len(),
            )
        } else {
            format!("Staged working set: {} file(s)\n", staged_files.len())
        }
    } else {
        format!(
            "Staged working set: {} file(s) in {}/\n",
            staged_files.len(),
            files_dir
        )
    };
    let mut context = format!(
        "Project context:\nProject: {} (mode: {})\nCorpus: {} files, {} extracted-text files, {} privileged files, {} total\nSession privileged: {}\n{coloc_line}",
        project.name,
        project.mode,
        stats.total_files,
        stats.text_files,
        stats.privileged_files,
        format_compact_bytes(stats.total_bytes),
        if project.session_privileged { "yes" } else { "no" },
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
    if colocation
        .map(|c| c.linked + c.written > 0)
        .unwrap_or(false)
    {
        context.push_str("All project files are available in the documents/ directory. The staged working set below contains the most relevant files for your query — browse documents/ directly for the full corpus.\n");
    } else {
        context.push_str("Selection policy: only the staged working set was materialized for this request. Do not assume unstaged corpus documents were reviewed.\n");
    }
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
    // Always include full file inventory so agent knows what exists
    if stats.total_files > 0 && remaining > 256 {
        let all_files = db
            .list_recent_project_files(project.id, 50, false)
            .unwrap_or_default();
        if !all_files.is_empty() {
            let heading = if colocation
                .map(|c| c.linked + c.written > 0)
                .unwrap_or(false)
            {
                "\nProject file inventory (browse documents/ directory or use search_documents MCP tool for semantic search):\n"
            } else {
                "\nProject file inventory (use `read_document` or `list_documents` MCP tools to access any file):\n"
            };
            if heading.len() < remaining {
                context.push_str(heading);
                remaining -= heading.len();
            }
            for f in &all_files {
                if remaining < 128 {
                    break;
                }
                let has_text = !f.extracted_text.trim().is_empty();
                let entry = format!(
                    "  - {} [{}; {}{}]\n",
                    f.file_name,
                    f.mime_type,
                    format_compact_bytes(f.size_bytes),
                    if has_text { "; text extracted" } else { "" },
                );
                if entry.len() >= remaining {
                    break;
                }
                context.push_str(&entry);
                remaining -= entry.len();
            }
        }
    }
    if staged_files.is_empty() && stats.total_files > 0 && remaining > 256 {
        let note = "No files were auto-staged for this request (e.g. no extracted text for binary files). Use MCP `read_document` tool to access any file listed above.\n";
        if note.len() < remaining {
            context.push_str(note);
            remaining -= note.len();
        }
    }

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

// ── Indexing helpers ──────────────────────────────────────────────────────

pub(crate) async fn chunk_embed_and_index(
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

pub(crate) async fn process_files_concurrently(state: &Arc<AppState>, files: Vec<ProjectFileRow>) {
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

    let embeddings = crate::ingestion::batch_embed_chunks(
        &all_chunk_texts,
        Some(embed_client),
        embed_client.dim(),
    )
    .await;

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

async fn process_uploaded_single_file(
    state: Arc<AppState>,
    project_id: i64,
    _session_id: i64,
    file_name: &str,
    mime_type: &str,
    assembled_path: &str,
    privileged: bool,
) -> anyhow::Result<String> {
    let content_hash = super::utils::sha256_hex_file(assembled_path).await?;
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
        super::utils::rand_suffix(),
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
                let content_hash = super::utils::sha256_hex_file_blocking(&tmp_path)?;
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
                    super::utils::rand_suffix(),
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

// ── Project handlers ──────────────────────────────────────────────────────

pub(crate) async fn list_projects(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    let mut projects = state
        .db
        .list_projects_in_workspace(workspace.id)
        .map_err(internal)?;

    // Include projects shared directly with this user from other workspaces
    let mut shared_roles: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    if user.id > 0 {
        let shared = state
            .db
            .list_user_shared_projects(user.id)
            .map_err(internal)?;
        let existing_ids: HashSet<i64> = projects.iter().map(|p| p.id).collect();
        for (p, role) in shared {
            if !existing_ids.contains(&p.id) {
                shared_roles.insert(p.id, role);
                projects.push(p);
            }
        }
    }

    let out: Vec<Value> = projects
        .into_iter()
        .map(|p| {
            let pid = p.id;
            let counts = state.db.project_task_status_counts(pid).ok();
            let mut j = serde_json::to_value(ProjectJson::from_row(p, counts)).unwrap();
            if let Some(role) = shared_roles.get(&pid) {
                j["shared_role"] = json!(role);
            }
            j
        })
        .collect();
    Ok(Json(json!(out)))
}

pub(crate) async fn search_projects(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<Value>, StatusCode> {
    let q = params.q.unwrap_or_default();
    if q.is_empty() {
        return list_projects(
            State(state),
            axum::Extension(user),
            axum::Extension(workspace),
        )
        .await;
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
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let (project, _role) =
        super::require_project_access_with_shares(state.as_ref(), &user, &workspace, id)?;
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
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let (_project, _role) =
        super::require_project_access_with_shares(state.as_ref(), &user, &workspace, id)?;
    let tasks = state.db.list_project_tasks(id).map_err(internal)?;
    Ok(Json(json!(tasks)))
}

pub(crate) async fn list_project_audit(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Value>, StatusCode> {
    let (_project, _role) =
        super::require_project_access_with_shares(state.as_ref(), &user, &workspace, id)?;
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

pub(crate) async fn list_project_documents(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let (project, _role) =
        super::require_project_access_with_shares(state.as_ref(), &user, &workspace, id)?;
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

        let file_list = match list_worktree_files(repo_path).await {
            Some(files) => files,
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
                    Ok(Ok(output)) if output.status.success() => output
                        .stdout
                        .split(|byte| *byte == b'\n')
                        .filter_map(|line| {
                            let entry = String::from_utf8_lossy(line).trim().to_string();
                            if entry.is_empty() {
                                None
                            } else {
                                Some(entry)
                            }
                        })
                        .collect(),
                    _ => continue,
                }
            },
            _ => continue,
        };

        for name in file_list {
            if !name.is_empty() && !name.starts_with('.') {
                documents.push(json!({
                    "task_id": task.id,
                    "branch": task.branch,
                    "file_name": name,
                    "repo_slug": slug,
                    "task_title": task.title,
                    "task_status": task.status,
                    "source": "pipeline",
                }));
            }
        }
    }

    // Chat artifacts: scan session directories for files created by chat agents
    for dir in chat_session_dirs(&state.config.data_dir, project.workspace_id, id) {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !meta.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !is_artifact_file(&name) {
                continue;
            }
            let created_at = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| {
                    chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                        .unwrap_or_default()
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string()
                })
                .unwrap_or_default();
            documents.push(json!({
                "task_id": 0,
                "file_name": name,
                "task_title": "Chat",
                "task_status": "completed",
                "source": "chat",
                "created_at": created_at,
            }));
        }
    }

    Ok(Json(json!(documents)))
}

fn chat_session_dirs(data_dir: &str, workspace_id: i64, project_id: i64) -> Vec<String> {
    vec![
        format!("{data_dir}/sessions/chat-web_workspace_{workspace_id}_web_project-{project_id}"),
        format!("{data_dir}/sessions/chat-project_{project_id}"),
    ]
}

const ARTIFACT_EXTENSIONS: &[&str] = &[
    "docx", "pdf", "md", "txt", "xlsx", "pptx", "csv", "html", "rtf", "json", "png", "jpg", "jpeg",
    "svg", "gif",
];

fn is_artifact_file(name: &str) -> bool {
    if name.starts_with('.') || name == "CLAUDE.md" || name == "package.json" || name == "bun.lock"
    {
        return false;
    }
    let ext = match name.rsplit_once('.') {
        Some((_, e)) => e.to_lowercase(),
        None => return false,
    };
    ARTIFACT_EXTENSIONS.contains(&ext.as_str())
}

pub(crate) async fn get_chat_artifact(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Query(q): Query<DocQuery>,
) -> Result<axum::response::Response, StatusCode> {
    let project = require_project_access(state.as_ref(), &workspace, id)?;
    let path = q.path.as_deref().ok_or(StatusCode::BAD_REQUEST)?;

    // Only allow plain filenames — no directory traversal
    if path.contains('/') || path.contains('\\') || path.contains("..") || path.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    for dir in chat_session_dirs(&state.config.data_dir, project.workspace_id, id) {
        let full = format!("{dir}/{path}");
        if let Ok(bytes) = tokio::fs::read(&full).await {
            let content_type = match path.rsplit_once('.').map(|(_, e)| e.to_lowercase()) {
                Some(ref e) if e == "docx" => {
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                },
                Some(ref e) if e == "xlsx" => {
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                },
                Some(ref e) if e == "pptx" => {
                    "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                },
                Some(ref e) if e == "pdf" => "application/pdf",
                Some(ref e) if e == "md" || e == "txt" || e == "csv" => "text/plain; charset=utf-8",
                Some(ref e) if e == "html" => "text/html; charset=utf-8",
                Some(ref e) if e == "json" => "application/json",
                Some(ref e) if e == "png" => "image/png",
                Some(ref e) if e == "jpg" || e == "jpeg" => "image/jpeg",
                Some(ref e) if e == "svg" => "image/svg+xml",
                Some(ref e) if e == "gif" => "image/gif",
                _ => "application/octet-stream",
            };
            return Ok(axum::response::Response::builder()
                .header("content-type", content_type)
                .header(
                    "content-disposition",
                    format!("attachment; filename=\"{path}\""),
                )
                .body(axum::body::Body::from(bytes))
                .unwrap());
        }
    }

    Err(StatusCode::NOT_FOUND)
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

    let bytes = if can_read_worktree_ref(q.ref_name.as_deref(), &task.branch) {
        match read_worktree_file(&task.repo_path, path).await {
            Some(bytes) => Some(bytes),
            None => git_show_file(&task.repo_path, slug, ref_name, path).await,
        }
    } else {
        git_show_file(&task.repo_path, slug, ref_name, path).await
    }
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

    let raw_md_bytes = if can_read_worktree_ref(q.ref_name.as_deref(), &task.branch) {
        match read_worktree_file(&task.repo_path, path).await {
            Some(bytes) => Some(bytes),
            None => git_show_file(&task.repo_path, slug, ref_name, path).await,
        }
    } else {
        git_show_file(&task.repo_path, slug, ref_name, path).await
    }
    .ok_or(StatusCode::NOT_FOUND)?;

    let add_toc = q.toc.unwrap_or(false);
    let number_sections = q.number_sections.unwrap_or(false);
    let title_page = q.title_page.unwrap_or(true);

    let raw_md = String::from_utf8_lossy(&raw_md_bytes);
    let mut md_content = preprocess_legal_markdown(&raw_md);

    if !project.privilege_level.is_empty() {
        md_content = format!(
            "**PRIVILEGED AND CONFIDENTIAL — {}**\n\n---\n\n{}",
            project.privilege_level.to_uppercase(),
            md_content
        );
    }

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

    let effective_template_id = q.template_id.or(project.default_template_id);
    let template_info = if let Some(tid) = effective_template_id {
        state
            .db
            .get_knowledge_file_in_workspace(project.workspace_id, tid)
            .map_err(internal)?
            .map(|f| {
                let p = super::knowledge::safe_knowledge_path(
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
    }

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
                let p = super::knowledge::safe_knowledge_path(
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
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Query(q): Query<ListProjectFilesQuery>,
) -> Result<Json<Value>, StatusCode> {
    let (_project, _role) =
        super::require_project_access_with_shares(state.as_ref(), &user, &workspace, id)?;
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

pub(crate) async fn delete_project_file(
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

    if let Err(err) = state.file_storage.delete(&file.stored_path).await {
        tracing::warn!(project_id, file_id, "failed to delete stored file: {err}");
    }
    if let Some(search) = &state.search {
        let _ = search.delete_file_chunks(project_id, file_id).await;
    }
    state
        .db
        .delete_project_file(project_id, file_id)
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
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
        let content_hash = super::utils::sha256_hex_bytes(&bytes);
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
            super::utils::rand_suffix(),
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

// ── Project sharing ─────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct AddProjectShareBody {
    pub email: String,
    pub role: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CreateShareLinkBody {
    pub label: Option<String>,
    pub expires_in_hours: Option<i64>,
}

pub(crate) async fn list_project_shares(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let (_project, role) =
        super::require_project_access_with_shares(state.as_ref(), &user, &workspace, id)?;
    super::require_min_role(&role, "viewer")?;
    let shares = state.db.list_project_shares(id).map_err(internal)?;
    Ok(Json(json!(shares)))
}

pub(crate) async fn add_project_share(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Json(body): Json<AddProjectShareBody>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let (_project, role) =
        super::require_project_access_with_shares(state.as_ref(), &user, &workspace, id)?;
    super::require_min_role(&role, "editor")?;

    let role_to_grant = body.role.as_deref().unwrap_or("viewer");
    if !["owner", "editor", "viewer"].contains(&role_to_grant) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let (target_user_id, _, _, _) = state
        .db
        .get_user_by_email(body.email.trim())
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let share_id = state
        .db
        .add_project_share(id, target_user_id, role_to_grant, user.id)
        .map_err(internal)?;

    Ok((StatusCode::CREATED, Json(json!({ "id": share_id }))))
}

pub(crate) async fn remove_project_share(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path((id, target_user_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, StatusCode> {
    let (_project, role) =
        super::require_project_access_with_shares(state.as_ref(), &user, &workspace, id)?;
    super::require_min_role(&role, "editor")?;

    let removed = state
        .db
        .remove_project_share(id, target_user_id)
        .map_err(internal)?;
    Ok(Json(json!({ "removed": removed })))
}

pub(crate) async fn list_project_share_links(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let (_project, role) =
        super::require_project_access_with_shares(state.as_ref(), &user, &workspace, id)?;
    super::require_min_role(&role, "editor")?;
    let links = state.db.list_project_share_links(id).map_err(internal)?;
    Ok(Json(json!(links)))
}

pub(crate) async fn create_project_share_link(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Json(body): Json<CreateShareLinkBody>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let (_project, role) =
        super::require_project_access_with_shares(state.as_ref(), &user, &workspace, id)?;
    super::require_min_role(&role, "editor")?;

    let token = crate::auth::generate_token();
    let hours = body.expires_in_hours.unwrap_or(72).max(1).min(720);
    let expires_at = (Utc::now() + chrono::Duration::hours(hours))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let label = body.label.as_deref().unwrap_or("");

    let link_id = state
        .db
        .create_project_share_link(id, &token, label, &expires_at, user.id)
        .map_err(internal)?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": link_id,
            "token": token,
            "expires_at": expires_at,
        })),
    ))
}

pub(crate) async fn revoke_project_share_link(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path((id, link_id)): Path<(i64, i64)>,
) -> Result<Json<Value>, StatusCode> {
    let (_project, role) =
        super::require_project_access_with_shares(state.as_ref(), &user, &workspace, id)?;
    super::require_min_role(&role, "editor")?;

    let revoked = state
        .db
        .revoke_project_share_link(link_id)
        .map_err(internal)?;
    Ok(Json(json!({ "revoked": revoked })))
}

fn resolve_public_share(
    state: &AppState,
) -> impl Fn(&str) -> Result<(borg_core::db::ProjectShareLinkRow, ProjectRow), StatusCode> + '_ {
    move |token: &str| {
        let link = state
            .db
            .get_project_share_link_by_token(token)
            .map_err(internal)?
            .ok_or(StatusCode::NOT_FOUND)?;
        if link.revoked {
            return Err(StatusCode::NOT_FOUND);
        }
        let expires = chrono::NaiveDateTime::parse_from_str(&link.expires_at, "%Y-%m-%d %H:%M:%S")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if Utc::now().naive_utc() > expires {
            return Err(StatusCode::GONE);
        }
        let project = state
            .db
            .get_project(link.project_id)
            .map_err(internal)?
            .ok_or(StatusCode::NOT_FOUND)?;
        Ok((link, project))
    }
}

pub(crate) async fn get_public_project(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let (_link, project) = resolve_public_share(state.as_ref())(&token)?;
    let counts = state.db.project_task_status_counts(project.id).ok();
    Ok(Json(json!(ProjectJson::from_row(project, counts))))
}

pub(crate) async fn get_public_project_tasks(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let (_link, project) = resolve_public_share(state.as_ref())(&token)?;
    let tasks = state.db.list_project_tasks(project.id).map_err(internal)?;
    Ok(Json(json!(tasks)))
}

pub(crate) async fn get_public_project_documents(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let (_link, project) = resolve_public_share(state.as_ref())(&token)?;
    let files = state.db.list_project_files(project.id).map_err(internal)?;
    let public_files: Vec<_> = files.into_iter().filter(|f| !f.privileged).collect();
    Ok(Json(json!(public_files)))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, process::Command};

    use super::{is_safe_repo_relative_path, list_worktree_files, read_worktree_file};

    fn run_git(repo_path: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(["-C", repo_path.to_str().expect("repo path utf8")])
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed: stdout={} stderr={}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn list_worktree_files_includes_untracked_outputs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_path = temp.path();

        run_git(repo_path, &["init"]);
        run_git(repo_path, &["config", "user.name", "Borg Test"]);
        run_git(repo_path, &["config", "user.email", "borg@example.com"]);

        fs::write(repo_path.join("brief.md"), "brief").expect("write tracked file");
        run_git(repo_path, &["add", "brief.md"]);
        run_git(repo_path, &["commit", "-m", "initial"]);

        fs::write(repo_path.join("intake_note.md"), "draft deliverable")
            .expect("write untracked file");
        fs::create_dir_all(repo_path.join(".borg")).expect("create hidden dir");
        fs::write(repo_path.join(".borg/phase-verdict.json"), "{}").expect("write hidden file");

        let files = list_worktree_files(repo_path.to_str().expect("repo path utf8"))
            .await
            .expect("worktree files");

        assert!(files.contains(&"brief.md".to_string()));
        assert!(files.contains(&"intake_note.md".to_string()));
        assert!(files.contains(&".borg/phase-verdict.json".to_string()));
    }

    #[tokio::test]
    async fn read_worktree_file_rejects_parent_traversal() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_path = temp.path();

        run_git(repo_path, &["init"]);
        fs::write(repo_path.join("brief.md"), "brief").expect("write tracked file");

        let bytes = read_worktree_file(repo_path.to_str().expect("repo path utf8"), "brief.md")
            .await
            .expect("read safe file");
        assert_eq!(String::from_utf8_lossy(&bytes), "brief");

        assert!(
            read_worktree_file(repo_path.to_str().expect("repo path utf8"), "../secret.txt")
                .await
                .is_none()
        );
        assert!(!is_safe_repo_relative_path("../secret.txt"));
        assert!(!is_safe_repo_relative_path("/tmp/secret.txt"));
        assert!(is_safe_repo_relative_path("nested/file.md"));
    }
}
