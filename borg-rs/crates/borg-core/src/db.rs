use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json;

use crate::{
    linked_credentials::LinkedCredentialBundle,
    pgcompat as pg,
    pgcompat::{params, Connection, ConnectionGuard, Mutex, OptionalExtension},
    types::{Proposal, QueueEntry, Task},
};

const SCHEMA_SQL: &str = include_str!("../../../schema.pg.sql");

pub struct Db {
    conn: Mutex<Connection>,
}

// ── Auxiliary types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ProjectTaskCounts {
    pub active: i64,
    pub review: i64,
    pub done: i64,
    pub failed: i64,
    pub total: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskOutput {
    pub id: i64,
    pub task_id: i64,
    pub phase: String,
    pub output: String,
    pub raw_stream: String,
    pub exit_code: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskMessage {
    pub id: i64,
    pub task_id: i64,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub delivered_phase: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RepoRow {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub mode: String,
    pub backend: Option<String>,
    pub test_cmd: String,
    pub prompt_file: String,
    pub auto_merge: bool,
    pub repo_slug: String,
}

#[derive(serde::Serialize)]
pub struct LegacyEvent {
    pub id: i64,
    pub ts: i64,
    pub level: String,
    pub category: String,
    pub message: String,
    pub metadata: String,
}

#[derive(serde::Serialize)]
pub struct ChatMessage {
    pub id: String,
    pub chat_jid: String,
    pub sender: Option<String>,
    pub sender_name: Option<String>,
    pub content: String,
    pub timestamp: String,
    pub is_from_me: bool,
    pub is_bot_message: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_stream: Option<String>,
}

#[derive(serde::Serialize)]
pub struct ApiKeyEntry {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<i64>,
    pub owner: String,
    pub provider: String,
    pub key_name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LinkedCredentialEntry {
    pub id: i64,
    pub user_id: i64,
    pub provider: String,
    pub auth_kind: String,
    pub account_email: String,
    pub account_label: String,
    pub status: String,
    pub expires_at: String,
    pub last_validated_at: String,
    pub last_used_at: String,
    pub last_error: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct LinkedCredentialSecret {
    pub entry: LinkedCredentialEntry,
    pub bundle: LinkedCredentialBundle,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkspaceRow {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub kind: String,
    pub owner_user_id: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkspaceMembershipRow {
    pub workspace_id: i64,
    pub name: String,
    pub slug: String,
    pub kind: String,
    pub role: String,
    pub is_default: bool,
    pub created_at: String,
}

#[derive(serde::Serialize)]
pub struct CitationVerification {
    pub id: i64,
    pub task_id: i64,
    pub citation_text: String,
    pub citation_type: String,
    pub status: String,
    pub source: String,
    pub treatment: String,
    pub checked_at: String,
    pub created_at: String,
}

pub struct RegisteredGroup {
    pub jid: String,
    pub name: String,
    pub folder: String,
    pub trigger_pattern: String,
    pub requires_trigger: bool,
}

pub struct ChatAgentRun {
    pub id: i64,
    pub jid: String,
    pub status: String,
    pub transport: String,
    pub original_id: String,
    pub trigger_msg_id: String,
    pub folder: String,
    pub output: String,
    pub new_session_id: String,
    pub last_msg_timestamp: String,
    pub started_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct UsageSummary {
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost_usd: f64,
    pub message_count: i64,
    pub task_count: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectRow {
    pub id: i64,
    pub workspace_id: i64,
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
    pub session_privileged: bool,
    pub default_template_id: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct ProjectShareRow {
    pub id: i64,
    pub project_id: i64,
    pub user_id: i64,
    pub role: String,
    pub granted_by: Option<i64>,
    pub username: String,
    pub display_name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SharedProjectRow {
    pub id: i64,
    pub workspace_id: i64,
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
    pub session_privileged: bool,
    pub default_template_id: Option<i64>,
    pub created_at: String,
    pub share_role: String,
    pub workspace_name: String,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct ProjectShareLinkRow {
    pub id: i64,
    pub project_id: i64,
    pub token: String,
    pub label: String,
    pub expires_at: String,
    pub created_by: Option<i64>,
    pub revoked: bool,
    pub created_at: String,
}

#[derive(serde::Serialize, Clone)]
pub struct ProjectFileRow {
    pub id: i64,
    pub project_id: i64,
    pub file_name: String,
    pub source_path: String,
    pub stored_path: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub extracted_text: String,
    pub content_hash: String,
    pub privileged: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct ProjectFileMetaRow {
    pub id: i64,
    pub project_id: i64,
    pub file_name: String,
    pub source_path: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub privileged: bool,
    pub has_text: bool,
    pub text_chars: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, serde::Serialize, Clone, Default)]
pub struct ProjectFileStats {
    pub project_id: i64,
    pub total_files: i64,
    pub total_bytes: i64,
    pub privileged_files: i64,
    pub text_files: i64,
    pub text_chars: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct ProjectFilePageCursor {
    pub created_at: String,
    pub id: i64,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct KnowledgeFile {
    pub id: i64,
    pub workspace_id: i64,
    pub file_name: String,
    pub description: String,
    pub size_bytes: i64,
    pub inline: bool,
    pub tags: String,
    pub category: String,
    pub jurisdiction: String,
    pub project_id: Option<i64>,
    pub user_id: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct KnowledgeRepo {
    pub id: i64,
    pub workspace_id: i64,
    pub user_id: Option<i64>,
    pub url: String,
    pub name: String,
    pub local_path: String,
    pub status: String,
    pub error_msg: String,
    pub created_at: String,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct AuditEvent {
    pub id: i64,
    pub task_id: Option<i64>,
    pub project_id: Option<i64>,
    pub actor: String,
    pub kind: String,
    pub payload: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct FtsResult {
    pub project_id: i64,
    pub task_id: i64,
    pub file_path: String,
    pub title_snippet: String,
    pub content_snippet: String,
    pub rank: f64,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct CloudConnection {
    pub id: i64,
    pub project_id: i64,
    /// "dropbox" | "google_drive" | "onedrive"
    pub provider: String,
    pub access_token: String,
    pub refresh_token: String,
    /// ISO 8601 expiry timestamp
    pub token_expiry: String,
    pub account_email: String,
    pub account_id: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct UploadSession {
    pub id: i64,
    pub project_id: i64,
    pub file_name: String,
    pub mime_type: String,
    pub file_size: i64,
    pub chunk_size: i64,
    pub total_chunks: i64,
    pub uploaded_bytes: i64,
    pub is_zip: bool,
    pub privileged: bool,
    pub status: String,
    pub stored_path: String,
    pub error: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct ThemeTerm {
    pub term: String,
    pub occurrences: i64,
    pub document_count: i64,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct ThemeSummary {
    pub documents_scanned: i64,
    pub tokens_scanned: i64,
    pub keywords: Vec<ThemeTerm>,
    pub phrases: Vec<ThemeTerm>,
}

// ── Timestamp helpers ─────────────────────────────────────────────────────

fn parse_ts(s: &str) -> DateTime<Utc> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .map(|ndt| ndt.and_utc())
        .unwrap_or_else(|e| {
            tracing::warn!("failed to parse timestamp '{s}': {e}");
            Utc::now()
        })
}

fn now_str() -> String {
    Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in input.chars().flat_map(|c| c.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn unique_slug(base: &str, suffix: i64) -> String {
    let slug = slugify(base);
    if suffix <= 0 {
        if slug.is_empty() {
            "workspace".to_string()
        } else {
            slug
        }
    } else if slug.is_empty() {
        format!("workspace-{suffix}")
    } else {
        format!("{slug}-{suffix}")
    }
}

fn row_to_workspace(row: &pg::Row<'_>) -> pg::Result<WorkspaceRow> {
    Ok(WorkspaceRow {
        id: row.get(0)?,
        name: row.get(1)?,
        slug: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
        kind: row.get(3)?,
        owner_user_id: row.get(4)?,
        created_at: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
    })
}

fn row_to_knowledge(row: &pg::Row<'_>) -> pg::Result<KnowledgeFile> {
    let inline_int: i64 = row.get(5)?;
    Ok(KnowledgeFile {
        id: row.get(0)?,
        workspace_id: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
        file_name: row.get(2)?,
        description: row.get(3)?,
        size_bytes: row.get(4)?,
        inline: inline_int != 0,
        created_at: row.get(6)?,
        tags: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
        category: row
            .get::<_, Option<String>>(8)?
            .unwrap_or_else(|| "general".to_string()),
        jurisdiction: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
        project_id: row.get::<_, Option<i64>>(10)?,
        user_id: row.get::<_, Option<i64>>(11)?,
    })
}

fn is_stopword(token: &str) -> bool {
    matches!(
        token,
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "been"
            | "being"
            | "but"
            | "by"
            | "can"
            | "could"
            | "did"
            | "do"
            | "does"
            | "for"
            | "from"
            | "had"
            | "has"
            | "have"
            | "if"
            | "in"
            | "into"
            | "is"
            | "it"
            | "its"
            | "may"
            | "might"
            | "must"
            | "not"
            | "of"
            | "on"
            | "or"
            | "our"
            | "shall"
            | "should"
            | "that"
            | "the"
            | "their"
            | "there"
            | "these"
            | "they"
            | "this"
            | "those"
            | "to"
            | "under"
            | "upon"
            | "was"
            | "were"
            | "will"
            | "with"
            | "would"
            | "you"
            | "your"
    )
}

fn tokenize_for_themes(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
            if current.len() >= 40 {
                out.push(current.clone());
                current.clear();
            }
        } else if !current.is_empty() {
            if current.len() >= 3 && !is_stopword(&current) {
                out.push(current.clone());
            }
            current.clear();
        }
    }
    if !current.is_empty() && current.len() >= 3 && !is_stopword(&current) {
        out.push(current);
    }
    out
}

fn push_theme_term(out: &mut Vec<ThemeTerm>, term: String, occurrences: i64, document_count: i64) {
    out.push(ThemeTerm {
        term,
        occurrences,
        document_count,
    });
}

fn row_to_cloud_connection(row: &pg::Row<'_>) -> pg::Result<CloudConnection> {
    Ok(CloudConnection {
        id: row.get(0)?,
        project_id: row.get(1)?,
        provider: row.get(2)?,
        access_token: row.get(3)?,
        refresh_token: row.get(4)?,
        token_expiry: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        account_email: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        account_id: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
        created_at: row.get::<_, String>(8).map(|s| parse_ts(&s))?,
    })
}

fn row_to_upload_session(row: &pg::Row<'_>) -> pg::Result<UploadSession> {
    let is_zip: i64 = row.get(8)?;
    let privileged: i64 = row.get(9)?;
    Ok(UploadSession {
        id: row.get(0)?,
        project_id: row.get(1)?,
        file_name: row.get(2)?,
        mime_type: row.get(3)?,
        file_size: row.get(4)?,
        chunk_size: row.get(5)?,
        total_chunks: row.get(6)?,
        uploaded_bytes: row.get(7)?,
        is_zip: is_zip != 0,
        privileged: privileged != 0,
        status: row.get(10)?,
        stored_path: row.get::<_, Option<String>>(11)?.unwrap_or_default(),
        error: row.get::<_, Option<String>>(12)?.unwrap_or_default(),
        created_at: row.get::<_, Option<String>>(13)?.unwrap_or_default(),
        updated_at: row.get::<_, Option<String>>(14)?.unwrap_or_default(),
    })
}

// ── Row mappers ───────────────────────────────────────────────────────────

const TASK_COLS: &str = "id, title, description, repo_path, branch, status, attempt, \
    max_attempts, last_error, created_by, notify_chat, created_at, \
    session_id, mode, backend, workspace_id, project_id, task_type, requires_exhaustive_corpus_review, \
    started_at, completed_at, duration_secs, review_status, revision_count, updated_at, chat_thread";

fn row_to_task(row: &pg::Row<'_>) -> pg::Result<Task> {
    let created_at_str: String = row.get(11)?;
    let started_at: Option<String> = row.get(19)?;
    let completed_at: Option<String> = row.get(20)?;
    let updated_at_str: String = row.get(24)?;
    Ok(Task {
        id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        repo_path: row.get(3)?,
        branch: row.get(4)?,
        status: row.get(5)?,
        attempt: row.get(6)?,
        max_attempts: row.get(7)?,
        last_error: row.get(8)?,
        created_by: row.get(9)?,
        notify_chat: row.get(10)?,
        created_at: parse_ts(&created_at_str),
        updated_at: parse_ts(&updated_at_str),
        session_id: row.get(12)?,
        mode: row.get(13)?,
        backend: row.get::<_, Option<String>>(14)?.unwrap_or_default(),
        workspace_id: row.get::<_, Option<i64>>(15)?.unwrap_or(0),
        project_id: row.get::<_, Option<i64>>(16)?.unwrap_or(0),
        task_type: row.get::<_, Option<String>>(17)?.unwrap_or_default(),
        requires_exhaustive_corpus_review: row.get::<_, Option<i64>>(18)?.unwrap_or(0) != 0,
        started_at: started_at.map(|s| parse_ts(&s)),
        completed_at: completed_at.map(|s| parse_ts(&s)),
        duration_secs: row.get(21)?,
        review_status: row.get(22)?,
        revision_count: row.get::<_, Option<i64>>(23)?.unwrap_or(0),
        chat_thread: row.get::<_, Option<String>>(25)?.unwrap_or_default(),
    })
}

fn row_to_proposal(row: &pg::Row<'_>) -> pg::Result<Proposal> {
    let created_at_str: String = row.get(6)?;
    Ok(Proposal {
        id: row.get(0)?,
        repo_path: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        rationale: row.get(4)?,
        status: row.get(5)?,
        created_at: parse_ts(&created_at_str),
        triage_score: row.get(7)?,
        triage_impact: row.get(8)?,
        triage_feasibility: row.get(9)?,
        triage_risk: row.get(10)?,
        triage_effort: row.get(11)?,
        triage_reasoning: row.get(12)?,
    })
}

fn row_to_queue_entry(row: &pg::Row<'_>) -> pg::Result<QueueEntry> {
    let queued_at_str: String = row.get(5)?;
    Ok(QueueEntry {
        id: row.get(0)?,
        task_id: row.get(1)?,
        branch: row.get(2)?,
        repo_path: row.get(3)?,
        status: row.get(4)?,
        queued_at: parse_ts(&queued_at_str),
        pr_number: row.get(6)?,
    })
}

fn row_to_task_output(row: &pg::Row<'_>) -> pg::Result<TaskOutput> {
    let created_at_str: String = row.get(6)?;
    Ok(TaskOutput {
        id: row.get(0)?,
        task_id: row.get(1)?,
        phase: row.get(2)?,
        output: row.get(3)?,
        raw_stream: row.get(4)?,
        exit_code: row.get(5)?,
        created_at: parse_ts(&created_at_str),
    })
}

fn row_to_task_message(row: &pg::Row<'_>) -> pg::Result<TaskMessage> {
    let created_at_str: String = row.get(4)?;
    Ok(TaskMessage {
        id: row.get(0)?,
        task_id: row.get(1)?,
        role: row.get(2)?,
        content: row.get(3)?,
        created_at: parse_ts(&created_at_str),
        delivered_phase: row.get(5)?,
    })
}

fn row_to_repo(row: &pg::Row<'_>) -> pg::Result<RepoRow> {
    let auto_merge_int: i64 = row.get(7)?;
    Ok(RepoRow {
        id: row.get(0)?,
        path: row.get(1)?,
        name: row.get(2)?,
        mode: row.get(3)?,
        backend: row.get(4)?,
        test_cmd: row.get(5)?,
        prompt_file: row.get(6)?,
        auto_merge: auto_merge_int != 0,
        repo_slug: row.get(8).unwrap_or_default(),
    })
}

fn row_to_chat_agent_run(row: &pg::Row<'_>) -> pg::Result<ChatAgentRun> {
    Ok(ChatAgentRun {
        id: row.get(0)?,
        jid: row.get(1)?,
        status: row.get(2)?,
        transport: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        original_id: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        trigger_msg_id: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        folder: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        output: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
        new_session_id: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
        last_msg_timestamp: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
        started_at: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
        completed_at: row.get(11)?,
    })
}

fn row_to_legacy_event(row: &pg::Row<'_>) -> pg::Result<LegacyEvent> {
    Ok(LegacyEvent {
        id: row.get(0)?,
        ts: row.get(1)?,
        level: row.get(2)?,
        category: row.get(3)?,
        message: row.get(4)?,
        metadata: row.get(5)?,
    })
}

const PROJECT_COLS: &str = "id, workspace_id, name, mode, repo_path, client_name, case_number, jurisdiction, \
    matter_type, opposing_counsel, deadline, privilege_level, status, default_template_id, created_at, session_privileged";

fn row_to_project(row: &pg::Row<'_>) -> pg::Result<ProjectRow> {
    let created_at_str: String = row.get(14)?;
    let session_privileged_int: i64 = row.get(15)?;
    Ok(ProjectRow {
        id: row.get(0)?,
        workspace_id: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
        name: row.get(2)?,
        mode: row.get(3)?,
        repo_path: row.get(4)?,
        client_name: row.get(5)?,
        case_number: row.get(6)?,
        jurisdiction: row.get(7)?,
        matter_type: row.get(8)?,
        opposing_counsel: row.get(9)?,
        deadline: row.get(10)?,
        privilege_level: row.get(11)?,
        status: row.get(12)?,
        session_privileged: session_privileged_int != 0,
        default_template_id: row.get(13)?,
        created_at: parse_ts(&created_at_str),
    })
}

const PROJECT_FILE_COLS: &str = "id, project_id, file_name, source_path, stored_path, mime_type, size_bytes, extracted_text, content_hash, created_at, privileged";
const PROJECT_FILE_META_COLS: &str = "id, project_id, file_name, source_path, mime_type, size_bytes, privileged, created_at, length(extracted_text)::BIGINT";

fn row_to_project_file(row: &pg::Row<'_>) -> pg::Result<ProjectFileRow> {
    let created_at_str: String = row.get(9)?;
    let privileged_int: i64 = row.get(10)?;
    Ok(ProjectFileRow {
        id: row.get(0)?,
        project_id: row.get(1)?,
        file_name: row.get(2)?,
        source_path: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        stored_path: row.get(4)?,
        mime_type: row.get(5)?,
        size_bytes: row.get(6)?,
        extracted_text: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
        content_hash: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
        privileged: privileged_int != 0,
        created_at: parse_ts(&created_at_str),
    })
}

fn row_to_project_file_meta(row: &pg::Row<'_>) -> pg::Result<ProjectFileMetaRow> {
    let created_at_str: String = row.get(7)?;
    let privileged_int: i64 = row.get(6)?;
    let text_chars: i64 = row.get::<_, Option<i64>>(8)?.unwrap_or(0);
    Ok(ProjectFileMetaRow {
        id: row.get(0)?,
        project_id: row.get(1)?,
        file_name: row.get(2)?,
        source_path: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        mime_type: row.get(4)?,
        size_bytes: row.get(5)?,
        privileged: privileged_int != 0,
        has_text: text_chars > 0,
        text_chars,
        created_at: parse_ts(&created_at_str),
    })
}

fn row_to_tool_call(row: &pg::Row<'_>) -> pg::Result<crate::tool_calls::ToolCallEvent> {
    Ok(crate::tool_calls::ToolCallEvent {
        id: row.get(0)?,
        task_id: row.get(1)?,
        chat_key: row.get(2)?,
        run_id: row.get(3)?,
        tool_name: row.get(4)?,
        input_summary: row.get(5)?,
        output_summary: row.get(6)?,
        started_at: row.get(7)?,
        duration_ms: row.get(8)?,
        success: row.get(9)?,
        error: row.get(10)?,
    })
}

// ── Db impl ───────────────────────────────────────────────────────────────

impl Db {
    pub fn open(database_url: &str) -> Result<Self> {
        let conn = Connection::open(database_url)
            .with_context(|| format!("failed to open Postgres database at {database_url:?}"))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn migrate(&mut self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute_batch(SCHEMA_SQL)
            .context("failed to apply clean-break Postgres schema")?;
        Self::backfill_workspaces(&conn).context("workspace backfill")?;
        Ok(())
    }

    fn get_or_create_workspace(
        conn: &ConnectionGuard,
        name: &str,
        kind: &str,
        owner_user_id: Option<i64>,
        preferred_slug: &str,
    ) -> Result<i64> {
        let existing = if owner_user_id.is_some() {
            conn.query_row(
                "SELECT id FROM workspaces WHERE owner_user_id = ?1 AND kind = ?2 ORDER BY id ASC LIMIT 1",
                params![owner_user_id, kind],
                |row| row.get(0),
            )
            .optional()?
        } else {
            conn.query_row(
                "SELECT id FROM workspaces WHERE slug = ?1 AND kind = ?2 ORDER BY id ASC LIMIT 1",
                params![preferred_slug, kind],
                |row| row.get(0),
            )
            .optional()?
        };
        if let Some(id) = existing {
            return Ok(id);
        }
        let slug = if preferred_slug.trim().is_empty() {
            unique_slug(name, 0)
        } else {
            preferred_slug.to_string()
        };
        conn.execute_returning_id(
            "INSERT INTO workspaces (name, slug, kind, owner_user_id) VALUES (?1, ?2, ?3, ?4)",
            params![name, slug, kind, owner_user_id],
        )
        .context("insert workspace")
    }

    fn backfill_workspaces(conn: &ConnectionGuard) -> Result<()> {
        let mut stmt = conn.prepare(
            "SELECT id, username, display_name FROM users \
             WHERE default_workspace_id IS NULL ORDER BY id ASC",
        )?;
        let users = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<pg::Result<Vec<_>>>()?;

        for (user_id, username, display_name) in users {
            let workspace_name = if display_name.trim().is_empty() {
                format!("{username} Personal")
            } else {
                format!("{display_name} Personal")
            };
            let workspace_id = Self::get_or_create_workspace(
                conn,
                &workspace_name,
                "personal",
                Some(user_id),
                &unique_slug(&format!("{username}-personal"), 0),
            )?;
            conn.execute(
                "INSERT INTO workspace_memberships (workspace_id, user_id, role) VALUES (?1, ?2, 'owner') \
                 ON CONFLICT (workspace_id, user_id) DO UPDATE SET role = EXCLUDED.role",
                params![workspace_id, user_id],
            )?;
            conn.execute(
                "UPDATE users SET default_workspace_id = ?1 WHERE id = ?2",
                params![workspace_id, user_id],
            )?;
        }

        let legacy_counts: i64 = conn
            .query_row(
                "SELECT \
                    (SELECT COUNT(*) FROM projects WHERE workspace_id IS NULL) + \
                    (SELECT COUNT(*) FROM pipeline_tasks WHERE workspace_id IS NULL) + \
                    (SELECT COUNT(*) FROM knowledge_files WHERE workspace_id IS NULL) + \
                    (SELECT COUNT(*) FROM api_keys WHERE workspace_id IS NULL)",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let system_workspace_id =
            Self::get_or_create_workspace(conn, "System Workspace", "system", None, "system")?;

        if legacy_counts > 0 {
            let legacy_workspace_id = Self::get_or_create_workspace(
                conn,
                "Legacy Shared",
                "shared",
                None,
                "legacy-shared",
            )?;
            let mut members = conn.prepare("SELECT id, is_admin FROM users ORDER BY id ASC")?;
            for row in members.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?))
            })? {
                let (user_id, is_admin) = row?;
                let role = if is_admin { "admin" } else { "member" };
                conn.execute(
                    "INSERT INTO workspace_memberships (workspace_id, user_id, role) VALUES (?1, ?2, ?3) \
                     ON CONFLICT (workspace_id, user_id) DO UPDATE SET role = EXCLUDED.role",
                    params![legacy_workspace_id, user_id, role],
                )?;
            }
            conn.execute(
                "UPDATE projects SET workspace_id = ?1 WHERE workspace_id IS NULL",
                params![legacy_workspace_id],
            )?;
            conn.execute(
                "UPDATE pipeline_tasks SET workspace_id = COALESCE((SELECT workspace_id FROM projects WHERE projects.id = pipeline_tasks.project_id), ?1) \
                 WHERE workspace_id IS NULL",
                params![legacy_workspace_id],
            )?;
            conn.execute(
                "UPDATE knowledge_files SET workspace_id = COALESCE((SELECT workspace_id FROM projects WHERE projects.id = knowledge_files.project_id), ?1) \
                 WHERE workspace_id IS NULL",
                params![legacy_workspace_id],
            )?;
            conn.execute(
                "UPDATE api_keys SET workspace_id = ?1 WHERE workspace_id IS NULL",
                params![legacy_workspace_id],
            )?;
        } else {
            conn.execute(
                "UPDATE pipeline_tasks SET workspace_id = COALESCE((SELECT workspace_id FROM projects WHERE projects.id = pipeline_tasks.project_id), ?1) \
                 WHERE workspace_id IS NULL",
                params![system_workspace_id],
            )?;
        }

        Ok(())
    }

    // ── Pipeline Tasks ────────────────────────────────────────────────────

    pub fn get_task(&self, id: i64) -> Result<Option<Task>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let result = conn
            .query_row(
                &format!("SELECT {TASK_COLS} FROM pipeline_tasks WHERE id = ?1"),
                params![id],
                row_to_task,
            )
            .optional()
            .context("get_task")?;
        Ok(result)
    }

    pub fn list_active_tasks(&self) -> Result<Vec<Task>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = format!(
            "SELECT {TASK_COLS} FROM pipeline_tasks \
             WHERE status NOT IN ('done', 'merged', 'failed', 'blocked', 'pending_review', 'human_review', 'purged') \
             ORDER BY CASE status \
               WHEN 'rebase' THEN 0 \
               WHEN 'validate' THEN 1 \
               WHEN 'implement' THEN 1 \
               WHEN 'impl' THEN 1 \
               WHEN 'retry' THEN 1 \
               WHEN 'qa' THEN 2 \
               WHEN 'spec' THEN 3 \
               ELSE 4 \
             END, id ASC",
        );
        let mut stmt = conn.prepare(&sql)?;
        let tasks = stmt
            .query_map([], row_to_task)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_active_tasks")?;
        Ok(tasks)
    }

    pub fn insert_task(&self, task: &Task) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let created_at = task.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
        let project_id = if task.project_id == 0 {
            None
        } else {
            Some(task.project_id)
        };
        let workspace_id = if task.workspace_id > 0 {
            Some(task.workspace_id)
        } else if let Some(project_id) = project_id {
            conn.query_row(
                "SELECT workspace_id FROM projects WHERE id = ?1",
                params![project_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten()
        } else {
            conn.query_row(
                "SELECT id FROM workspaces WHERE kind = 'system' ORDER BY id ASC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?
        };
        let id = conn.execute_returning_id(
            "INSERT INTO pipeline_tasks \
             (title, description, repo_path, branch, status, attempt, max_attempts, \
              last_error, created_by, notify_chat, created_at, session_id, mode, backend, workspace_id, project_id, task_type, \
              requires_exhaustive_corpus_review, chat_thread) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                task.title,
                task.description,
                task.repo_path,
                task.branch,
                task.status,
                task.attempt,
                task.max_attempts,
                task.last_error,
                task.created_by,
                task.notify_chat,
                created_at,
                task.session_id,
                task.mode,
                if task.backend.is_empty() {
                    None
                } else {
                    Some(task.backend.as_str())
                },
                workspace_id,
                project_id,
                &task.task_type,
                if task.requires_exhaustive_corpus_review {
                    1i64
                } else {
                    0i64
                },
                &task.chat_thread,
            ],
        )
        .context("insert_task")?;
        Ok(id)
    }

    pub fn update_task_status(&self, id: i64, status: &str, error: Option<&str>) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let updated_at = now_str();
        conn.execute(
            "UPDATE pipeline_tasks SET status = ?1, last_error = COALESCE(?2, last_error), \
             updated_at = ?3 WHERE id = ?4",
            params![status, error, updated_at, id],
        )
        .context("update_task_status")?;
        Ok(())
    }

    pub fn mark_task_started(&self, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let now = now_str();
        conn.execute(
            "UPDATE pipeline_tasks SET started_at = COALESCE(started_at, ?1) WHERE id = ?2",
            params![now, id],
        )
        .context("mark_task_started")?;
        Ok(())
    }

    pub fn mark_task_completed(&self, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let now = now_str();
        conn.execute(
            "UPDATE pipeline_tasks SET completed_at = ?1, \
             duration_secs = CASE WHEN started_at IS NOT NULL AND started_at != '' \
               THEN GREATEST(0, CAST(EXTRACT(EPOCH FROM ((?2)::timestamp - started_at::timestamp)) AS BIGINT)) \
               ELSE NULL END \
             WHERE id = ?3",
            params![now.clone(), now, id],
        )
        .context("mark_task_completed")?;
        Ok(())
    }

    pub fn set_review_status(&self, id: i64, status: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET review_status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, now_str(), id],
        )
        .context("set_review_status")?;
        Ok(())
    }

    pub fn request_task_revision(&self, id: i64, target_phase: &str, feedback: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let tx = conn
            .transaction()
            .context("request_task_revision transaction")?;
        let updated_at = now_str();
        tx.execute(
            "INSERT INTO task_messages (task_id, role, content, created_at) \
             VALUES (?1, 'user', ?2, ?3)",
            params![id, feedback, updated_at],
        )
        .context("request_task_revision insert_task_message")?;
        tx.execute(
            "UPDATE pipeline_tasks SET status = ?1, review_status = 'revision_requested', \
             revision_count = revision_count + 1, attempt = 0, session_id = '', \
             last_error = '', updated_at = ?2 WHERE id = ?3",
            params![target_phase, updated_at, id],
        )
        .context("request_task_revision update_task")?;
        tx.commit().context("request_task_revision commit")?;
        Ok(())
    }

    pub fn increment_revision_count(&self, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET revision_count = revision_count + 1, updated_at = ?1 WHERE id = ?2",
            params![now_str(), id],
        )
        .context("increment_revision_count")?;
        Ok(())
    }

    pub fn get_task_revision_count(&self, id: i64) -> i64 {
        let Ok(conn) = self.conn.lock() else { return 0 };
        conn.query_row(
            "SELECT revision_count FROM pipeline_tasks WHERE id = ?1",
            params![id],
            |r: &pg::Row| r.get(0),
        )
        .unwrap_or(0)
    }

    pub fn update_task_branch(&self, id: i64, branch: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET branch = ?1 WHERE id = ?2",
            params![branch, id],
        )
        .context("update_task_branch")?;
        Ok(())
    }

    pub fn update_task_repo_path(&self, id: i64, repo_path: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET repo_path = ?1 WHERE id = ?2",
            params![repo_path, id],
        )
        .context("update_task_repo_path")?;
        Ok(())
    }

    pub fn update_task_session(&self, id: i64, session_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET session_id = ?1 WHERE id = ?2",
            params![session_id, id],
        )
        .context("update_task_session")?;
        Ok(())
    }

    pub fn update_task_description(&self, id: i64, title: &str, description: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET title = ?1, description = ?2 WHERE id = ?3",
            params![title, description, id],
        )
        .context("update_task_description")?;
        Ok(())
    }

    pub fn requeue_task(&self, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let updated_at = now_str();
        conn.execute(
            "UPDATE pipeline_tasks SET status = 'backlog', attempt = 0, \
             session_id = '', last_error = '', updated_at = ?1 WHERE id = ?2",
            params![updated_at, id],
        )
        .context("requeue_task")?;
        Ok(())
    }

    pub fn increment_attempt(&self, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET attempt = attempt + 1 WHERE id = ?1",
            params![id],
        )
        .context("increment_attempt")?;
        Ok(())
    }

    pub fn update_task_backend(&self, id: i64, backend: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET backend = ?1 WHERE id = ?2",
            params![
                if backend.is_empty() {
                    None
                } else {
                    Some(backend)
                },
                id
            ],
        )
        .context("update_task_backend")?;
        Ok(())
    }

    pub fn update_task_structured_data(&self, id: i64, data: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET structured_data = ?1 WHERE id = ?2",
            params![data, id],
        )
        .context("update_task_structured_data")?;
        Ok(())
    }

    pub fn get_task_structured_data(&self, id: i64) -> Result<String> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let data: String = conn
            .query_row(
                "SELECT structured_data FROM pipeline_tasks WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap_or_default();
        Ok(data)
    }

    // ── Proposals ─────────────────────────────────────────────────────────

    pub fn list_proposals(&self, repo_path: &str) -> Result<Vec<Proposal>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, repo_path, title, description, rationale, status, created_at, \
             triage_score, triage_impact, triage_feasibility, triage_risk, triage_effort, \
             triage_reasoning \
             FROM proposals WHERE repo_path = ?1 ORDER BY id ASC",
        )?;
        let proposals = stmt
            .query_map(params![repo_path], row_to_proposal)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_proposals")?;
        Ok(proposals)
    }

    pub fn list_all_proposals(&self, repo_path: Option<&str>) -> Result<Vec<Proposal>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = if repo_path.is_some() {
            "SELECT id, repo_path, title, description, rationale, status, created_at, \
             triage_score, triage_impact, triage_feasibility, triage_risk, triage_effort, \
             triage_reasoning \
             FROM proposals \
             WHERE repo_path = ?1 \
             ORDER BY id DESC"
        } else {
            "SELECT id, repo_path, title, description, rationale, status, created_at, \
             triage_score, triage_impact, triage_feasibility, triage_risk, triage_effort, \
             triage_reasoning \
             FROM proposals \
             ORDER BY id DESC"
        };
        let mut stmt = conn.prepare(sql)?;
        let proposals = if let Some(repo_path) = repo_path {
            stmt.query_map(params![repo_path], row_to_proposal)?
                .collect::<pg::Result<Vec<_>>>()
        } else {
            stmt.query_map([], row_to_proposal)?
                .collect::<pg::Result<Vec<_>>>()
        }
        .context("list_all_proposals")?;
        Ok(proposals)
    }

    pub fn get_proposal(&self, id: i64) -> Result<Option<Proposal>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let result = conn
            .query_row(
                "SELECT id, repo_path, title, description, rationale, status, created_at, \
                 triage_score, triage_impact, triage_feasibility, triage_risk, triage_effort, \
                 triage_reasoning \
                 FROM proposals WHERE id = ?1",
                params![id],
                row_to_proposal,
            )
            .optional()
            .context("get_proposal")?;
        Ok(result)
    }

    pub fn task_stats(&self) -> Result<(i64, i64, i64, i64)> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM pipeline_tasks", [], |r| r.get(0))
            .context("task_stats total")?;
        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pipeline_tasks WHERE status NOT IN ('done','merged','failed','blocked','pending_review','human_review','purged')",
                [],
                |r| r.get(0),
            )
            .context("task_stats active")?;
        let merged: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pipeline_tasks WHERE status = 'merged'",
                [],
                |r| r.get(0),
            )
            .context("task_stats merged")?;
        let failed: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pipeline_tasks WHERE status = 'failed'",
                [],
                |r| r.get(0),
            )
            .context("task_stats failed")?;
        Ok((active, merged, failed, total))
    }

    pub fn count_tasks_with_status(&self, status: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pipeline_tasks WHERE status = ?1",
                params![status],
                |r| r.get(0),
            )
            .context("count_tasks_with_status")?;
        Ok(n)
    }

    pub fn project_task_status_counts(&self, project_id: i64) -> Result<ProjectTaskCounts> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT status, COUNT(*) FROM pipeline_tasks WHERE project_id = ?1 GROUP BY status",
        )?;
        let mut counts = ProjectTaskCounts::default();
        let rows = stmt.query_map(params![project_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (status, n) = row?;
            match status.as_str() {
                "running" | "backlog" => counts.active += n,
                "human_review" => counts.review += n,
                "done" => counts.done += n,
                "failed" => counts.failed += n,
                _ => {},
            }
        }
        counts.total = counts.active + counts.review + counts.done + counts.failed;
        Ok(counts)
    }

    pub fn count_queue_with_status(&self, status: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM integration_queue WHERE status = ?1",
                params![status],
                |r| r.get(0),
            )
            .context("count_queue_with_status")?;
        Ok(n)
    }

    pub fn insert_proposal(&self, proposal: &Proposal) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let created_at = proposal.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
        let id = conn
            .execute_returning_id(
                "INSERT INTO proposals \
             (repo_path, title, description, rationale, status, created_at, \
              triage_score, triage_impact, triage_feasibility, triage_risk, \
              triage_effort, triage_reasoning) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    proposal.repo_path,
                    proposal.title,
                    proposal.description,
                    proposal.rationale,
                    proposal.status,
                    created_at,
                    proposal.triage_score,
                    proposal.triage_impact,
                    proposal.triage_feasibility,
                    proposal.triage_risk,
                    proposal.triage_effort,
                    proposal.triage_reasoning,
                ],
            )
            .context("insert_proposal")?;
        Ok(id)
    }

    pub fn update_proposal_status(&self, id: i64, status: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE proposals SET status = ?1 WHERE id = ?2",
            params![status, id],
        )
        .context("update_proposal_status")?;
        Ok(())
    }

    pub fn update_proposal_triage(
        &self,
        id: i64,
        score: i64,
        impact: i64,
        feasibility: i64,
        risk: i64,
        effort: i64,
        reasoning: &str,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE proposals SET triage_score=?1, triage_impact=?2, triage_feasibility=?3, \
             triage_risk=?4, triage_effort=?5, triage_reasoning=?6 WHERE id=?7",
            params![score, impact, feasibility, risk, effort, reasoning, id],
        )
        .context("update_proposal_triage")?;
        Ok(())
    }

    // ── Projects ──────────────────────────────────────────────────────────

    pub fn list_projects(&self) -> Result<Vec<ProjectRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = format!("SELECT {PROJECT_COLS} FROM projects ORDER BY id DESC");
        let mut stmt = conn.prepare(&sql)?;
        let projects = stmt
            .query_map([], row_to_project)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_projects")?;
        Ok(projects)
    }

    pub fn list_projects_in_workspace(&self, workspace_id: i64) -> Result<Vec<ProjectRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql =
            format!("SELECT {PROJECT_COLS} FROM projects WHERE workspace_id = ?1 ORDER BY id DESC");
        let mut stmt = conn.prepare(&sql)?;
        let projects = stmt
            .query_map(params![workspace_id], row_to_project)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_projects_in_workspace")?;
        Ok(projects)
    }

    pub fn search_projects(&self, query: &str) -> Result<Vec<ProjectRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let pattern = format!("%{query}%");
        let sql = format!(
            "SELECT {PROJECT_COLS} FROM projects \
             WHERE name LIKE ?1 OR client_name LIKE ?1 OR case_number LIKE ?1 \
             OR jurisdiction LIKE ?1 OR matter_type LIKE ?1 \
             ORDER BY id DESC LIMIT 50"
        );
        let mut stmt = conn.prepare(&sql)?;
        let projects = stmt
            .query_map(params![pattern], row_to_project)?
            .collect::<pg::Result<Vec<_>>>()
            .context("search_projects")?;
        Ok(projects)
    }

    pub fn search_projects_in_workspace(
        &self,
        workspace_id: i64,
        query: &str,
    ) -> Result<Vec<ProjectRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let pattern = format!("%{query}%");
        let sql = format!(
            "SELECT {PROJECT_COLS} FROM projects \
             WHERE workspace_id = ?1 AND (name LIKE ?2 OR client_name LIKE ?2 OR case_number LIKE ?2 \
             OR jurisdiction LIKE ?2 OR matter_type LIKE ?2) \
             ORDER BY id DESC LIMIT 50"
        );
        let mut stmt = conn.prepare(&sql)?;
        let projects = stmt
            .query_map(params![workspace_id, pattern], row_to_project)?
            .collect::<pg::Result<Vec<_>>>()
            .context("search_projects_in_workspace")?;
        Ok(projects)
    }

    pub fn get_project(&self, id: i64) -> Result<Option<ProjectRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = format!("SELECT {PROJECT_COLS} FROM projects WHERE id=?1");
        let project = conn
            .query_row(&sql, params![id], row_to_project)
            .optional()
            .context("get_project")?;
        Ok(project)
    }

    pub fn get_project_in_workspace(
        &self,
        workspace_id: i64,
        id: i64,
    ) -> Result<Option<ProjectRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = format!("SELECT {PROJECT_COLS} FROM projects WHERE id=?1 AND workspace_id = ?2");
        let project = conn
            .query_row(&sql, params![id, workspace_id], row_to_project)
            .optional()
            .context("get_project_in_workspace")?;
        Ok(project)
    }

    pub fn insert_project(
        &self,
        workspace_id: i64,
        name: &str,
        mode: &str,
        repo_path: &str,
        client_name: &str,
        jurisdiction: &str,
        matter_type: &str,
        privilege_level: &str,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let created_at = now_str();
        let id = conn.execute_returning_id(
            "INSERT INTO projects (name, mode, repo_path, client_name, jurisdiction, matter_type, \
             privilege_level, workspace_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                name,
                mode,
                repo_path,
                client_name,
                jurisdiction,
                matter_type,
                privilege_level,
                workspace_id,
                created_at
            ],
        )
        .context("insert_project")?;
        Ok(id)
    }

    pub fn update_project(
        &self,
        id: i64,
        name: Option<&str>,
        client_name: Option<&str>,
        case_number: Option<&str>,
        jurisdiction: Option<&str>,
        matter_type: Option<&str>,
        opposing_counsel: Option<&str>,
        deadline: Option<Option<&str>>,
        privilege_level: Option<&str>,
        status: Option<&str>,
        repo_path: Option<&str>,
        default_template_id: Option<Option<i64>>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut sets = Vec::new();
        let mut vals: Vec<Box<dyn pg::ToSql>> = Vec::new();
        let mut idx = 1;

        macro_rules! maybe_set {
            ($field:expr, $col:expr) => {
                if let Some(v) = $field {
                    sets.push(format!("{} = ?{}", $col, idx));
                    vals.push(Box::new(v.to_string()));
                    idx += 1;
                }
            };
        }
        maybe_set!(name, "name");
        maybe_set!(client_name, "client_name");
        maybe_set!(case_number, "case_number");
        maybe_set!(jurisdiction, "jurisdiction");
        maybe_set!(matter_type, "matter_type");
        maybe_set!(opposing_counsel, "opposing_counsel");
        maybe_set!(privilege_level, "privilege_level");
        maybe_set!(status, "status");
        maybe_set!(repo_path, "repo_path");

        if let Some(dl) = deadline {
            sets.push(format!("deadline = ?{}", idx));
            vals.push(Box::new(dl.map(|s| s.to_string())));
            idx += 1;
        }

        if let Some(tid) = default_template_id {
            sets.push(format!("default_template_id = ?{}", idx));
            vals.push(Box::new(tid));
            idx += 1;
        }

        if sets.is_empty() {
            return Ok(());
        }

        let sql = format!(
            "UPDATE projects SET {} WHERE id = ?{}",
            sets.join(", "),
            idx,
        );
        vals.push(Box::new(id));
        let params: Vec<&dyn pg::ToSql> = vals.iter().map(|v| v.as_ref()).collect();
        conn.execute(&sql, params.as_slice())
            .context("update_project")?;
        Ok(())
    }

    pub fn delete_project(&self, id: i64) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let tx = conn.transaction().context("delete_project transaction")?;

        tx.execute("DELETE FROM embeddings WHERE project_id=?1", params![id])
            .context("delete embeddings for project")?;
        tx.execute("DELETE FROM legal_fts WHERE project_id=?1", params![id])
            .context("delete legal_fts for project")?;
        tx.execute(
            "DELETE FROM project_corpus_stats WHERE project_id=?1",
            params![id],
        )
        .context("delete project_corpus_stats for project")?;
        tx.execute(
            "DELETE FROM upload_sessions WHERE project_id=?1",
            params![id],
        )
        .context("delete upload_sessions for project")?;
        tx.execute(
            "DELETE FROM cloud_connections WHERE project_id=?1",
            params![id],
        )
        .context("delete cloud_connections for project")?;
        tx.execute("DELETE FROM deadlines WHERE project_id=?1", params![id])
            .context("delete deadlines for project")?;
        tx.execute("DELETE FROM parties WHERE project_id=?1", params![id])
            .context("delete parties for project")?;
        tx.execute("DELETE FROM project_files WHERE project_id=?1", params![id])
            .context("delete project_files for project")?;
        tx.execute(
            "UPDATE knowledge_files SET project_id=NULL WHERE project_id=?1",
            params![id],
        )
        .context("unlink knowledge_files from project")?;
        tx.execute(
            "UPDATE pipeline_tasks SET project_id=NULL WHERE project_id=?1",
            params![id],
        )
        .context("unlink tasks from project")?;
        tx.execute(
            "UPDATE pipeline_events SET project_id=NULL WHERE project_id=?1",
            params![id],
        )
        .context("unlink pipeline_events from project")?;
        let affected = tx
            .execute("DELETE FROM projects WHERE id=?1", params![id])
            .context("delete_project")?;
        tx.commit().context("delete_project commit")?;
        Ok(affected > 0)
    }

    // ── Project sharing ──────────────────────────────────────────────────

    pub fn add_project_share(
        &self,
        project_id: i64,
        user_id: i64,
        role: &str,
        granted_by: i64,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let created_at = now_str();
        let id = conn
            .execute_returning_id(
                "INSERT INTO project_shares (project_id, user_id, role, granted_by, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (project_id, user_id) DO UPDATE SET role = EXCLUDED.role",
                params![project_id, user_id, role, granted_by, created_at],
            )
            .context("add_project_share")?;
        Ok(id)
    }

    pub fn remove_project_share(&self, project_id: i64, user_id: i64) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let affected = conn
            .execute(
                "DELETE FROM project_shares WHERE project_id = ?1 AND user_id = ?2",
                params![project_id, user_id],
            )
            .context("remove_project_share")?;
        Ok(affected > 0)
    }

    pub fn list_project_shares(&self, project_id: i64) -> Result<Vec<ProjectShareRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT ps.id, ps.project_id, ps.user_id, ps.role, ps.granted_by, \
                    u.username, u.display_name, ps.created_at \
             FROM project_shares ps JOIN users u ON u.id = ps.user_id \
             WHERE ps.project_id = ?1 ORDER BY ps.created_at",
        )?;
        let rows = stmt
            .query_map(params![project_id], |row| {
                Ok(ProjectShareRow {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    user_id: row.get(2)?,
                    role: row.get(3)?,
                    granted_by: row.get(4)?,
                    username: row.get(5)?,
                    display_name: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_project_shares")?;
        Ok(rows)
    }

    pub fn get_user_project_share(
        &self,
        project_id: i64,
        user_id: i64,
    ) -> Result<Option<ProjectShareRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let row = conn
            .query_row(
                "SELECT ps.id, ps.project_id, ps.user_id, ps.role, ps.granted_by, \
                        u.username, u.display_name, ps.created_at \
                 FROM project_shares ps JOIN users u ON u.id = ps.user_id \
                 WHERE ps.project_id = ?1 AND ps.user_id = ?2",
                params![project_id, user_id],
                |row| {
                    Ok(ProjectShareRow {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        user_id: row.get(2)?,
                        role: row.get(3)?,
                        granted_by: row.get(4)?,
                        username: row.get(5)?,
                        display_name: row.get(6)?,
                        created_at: row.get(7)?,
                    })
                },
            )
            .optional()
            .context("get_user_project_share")?;
        Ok(row)
    }

    pub fn list_user_shared_projects(&self, user_id: i64) -> Result<Vec<(ProjectRow, String)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql =
            "SELECT p.id, p.workspace_id, p.name, p.mode, p.repo_path, p.client_name, \
             p.case_number, p.jurisdiction, p.matter_type, p.opposing_counsel, p.deadline, \
             p.privilege_level, p.status, p.default_template_id, p.created_at, p.session_privileged, \
             ps.role \
             FROM projects p \
             JOIN project_shares ps ON ps.project_id = p.id \
             WHERE ps.user_id = ?1 ORDER BY p.id DESC";
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![user_id], |row| {
                let project = row_to_project(row)?;
                let role: String = row.get(16)?;
                Ok((project, role))
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_user_shared_projects")?;
        Ok(rows)
    }

    pub fn list_projects_shared_with_user(&self, user_id: i64) -> Result<Vec<SharedProjectRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = "SELECT p.id, p.workspace_id, p.name, p.mode, p.repo_path, p.client_name, \
             p.case_number, p.jurisdiction, p.matter_type, p.opposing_counsel, p.deadline, \
             p.privilege_level, p.status, p.default_template_id, p.created_at, p.session_privileged, \
             ps.role, w.name \
             FROM project_shares ps \
             JOIN projects p ON p.id = ps.project_id \
             JOIN workspaces w ON w.id = p.workspace_id \
             WHERE ps.user_id = ?1 \
             ORDER BY ps.created_at DESC";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt
            .query_map(params![user_id], |row| {
                let created_at_str: String = row.get(14)?;
                let session_privileged_int: i64 = row.get(15)?;
                Ok(SharedProjectRow {
                    id: row.get(0)?,
                    workspace_id: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    name: row.get(2)?,
                    mode: row.get(3)?,
                    repo_path: row.get(4)?,
                    client_name: row.get(5)?,
                    case_number: row.get(6)?,
                    jurisdiction: row.get(7)?,
                    matter_type: row.get(8)?,
                    opposing_counsel: row.get(9)?,
                    deadline: row.get(10)?,
                    privilege_level: row.get(11)?,
                    status: row.get(12)?,
                    default_template_id: row.get(13)?,
                    created_at: created_at_str,
                    session_privileged: session_privileged_int != 0,
                    share_role: row.get(16)?,
                    workspace_name: row.get(17)?,
                })
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_projects_shared_with_user")?;
        Ok(rows)
    }

    pub fn create_project_share_link(
        &self,
        project_id: i64,
        token: &str,
        label: &str,
        expires_at: &str,
        created_by: i64,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let created_at = now_str();
        let id = conn.execute_returning_id(
            "INSERT INTO project_share_links (project_id, token, label, expires_at, created_by, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![project_id, token, label, expires_at, created_by, created_at],
        )
        .context("create_project_share_link")?;
        Ok(id)
    }

    pub fn get_project_share_link_by_token(
        &self,
        token: &str,
    ) -> Result<Option<ProjectShareLinkRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let row = conn
            .query_row(
                "SELECT id, project_id, token, label, expires_at, created_by, revoked, created_at \
                 FROM project_share_links WHERE token = ?1 AND revoked = 0",
                params![token],
                |row| {
                    let revoked_int: i64 = row.get(6)?;
                    Ok(ProjectShareLinkRow {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        token: row.get(2)?,
                        label: row.get(3)?,
                        expires_at: row.get(4)?,
                        created_by: row.get(5)?,
                        revoked: revoked_int != 0,
                        created_at: row.get(7)?,
                    })
                },
            )
            .optional()
            .context("get_project_share_link_by_token")?;
        Ok(row)
    }

    pub fn list_project_share_links(&self, project_id: i64) -> Result<Vec<ProjectShareLinkRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, token, label, expires_at, created_by, revoked, created_at \
             FROM project_share_links WHERE project_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![project_id], |row| {
                let revoked_int: i64 = row.get(6)?;
                Ok(ProjectShareLinkRow {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    token: row.get(2)?,
                    label: row.get(3)?,
                    expires_at: row.get(4)?,
                    created_by: row.get(5)?,
                    revoked: revoked_int != 0,
                    created_at: row.get(7)?,
                })
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_project_share_links")?;
        Ok(rows)
    }

    pub fn revoke_project_share_link(&self, id: i64) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let affected = conn
            .execute(
                "UPDATE project_share_links SET revoked = 1 WHERE id = ?1",
                params![id],
            )
            .context("revoke_project_share_link")?;
        Ok(affected > 0)
    }

    // ── Full-text search ──────────────────────────────────────────────────

    pub fn fts_index_document(
        &self,
        project_id: i64,
        task_id: i64,
        file_path: &str,
        title: &str,
        content: &str,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        // Delete existing entry for this task+file, then re-insert
        conn.execute(
            "DELETE FROM legal_fts WHERE task_id = ?1 AND file_path = ?2",
            params![task_id, file_path],
        )?;
        conn.execute(
            "INSERT INTO legal_fts (project_id, task_id, file_path, title, content) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project_id, task_id, file_path, title, content],
        ).context("fts_index_document")?;
        Ok(())
    }

    pub fn fts_remove_task(&self, task_id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute("DELETE FROM legal_fts WHERE task_id = ?1", params![task_id])?;
        Ok(())
    }

    pub fn fts_search(
        &self,
        query: &str,
        project_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<FtsResult>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = if project_id.is_some() {
            "SELECT project_id, task_id, file_path, \
                    left(title, 240) as title_snip, \
                    left(content, 640) as content_snip, \
                    ts_rank_cd(search_vector, websearch_to_tsquery('english', ?1)) as rank \
             FROM legal_fts \
             WHERE search_vector @@ websearch_to_tsquery('english', ?1) AND project_id = ?2 \
             ORDER BY rank DESC, task_id DESC LIMIT ?3"
        } else {
            "SELECT project_id, task_id, file_path, \
                    left(title, 240) as title_snip, \
                    left(content, 640) as content_snip, \
                    ts_rank_cd(search_vector, websearch_to_tsquery('english', ?1)) as rank \
             FROM legal_fts \
             WHERE search_vector @@ websearch_to_tsquery('english', ?1) \
             ORDER BY rank DESC, task_id DESC LIMIT ?2"
        };
        let mut stmt = conn.prepare(sql)?;
        let results = if let Some(pid) = project_id {
            stmt.query_map(params![query, pid, limit], |r| {
                Ok(FtsResult {
                    project_id: r.get(0)?,
                    task_id: r.get(1)?,
                    file_path: r.get(2)?,
                    title_snippet: r.get(3)?,
                    content_snippet: r.get(4)?,
                    rank: r.get(5)?,
                })
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("fts_search")?
        } else {
            stmt.query_map(params![query, limit], |r| {
                Ok(FtsResult {
                    project_id: r.get(0)?,
                    task_id: r.get(1)?,
                    file_path: r.get(2)?,
                    title_snippet: r.get(3)?,
                    content_snippet: r.get(4)?,
                    rank: r.get(5)?,
                })
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("fts_search")?
        };
        Ok(results)
    }

    pub fn list_project_tasks(&self, project_id: i64) -> Result<Vec<Task>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = format!(
            "SELECT {TASK_COLS} FROM pipeline_tasks WHERE project_id = ?1 ORDER BY id DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let tasks = stmt
            .query_map(params![project_id], row_to_task)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_project_tasks")?;
        Ok(tasks)
    }

    pub fn list_project_files(&self, project_id: i64) -> Result<Vec<ProjectFileRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {PROJECT_FILE_COLS} FROM project_files WHERE project_id=?1 ORDER BY id ASC"
        ))?;
        let files = stmt
            .query_map(params![project_id], row_to_project_file)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_project_files")?;
        Ok(files)
    }

    pub fn list_project_file_page(
        &self,
        project_id: i64,
        query: Option<&str>,
        limit: i64,
        offset: i64,
        cursor: Option<&ProjectFilePageCursor>,
        has_text: Option<bool>,
        privileged_only: Option<bool>,
    ) -> Result<(Vec<ProjectFileMetaRow>, i64)> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let trimmed_query = query.map(str::trim).filter(|q| !q.is_empty());
        let mut base_where = vec!["project_id = ?".to_string()];
        let mut base_params: Vec<Box<dyn pg::types::ToSql>> = vec![Box::new(project_id)];

        if let Some(q) = trimmed_query {
            base_where.push("(lower(file_name) LIKE ? OR lower(source_path) LIKE ?)".to_string());
            let like = format!("%{}%", q.to_lowercase());
            base_params.push(Box::new(like.clone()));
            base_params.push(Box::new(like));
        }
        if let Some(flag) = has_text {
            base_where.push(if flag {
                "extracted_text != ''".to_string()
            } else {
                "extracted_text = ''".to_string()
            });
        }
        if let Some(flag) = privileged_only {
            base_where.push("privileged = ?".to_string());
            base_params.push(Box::new(if flag { 1_i64 } else { 0_i64 }));
        }

        let base_where_sql = base_where.join(" AND ");
        let fast_total = if trimmed_query.is_none() {
            conn.query_row(
                "SELECT total_files, privileged_files, text_files FROM project_corpus_stats WHERE project_id = ?1",
                params![project_id],
                |row| Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                )),
            )
            .ok()
            .map(|(total_files, privileged_files, text_files)| match (has_text, privileged_only) {
                (None, None) => total_files,
                (Some(true), None) => text_files,
                (Some(false), None) => (total_files - text_files).max(0),
                (None, Some(true)) => privileged_files,
                (None, Some(false)) => (total_files - privileged_files).max(0),
                _ => -1,
            })
            .filter(|n| *n >= 0)
        } else {
            None
        };
        let total: i64 = if let Some(total) = fast_total {
            total
        } else {
            let total_sql = format!("SELECT COUNT(*) FROM project_files WHERE {base_where_sql}");
            let total_params: Vec<&dyn pg::types::ToSql> =
                base_params.iter().map(|p| p.as_ref()).collect();
            conn.query_row(&total_sql, total_params.as_slice(), |row| row.get(0))
                .context("list_project_file_page count")?
        };

        let lim = limit.clamp(1, 200);
        let off = offset.max(0);
        let mut page_where = base_where;
        let mut page_params: Vec<Box<dyn pg::types::ToSql>> = base_params;
        if let Some(cursor) = cursor {
            page_where.push("(created_at < ? OR (created_at = ? AND id < ?))".to_string());
            page_params.push(Box::new(cursor.created_at.clone()));
            page_params.push(Box::new(cursor.created_at.clone()));
            page_params.push(Box::new(cursor.id));
        }
        page_params.push(Box::new(lim));
        if cursor.is_none() {
            page_params.push(Box::new(off));
        }
        let page_refs: Vec<&dyn pg::types::ToSql> =
            page_params.iter().map(|p| p.as_ref()).collect();
        let page_where_sql = page_where.join(" AND ");
        let sql = if cursor.is_some() {
            format!(
                "SELECT {PROJECT_FILE_META_COLS} FROM project_files \
                 WHERE {page_where_sql} ORDER BY created_at DESC, id DESC LIMIT ?"
            )
        } else {
            format!(
                "SELECT {PROJECT_FILE_META_COLS} FROM project_files \
                 WHERE {page_where_sql} ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
            )
        };
        let mut stmt = conn
            .prepare(&sql)
            .context("list_project_file_page prepare")?;
        let items = stmt
            .query_map(page_refs.as_slice(), row_to_project_file_meta)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_project_file_page rows")?;
        Ok((items, total))
    }

    pub fn search_project_file_name_hits(
        &self,
        project_id: i64,
        query: &str,
        limit: i64,
    ) -> Result<Vec<ProjectFileMetaRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let lim = limit.clamp(1, 50);
        let like = format!("%{q}%");
        let sql = format!(
            "SELECT {PROJECT_FILE_META_COLS} FROM project_files \
             WHERE project_id = ?1 AND (lower(file_name) LIKE ?2 OR lower(source_path) LIKE ?2) \
             ORDER BY created_at DESC, id DESC LIMIT ?3"
        );
        let mut stmt = conn
            .prepare(&sql)
            .context("search_project_file_name_hits prepare")?;
        let rows = stmt
            .query_map(params![project_id, like, lim], row_to_project_file_meta)?
            .collect::<pg::Result<Vec<_>>>()
            .context("search_project_file_name_hits rows")?;
        Ok(rows)
    }

    pub fn get_project_file(
        &self,
        project_id: i64,
        file_id: i64,
    ) -> Result<Option<ProjectFileRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.query_row(
            &format!("SELECT {PROJECT_FILE_COLS} FROM project_files WHERE id=?1 AND project_id=?2"),
            params![file_id, project_id],
            row_to_project_file,
        )
        .optional()
        .context("get_project_file")
    }

    pub fn delete_project_file(&self, project_id: i64, file_id: i64) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let n = conn
            .execute(
                "DELETE FROM project_files WHERE id = ?1 AND project_id = ?2",
                params![file_id, project_id],
            )
            .context("delete_project_file")?;
        Ok(n > 0)
    }

    pub fn delete_all_project_files(&self, project_id: i64) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let tx = conn
            .transaction()
            .context("delete_all_project_files transaction")?;
        tx.execute(
            "DELETE FROM embeddings WHERE project_id=?1",
            params![project_id],
        )
        .context("delete embeddings for project files")?;
        tx.execute(
            "DELETE FROM legal_fts WHERE project_id=?1",
            params![project_id],
        )
        .context("delete legal_fts for project files")?;
        let deleted = tx
            .execute(
                "DELETE FROM project_files WHERE project_id=?1",
                params![project_id],
            )
            .context("delete project files")?;
        tx.execute(
            "DELETE FROM project_corpus_stats WHERE project_id=?1",
            params![project_id],
        )
        .context("delete project corpus stats")?;
        tx.commit().context("delete_all_project_files commit")?;
        Ok(deleted as i64)
    }

    pub fn find_latest_project_file_by_source_path(
        &self,
        project_id: i64,
        source_path: &str,
    ) -> Result<Option<ProjectFileRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.query_row(
            &format!(
                "SELECT {PROJECT_FILE_COLS} FROM project_files \
                 WHERE project_id=?1 AND source_path=?2 ORDER BY id DESC LIMIT 1"
            ),
            params![project_id, source_path],
            row_to_project_file,
        )
        .optional()
        .context("find_latest_project_file_by_source_path")
    }

    pub fn insert_project_file(
        &self,
        project_id: i64,
        file_name: &str,
        source_path: &str,
        stored_path: &str,
        mime_type: &str,
        size_bytes: i64,
        content_hash: &str,
        privileged: bool,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let created_at = now_str();
        let id = conn.execute_returning_id(
            "INSERT INTO project_files \
             (project_id, file_name, source_path, stored_path, mime_type, size_bytes, content_hash, created_at, privileged) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                project_id,
                file_name,
                source_path,
                stored_path,
                mime_type,
                size_bytes,
                content_hash,
                created_at,
                if privileged { 1i64 } else { 0i64 }
            ],
        )
        .context("insert_project_file")?;
        conn.execute(
            "INSERT INTO project_corpus_stats \
             (project_id, total_files, total_bytes, privileged_files, text_files, text_chars, updated_at) \
             VALUES (?1, 1, ?2, ?3, 0, 0, ?4) \
             ON CONFLICT(project_id) DO UPDATE SET
               total_files = project_corpus_stats.total_files + 1,
               total_bytes = project_corpus_stats.total_bytes + excluded.total_bytes,
               privileged_files = project_corpus_stats.privileged_files + excluded.privileged_files,
               updated_at = excluded.updated_at",
            params![
                project_id,
                size_bytes,
                if privileged { 1_i64 } else { 0_i64 },
                created_at,
            ],
        )
        .context("insert_project_file stats")?;
        if privileged {
            conn.execute(
                "UPDATE projects SET session_privileged = 1 WHERE id = ?1",
                params![project_id],
            )?;
        }
        Ok(id)
    }

    pub fn is_session_privileged(&self, project_id: i64) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let priv_int: i64 = conn
            .query_row(
                "SELECT session_privileged FROM projects WHERE id = ?1",
                params![project_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(priv_int != 0)
    }

    pub fn set_session_privileged(&self, project_id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE projects SET session_privileged = 1 WHERE id = ?1",
            params![project_id],
        )
        .context("set_session_privileged")?;
        Ok(())
    }

    pub fn find_project_file_by_hash(
        &self,
        project_id: i64,
        content_hash: &str,
    ) -> Result<Option<ProjectFileRow>> {
        if content_hash.trim().is_empty() {
            return Ok(None);
        }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            &format!("SELECT {PROJECT_FILE_COLS} FROM project_files WHERE project_id=?1 AND content_hash=?2 ORDER BY id ASC LIMIT 1"),
            params![project_id, content_hash],
            row_to_project_file,
        )
        .optional()
        .context("find_project_file_by_hash")
    }

    pub fn update_project_file_text(&self, file_id: i64, text: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let (project_id, old_chars): (i64, i64) = conn.query_row(
            "SELECT project_id, COALESCE(length(extracted_text), 0)::bigint FROM project_files WHERE id = ?1",
            params![file_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        conn.execute(
            "UPDATE project_files SET extracted_text = ?1 WHERE id = ?2",
            params![text, file_id],
        )?;
        let new_chars = text.chars().count() as i64;
        conn.execute(
            "INSERT INTO project_corpus_stats \
             (project_id, total_files, total_bytes, privileged_files, text_files, text_chars, updated_at) \
             VALUES (?1, 0, 0, 0, ?2, ?3, ?4) \
             ON CONFLICT(project_id) DO UPDATE SET
               text_files = project_corpus_stats.text_files + excluded.text_files,
               text_chars = project_corpus_stats.text_chars + excluded.text_chars,
               updated_at = excluded.updated_at",
            params![
                project_id,
                if old_chars == 0 && new_chars > 0 { 1_i64 } else { 0_i64 },
                new_chars - old_chars,
                now_str(),
            ],
        )
        .context("update_project_file_text stats")?;
        Ok(())
    }

    pub fn total_project_file_bytes(&self, project_id: i64) -> Result<i64> {
        Ok(self.get_project_file_stats(project_id)?.total_bytes)
    }

    pub fn get_project_file_stats(&self, project_id: i64) -> Result<ProjectFileStats> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let stats = conn
            .query_row(
                "SELECT project_id, total_files, total_bytes, privileged_files, text_files, text_chars, updated_at \
                 FROM project_corpus_stats WHERE project_id=?1",
                params![project_id],
                |row| {
                    Ok(ProjectFileStats {
                        project_id: row.get(0)?,
                        total_files: row.get(1)?,
                        total_bytes: row.get(2)?,
                        privileged_files: row.get(3)?,
                        text_files: row.get(4)?,
                        text_chars: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                },
            )
            .optional()
            .context("get_project_file_stats")?;
        Ok(stats.unwrap_or(ProjectFileStats {
            project_id,
            ..ProjectFileStats::default()
        }))
    }

    pub fn create_upload_session(
        &self,
        project_id: i64,
        file_name: &str,
        mime_type: &str,
        file_size: i64,
        chunk_size: i64,
        total_chunks: i64,
        is_zip: bool,
        privileged: bool,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_str();
        let id = conn.execute_returning_id(
            "INSERT INTO upload_sessions \
             (project_id, file_name, mime_type, file_size, chunk_size, total_chunks, uploaded_bytes, is_zip, privileged, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, 'uploading', ?9, ?9)",
            params![
                project_id,
                file_name,
                mime_type,
                file_size,
                chunk_size,
                total_chunks,
                if is_zip { 1i64 } else { 0i64 },
                if privileged { 1i64 } else { 0i64 },
                now
            ],
        )
        .context("create_upload_session")?;
        Ok(id)
    }

    pub fn get_upload_session(&self, session_id: i64) -> Result<Option<UploadSession>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT id, project_id, file_name, mime_type, file_size, chunk_size, total_chunks, \
                    uploaded_bytes, is_zip, privileged, status, stored_path, error, created_at, updated_at \
             FROM upload_sessions WHERE id = ?1",
            params![session_id],
            row_to_upload_session,
        )
        .optional()
        .context("get_upload_session")
    }

    pub fn list_upload_sessions(
        &self,
        project_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<UploadSession>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let lim = limit.clamp(1, 500);
        let sql = if project_id.is_some() {
            "SELECT id, project_id, file_name, mime_type, file_size, chunk_size, total_chunks, uploaded_bytes, \
                    is_zip, privileged, status, stored_path, error, created_at, updated_at \
             FROM upload_sessions WHERE project_id=?1 ORDER BY id DESC LIMIT ?2"
        } else {
            "SELECT id, project_id, file_name, mime_type, file_size, chunk_size, total_chunks, uploaded_bytes, \
                    is_zip, privileged, status, stored_path, error, created_at, updated_at \
             FROM upload_sessions ORDER BY id DESC LIMIT ?1"
        };
        let mut stmt = conn.prepare(sql).context("list_upload_sessions prepare")?;
        let out = if let Some(pid) = project_id {
            stmt.query_map(params![pid, lim], row_to_upload_session)?
                .collect::<pg::Result<Vec<_>>>()
                .context("list_upload_sessions map")?
        } else {
            stmt.query_map(params![lim], row_to_upload_session)?
                .collect::<pg::Result<Vec<_>>>()
                .context("list_upload_sessions map")?
        };
        Ok(out)
    }

    pub fn count_upload_sessions_by_status(
        &self,
        project_id: Option<i64>,
    ) -> Result<HashMap<String, i64>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let sql = if project_id.is_some() {
            "SELECT status, COUNT(*) FROM upload_sessions WHERE project_id=?1 GROUP BY status"
        } else {
            "SELECT status, COUNT(*) FROM upload_sessions GROUP BY status"
        };
        let mut stmt = conn
            .prepare(sql)
            .context("count_upload_sessions_by_status prepare")?;
        let mut out = HashMap::new();
        if let Some(pid) = project_id {
            let rows = stmt.query_map(params![pid], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (status, count) = row?;
                out.insert(status, count);
            }
        } else {
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (status, count) = row?;
                out.insert(status, count);
            }
        }
        Ok(out)
    }

    pub fn count_active_upload_sessions(&self, project_id: i64) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let count = conn
            .query_row(
                "SELECT COUNT(*) FROM upload_sessions \
                 WHERE project_id=?1 AND status IN ('uploading','processing')",
                params![project_id],
                |row| row.get::<_, i64>(0),
            )
            .context("count_active_upload_sessions")?;
        Ok(count)
    }

    pub fn list_uploaded_chunks(&self, session_id: i64) -> Result<Vec<i64>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT chunk_index FROM upload_session_chunks WHERE session_id=?1 ORDER BY chunk_index ASC",
        )?;
        let rows = stmt
            .query_map(params![session_id], |row| row.get::<_, i64>(0))?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_uploaded_chunks")?;
        Ok(rows)
    }

    pub fn upsert_upload_chunk(
        &self,
        session_id: i64,
        chunk_index: i64,
        size_bytes: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_str();
        conn.execute(
            "INSERT INTO upload_session_chunks (session_id, chunk_index, size_bytes, created_at) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(session_id, chunk_index) DO UPDATE SET size_bytes=excluded.size_bytes",
            params![session_id, chunk_index, size_bytes, now],
        )
        .context("upsert_upload_chunk")?;
        conn.execute(
            "UPDATE upload_sessions \
             SET uploaded_bytes = (SELECT COALESCE(SUM(size_bytes), 0) FROM upload_session_chunks WHERE session_id = ?1), \
                 updated_at = ?2 \
             WHERE id = ?1",
            params![session_id, now],
        )
        .context("upsert_upload_chunk aggregate")?;
        Ok(())
    }

    pub fn set_upload_session_state(
        &self,
        session_id: i64,
        status: &str,
        stored_path: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE upload_sessions \
             SET status = ?1, \
                 stored_path = COALESCE(?2, stored_path), \
                 error = COALESCE(?3, error), \
                 updated_at = ?4 \
             WHERE id = ?5",
            params![status, stored_path, error, now_str(), session_id],
        )
        .context("set_upload_session_state")?;
        Ok(())
    }

    pub fn summarize_themes(
        &self,
        project_id: Option<i64>,
        limit: i64,
        min_document_count: i64,
    ) -> Result<ThemeSummary> {
        const MAX_THEME_DOCUMENTS: i64 = 5_000;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut keyword_counts: HashMap<String, i64> = HashMap::new();
        let mut keyword_docs: HashMap<String, i64> = HashMap::new();
        let mut phrase_counts: HashMap<String, i64> = HashMap::new();
        let mut phrase_docs: HashMap<String, i64> = HashMap::new();
        let mut documents_scanned = 0i64;
        let mut tokens_scanned = 0i64;

        let sql = if project_id.is_some() {
            "SELECT extracted_text FROM project_files \
             WHERE project_id = ?1 AND extracted_text != '' \
             ORDER BY created_at DESC, id DESC LIMIT ?2"
        } else {
            "SELECT extracted_text FROM project_files \
             WHERE extracted_text != '' \
             ORDER BY created_at DESC, id DESC LIMIT ?1"
        };
        let mut stmt = conn.prepare(sql).context("summarize_themes prepare")?;
        let mut consume_text = |text: String| {
            documents_scanned += 1;
            let tokens = tokenize_for_themes(&text);
            tokens_scanned += tokens.len() as i64;
            if tokens.is_empty() {
                return;
            }
            let mut seen_keywords: HashSet<String> = HashSet::new();
            let mut seen_phrases: HashSet<String> = HashSet::new();
            for token in &tokens {
                *keyword_counts.entry(token.clone()).or_insert(0) += 1;
                seen_keywords.insert(token.clone());
            }
            for term in seen_keywords {
                *keyword_docs.entry(term).or_insert(0) += 1;
            }
            for pair in tokens.windows(2) {
                let phrase = format!("{} {}", pair[0], pair[1]);
                *phrase_counts.entry(phrase.clone()).or_insert(0) += 1;
                seen_phrases.insert(phrase);
            }
            for phrase in seen_phrases {
                *phrase_docs.entry(phrase).or_insert(0) += 1;
            }
        };
        if let Some(pid) = project_id {
            let rows =
                stmt.query_map(params![pid, MAX_THEME_DOCUMENTS], |r| r.get::<_, String>(0))?;
            for row in rows {
                consume_text(row?);
            }
        } else {
            let rows = stmt.query_map(params![MAX_THEME_DOCUMENTS], |r| r.get::<_, String>(0))?;
            for row in rows {
                consume_text(row?);
            }
        }

        let min_doc = min_document_count.max(1);
        let mut keywords = Vec::new();
        for (term, occurrences) in keyword_counts {
            let doc_count = keyword_docs.get(&term).copied().unwrap_or(0);
            if doc_count >= min_doc {
                push_theme_term(&mut keywords, term, occurrences, doc_count);
            }
        }
        keywords.sort_by(|a, b| {
            b.document_count
                .cmp(&a.document_count)
                .then_with(|| b.occurrences.cmp(&a.occurrences))
                .then_with(|| a.term.cmp(&b.term))
        });
        keywords.truncate(limit.max(1) as usize);

        let mut phrases = Vec::new();
        for (term, occurrences) in phrase_counts {
            let doc_count = phrase_docs.get(&term).copied().unwrap_or(0);
            if doc_count >= min_doc {
                push_theme_term(&mut phrases, term, occurrences, doc_count);
            }
        }
        phrases.sort_by(|a, b| {
            b.document_count
                .cmp(&a.document_count)
                .then_with(|| b.occurrences.cmp(&a.occurrences))
                .then_with(|| a.term.cmp(&b.term))
        });
        phrases.truncate(limit.max(1) as usize);

        Ok(ThemeSummary {
            documents_scanned,
            tokens_scanned,
            keywords,
            phrases,
        })
    }

    pub fn summarize_themes_for_workspace(
        &self,
        workspace_id: i64,
        limit: i64,
        min_document_count: i64,
    ) -> Result<ThemeSummary> {
        const MAX_THEME_DOCUMENTS: i64 = 5_000;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut keyword_counts: HashMap<String, i64> = HashMap::new();
        let mut keyword_docs: HashMap<String, i64> = HashMap::new();
        let mut phrase_counts: HashMap<String, i64> = HashMap::new();
        let mut phrase_docs: HashMap<String, i64> = HashMap::new();
        let mut documents_scanned = 0i64;
        let mut tokens_scanned = 0i64;

        let mut stmt = conn
            .prepare(
                "SELECT pf.extracted_text FROM project_files pf \
                 JOIN projects p ON p.id = pf.project_id \
                 WHERE p.workspace_id = ?1 AND pf.extracted_text != '' \
                 ORDER BY pf.created_at DESC, pf.id DESC LIMIT ?2",
            )
            .context("summarize_themes_for_workspace prepare")?;
        let rows = stmt.query_map(params![workspace_id, MAX_THEME_DOCUMENTS], |r| {
            r.get::<_, String>(0)
        })?;
        let mut consume_text = |text: String| {
            documents_scanned += 1;
            let tokens = tokenize_for_themes(&text);
            tokens_scanned += tokens.len() as i64;
            if tokens.is_empty() {
                return;
            }
            let mut seen_keywords: HashSet<String> = HashSet::new();
            let mut seen_phrases: HashSet<String> = HashSet::new();
            for token in &tokens {
                *keyword_counts.entry(token.clone()).or_insert(0) += 1;
                seen_keywords.insert(token.clone());
            }
            for term in seen_keywords {
                *keyword_docs.entry(term).or_insert(0) += 1;
            }
            for pair in tokens.windows(2) {
                let phrase = format!("{} {}", pair[0], pair[1]);
                *phrase_counts.entry(phrase.clone()).or_insert(0) += 1;
                seen_phrases.insert(phrase);
            }
            for phrase in seen_phrases {
                *phrase_docs.entry(phrase).or_insert(0) += 1;
            }
        };
        for row in rows {
            consume_text(row?);
        }

        let min_doc = min_document_count.max(1);
        let mut keywords = Vec::new();
        for (term, occurrences) in keyword_counts {
            let doc_count = keyword_docs.get(&term).copied().unwrap_or(0);
            if doc_count >= min_doc {
                push_theme_term(&mut keywords, term, occurrences, doc_count);
            }
        }
        keywords.sort_by(|a, b| {
            b.document_count
                .cmp(&a.document_count)
                .then_with(|| b.occurrences.cmp(&a.occurrences))
                .then_with(|| a.term.cmp(&b.term))
        });
        keywords.truncate(limit.max(1) as usize);

        let mut phrases = Vec::new();
        for (term, occurrences) in phrase_counts {
            let doc_count = phrase_docs.get(&term).copied().unwrap_or(0);
            if doc_count >= min_doc {
                push_theme_term(&mut phrases, term, occurrences, doc_count);
            }
        }
        phrases.sort_by(|a, b| {
            b.document_count
                .cmp(&a.document_count)
                .then_with(|| b.occurrences.cmp(&a.occurrences))
                .then_with(|| a.term.cmp(&b.term))
        });
        phrases.truncate(limit.max(1) as usize);

        Ok(ThemeSummary {
            documents_scanned,
            tokens_scanned,
            keywords,
            phrases,
        })
    }

    // ── Cloud connections ─────────────────────────────────────────────────

    pub fn insert_cloud_connection(
        &self,
        project_id: i64,
        provider: &str,
        access_token: &str,
        refresh_token: &str,
        token_expiry: &str,
        account_email: &str,
        account_id: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let id = conn.execute_returning_id(
            "INSERT INTO cloud_connections \
             (project_id, provider, access_token, refresh_token, token_expiry, account_email, account_id, created_at) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![project_id, provider, access_token, refresh_token, token_expiry,
                    account_email, account_id, now_str()],
        ).context("insert_cloud_connection")?;
        Ok(id)
    }

    pub fn list_cloud_connections(&self, project_id: i64) -> Result<Vec<CloudConnection>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, project_id, provider, access_token, refresh_token, token_expiry, \
                    account_email, account_id, created_at \
             FROM cloud_connections WHERE project_id=?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![project_id], row_to_cloud_connection)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn get_cloud_connection(&self, id: i64) -> Result<Option<CloudConnection>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT id, project_id, provider, access_token, refresh_token, token_expiry, \
                    account_email, account_id, created_at \
             FROM cloud_connections WHERE id=?1",
            params![id],
            row_to_cloud_connection,
        )
        .optional()
        .context("get_cloud_connection")
    }

    pub fn update_cloud_connection_tokens(
        &self,
        id: i64,
        access_token: &str,
        refresh_token: &str,
        token_expiry: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE cloud_connections SET access_token=?1, refresh_token=?2, token_expiry=?3 WHERE id=?4",
            params![access_token, refresh_token, token_expiry, id],
        ).context("update_cloud_connection_tokens")?;
        Ok(())
    }

    pub fn delete_cloud_connection(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("DELETE FROM cloud_connections WHERE id=?1", params![id])
            .context("delete_cloud_connection")?;
        Ok(())
    }

    // ── Knowledge files ───────────────────────────────────────────────────

    pub fn total_knowledge_file_bytes(&self) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let total = conn
            .query_row(
                "SELECT COALESCE(SUM(size_bytes), 0)::bigint FROM knowledge_files",
                [],
                |r| r.get(0),
            )
            .context("total_knowledge_file_bytes")?;
        Ok(total)
    }

    pub fn total_knowledge_file_bytes_in_workspace(&self, workspace_id: i64) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let total = conn
            .query_row(
                "SELECT COALESCE(SUM(size_bytes), 0)::bigint FROM knowledge_files WHERE workspace_id = ?1",
                params![workspace_id],
                |r| r.get(0),
            )
            .context("total_knowledge_file_bytes_in_workspace")?;
        Ok(total)
    }

    pub fn list_knowledge_files(&self) -> Result<Vec<KnowledgeFile>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, file_name, description, size_bytes, \"inline\", created_at, \
                    tags, category, jurisdiction, project_id, user_id \
             FROM knowledge_files ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], row_to_knowledge)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn list_knowledge_files_in_workspace(
        &self,
        workspace_id: i64,
    ) -> Result<Vec<KnowledgeFile>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, file_name, description, size_bytes, \"inline\", created_at, \
                    tags, category, jurisdiction, project_id, user_id \
             FROM knowledge_files WHERE workspace_id = ?1 AND user_id IS NULL ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![workspace_id], row_to_knowledge)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn list_knowledge_file_page(
        &self,
        query: Option<&str>,
        category: Option<&str>,
        jurisdiction: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<KnowledgeFile>, i64)> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut where_clauses = vec!["1=1".to_string()];
        let mut params_vec: Vec<Box<dyn pg::types::ToSql>> = Vec::new();

        if let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) {
            where_clauses.push(
                "(lower(file_name) LIKE ? OR lower(description) LIKE ? OR lower(tags) LIKE ?)"
                    .to_string(),
            );
            let pattern = format!("%{}%", q.to_ascii_lowercase());
            params_vec.push(Box::new(pattern.clone()));
            params_vec.push(Box::new(pattern.clone()));
            params_vec.push(Box::new(pattern));
        }
        if let Some(cat) = category.map(str::trim).filter(|c| !c.is_empty()) {
            where_clauses.push("category = ?".to_string());
            params_vec.push(Box::new(cat.to_string()));
        }
        if let Some(jur) = jurisdiction.map(str::trim).filter(|j| !j.is_empty()) {
            where_clauses.push("(jurisdiction = ? OR jurisdiction = '')".to_string());
            params_vec.push(Box::new(jur.to_string()));
        }

        let where_sql = where_clauses.join(" AND ");
        let total_sql = format!("SELECT COUNT(*) FROM knowledge_files WHERE {where_sql}");
        let total_params: Vec<&dyn pg::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let total: i64 = conn
            .query_row(&total_sql, total_params.as_slice(), |row| row.get(0))
            .context("list_knowledge_file_page count")?;

        let lim = limit.clamp(1, 200);
        let off = offset.max(0);
        let mut page_params: Vec<Box<dyn pg::types::ToSql>> = params_vec;
        page_params.push(Box::new(lim));
        page_params.push(Box::new(off));
        let page_refs: Vec<&dyn pg::types::ToSql> =
            page_params.iter().map(|p| p.as_ref()).collect();
        let sql = format!(
            "SELECT id, workspace_id, file_name, description, size_bytes, \"inline\", created_at, \
                    tags, category, jurisdiction, project_id, user_id \
             FROM knowledge_files WHERE {where_sql} \
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        );
        let mut stmt = conn
            .prepare(&sql)
            .context("list_knowledge_file_page prepare")?;
        let items = stmt
            .query_map(page_refs.as_slice(), row_to_knowledge)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_knowledge_file_page rows")?;
        Ok((items, total))
    }

    pub fn list_knowledge_file_page_in_workspace(
        &self,
        workspace_id: i64,
        query: Option<&str>,
        category: Option<&str>,
        jurisdiction: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<KnowledgeFile>, i64)> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut where_clauses = vec![
            "workspace_id = ?".to_string(),
            "user_id IS NULL".to_string(),
        ];
        let mut params_vec: Vec<Box<dyn pg::types::ToSql>> = vec![Box::new(workspace_id)];

        if let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) {
            where_clauses.push(
                "(lower(file_name) LIKE ? OR lower(description) LIKE ? OR lower(tags) LIKE ?)"
                    .to_string(),
            );
            let pattern = format!("%{}%", q.to_ascii_lowercase());
            params_vec.push(Box::new(pattern.clone()));
            params_vec.push(Box::new(pattern.clone()));
            params_vec.push(Box::new(pattern));
        }
        if let Some(cat) = category.map(str::trim).filter(|c| !c.is_empty()) {
            where_clauses.push("category = ?".to_string());
            params_vec.push(Box::new(cat.to_string()));
        }
        if let Some(jur) = jurisdiction.map(str::trim).filter(|j| !j.is_empty()) {
            where_clauses.push("(jurisdiction = ? OR jurisdiction = '')".to_string());
            params_vec.push(Box::new(jur.to_string()));
        }

        let where_sql = where_clauses.join(" AND ");
        let total_sql = format!("SELECT COUNT(*) FROM knowledge_files WHERE {where_sql}");
        let total_params: Vec<&dyn pg::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let total: i64 = conn
            .query_row(&total_sql, total_params.as_slice(), |row| row.get(0))
            .context("list_knowledge_file_page_in_workspace count")?;

        let lim = limit.clamp(1, 200);
        let off = offset.max(0);
        let mut page_params: Vec<Box<dyn pg::types::ToSql>> = params_vec;
        page_params.push(Box::new(lim));
        page_params.push(Box::new(off));
        let page_refs: Vec<&dyn pg::types::ToSql> =
            page_params.iter().map(|p| p.as_ref()).collect();
        let sql = format!(
            "SELECT id, workspace_id, file_name, description, size_bytes, \"inline\", created_at, \
                    tags, category, jurisdiction, project_id, user_id \
             FROM knowledge_files WHERE {where_sql} \
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        );
        let mut stmt = conn
            .prepare(&sql)
            .context("list_knowledge_file_page_in_workspace prepare")?;
        let items = stmt
            .query_map(page_refs.as_slice(), row_to_knowledge)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_knowledge_file_page_in_workspace rows")?;
        Ok((items, total))
    }

    /// Like list_knowledge_file_page_in_workspace but includes user-scoped files too.
    pub fn list_all_knowledge_in_workspace(
        &self,
        workspace_id: i64,
        query: Option<&str>,
        jurisdiction: Option<&str>,
        limit: i64,
    ) -> Result<Vec<KnowledgeFile>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut where_clauses = vec!["workspace_id = ?".to_string()];
        let mut params_vec: Vec<Box<dyn pg::types::ToSql>> = vec![Box::new(workspace_id)];
        if let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) {
            where_clauses.push(
                "(lower(file_name) LIKE ? OR lower(description) LIKE ? OR lower(tags) LIKE ?)"
                    .to_string(),
            );
            let pattern = format!("%{}%", q.to_ascii_lowercase());
            params_vec.push(Box::new(pattern.clone()));
            params_vec.push(Box::new(pattern.clone()));
            params_vec.push(Box::new(pattern));
        }
        if let Some(jur) = jurisdiction.map(str::trim).filter(|j| !j.is_empty()) {
            where_clauses.push("(jurisdiction = ? OR jurisdiction = '')".to_string());
            params_vec.push(Box::new(jur.to_string()));
        }
        let lim = limit.clamp(1, 200);
        params_vec.push(Box::new(lim));
        let where_sql = where_clauses.join(" AND ");
        let page_refs: Vec<&dyn pg::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let sql = format!(
            "SELECT id, workspace_id, file_name, description, size_bytes, \"inline\", created_at, \
                    tags, category, jurisdiction, project_id, user_id \
             FROM knowledge_files WHERE {where_sql} \
             ORDER BY created_at DESC, id DESC LIMIT ?"
        );
        let mut stmt = conn
            .prepare(&sql)
            .context("list_all_knowledge_in_workspace")?;
        let items = stmt
            .query_map(page_refs.as_slice(), row_to_knowledge)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_all_knowledge_in_workspace rows")?;
        Ok(items)
    }

    pub fn get_knowledge_file(&self, id: i64) -> Result<Option<KnowledgeFile>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.query_row(
            "SELECT id, workspace_id, file_name, description, size_bytes, \"inline\", created_at, \
                    tags, category, jurisdiction, project_id, user_id \
             FROM knowledge_files WHERE id=?1",
            params![id],
            row_to_knowledge,
        )
        .optional()
        .context("get_knowledge_file")
    }

    pub fn get_knowledge_file_in_workspace(
        &self,
        workspace_id: i64,
        id: i64,
    ) -> Result<Option<KnowledgeFile>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.query_row(
            "SELECT id, workspace_id, file_name, description, size_bytes, \"inline\", created_at, \
                    tags, category, jurisdiction, project_id, user_id \
             FROM knowledge_files WHERE id=?1 AND workspace_id = ?2",
            params![id, workspace_id],
            row_to_knowledge,
        )
        .optional()
        .context("get_knowledge_file_in_workspace")
    }

    pub fn list_templates(
        &self,
        category: Option<&str>,
        jurisdiction: Option<&str>,
    ) -> Result<Vec<KnowledgeFile>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut where_clauses = Vec::new();
        let mut params_vec: Vec<Box<dyn pg::types::ToSql>> = Vec::new();
        if let Some(category) = category.map(str::trim).filter(|c| !c.is_empty()) {
            where_clauses.push("category = ?".to_string());
            params_vec.push(Box::new(category.to_string()));
        }
        if let Some(jurisdiction) = jurisdiction.map(str::trim).filter(|j| !j.is_empty()) {
            where_clauses.push("(jurisdiction = ? OR jurisdiction = '')".to_string());
            params_vec.push(Box::new(jurisdiction.to_string()));
        }
        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };
        let sql = format!(
            "SELECT id, workspace_id, file_name, description, size_bytes, \"inline\", created_at, \
                    tags, category, jurisdiction, project_id, user_id \
             FROM knowledge_files{where_sql} \
             ORDER BY category, file_name"
        );
        let param_refs: Vec<&dyn pg::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), row_to_knowledge)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn list_templates_in_workspace(
        &self,
        workspace_id: i64,
        category: Option<&str>,
        jurisdiction: Option<&str>,
    ) -> Result<Vec<KnowledgeFile>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut where_clauses = vec!["workspace_id = ?".to_string()];
        let mut params_vec: Vec<Box<dyn pg::types::ToSql>> = vec![Box::new(workspace_id)];
        if let Some(category) = category.map(str::trim).filter(|c| !c.is_empty()) {
            where_clauses.push("category = ?".to_string());
            params_vec.push(Box::new(category.to_string()));
        }
        if let Some(jurisdiction) = jurisdiction.map(str::trim).filter(|j| !j.is_empty()) {
            where_clauses.push("(jurisdiction = ? OR jurisdiction = '')".to_string());
            params_vec.push(Box::new(jurisdiction.to_string()));
        }
        let where_sql = format!(" WHERE {}", where_clauses.join(" AND "));
        let sql = format!(
            "SELECT id, workspace_id, file_name, description, size_bytes, \"inline\", created_at, \
                    tags, category, jurisdiction, project_id, user_id \
             FROM knowledge_files{where_sql} \
             ORDER BY category, file_name"
        );
        let param_refs: Vec<&dyn pg::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), row_to_knowledge)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn insert_knowledge_file(
        &self,
        workspace_id: i64,
        file_name: &str,
        description: &str,
        size_bytes: i64,
        inline: bool,
    ) -> Result<i64> {
        self.insert_knowledge_file_for_user(
            workspace_id,
            None,
            file_name,
            description,
            size_bytes,
            inline,
        )
    }

    pub fn insert_knowledge_file_for_user(
        &self,
        workspace_id: i64,
        user_id: Option<i64>,
        file_name: &str,
        description: &str,
        size_bytes: i64,
        inline: bool,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let id = conn.execute_returning_id(
            "INSERT INTO knowledge_files (workspace_id, user_id, file_name, description, size_bytes, \"inline\") \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![workspace_id, user_id, file_name, description, size_bytes, inline as i64],
        )?;
        Ok(id)
    }

    pub fn delete_knowledge_file(&self, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute("DELETE FROM knowledge_files WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn delete_knowledge_file_in_workspace(&self, workspace_id: i64, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "DELETE FROM knowledge_files WHERE id=?1 AND workspace_id = ?2",
            params![id, workspace_id],
        )?;
        Ok(())
    }

    pub fn delete_all_knowledge_files(&self) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let deleted = conn
            .execute("DELETE FROM knowledge_files", [])
            .context("delete_all_knowledge_files")?;
        Ok(deleted as i64)
    }

    pub fn delete_all_knowledge_files_in_workspace(&self, workspace_id: i64) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let deleted = conn
            .execute(
                "DELETE FROM knowledge_files WHERE workspace_id = ?1 AND user_id IS NULL",
                params![workspace_id],
            )
            .context("delete_all_knowledge_files_in_workspace")?;
        Ok(deleted as i64)
    }

    // ── User-scoped knowledge ("My Knowledge") ──────────────────────────

    pub fn list_user_knowledge_page(
        &self,
        workspace_id: i64,
        user_id: i64,
        query: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<KnowledgeFile>, i64)> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut where_clauses = vec!["workspace_id = ?".to_string(), "user_id = ?".to_string()];
        let mut params_vec: Vec<Box<dyn pg::types::ToSql>> =
            vec![Box::new(workspace_id), Box::new(user_id)];
        if let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) {
            where_clauses.push(
                "(lower(file_name) LIKE ? OR lower(description) LIKE ? OR lower(tags) LIKE ?)"
                    .to_string(),
            );
            let pattern = format!("%{}%", q.to_ascii_lowercase());
            params_vec.push(Box::new(pattern.clone()));
            params_vec.push(Box::new(pattern.clone()));
            params_vec.push(Box::new(pattern));
        }
        let where_sql = where_clauses.join(" AND ");
        let total_sql = format!("SELECT COUNT(*) FROM knowledge_files WHERE {where_sql}");
        let total_params: Vec<&dyn pg::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let total: i64 = conn
            .query_row(&total_sql, total_params.as_slice(), |row| row.get(0))
            .context("list_user_knowledge_page count")?;
        let lim = limit.clamp(1, 200);
        let off = offset.max(0);
        let mut page_params: Vec<Box<dyn pg::types::ToSql>> = params_vec;
        page_params.push(Box::new(lim));
        page_params.push(Box::new(off));
        let page_refs: Vec<&dyn pg::types::ToSql> =
            page_params.iter().map(|p| p.as_ref()).collect();
        let sql = format!(
            "SELECT id, workspace_id, file_name, description, size_bytes, \"inline\", created_at, \
                    tags, category, jurisdiction, project_id, user_id \
             FROM knowledge_files WHERE {where_sql} \
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        );
        let mut stmt = conn
            .prepare(&sql)
            .context("list_user_knowledge_page prepare")?;
        let items = stmt
            .query_map(page_refs.as_slice(), row_to_knowledge)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_user_knowledge_page rows")?;
        Ok((items, total))
    }

    pub fn list_user_knowledge_files(
        &self,
        workspace_id: i64,
        user_id: i64,
    ) -> Result<Vec<KnowledgeFile>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, file_name, description, size_bytes, \"inline\", created_at, \
                    tags, category, jurisdiction, project_id, user_id \
             FROM knowledge_files WHERE workspace_id = ?1 AND user_id = ?2 ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![workspace_id, user_id], row_to_knowledge)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn get_user_knowledge_file(
        &self,
        workspace_id: i64,
        user_id: i64,
        id: i64,
    ) -> Result<Option<KnowledgeFile>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.query_row(
            "SELECT id, workspace_id, file_name, description, size_bytes, \"inline\", created_at, \
                    tags, category, jurisdiction, project_id, user_id \
             FROM knowledge_files WHERE id=?1 AND workspace_id = ?2 AND user_id = ?3",
            params![id, workspace_id, user_id],
            row_to_knowledge,
        )
        .optional()
        .context("get_user_knowledge_file")
    }

    pub fn delete_user_knowledge_file(
        &self,
        workspace_id: i64,
        user_id: i64,
        id: i64,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "DELETE FROM knowledge_files WHERE id=?1 AND workspace_id = ?2 AND user_id = ?3",
            params![id, workspace_id, user_id],
        )?;
        Ok(())
    }

    pub fn delete_all_user_knowledge_files(&self, workspace_id: i64, user_id: i64) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let deleted = conn
            .execute(
                "DELETE FROM knowledge_files WHERE workspace_id = ?1 AND user_id = ?2",
                params![workspace_id, user_id],
            )
            .context("delete_all_user_knowledge_files")?;
        Ok(deleted as i64)
    }

    pub fn total_user_knowledge_bytes(&self, workspace_id: i64, user_id: i64) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let total = conn
            .query_row(
                "SELECT COALESCE(SUM(size_bytes), 0)::bigint FROM knowledge_files WHERE workspace_id = ?1 AND user_id = ?2",
                params![workspace_id, user_id],
                |r| r.get(0),
            )
            .context("total_user_knowledge_bytes")?;
        Ok(total)
    }

    pub fn update_knowledge_file(
        &self,
        id: i64,
        description: Option<&str>,
        inline: Option<bool>,
        tags: Option<&str>,
        category: Option<&str>,
        jurisdiction: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        if let Some(d) = description {
            conn.execute(
                "UPDATE knowledge_files SET description=?1 WHERE id=?2",
                params![d, id],
            )?;
        }
        if let Some(i) = inline {
            conn.execute(
                "UPDATE knowledge_files SET \"inline\"=?1 WHERE id=?2",
                params![i as i64, id],
            )?;
        }
        if let Some(t) = tags {
            conn.execute(
                "UPDATE knowledge_files SET tags=?1 WHERE id=?2",
                params![t, id],
            )?;
        }
        if let Some(c) = category {
            conn.execute(
                "UPDATE knowledge_files SET category=?1 WHERE id=?2",
                params![c, id],
            )?;
        }
        if let Some(j) = jurisdiction {
            conn.execute(
                "UPDATE knowledge_files SET jurisdiction=?1 WHERE id=?2",
                params![j, id],
            )?;
        }
        Ok(())
    }

    pub fn update_knowledge_file_in_workspace(
        &self,
        workspace_id: i64,
        id: i64,
        description: Option<&str>,
        inline: Option<bool>,
        tags: Option<&str>,
        category: Option<&str>,
        jurisdiction: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        if let Some(d) = description {
            conn.execute(
                "UPDATE knowledge_files SET description=?1 WHERE id=?2 AND workspace_id = ?3",
                params![d, id, workspace_id],
            )?;
        }
        if let Some(i) = inline {
            conn.execute(
                "UPDATE knowledge_files SET \"inline\"=?1 WHERE id=?2 AND workspace_id = ?3",
                params![i as i64, id, workspace_id],
            )?;
        }
        if let Some(t) = tags {
            conn.execute(
                "UPDATE knowledge_files SET tags=?1 WHERE id=?2 AND workspace_id = ?3",
                params![t, id, workspace_id],
            )?;
        }
        if let Some(c) = category {
            conn.execute(
                "UPDATE knowledge_files SET category=?1 WHERE id=?2 AND workspace_id = ?3",
                params![c, id, workspace_id],
            )?;
        }
        if let Some(j) = jurisdiction {
            conn.execute(
                "UPDATE knowledge_files SET jurisdiction=?1 WHERE id=?2 AND workspace_id = ?3",
                params![j, id, workspace_id],
            )?;
        }
        Ok(())
    }

    // ── Embeddings ────────────────────────────────────────────────────────

    pub fn upsert_embedding(
        &self,
        project_id: Option<i64>,
        task_id: Option<i64>,
        chunk_text: &str,
        file_path: &str,
        embedding: &[f32],
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let hash = crate::knowledge::hash_chunk(chunk_text);
        let blob = crate::knowledge::embedding_to_bytes(embedding);
        conn.execute(
            "INSERT INTO embeddings (project_id, task_id, chunk_text, chunk_hash, file_path, embedding, dims) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(chunk_hash) DO UPDATE SET embedding = excluded.embedding",
            params![project_id, task_id, chunk_text, hash, file_path, blob, embedding.len() as i64],
        )
        .context("upsert_embedding")?;
        Ok(())
    }

    pub fn remove_task_embeddings(&self, task_id: i64) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let n = conn
            .execute(
                "DELETE FROM embeddings WHERE task_id = ?1",
                params![task_id],
            )
            .context("remove_task_embeddings")?;
        Ok(n)
    }

    pub fn search_embeddings(
        &self,
        query_embedding: &[f32],
        limit: usize,
        project_id: Option<i64>,
    ) -> Result<Vec<crate::knowledge::EmbeddingSearchResult>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let cap = limit.max(1).min(5000);
        let (sql, params_vec): (String, Vec<Box<dyn pg::types::ToSql>>) = match project_id {
            Some(pid) => (
                "SELECT id, project_id, task_id, chunk_text, file_path, embedding FROM embeddings WHERE project_id = ?1".to_string(),
                vec![Box::new(pid) as Box<dyn pg::types::ToSql>],
            ),
            None => (
                "SELECT id, project_id, task_id, chunk_text, file_path, embedding FROM embeddings".to_string(),
                vec![],
            ),
        };
        let mut stmt = conn.prepare(&sql)?;
        let params_refs: Vec<&dyn pg::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(params_refs.as_slice(), |row: &pg::Row| {
            Ok((
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Vec<u8>>(5)?,
            ))
        })?;

        let mut results = Vec::with_capacity(cap.min(128));
        let flush_at = cap.saturating_mul(4).max(cap + 8);
        for row in rows {
            let (pid, tid, text, path, blob) = row.context("search_embeddings row")?;
            let emb = crate::knowledge::bytes_to_embedding(&blob);
            let score = crate::knowledge::cosine_similarity(query_embedding, &emb);
            results.push(crate::knowledge::EmbeddingSearchResult {
                chunk_text: text,
                file_path: path,
                project_id: pid,
                task_id: tid,
                score,
            });
            if results.len() >= flush_at {
                results.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                results.truncate(cap);
            }
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(cap);
        Ok(results)
    }

    pub fn list_recent_project_files(
        &self,
        project_id: i64,
        limit: i64,
        require_text: bool,
    ) -> Result<Vec<ProjectFileRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let lim = limit.clamp(1, 100);
        let sql = if require_text {
            format!(
                "SELECT {PROJECT_FILE_COLS} FROM project_files \
                 WHERE project_id=?1 AND extracted_text != '' \
                 ORDER BY created_at DESC, id DESC LIMIT ?2"
            )
        } else {
            format!(
                "SELECT {PROJECT_FILE_COLS} FROM project_files \
                 WHERE project_id=?1 ORDER BY created_at DESC, id DESC LIMIT ?2"
            )
        };
        let mut stmt = conn
            .prepare(&sql)
            .context("list_recent_project_files prepare")?;
        let files = stmt
            .query_map(params![project_id, lim], row_to_project_file)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_recent_project_files rows")?;
        Ok(files)
    }

    pub fn list_recent_completed_project_tasks(
        &self,
        project_id: i64,
        limit: i64,
    ) -> Result<Vec<Task>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let lim = limit.clamp(1, 50);
        let sql = format!(
            "SELECT {TASK_COLS} FROM pipeline_tasks \
             WHERE project_id = ?1 AND status IN ('merged','done','complete','purge','purged') \
             ORDER BY id DESC LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let tasks = stmt
            .query_map(params![project_id, lim], row_to_task)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_recent_completed_project_tasks")?;
        Ok(tasks)
    }

    pub fn embedding_count(&self) -> i64 {
        let Ok(conn) = self.conn.lock() else { return 0 };
        conn.query_row("SELECT COUNT(*) FROM embeddings", [], |r: &pg::Row| {
            r.get(0)
        })
        .unwrap_or(0)
    }

    // ── Citation verifications ──────────────────────────────────────────

    pub fn insert_citation_verification(
        &self,
        task_id: i64,
        citation_text: &str,
        citation_type: &str,
        status: &str,
        source: &str,
        treatment: &str,
        checked_at: &str,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let id = conn.execute_returning_id(
            "INSERT INTO citation_verifications (task_id, citation_text, citation_type, status, source, treatment, checked_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![task_id, citation_text, citation_type, status, source, treatment, checked_at],
        )?;
        Ok(id)
    }

    pub fn get_task_citations(&self, task_id: i64) -> Result<Vec<CitationVerification>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, citation_text, citation_type, status, source, treatment, checked_at, created_at \
             FROM citation_verifications WHERE task_id = ?1 ORDER BY id"
        )?;
        let rows = stmt
            .query_map(params![task_id], |r: &pg::Row| {
                Ok(CitationVerification {
                    id: r.get(0)?,
                    task_id: r.get(1)?,
                    citation_text: r.get(2)?,
                    citation_type: r.get(3)?,
                    status: r.get(4)?,
                    source: r.get(5)?,
                    treatment: r.get(6)?,
                    checked_at: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
                    created_at: r.get::<_, String>(8)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn delete_task_citations(&self, task_id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "DELETE FROM citation_verifications WHERE task_id = ?1",
            params![task_id],
        )?;
        Ok(())
    }

    pub fn get_top_scored_proposals(&self, threshold: i64, limit: i64) -> Result<Vec<Proposal>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, repo_path, title, description, rationale, status, created_at, \
             triage_score, triage_impact, triage_feasibility, triage_risk, triage_effort, \
             triage_reasoning \
             FROM proposals WHERE status='proposed' AND triage_score >= ?1 \
             ORDER BY triage_score DESC LIMIT ?2",
        )?;
        let proposals = stmt
            .query_map(params![threshold, limit], row_to_proposal)?
            .collect::<pg::Result<Vec<_>>>()
            .context("get_top_scored_proposals")?;
        Ok(proposals)
    }

    pub fn count_unscored_proposals(&self) -> i64 {
        let Ok(conn) = self.conn.lock() else { return 0 };
        conn.query_row(
            "SELECT COUNT(*) FROM proposals WHERE status='proposed' AND triage_score=0",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0)
    }

    pub fn list_untriaged_proposals(&self) -> Result<Vec<Proposal>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, repo_path, title, description, rationale, status, created_at, \
             triage_score, triage_impact, triage_feasibility, triage_risk, triage_effort, \
             triage_reasoning \
             FROM proposals WHERE status='proposed' AND triage_score=0 ORDER BY id ASC",
        )?;
        let proposals = stmt
            .query_map([], row_to_proposal)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_untriaged_proposals")?;
        Ok(proposals)
    }

    // ── Merge Queue ───────────────────────────────────────────────────────

    pub fn list_queue(&self) -> Result<Vec<QueueEntry>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, branch, repo_path, status, queued_at, pr_number \
             FROM integration_queue WHERE status = 'queued' ORDER BY id ASC",
        )?;
        let entries = stmt
            .query_map([], row_to_queue_entry)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_queue")?;
        Ok(entries)
    }

    pub fn enqueue(
        &self,
        task_id: i64,
        branch: &str,
        repo_path: &str,
        pr_number: i64,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let queued_at = now_str();
        let id = conn.execute_returning_id(
            "INSERT INTO integration_queue (task_id, branch, repo_path, status, queued_at, pr_number) \
             VALUES (?1, ?2, ?3, 'queued', ?4, ?5)",
            params![task_id, branch, repo_path, queued_at, pr_number],
        )
        .context("enqueue")?;
        Ok(id)
    }

    /// Ensure a task/branch has exactly one active queue entry.
    ///
    /// If an existing non-merged row exists, it is recycled back to `queued`
    /// instead of inserting another row. This prevents unbounded queue growth
    /// when tasks repeatedly cycle through done -> rebase -> done.
    pub fn enqueue_or_requeue(
        &self,
        task_id: i64,
        branch: &str,
        repo_path: &str,
        pr_number: i64,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM integration_queue \
                 WHERE task_id = ?1 AND branch = ?2 AND status IN ('queued','merging','excluded','pending_review') \
                 ORDER BY id DESC LIMIT 1",
                params![task_id, branch],
                |r| r.get(0),
            )
            .optional()
            .context("enqueue_or_requeue select existing")?;

        let queued_at = now_str();
        if let Some(id) = existing {
            conn.execute(
                "UPDATE integration_queue
                 SET status = 'queued',
                     repo_path = ?1,
                     queued_at = ?2,
                     pr_number = ?3,
                     error_msg = '',
                     unknown_retries = 0
                 WHERE id = ?4",
                params![repo_path, queued_at, pr_number, id],
            )
            .context("enqueue_or_requeue update existing")?;
            return Ok(id);
        }

        let id = conn.execute_returning_id(
            "INSERT INTO integration_queue (task_id, branch, repo_path, status, queued_at, pr_number) \
             VALUES (?1, ?2, ?3, 'queued', ?4, ?5)",
            params![task_id, branch, repo_path, queued_at, pr_number],
        )
        .context("enqueue_or_requeue insert")?;
        Ok(id)
    }

    pub fn update_queue_status(&self, id: i64, status: &str) -> Result<()> {
        self.update_queue_status_with_error(id, status, "")
    }

    pub fn update_queue_status_with_error(
        &self,
        id: i64,
        status: &str,
        error_msg: &str,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE integration_queue SET status = ?1, error_msg = ?2 WHERE id = ?3",
            params![status, error_msg, id],
        )
        .context("update_queue_status_with_error")?;
        Ok(())
    }

    pub fn get_queued_branches_for_repo(&self, repo_path: &str) -> Result<Vec<QueueEntry>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, branch, repo_path, status, queued_at, pr_number \
             FROM integration_queue WHERE repo_path = ?1 AND status = 'queued' ORDER BY task_id ASC",
        )?;
        let entries = stmt
            .query_map(params![repo_path], row_to_queue_entry)?
            .collect::<pg::Result<Vec<_>>>()
            .context("get_queued_branches_for_repo")?;
        Ok(entries)
    }

    pub fn get_queue_entries_for_task(&self, task_id: i64) -> Result<Vec<QueueEntry>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, branch, repo_path, status, queued_at, pr_number \
             FROM integration_queue WHERE task_id = ?1 ORDER BY id ASC",
        )?;
        let entries = stmt
            .query_map(params![task_id], row_to_queue_entry)?
            .collect::<pg::Result<Vec<_>>>()
            .context("get_queue_entries_for_task")?;
        Ok(entries)
    }

    pub fn get_unknown_retries(&self, id: i64) -> i64 {
        let Ok(conn) = self.conn.lock() else { return 0 };
        conn.query_row(
            "SELECT unknown_retries FROM integration_queue WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap_or(0)
    }

    pub fn increment_unknown_retries(&self, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE integration_queue SET unknown_retries = unknown_retries + 1 WHERE id = ?1",
            params![id],
        )
        .context("increment_unknown_retries")?;
        Ok(())
    }

    pub fn reset_unknown_retries(&self, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE integration_queue SET unknown_retries = 0 WHERE id = ?1",
            params![id],
        )
        .context("reset_unknown_retries")?;
        Ok(())
    }

    // ── Task Outputs ──────────────────────────────────────────────────────

    pub fn insert_task_output(
        &self,
        task_id: i64,
        phase: &str,
        output: &str,
        raw_stream: &str,
        exit_code: i64,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let created_at = now_str();
        let id = conn.execute_returning_id(
            "INSERT INTO task_outputs (task_id, phase, output, raw_stream, exit_code, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![task_id, phase, output, raw_stream, exit_code, created_at],
        )
        .context("insert_task_output")?;
        Ok(id)
    }

    pub fn purge_task_data(&self, task_id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;

        // Delete vector embeddings
        conn.execute(
            "DELETE FROM embeddings WHERE task_id = ?1",
            params![task_id],
        )
        .context("delete embeddings")?;

        // Delete chat history (keep outputs for UI visibility)
        conn.execute(
            "DELETE FROM task_messages WHERE task_id = ?1",
            params![task_id],
        )
        .context("delete messages")?;

        Ok(())
    }

    pub fn get_task_outputs(&self, task_id: i64) -> Result<Vec<TaskOutput>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, phase, output, raw_stream, exit_code, created_at \
             FROM task_outputs WHERE task_id = ?1 ORDER BY id ASC",
        )?;
        let outputs = stmt
            .query_map(params![task_id], row_to_task_output)?
            .collect::<pg::Result<Vec<_>>>()
            .context("get_task_outputs")?;
        Ok(outputs)
    }

    // ── Task Messages ─────────────────────────────────────────────────────

    pub fn insert_task_message(&self, task_id: i64, role: &str, content: &str) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let created_at = now_str();
        let id = conn
            .execute_returning_id(
                "INSERT INTO task_messages (task_id, role, content, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
                params![task_id, role, content, created_at],
            )
            .context("insert_task_message")?;
        Ok(id)
    }

    pub fn get_task_messages(&self, task_id: i64) -> Result<Vec<TaskMessage>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, role, content, created_at, delivered_phase \
             FROM task_messages WHERE task_id = ?1 ORDER BY id ASC",
        )?;
        let messages = stmt
            .query_map(params![task_id], row_to_task_message)?
            .collect::<pg::Result<Vec<_>>>()
            .context("get_task_messages")?;
        Ok(messages)
    }

    pub fn get_pending_task_messages(&self, task_id: i64) -> Result<Vec<TaskMessage>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, role, content, created_at, delivered_phase \
             FROM task_messages WHERE task_id = ?1 AND delivered_phase IS NULL ORDER BY id ASC",
        )?;
        let messages = stmt
            .query_map(params![task_id], row_to_task_message)?
            .collect::<pg::Result<Vec<_>>>()
            .context("get_pending_task_messages")?;
        Ok(messages)
    }

    pub fn mark_messages_delivered(&self, task_id: i64, phase: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE task_messages SET delivered_phase = ?1 \
             WHERE task_id = ?2 AND delivered_phase IS NULL",
            params![phase, task_id],
        )
        .context("mark_messages_delivered")?;
        Ok(())
    }

    // ── Knowledge Repos ───────────────────────────────────────────────────

    fn row_to_knowledge_repo(row: &pg::Row<'_>) -> pg::Result<KnowledgeRepo> {
        Ok(KnowledgeRepo {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            user_id: row.get(2)?,
            url: row.get(3)?,
            name: row.get(4)?,
            local_path: row.get(5)?,
            status: row.get(6)?,
            error_msg: row.get(7)?,
            created_at: row.get(8)?,
        })
    }

    pub fn list_knowledge_repos(
        &self,
        workspace_id: i64,
        user_id: Option<i64>,
    ) -> Result<Vec<KnowledgeRepo>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = if user_id.is_some() {
            conn.prepare(
                "SELECT id, workspace_id, user_id, url, name, local_path, status, error_msg, created_at \
                 FROM knowledge_repos WHERE workspace_id = ?1 AND user_id = ?2 ORDER BY created_at ASC",
            )?
        } else {
            conn.prepare(
                "SELECT id, workspace_id, user_id, url, name, local_path, status, error_msg, created_at \
                 FROM knowledge_repos WHERE workspace_id = ?1 AND user_id IS NULL ORDER BY created_at ASC",
            )?
        };
        let rows = if let Some(uid) = user_id {
            stmt.query_map(params![workspace_id, uid], Self::row_to_knowledge_repo)?
        } else {
            stmt.query_map(params![workspace_id], Self::row_to_knowledge_repo)?
        };
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn list_all_knowledge_repos(&self) -> Result<Vec<KnowledgeRepo>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, user_id, url, name, local_path, status, error_msg, created_at \
             FROM knowledge_repos ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], Self::row_to_knowledge_repo)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn insert_knowledge_repo(
        &self,
        workspace_id: i64,
        user_id: Option<i64>,
        url: &str,
        name: &str,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        Ok(conn.execute_returning_id(
            "INSERT INTO knowledge_repos (workspace_id, user_id, url, name, status) VALUES (?1, ?2, ?3, ?4, 'pending')",
            params![workspace_id, user_id, url, name],
        )?)
    }

    pub fn update_knowledge_repo_status(
        &self,
        id: i64,
        status: &str,
        local_path: &str,
        error_msg: &str,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE knowledge_repos SET status = ?1, local_path = ?2, error_msg = ?3 WHERE id = ?4",
            params![status, local_path, error_msg, id],
        )?;
        Ok(())
    }

    pub fn delete_knowledge_repo(&self, id: i64, workspace_id: i64) -> Result<String> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let local_path: Option<String> = conn
            .query_row(
                "SELECT local_path FROM knowledge_repos WHERE id = ?1 AND workspace_id = ?2",
                params![id, workspace_id],
                |r| r.get(0),
            )
            .optional()?;
        conn.execute(
            "DELETE FROM knowledge_repos WHERE id = ?1 AND workspace_id = ?2",
            params![id, workspace_id],
        )?;
        Ok(local_path.unwrap_or_default())
    }

    // ── Repos ─────────────────────────────────────────────────────────────

    pub fn upsert_repo(
        &self,
        path: &str,
        name: &str,
        mode: &str,
        test_cmd: &str,
        prompt_file: &str,
        auto_merge: bool,
        backend: Option<&str>,
        repo_slug: &str,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let auto_merge_int: i64 = if auto_merge { 1 } else { 0 };
        conn.execute(
            "INSERT INTO repos (path, name, mode, test_cmd, prompt_file, auto_merge, backend, repo_slug) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT(path) DO UPDATE SET \
               name = excluded.name, \
               mode = COALESCE(NULLIF(excluded.mode, ''), repos.mode), \
               test_cmd = COALESCE(NULLIF(excluded.test_cmd, ''), repos.test_cmd), \
               prompt_file = COALESCE(NULLIF(excluded.prompt_file, ''), repos.prompt_file), \
               auto_merge = excluded.auto_merge, \
               backend = COALESCE(NULLIF(excluded.backend, ''), repos.backend), \
               repo_slug = COALESCE(NULLIF(excluded.repo_slug, ''), repos.repo_slug)",
            params![
                path,
                name,
                mode,
                test_cmd,
                prompt_file,
                auto_merge_int,
                backend,
                repo_slug
            ],
        )
        .context("upsert_repo")?;
        let id: i64 = conn
            .query_row(
                "SELECT id FROM repos WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .context("upsert_repo get id")?;
        Ok(id)
    }

    pub fn list_repos(&self) -> Result<Vec<RepoRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, path, name, mode, backend, test_cmd, prompt_file, auto_merge, repo_slug \
             FROM repos ORDER BY id ASC",
        )?;
        let repos = stmt
            .query_map([], row_to_repo)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_repos")?;
        Ok(repos)
    }

    pub fn get_repo_by_path(&self, path: &str) -> Result<Option<RepoRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let result = conn
            .query_row(
                "SELECT id, path, name, mode, backend, test_cmd, prompt_file, auto_merge, repo_slug \
                 FROM repos WHERE path = ?1",
                params![path],
                row_to_repo,
            )
            .optional()
            .context("get_repo_by_path")?;
        Ok(result)
    }

    pub fn update_repo_backend(&self, id: i64, backend: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE repos SET backend = ?1 WHERE id = ?2",
            params![
                if backend.is_empty() {
                    None
                } else {
                    Some(backend)
                },
                id
            ],
        )
        .context("update_repo_backend")?;
        Ok(())
    }

    // ── Pipeline Events ───────────────────────────────────────────────────

    pub fn log_event(
        &self,
        task_id: Option<i64>,
        repo_id: Option<i64>,
        kind: &str,
        payload: &serde_json::Value,
    ) -> Result<i64> {
        self.log_event_full(task_id, repo_id, None, "", kind, payload)
    }

    pub fn log_event_full(
        &self,
        task_id: Option<i64>,
        repo_id: Option<i64>,
        project_id: Option<i64>,
        actor: &str,
        kind: &str,
        payload: &serde_json::Value,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let payload_str = payload.to_string();
        let created_at = now_str();
        let id = conn.execute_returning_id(
            "INSERT INTO pipeline_events (task_id, repo_id, project_id, actor, kind, payload, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![task_id, repo_id, project_id, actor, kind, payload_str, created_at],
        )
        .context("log_event")?;
        Ok(id)
    }

    pub fn list_project_events(&self, project_id: i64, limit: i64) -> Result<Vec<AuditEvent>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, project_id, actor, kind, payload, created_at \
             FROM pipeline_events WHERE project_id = ?1 \
             ORDER BY created_at DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![project_id, limit], |r| {
                let ts: String = r.get(6)?;
                Ok(AuditEvent {
                    id: r.get(0)?,
                    task_id: r.get::<_, Option<i64>>(1)?,
                    project_id: r.get::<_, Option<i64>>(2)?,
                    actor: r.get(3)?,
                    kind: r.get(4)?,
                    payload: r.get(5)?,
                    created_at: parse_ts(&ts),
                })
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_project_events")?;
        Ok(rows)
    }

    pub fn list_task_events(&self, task_id: i64, limit: i64) -> Result<Vec<AuditEvent>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, task_id, project_id, actor, kind, payload, created_at \
             FROM pipeline_events WHERE task_id = ?1 \
             ORDER BY created_at DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![task_id, limit], |r| {
                let ts: String = r.get(6)?;
                Ok(AuditEvent {
                    id: r.get(0)?,
                    task_id: r.get::<_, Option<i64>>(1)?,
                    project_id: r.get::<_, Option<i64>>(2)?,
                    actor: r.get(3)?,
                    kind: r.get(4)?,
                    payload: r.get(5)?,
                    created_at: parse_ts(&ts),
                })
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_task_events")?;
        Ok(rows)
    }

    // ── Users ─────────────────────────────────────────────────────────────

    pub fn get_user_default_workspace_id(&self, user_id: i64) -> Result<Option<i64>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let result = conn
            .query_row(
                "SELECT default_workspace_id FROM users WHERE id = ?1",
                params![user_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()
            .context("get_user_default_workspace_id")?;
        Ok(result.flatten())
    }

    pub fn set_user_default_workspace_id(&self, user_id: i64, workspace_id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE users SET default_workspace_id = ?1 WHERE id = ?2",
            params![workspace_id, user_id],
        )
        .context("set_user_default_workspace_id")?;
        Ok(())
    }

    pub fn set_preferred_admin_workspace(&self, user_id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let preferred = conn
            .query_row(
                "SELECT id FROM workspaces \
                 WHERE kind IN ('shared', 'system') \
                 ORDER BY CASE kind WHEN 'system' THEN 0 ELSE 1 END, id ASC LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(workspace_id) = preferred {
            conn.execute(
                "UPDATE users SET default_workspace_id = ?1 WHERE id = ?2",
                params![workspace_id, user_id],
            )
            .context("set_preferred_admin_workspace")?;
        }
        Ok(())
    }

    pub fn list_user_workspaces(&self, user_id: i64) -> Result<Vec<WorkspaceMembershipRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let default_workspace_id = conn
            .query_row(
                "SELECT default_workspace_id FROM users WHERE id = ?1",
                params![user_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten()
            .unwrap_or(0);
        let mut stmt = conn.prepare(
            "SELECT w.id, w.name, w.slug, w.kind, wm.role, w.created_at \
             FROM workspace_memberships wm \
             JOIN workspaces w ON w.id = wm.workspace_id \
             WHERE wm.user_id = ?1 ORDER BY w.kind, w.name",
        )?;
        let rows = stmt
            .query_map(params![user_id], |row| {
                let workspace_id: i64 = row.get(0)?;
                Ok(WorkspaceMembershipRow {
                    workspace_id,
                    name: row.get(1)?,
                    slug: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    kind: row.get(3)?,
                    role: row.get(4)?,
                    is_default: workspace_id == default_workspace_id,
                    created_at: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                })
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_user_workspaces")?;
        Ok(rows)
    }

    pub fn get_user_workspace_membership(
        &self,
        user_id: i64,
        workspace_id: i64,
    ) -> Result<Option<WorkspaceMembershipRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let default_workspace_id = conn
            .query_row(
                "SELECT default_workspace_id FROM users WHERE id = ?1",
                params![user_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten()
            .unwrap_or(0);
        conn.query_row(
            "SELECT w.id, w.name, w.slug, w.kind, wm.role, w.created_at \
             FROM workspace_memberships wm \
             JOIN workspaces w ON w.id = wm.workspace_id \
             WHERE wm.user_id = ?1 AND wm.workspace_id = ?2",
            params![user_id, workspace_id],
            |row| {
                let resolved_workspace_id: i64 = row.get(0)?;
                Ok(WorkspaceMembershipRow {
                    workspace_id: resolved_workspace_id,
                    name: row.get(1)?,
                    slug: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    kind: row.get(3)?,
                    role: row.get(4)?,
                    is_default: resolved_workspace_id == default_workspace_id,
                    created_at: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                })
            },
        )
        .optional()
        .context("get_user_workspace_membership")
    }

    pub fn user_has_workspace_access(&self, user_id: i64, workspace_id: i64) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let exists = conn
            .query_row(
                "SELECT 1 FROM workspace_memberships WHERE user_id = ?1 AND workspace_id = ?2",
                params![user_id, workspace_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .context("user_has_workspace_access")?
            .is_some();
        Ok(exists)
    }

    pub fn get_workspace(&self, workspace_id: i64) -> Result<Option<WorkspaceRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.query_row(
            "SELECT id, name, slug, kind, owner_user_id, created_at FROM workspaces WHERE id = ?1",
            params![workspace_id],
            row_to_workspace,
        )
        .optional()
        .context("get_workspace")
    }

    pub fn get_system_workspace(&self) -> Result<Option<WorkspaceRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.query_row(
            "SELECT id, name, slug, kind, owner_user_id, created_at FROM workspaces WHERE kind = 'system' ORDER BY id ASC LIMIT 1",
            [],
            row_to_workspace,
        )
        .optional()
        .context("get_system_workspace")
    }

    pub fn get_first_workspace_by_kind(&self, kind: &str) -> Result<Option<WorkspaceRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.query_row(
            "SELECT id, name, slug, kind, owner_user_id, created_at FROM workspaces WHERE kind = ?1 ORDER BY id ASC LIMIT 1",
            params![kind],
            row_to_workspace,
        )
        .optional()
        .context("get_first_workspace_by_kind")
    }

    pub fn list_all_workspaces(&self) -> Result<Vec<WorkspaceRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, name, slug, kind, owner_user_id, created_at FROM workspaces ORDER BY kind, name, id",
        )?;
        let rows = stmt
            .query_map([], row_to_workspace)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_all_workspaces")?;
        Ok(rows)
    }

    pub fn create_workspace(
        &self,
        name: &str,
        kind: &str,
        owner_user_id: Option<i64>,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let base_slug = unique_slug(name, 0);
        let slug = if base_slug.is_empty() {
            format!("workspace-{}", Utc::now().timestamp())
        } else {
            let mut candidate = base_slug.clone();
            let mut suffix = 2;
            loop {
                let taken = conn
                    .query_row(
                        "SELECT 1 FROM workspaces WHERE slug = ?1",
                        params![candidate.clone()],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?
                    .is_some();
                if !taken {
                    break candidate;
                }
                candidate = unique_slug(&base_slug, suffix);
                suffix += 1;
            }
        };
        let id = conn.execute_returning_id(
            "INSERT INTO workspaces (name, slug, kind, owner_user_id) VALUES (?1, ?2, ?3, ?4)",
            params![name, slug, kind, owner_user_id],
        )?;
        Ok(id)
    }

    pub fn add_workspace_member(&self, workspace_id: i64, user_id: i64, role: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "INSERT INTO workspace_memberships (workspace_id, user_id, role) VALUES (?1, ?2, ?3) \
             ON CONFLICT (workspace_id, user_id) DO UPDATE SET role = EXCLUDED.role",
            params![workspace_id, user_id, role],
        )
        .context("add_workspace_member")?;
        Ok(())
    }

    pub fn ensure_system_workspace_membership(&self, user_id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let system_ws = conn
            .query_row(
                "SELECT id FROM workspaces WHERE kind = 'system' ORDER BY id ASC LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(workspace_id) = system_ws {
            conn.execute(
                "INSERT INTO workspace_memberships (workspace_id, user_id, role) VALUES (?1, ?2, 'member') \
                 ON CONFLICT (workspace_id, user_id) DO NOTHING",
                params![workspace_id, user_id],
            )
            .context("ensure_system_workspace_membership")?;
        }
        Ok(())
    }

    pub fn ensure_admin_workspace_memberships(&self, user_id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare("SELECT id FROM workspaces ORDER BY id ASC")?;
        let workspace_ids = stmt
            .query_map([], |row| row.get::<_, i64>(0))?
            .collect::<pg::Result<Vec<_>>>()?;
        for workspace_id in workspace_ids {
            conn.execute(
                "INSERT INTO workspace_memberships (workspace_id, user_id, role) VALUES (?1, ?2, 'admin') \
                 ON CONFLICT (workspace_id, user_id) DO UPDATE SET role = 'admin'",
                params![workspace_id, user_id],
            )?;
        }
        Ok(())
    }

    pub fn count_users(&self) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM users", params![], |row| row.get(0))
            .context("count_users")?;
        Ok(count)
    }

    pub fn count_admin_users(&self) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM users WHERE is_admin = true",
                params![],
                |row| row.get(0),
            )
            .context("count_admin_users")?;
        Ok(count)
    }

    pub fn create_user(
        &self,
        username: &str,
        display_name: &str,
        password_hash: &str,
        is_admin: bool,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let id: i64 = conn
            .query_row(
                "INSERT INTO users (username, display_name, password_hash, is_admin) \
                 VALUES (?1, ?2, ?3, ?4) RETURNING id",
                params![username, display_name, password_hash, is_admin],
                |row| row.get(0),
            )
            .context("create_user")?;
        let workspace_name = if display_name.trim().is_empty() {
            format!("{username} Personal")
        } else {
            format!("{display_name} Personal")
        };
        let workspace_id = Self::get_or_create_workspace(
            &conn,
            &workspace_name,
            "personal",
            Some(id),
            &unique_slug(&format!("{username}-personal"), 0),
        )?;
        conn.execute(
            "INSERT INTO workspace_memberships (workspace_id, user_id, role) VALUES (?1, ?2, 'owner') \
             ON CONFLICT (workspace_id, user_id) DO UPDATE SET role = EXCLUDED.role",
            params![workspace_id, id],
        )
        .context("create_user workspace membership")?;
        conn.execute(
            "UPDATE users SET default_workspace_id = ?1 WHERE id = ?2",
            params![workspace_id, id],
        )
        .context("create_user default workspace")?;
        Ok(id)
    }

    pub fn get_user_by_username(
        &self,
        username: &str,
    ) -> Result<Option<(i64, String, String, String, bool)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let result = conn
            .query_row(
                "SELECT id, username, display_name, password_hash, is_admin FROM users WHERE username = ?1",
                params![username],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .optional()
            .context("get_user_by_username")?;
        Ok(result)
    }

    /// Look up a user by email address.
    /// For SSO users the username IS their email; as a fallback also checks the
    /// `contact_email` user setting.
    pub fn get_user_by_email(&self, email: &str) -> Result<Option<(i64, String, String, bool)>> {
        // SSO users have their email as username
        if let Ok(Some((id, username, display_name, _, is_admin))) =
            self.get_user_by_username(email)
        {
            return Ok(Some((id, username, display_name, is_admin)));
        }
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let result = conn
            .query_row(
                "SELECT u.id, u.username, u.display_name, u.is_admin \
                 FROM users u \
                 JOIN user_settings us ON us.user_id = u.id \
                 WHERE us.key = 'contact_email' AND LOWER(us.value) = LOWER(?1) \
                 LIMIT 1",
                params![email],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .context("get_user_by_email")?;
        Ok(result)
    }

    pub fn get_user_by_id(&self, id: i64) -> Result<Option<(i64, String, String, bool)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let result = conn
            .query_row(
                "SELECT id, username, display_name, is_admin FROM users WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .context("get_user_by_id")?;
        Ok(result)
    }

    pub fn set_user_admin(&self, id: i64, is_admin: bool) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE users SET is_admin = ?1 WHERE id = ?2",
            params![is_admin, id],
        )
        .context("set_user_admin")?;
        Ok(())
    }

    pub fn list_users(&self) -> Result<Vec<(i64, String, String, bool, String)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, username, display_name, is_admin, created_at FROM users ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_users")?;
        Ok(rows)
    }

    pub fn delete_user(&self, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute("DELETE FROM users WHERE id = ?1", params![id])
            .context("delete_user")?;
        Ok(())
    }

    pub fn update_user_password(&self, id: i64, password_hash: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE users SET password_hash = ?1 WHERE id = ?2",
            params![password_hash, id],
        )
        .context("update_user_password")?;
        Ok(())
    }

    // ── User Settings ────────────────────────────────────────────────────

    pub fn get_user_setting(&self, user_id: i64, key: &str) -> Result<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let result = conn
            .query_row(
                "SELECT value FROM user_settings WHERE user_id = ?1 AND key = ?2",
                params![user_id, key],
                |row| row.get(0),
            )
            .optional()
            .context("get_user_setting")?;
        Ok(result)
    }

    pub fn set_user_setting(&self, user_id: i64, key: &str, value: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "INSERT INTO user_settings (user_id, key, value) VALUES (?1, ?2, ?3) \
             ON CONFLICT(user_id, key) DO UPDATE SET value = excluded.value",
            params![user_id, key, value],
        )
        .context("set_user_setting")?;
        Ok(())
    }

    pub fn get_all_user_settings(&self, user_id: i64) -> Result<HashMap<String, String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare("SELECT key, value FROM user_settings WHERE user_id = ?1")?;
        let rows = stmt
            .query_map(params![user_id], |row| {
                let k: String = row.get(0)?;
                let v: String = row.get(1)?;
                Ok((k, v))
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("get_all_user_settings")?;
        Ok(rows.into_iter().collect())
    }

    pub fn delete_user_setting(&self, user_id: i64, key: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "DELETE FROM user_settings WHERE user_id = ?1 AND key = ?2",
            params![user_id, key],
        )
        .context("delete_user_setting")?;
        Ok(())
    }

    // ── Config ────────────────────────────────────────────────────────────

    pub fn get_config(&self, key: &str) -> Result<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let result = conn
            .query_row(
                "SELECT value FROM config WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .context("get_config")?;
        Ok(result)
    }

    pub fn ensure_config(&self, key: &str, value: &str) -> Result<()> {
        if self.get_config(key)?.is_none() {
            self.set_config(key, value)?;
        }
        Ok(())
    }

    pub fn set_config(&self, key: &str, value: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let updated_at = now_str();
        conn.execute(
            "INSERT INTO config (key, value, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value, updated_at],
        )
        .context("set_config")?;
        Ok(())
    }

    // ── Legacy Event Log ──────────────────────────────────────────────────

    pub fn log_legacy_event(
        &self,
        level: &str,
        category: &str,
        message: &str,
        metadata: &str,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let ts = Utc::now().timestamp();
        let id = conn
            .execute_returning_id(
                "INSERT INTO events (ts, level, category, message, metadata) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
                params![ts, level, category, message, metadata],
            )
            .context("log_legacy_event")?;
        Ok(id)
    }

    pub fn get_recent_events(&self, limit: i64) -> Result<Vec<LegacyEvent>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, ts, level, category, message, metadata \
             FROM events ORDER BY ts DESC, id DESC LIMIT ?1",
        )?;
        let events = stmt
            .query_map(params![limit], row_to_legacy_event)?
            .collect::<pg::Result<Vec<_>>>()
            .context("get_recent_events")?;
        Ok(events)
    }

    pub fn create_pipeline_task(
        &self,
        title: &str,
        description: &str,
        repo_path: &str,
        source: &str,
        notify_chat: &str,
        mode: &str,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let system_workspace_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM workspaces WHERE kind = 'system' ORDER BY id ASC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let id = conn
            .execute_returning_id(
                "INSERT INTO pipeline_tasks \
             (title, description, repo_path, status, attempt, max_attempts, last_error, \
              created_by, notify_chat, created_at, session_id, mode, backend, workspace_id) \
             VALUES (?1, ?2, ?3, 'backlog', 0, 5, '', ?4, ?5, ?6, '', ?7, '', ?8)",
                params![
                    title,
                    description,
                    repo_path,
                    source,
                    notify_chat,
                    now_str(),
                    mode,
                    system_workspace_id,
                ],
            )
            .context("create_pipeline_task")?;
        Ok(id)
    }

    /// Return "done" tasks that have no integration_queue entry (orphaned after restart).
    pub fn list_done_tasks_without_queue(&self) -> Result<Vec<Task>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = format!(
            "SELECT {TASK_COLS} FROM pipeline_tasks \
             WHERE status = 'done' \
             AND NOT EXISTS ( \
               SELECT 1 FROM integration_queue q \
               WHERE q.task_id = pipeline_tasks.id \
               AND q.status IN ('queued', 'excluded', 'merged') \
             )",
        );
        let mut stmt = conn.prepare(&sql)?;
        let tasks = stmt
            .query_map([], row_to_task)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_done_tasks_without_queue")?;
        Ok(tasks)
    }

    /// Reset integration_queue entries stuck in "merging" where the task is not yet merged.
    pub fn reset_stale_merging_queue(&self) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let n = conn.execute(
            "UPDATE integration_queue SET status = 'queued' \
             WHERE status = 'merging' \
             AND task_id IN (SELECT id FROM pipeline_tasks WHERE status != 'merged')",
            [],
        )?;
        Ok(n)
    }

    pub fn active_task_count(&self) -> i64 {
        let Ok(conn) = self.conn.lock() else { return 0 };
        conn.query_row(
            "SELECT COUNT(*) FROM pipeline_tasks WHERE status NOT IN ('done','merged','failed','blocked','pending_review','human_review','purged')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0)
    }

    pub fn get_recent_merged_tasks(&self, limit: i64) -> Result<Vec<Task>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = format!(
            "SELECT {TASK_COLS} FROM pipeline_tasks WHERE status = 'merged' ORDER BY id DESC LIMIT ?1"
        );
        let mut stmt = conn.prepare(&sql)?;
        let tasks = stmt
            .query_map(params![limit], row_to_task)?
            .collect::<pg::Result<Vec<_>>>()
            .context("get_recent_merged_tasks")?;
        Ok(tasks)
    }

    pub fn recycle_failed_tasks(&self, repo_path: &str) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let n = conn
            .execute(
                "UPDATE pipeline_tasks SET status='backlog', attempt=0, last_error='' \
             WHERE status='failed' AND repo_path=?1",
                params![repo_path],
            )
            .context("recycle_failed_tasks")?;
        Ok(n)
    }

    pub fn reset_task_attempt(&self, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET attempt=0 WHERE id=?1",
            params![id],
        )
        .context("reset_task_attempt")?;
        Ok(())
    }

    // ── Timing state (persisted across restarts) ──────────────────────────

    pub fn get_ts(&self, key: &str) -> i64 {
        self.get_config(key)
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    pub fn set_ts(&self, key: &str, value: i64) {
        let _ = self.set_config(key, &value.to_string());
    }

    // ── Full Task List ────────────────────────────────────────────────────

    pub fn list_all_tasks(&self, repo_path: Option<&str>) -> Result<Vec<Task>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = if repo_path.is_some() {
            format!(
                "SELECT {TASK_COLS} FROM pipeline_tasks \
                 WHERE repo_path = ?1 \
                 ORDER BY id DESC"
            )
        } else {
            format!("SELECT {TASK_COLS} FROM pipeline_tasks ORDER BY id DESC")
        };
        let mut stmt = conn.prepare(&sql)?;
        let tasks = if let Some(repo_path) = repo_path {
            stmt.query_map(params![repo_path], row_to_task)?
                .collect::<pg::Result<Vec<_>>>()
        } else {
            stmt.query_map([], row_to_task)?
                .collect::<pg::Result<Vec<_>>>()
        }
        .context("list_all_tasks")?;
        Ok(tasks)
    }

    pub fn list_all_tasks_in_workspace(
        &self,
        workspace_id: i64,
        repo_path: Option<&str>,
    ) -> Result<Vec<Task>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = if repo_path.is_some() {
            format!(
                "SELECT {TASK_COLS} FROM pipeline_tasks \
                 WHERE workspace_id = ?1 AND repo_path = ?2 \
                 ORDER BY id DESC"
            )
        } else {
            format!(
                "SELECT {TASK_COLS} FROM pipeline_tasks WHERE workspace_id = ?1 ORDER BY id DESC"
            )
        };
        let mut stmt = conn.prepare(&sql)?;
        let tasks = if let Some(repo_path) = repo_path {
            stmt.query_map(params![workspace_id, repo_path], row_to_task)?
                .collect::<pg::Result<Vec<_>>>()
        } else {
            stmt.query_map(params![workspace_id], row_to_task)?
                .collect::<pg::Result<Vec<_>>>()
        }
        .context("list_all_tasks_in_workspace")?;
        Ok(tasks)
    }

    pub fn get_task_in_workspace(&self, workspace_id: i64, id: i64) -> Result<Option<Task>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let result = conn
            .query_row(
                &format!(
                    "SELECT {TASK_COLS} FROM pipeline_tasks WHERE id = ?1 AND workspace_id = ?2"
                ),
                params![id, workspace_id],
                row_to_task,
            )
            .optional()
            .context("get_task_in_workspace")?;
        Ok(result)
    }

    pub fn get_task_with_outputs(&self, id: i64) -> Result<Option<(Task, Vec<TaskOutput>)>> {
        let task = self.get_task(id)?;
        match task {
            None => Ok(None),
            Some(t) => {
                let outputs = self.get_task_outputs(id)?;
                Ok(Some((t, outputs)))
            },
        }
    }

    pub fn get_task_with_outputs_in_workspace(
        &self,
        workspace_id: i64,
        id: i64,
    ) -> Result<Option<(Task, Vec<TaskOutput>)>> {
        let task = self.get_task_in_workspace(workspace_id, id)?;
        match task {
            None => Ok(None),
            Some(t) => {
                let outputs = self.get_task_outputs(id)?;
                Ok(Some((t, outputs)))
            },
        }
    }

    // ── Chat message history ──────────────────────────────────────────────

    /// Insert a chat message (incoming or outgoing) into the messages table.
    pub fn insert_chat_message(
        &self,
        id: &str,
        chat_jid: &str,
        sender: Option<&str>,
        sender_name: Option<&str>,
        content: &str,
        is_from_me: bool,
        is_bot_message: bool,
    ) -> Result<()> {
        self.insert_chat_message_with_stream(
            id,
            chat_jid,
            sender,
            sender_name,
            content,
            is_from_me,
            is_bot_message,
            None,
        )
    }

    /// Insert a chat message with optional raw NDJSON stream for agent interactions.
    pub fn insert_chat_message_with_stream(
        &self,
        id: &str,
        chat_jid: &str,
        sender: Option<&str>,
        sender_name: Option<&str>,
        content: &str,
        is_from_me: bool,
        is_bot_message: bool,
        raw_stream: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let ts = now_str();
        conn.execute(
            "INSERT INTO messages (id, chat_jid, sender, sender_name, content, timestamp, is_from_me, is_bot_message, raw_stream) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT DO NOTHING",
            params![id, chat_jid, sender, sender_name, content, ts,
                    if is_from_me { 1i64 } else { 0i64 },
                    if is_bot_message { 1i64 } else { 0i64 },
                    raw_stream],
        )
        .context("insert_chat_message")?;
        Ok(())
    }

    /// List all chat threads (distinct chat_jid values) with msg count and last timestamp.
    pub fn get_chat_threads(&self) -> Result<Vec<(String, i64, String)>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT chat_jid, COUNT(*) as msg_count, MAX(timestamp) as last_ts \
             FROM messages GROUP BY chat_jid ORDER BY last_ts DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("get_chat_threads")?;
        Ok(rows)
    }

    /// Get messages for a specific chat thread, newest last.
    pub fn get_chat_messages(&self, chat_jid: &str, limit: i64) -> Result<Vec<ChatMessage>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, chat_jid, sender, sender_name, content, timestamp, is_from_me, is_bot_message, raw_stream \
             FROM messages WHERE chat_jid = ?1 ORDER BY timestamp ASC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![chat_jid, limit], |row| {
                Ok(ChatMessage {
                    id: row.get(0)?,
                    chat_jid: row.get(1)?,
                    sender: row.get(2)?,
                    sender_name: row.get(3)?,
                    content: row.get(4)?,
                    timestamp: row.get(5)?,
                    is_from_me: row.get::<_, i64>(6)? != 0,
                    is_bot_message: row.get::<_, i64>(7)? != 0,
                    raw_stream: row.get(8)?,
                })
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("get_chat_messages")?;
        Ok(rows)
    }

    // ── Registered groups ─────────────────────────────────────────────────

    pub fn get_all_groups(&self) -> Result<Vec<RegisteredGroup>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT jid, name, folder, trigger_pattern, requires_trigger FROM registered_groups ORDER BY added_at ASC",
        )?;
        let groups = stmt
            .query_map([], |row| {
                Ok(RegisteredGroup {
                    jid: row.get(0)?,
                    name: row.get(1)?,
                    folder: row.get(2)?,
                    trigger_pattern: row
                        .get::<_, Option<String>>(3)?
                        .unwrap_or_else(|| "@Borg".into()),
                    requires_trigger: row.get::<_, i64>(4)? != 0,
                })
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("get_all_groups")?;
        Ok(groups)
    }

    pub fn register_group(
        &self,
        jid: &str,
        name: &str,
        folder: &str,
        trigger_pattern: &str,
        requires_trigger: bool,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "INSERT INTO registered_groups (jid, name, folder, trigger_pattern, requires_trigger) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(jid) DO UPDATE SET name=excluded.name, folder=excluded.folder, \
               trigger_pattern=excluded.trigger_pattern, requires_trigger=excluded.requires_trigger",
            params![jid, name, folder, trigger_pattern, if requires_trigger { 1i64 } else { 0i64 }],
        )
        .context("register_group")?;
        Ok(())
    }

    pub fn unregister_group(&self, jid: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute("DELETE FROM registered_groups WHERE jid = ?1", params![jid])
            .context("unregister_group")?;
        Ok(())
    }

    // ── Chat sessions ─────────────────────────────────────────────────────

    pub fn get_session(&self, folder: &str) -> Result<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.query_row(
            "SELECT session_id FROM sessions WHERE folder = ?1",
            params![folder],
            |r| r.get(0),
        )
        .optional()
        .context("get_session")
    }

    pub fn set_session(&self, folder: &str, session_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "INSERT INTO sessions (folder, session_id, created_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(folder) DO UPDATE SET session_id=excluded.session_id, created_at=excluded.created_at",

            params![folder, session_id, now_str()],
        )
        .context("set_session")?;
        Ok(())
    }

    pub fn get_seed_cooldowns(&self) -> Result<HashMap<(String, String), i64>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn
            .prepare("SELECT folder, session_id FROM sessions WHERE folder LIKE 'seed:%'")
            .context("get_seed_cooldowns")?;
        let rows = stmt
            .query_map([], |r| {
                let folder: String = r.get(0)?;
                let ts: String = r.get(1)?;
                Ok((folder, ts))
            })
            .context("get_seed_cooldowns")?;
        let mut map = HashMap::new();
        for row in rows {
            if let Ok((folder, ts)) = row {
                let parts: Vec<&str> = folder.splitn(3, ':').collect();
                if parts.len() == 3 {
                    if let Ok(t) = ts.parse::<i64>() {
                        map.insert((parts[1].to_string(), parts[2].to_string()), t);
                    }
                }
            }
        }
        Ok(map)
    }

    pub fn set_seed_cooldown(&self, repo_path: &str, seed_name: &str, ts: i64) -> Result<()> {
        let folder = format!("seed:{repo_path}:{seed_name}");
        self.set_session(&folder, &ts.to_string())
    }

    pub fn expire_sessions(&self, max_age_hours: i64) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let n = conn
            .execute(
                "DELETE FROM sessions \
                 WHERE NULLIF(created_at, '') IS NOT NULL \
                   AND created_at::timestamp < (timezone('UTC', now()) - make_interval(hours => ?1::int))",
                params![max_age_hours],
            )
            .context("expire_sessions")?;
        Ok(n)
    }

    // ── Chat agent runs ───────────────────────────────────────────────────

    pub fn create_chat_agent_run(
        &self,
        jid: &str,
        transport: &str,
        original_id: &str,
        trigger_msg_id: &str,
        folder: &str,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let id = conn.execute_returning_id(
            "INSERT INTO chat_agent_runs (jid, status, transport, original_id, trigger_msg_id, folder) \
             VALUES (?1, 'running', ?2, ?3, ?4, ?5)",
            params![jid, transport, original_id, trigger_msg_id, folder],
        )
        .context("create_chat_agent_run")?;
        Ok(id)
    }

    pub fn complete_chat_agent_run(
        &self,
        id: i64,
        output: &str,
        new_session_id: &str,
        last_msg_timestamp: &str,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE chat_agent_runs SET status='completed', output=?1, new_session_id=?2, \
             last_msg_timestamp=?3, completed_at=?4 WHERE id=?5",
            params![output, new_session_id, last_msg_timestamp, now_str(), id],
        )
        .context("complete_chat_agent_run")?;
        Ok(())
    }

    pub fn mark_chat_agent_run_delivered(&self, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE chat_agent_runs SET status='delivered' WHERE id=?1",
            params![id],
        )
        .context("mark_chat_agent_run_delivered")?;
        Ok(())
    }

    pub fn get_undelivered_runs(&self, jid: &str) -> Result<Vec<ChatAgentRun>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, jid, status, transport, original_id, trigger_msg_id, folder, \
             output, new_session_id, last_msg_timestamp, started_at, completed_at \
             FROM chat_agent_runs WHERE jid=?1 AND status='completed' ORDER BY id ASC",
        )?;
        let runs = stmt
            .query_map(params![jid], row_to_chat_agent_run)?
            .collect::<pg::Result<Vec<_>>>()
            .context("get_undelivered_runs")?;
        Ok(runs)
    }

    pub fn fail_chat_agent_run(&self, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE chat_agent_runs SET status='failed', completed_at=?1 WHERE id=?2",
            params![now_str(), id],
        )
        .context("fail_chat_agent_run")?;
        Ok(())
    }

    pub fn has_running_chat_agent(&self, jid: &str) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chat_agent_runs WHERE jid=?1 AND status='running'",
                params![jid],
                |row| row.get(0),
            )
            .context("has_running_chat_agent")?;
        Ok(count > 0)
    }

    pub fn abandon_running_agents(&self) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let n = conn
            .execute(
                "UPDATE chat_agent_runs SET status='abandoned' WHERE status='running'",
                [],
            )
            .context("abandon_running_agents")?;
        Ok(n)
    }

    pub fn get_messages_since(
        &self,
        chat_jid: &str,
        since_ts: &str,
        limit: i64,
    ) -> Result<Vec<ChatMessage>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, chat_jid, sender, sender_name, content, timestamp, is_from_me, is_bot_message, raw_stream \
             FROM messages WHERE chat_jid=?1 AND timestamp > ?2 ORDER BY timestamp ASC LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![chat_jid, since_ts, limit], |row| {
                Ok(ChatMessage {
                    id: row.get(0)?,
                    chat_jid: row.get(1)?,
                    sender: row.get(2)?,
                    sender_name: row.get(3)?,
                    content: row.get(4)?,
                    timestamp: row.get(5)?,
                    is_from_me: row.get::<_, i64>(6)? != 0,
                    is_bot_message: row.get::<_, i64>(7)? != 0,
                    raw_stream: row.get(8)?,
                })
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("get_messages_since")?;
        Ok(rows)
    }

    // ── Events query ──────────────────────────────────────────────────────

    /// Query the legacy events table with optional filters.
    pub fn get_events_filtered(
        &self,
        category: Option<&str>,
        level: Option<&str>,
        since_ts: Option<i64>,
        limit: i64,
    ) -> Result<Vec<LegacyEvent>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut where_clauses = Vec::new();
        let mut params_vec: Vec<Box<dyn pg::types::ToSql>> = Vec::new();
        if let Some(category) = category.map(str::trim).filter(|c| !c.is_empty()) {
            where_clauses.push("category = ?".to_string());
            params_vec.push(Box::new(category.to_string()));
        }
        if let Some(level) = level.map(str::trim).filter(|l| !l.is_empty()) {
            where_clauses.push("level = ?".to_string());
            params_vec.push(Box::new(level.to_string()));
        }
        if let Some(since_ts) = since_ts {
            where_clauses.push("ts >= ?".to_string());
            params_vec.push(Box::new(since_ts));
        }
        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };
        params_vec.push(Box::new(limit));
        let param_refs: Vec<&dyn pg::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let sql = format!(
            "SELECT id, ts, level, category, message, metadata FROM events \
             {where_sql} \
             ORDER BY ts DESC, id DESC LIMIT ?"
        );
        let mut stmt = conn.prepare(&sql)?;
        let events = stmt
            .query_map(param_refs.as_slice(), row_to_legacy_event)?
            .collect::<pg::Result<Vec<_>>>()
            .context("get_events_filtered")?;
        Ok(events)
    }

    // ── API Keys (BYOK) ──────────────────────────────────────────────────

    fn block_on_async_option<F, T>(fut: F) -> Option<T>
    where
        F: std::future::Future<Output = Option<T>>,
    {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            tokio::task::block_in_place(|| handle.block_on(fut))
        } else {
            let rt = tokio::runtime::Runtime::new().ok()?;
            rt.block_on(fut)
        }
    }

    fn decode_master_key_hex(key_hex: &str) -> Option<[u8; 32]> {
        if key_hex.len() != 64 {
            return None;
        }
        let key_bytes = hex::decode(key_hex).ok()?;
        if key_bytes.len() != 32 {
            return None;
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&key_bytes);
        Some(out)
    }

    fn load_master_key_from_kms() -> Option<[u8; 32]> {
        use aws_config::{BehaviorVersion, Region};
        use aws_sdk_kms::primitives::Blob;

        let ciphertext_b64 = std::env::var("BORG_MASTER_KEY_KMS_CIPHERTEXT_B64").ok()?;
        let ciphertext = {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(ciphertext_b64)
                .ok()?
        };
        if ciphertext.is_empty() {
            return None;
        }

        Self::block_on_async_option(async move {
            let region = std::env::var("BORG_MASTER_KEY_KMS_REGION")
                .ok()
                .filter(|r| !r.trim().is_empty())
                .or_else(|| std::env::var("AWS_REGION").ok());

            let mut loader = aws_config::defaults(BehaviorVersion::latest());
            if let Some(region) = region {
                loader = loader.region(Region::new(region));
            }
            let shared = loader.load().await;
            let client = aws_sdk_kms::Client::new(&shared);

            let mut req = client.decrypt().ciphertext_blob(Blob::new(ciphertext));
            if let Ok(key_id) = std::env::var("BORG_MASTER_KEY_KMS_KEY_ID") {
                if !key_id.trim().is_empty() {
                    req = req.key_id(key_id);
                }
            }
            let out = req.send().await.ok()?;
            let plaintext = out.plaintext()?.as_ref();
            if plaintext.len() != 32 {
                return None;
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(plaintext);
            Some(key)
        })
    }

    fn master_key_bytes() -> Option<[u8; 32]> {
        static MASTER_KEY_CACHE: std::sync::OnceLock<Option<[u8; 32]>> = std::sync::OnceLock::new();
        *MASTER_KEY_CACHE.get_or_init(|| {
            if let Ok(key_hex) = std::env::var("BORG_MASTER_KEY") {
                if let Some(key) = Self::decode_master_key_hex(&key_hex) {
                    return Some(key);
                }
                tracing::warn!("BORG_MASTER_KEY is set but invalid (expected 64-char hex)");
            }
            let kms_key = Self::load_master_key_from_kms();
            if kms_key.is_none() && std::env::var("BORG_MASTER_KEY_KMS_CIPHERTEXT_B64").is_ok() {
                tracing::warn!("failed to resolve master key from AWS KMS ciphertext");
            }
            kms_key
        })
    }

    fn encrypt_secret(secret: &str) -> String {
        if let Some(key_bytes) = Self::master_key_bytes() {
            use aes_gcm::{
                aead::{Aead, AeadCore, KeyInit, OsRng},
                Aes256Gcm,
            };
            let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&key_bytes);
            let cipher = Aes256Gcm::new(key);
            let nonce = Aes256Gcm::generate_nonce(&mut OsRng); // 96-bits
            if let Ok(ciphertext) = cipher.encrypt(&nonce, secret.as_bytes()) {
                let mut combined = nonce.to_vec();
                combined.extend_from_slice(&ciphertext);
                use base64::Engine;
                return format!(
                    "enc:v1:{}",
                    base64::engine::general_purpose::STANDARD.encode(&combined)
                );
            }
        }
        secret.to_string()
    }

    fn decrypt_secret(secret: &str) -> String {
        if secret.starts_with("enc:v1:") {
            if let Some(key_bytes) = Self::master_key_bytes() {
                use base64::Engine;
                if let Ok(combined) = base64::engine::general_purpose::STANDARD.decode(&secret[7..])
                {
                    if combined.len() > 12 {
                        use aes_gcm::{
                            aead::{Aead, KeyInit},
                            Aes256Gcm, Nonce,
                        };
                        let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&key_bytes);
                        let cipher = Aes256Gcm::new(key);
                        let nonce = Nonce::from_slice(&combined[..12]);
                        if let Ok(plaintext) = cipher.decrypt(nonce, &combined[12..]) {
                            if let Ok(s) = String::from_utf8(plaintext) {
                                return s;
                            }
                        }
                    }
                }
            }
        }
        secret.to_string()
    }

    pub fn store_api_key(
        &self,
        owner: &str,
        provider: &str,
        key_name: &str,
        key_value: &str,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let encrypted_value = Self::encrypt_secret(key_value);
        let id = conn.execute_returning_id(
            "INSERT INTO api_keys (owner, provider, key_name, key_value) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(owner, provider) DO UPDATE SET key_name=excluded.key_name, key_value=excluded.key_value",
            params![owner, provider, key_name, encrypted_value],
        )?;
        Ok(id)
    }

    pub fn store_workspace_api_key(
        &self,
        workspace_id: i64,
        provider: &str,
        key_name: &str,
        key_value: &str,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let encrypted_value = Self::encrypt_secret(key_value);
        let owner = format!("workspace:{workspace_id}");
        let id = conn.execute_returning_id(
            "INSERT INTO api_keys (workspace_id, owner, provider, key_name, key_value) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(owner, provider) DO UPDATE SET workspace_id=excluded.workspace_id, key_name=excluded.key_name, key_value=excluded.key_value",
            params![workspace_id, owner, provider, key_name, encrypted_value],
        )?;
        Ok(id)
    }

    pub fn get_api_key(&self, owner: &str, provider: &str) -> Result<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        // Try owner-specific first, then fall back to global
        let result = conn
            .query_row(
                "SELECT key_value FROM api_keys WHERE owner = ?1 AND provider = ?2",
                params![owner, provider],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("get_api_key")?;
        if let Some(val) = result {
            return Ok(Some(Self::decrypt_secret(&val)));
        }
        if owner != "global" {
            let global = conn
                .query_row(
                    "SELECT key_value FROM api_keys WHERE owner = 'global' AND provider = ?1",
                    params![provider],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .context("get_api_key global fallback")?;
            if let Some(val) = global {
                return Ok(Some(Self::decrypt_secret(&val)));
            }
        }
        Ok(None)
    }

    pub fn get_api_key_exact(&self, owner: &str, provider: &str) -> Result<Option<String>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let result = conn
            .query_row(
                "SELECT key_value FROM api_keys WHERE owner = ?1 AND provider = ?2",
                params![owner, provider],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("get_api_key_exact")?;
        Ok(result.map(|val| Self::decrypt_secret(&val)))
    }

    pub fn list_api_keys(&self, owner: &str) -> Result<Vec<ApiKeyEntry>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, owner, provider, key_name, created_at FROM api_keys \
             WHERE owner = ?1 OR owner = 'global' ORDER BY provider",
        )?;
        let keys = stmt
            .query_map(params![owner], |row| {
                Ok(ApiKeyEntry {
                    id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    owner: row.get(2)?,
                    provider: row.get(3)?,
                    key_name: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_api_keys")?;
        Ok(keys)
    }

    pub fn list_workspace_api_keys(&self, workspace_id: i64) -> Result<Vec<ApiKeyEntry>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let owner = format!("workspace:{workspace_id}");
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, owner, provider, key_name, created_at FROM api_keys \
             WHERE owner = ?1 ORDER BY provider",
        )?;
        let keys = stmt
            .query_map(params![owner], |row| {
                Ok(ApiKeyEntry {
                    id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    owner: row.get(2)?,
                    provider: row.get(3)?,
                    key_name: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_workspace_api_keys")?;
        Ok(keys)
    }

    pub fn delete_api_key(&self, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute("DELETE FROM api_keys WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn delete_workspace_api_key(&self, workspace_id: i64, id: i64) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let owner = format!("workspace:{workspace_id}");
        conn.execute(
            "DELETE FROM api_keys WHERE id = ?1 AND owner = ?2",
            params![id, owner],
        )?;
        Ok(())
    }

    pub fn list_user_linked_credentials(&self, user_id: i64) -> Result<Vec<LinkedCredentialEntry>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, user_id, provider, auth_kind, account_email, account_label, status, \
                    expires_at, last_validated_at, last_used_at, last_error, created_at, updated_at \
             FROM linked_credentials WHERE user_id = ?1 ORDER BY provider",
        )?;
        let rows = stmt
            .query_map(params![user_id], |row| {
                Ok(LinkedCredentialEntry {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    provider: row.get(2)?,
                    auth_kind: row.get(3)?,
                    account_email: row.get(4)?,
                    account_label: row.get(5)?,
                    status: row.get(6)?,
                    expires_at: row.get(7)?,
                    last_validated_at: row.get(8)?,
                    last_used_at: row.get(9)?,
                    last_error: row.get(10)?,
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                })
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_user_linked_credentials")?;
        Ok(rows)
    }

    pub fn list_all_linked_credentials(&self) -> Result<Vec<LinkedCredentialEntry>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, user_id, provider, auth_kind, account_email, account_label, status, \
                    expires_at, last_validated_at, last_used_at, last_error, created_at, updated_at \
             FROM linked_credentials ORDER BY user_id, provider",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(LinkedCredentialEntry {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    provider: row.get(2)?,
                    auth_kind: row.get(3)?,
                    account_email: row.get(4)?,
                    account_label: row.get(5)?,
                    status: row.get(6)?,
                    expires_at: row.get(7)?,
                    last_validated_at: row.get(8)?,
                    last_used_at: row.get(9)?,
                    last_error: row.get(10)?,
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                })
            })?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_all_linked_credentials")?;
        Ok(rows)
    }

    pub fn get_user_linked_credential(
        &self,
        user_id: i64,
        provider: &str,
    ) -> Result<Option<LinkedCredentialSecret>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let row = conn
            .query_row(
                "SELECT id, user_id, provider, auth_kind, account_email, account_label, status, \
                        expires_at, last_validated_at, last_used_at, last_error, created_at, updated_at, credential_bundle \
                 FROM linked_credentials WHERE user_id = ?1 AND provider = ?2",
                params![user_id, provider],
                |row| {
                    Ok((
                        LinkedCredentialEntry {
                            id: row.get(0)?,
                            user_id: row.get(1)?,
                            provider: row.get(2)?,
                            auth_kind: row.get(3)?,
                            account_email: row.get(4)?,
                            account_label: row.get(5)?,
                            status: row.get(6)?,
                            expires_at: row.get(7)?,
                            last_validated_at: row.get(8)?,
                            last_used_at: row.get(9)?,
                            last_error: row.get(10)?,
                            created_at: row.get(11)?,
                            updated_at: row.get(12)?,
                        },
                        row.get::<_, String>(13)?,
                    ))
                },
            )
            .optional()
            .context("get_user_linked_credential")?;
        let Some((entry, encrypted_bundle)) = row else {
            return Ok(None);
        };
        let bundle_json = Self::decrypt_secret(&encrypted_bundle);
        let bundle = serde_json::from_str::<LinkedCredentialBundle>(&bundle_json)
            .context("decode linked credential bundle")?;
        Ok(Some(LinkedCredentialSecret { entry, bundle }))
    }

    pub fn upsert_user_linked_credential(
        &self,
        user_id: i64,
        provider: &str,
        auth_kind: &str,
        account_email: &str,
        account_label: &str,
        status: &str,
        expires_at: &str,
        last_validated_at: &str,
        last_used_at: &str,
        last_error: &str,
        bundle: &LinkedCredentialBundle,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let bundle_json =
            serde_json::to_string(bundle).context("encode linked credential bundle")?;
        let encrypted_bundle = Self::encrypt_secret(&bundle_json);
        let id = conn.execute_returning_id(
            "INSERT INTO linked_credentials \
                (user_id, provider, auth_kind, account_email, account_label, credential_bundle, \
                 status, expires_at, last_validated_at, last_used_at, last_error, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS')) \
             ON CONFLICT(user_id, provider) DO UPDATE SET \
                 auth_kind = excluded.auth_kind, \
                 account_email = excluded.account_email, \
                 account_label = excluded.account_label, \
                 credential_bundle = excluded.credential_bundle, \
                 status = excluded.status, \
                 expires_at = excluded.expires_at, \
                 last_validated_at = excluded.last_validated_at, \
                 last_used_at = excluded.last_used_at, \
                 last_error = excluded.last_error, \
                 updated_at = to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS')",
            params![
                user_id,
                provider,
                auth_kind,
                account_email,
                account_label,
                encrypted_bundle,
                status,
                expires_at,
                last_validated_at,
                last_used_at,
                last_error
            ],
        )?;
        Ok(id)
    }

    pub fn update_user_linked_credential_state(
        &self,
        user_id: i64,
        provider: &str,
        auth_kind: &str,
        account_email: &str,
        account_label: &str,
        status: &str,
        expires_at: &str,
        last_validated_at: &str,
        last_error: &str,
        bundle: Option<&LinkedCredentialBundle>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let encrypted_bundle = match bundle {
            Some(bundle) => {
                let bundle_json =
                    serde_json::to_string(bundle).context("encode linked credential bundle")?;
                Some(Self::encrypt_secret(&bundle_json))
            },
            None => None,
        };
        conn.execute(
            "UPDATE linked_credentials SET \
                 auth_kind = ?3, \
                 account_email = ?4, \
                 account_label = ?5, \
                 status = ?6, \
                 expires_at = ?7, \
                 last_validated_at = ?8, \
                 last_error = ?9, \
                 credential_bundle = COALESCE(?10, credential_bundle), \
                 updated_at = to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS') \
             WHERE user_id = ?1 AND provider = ?2",
            params![
                user_id,
                provider,
                auth_kind,
                account_email,
                account_label,
                status,
                expires_at,
                last_validated_at,
                last_error,
                encrypted_bundle
            ],
        )?;
        Ok(())
    }

    pub fn touch_user_linked_credential_used(&self, user_id: i64, provider: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE linked_credentials SET last_used_at = ?3, updated_at = to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS') \
             WHERE user_id = ?1 AND provider = ?2",
            params![user_id, provider, now],
        )?;
        Ok(())
    }

    pub fn delete_user_linked_credential(&self, user_id: i64, provider: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "DELETE FROM linked_credentials WHERE user_id = ?1 AND provider = ?2",
            params![user_id, provider],
        )?;
        Ok(())
    }

    // ── Cron scheduling ───────────────────────────────────────────────────

    pub fn list_cron_jobs(&self) -> Result<Vec<crate::cron::CronJob>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, name, schedule, job_type, config, project_id, enabled, \
             last_run, next_run, created_at \
             FROM cron_jobs ORDER BY id ASC",
        )?;
        let rows = stmt
            .query_map([], crate::cron::row_to_cron_job)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_cron_jobs")?;
        Ok(rows)
    }

    pub fn get_cron_job(&self, id: i64) -> Result<Option<crate::cron::CronJob>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.query_row(
            "SELECT id, name, schedule, job_type, config, project_id, enabled, \
             last_run, next_run, created_at \
             FROM cron_jobs WHERE id = ?1",
            params![id],
            crate::cron::row_to_cron_job,
        )
        .optional()
        .context("get_cron_job")
    }

    pub fn insert_cron_job(
        &self,
        name: &str,
        schedule: &str,
        job_type: &crate::cron::CronJobType,
        config: &serde_json::Value,
        project_id: Option<i64>,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let config_str = serde_json::to_string(config).unwrap_or_else(|_| "{}".into());
        let next_run = crate::cron::compute_next_run(schedule, Utc::now())
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());
        let id = conn
            .execute_returning_id(
                "INSERT INTO cron_jobs (name, schedule, job_type, config, project_id, next_run) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    name,
                    schedule,
                    job_type.as_str(),
                    config_str,
                    project_id,
                    next_run
                ],
            )
            .context("insert_cron_job")?;
        Ok(id)
    }

    pub fn update_cron_job(
        &self,
        id: i64,
        name: Option<&str>,
        schedule: Option<&str>,
        job_type: Option<&crate::cron::CronJobType>,
        config: Option<&serde_json::Value>,
        project_id: Option<Option<i64>>,
        enabled: Option<bool>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut sets = Vec::new();
        let mut vals: Vec<Box<dyn pg::ToSql>> = Vec::new();
        let mut idx = 1;

        if let Some(v) = name {
            sets.push(format!("name = ?{idx}"));
            vals.push(Box::new(v.to_string()));
            idx += 1;
        }
        if let Some(v) = schedule {
            sets.push(format!("schedule = ?{idx}"));
            vals.push(Box::new(v.to_string()));
            idx += 1;
            let next = crate::cron::compute_next_run(v, Utc::now())
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());
            sets.push(format!("next_run = ?{idx}"));
            vals.push(Box::new(next));
            idx += 1;
        }
        if let Some(v) = job_type {
            sets.push(format!("job_type = ?{idx}"));
            vals.push(Box::new(v.as_str().to_string()));
            idx += 1;
        }
        if let Some(v) = config {
            sets.push(format!("config = ?{idx}"));
            vals.push(Box::new(serde_json::to_string(v).unwrap_or_else(|_| "{}".into())));
            idx += 1;
        }
        if let Some(v) = project_id {
            sets.push(format!("project_id = ?{idx}"));
            vals.push(Box::new(v));
            idx += 1;
        }
        if let Some(v) = enabled {
            sets.push(format!("enabled = ?{idx}"));
            vals.push(Box::new(if v { 1i64 } else { 0i64 }));
            idx += 1;
        }

        if sets.is_empty() {
            return Ok(());
        }

        let sql = format!("UPDATE cron_jobs SET {} WHERE id = ?{}", sets.join(", "), idx);
        vals.push(Box::new(id));
        let params: Vec<&dyn pg::ToSql> = vals.iter().map(|v| v.as_ref()).collect();
        conn.execute(&sql, params.as_slice())
            .context("update_cron_job")?;
        Ok(())
    }

    pub fn delete_cron_job(&self, id: i64) -> Result<bool> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let n = conn
            .execute("DELETE FROM cron_jobs WHERE id = ?1", params![id])
            .context("delete_cron_job")?;
        Ok(n > 0)
    }

    pub fn list_due_cron_jobs(&self) -> Result<Vec<crate::cron::CronJob>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let now = now_str();
        let mut stmt = conn.prepare(
            "SELECT id, name, schedule, job_type, config, project_id, enabled, \
             last_run, next_run, created_at \
             FROM cron_jobs \
             WHERE enabled = 1 AND next_run IS NOT NULL AND next_run <= ?1 \
             ORDER BY next_run ASC",
        )?;
        let rows = stmt
            .query_map(params![now], crate::cron::row_to_cron_job)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_due_cron_jobs")?;
        Ok(rows)
    }

    pub fn update_cron_job_after_run(
        &self,
        id: i64,
        last_run: &DateTime<Utc>,
        next_run: Option<&DateTime<Utc>>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let last_str = last_run.format("%Y-%m-%d %H:%M:%S").to_string();
        let next_str = next_run.map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());
        conn.execute(
            "UPDATE cron_jobs SET last_run = ?1, next_run = ?2 WHERE id = ?3",
            params![last_str, next_str, id],
        )
        .context("update_cron_job_after_run")?;
        Ok(())
    }

    pub fn insert_cron_run(&self, job_id: i64) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let id = conn
            .execute_returning_id(
                "INSERT INTO cron_runs (job_id, status) VALUES (?1, 'running')",
                params![job_id],
            )
            .context("insert_cron_run")?;
        Ok(id)
    }

    pub fn update_cron_run(
        &self,
        id: i64,
        status: &str,
        result: Option<&str>,
        error: Option<&str>,
        task_id: Option<i64>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let finished = now_str();
        conn.execute(
            "UPDATE cron_runs SET status = ?1, result = ?2, error = ?3, \
             finished_at = ?4, task_id = ?5 WHERE id = ?6",
            params![status, result, error, finished, task_id, id],
        )
        .context("update_cron_run")?;
        Ok(())
    }

    pub fn list_cron_runs(&self, job_id: i64, limit: i64) -> Result<Vec<crate::cron::CronRun>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, job_id, started_at, finished_at, status, result, error, task_id \
             FROM cron_runs WHERE job_id = ?1 \
             ORDER BY started_at DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![job_id, limit], crate::cron::row_to_cron_run)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_cron_runs")?;
        Ok(rows)
    }

    // ── Cost Tracking ────────────────────────────────────────────────────

    pub fn update_message_usage(
        &self,
        message_id: &str,
        chat_jid: &str,
        input_tokens: i64,
        output_tokens: i64,
        cost_usd: f64,
        model: &str,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE messages SET input_tokens = ?1, output_tokens = ?2, \
             cost_usd = ?3, model = ?4 WHERE chat_jid = ?5 AND id = ?6",
            params![input_tokens, output_tokens, cost_usd, model, chat_jid, message_id],
        )
        .context("update_message_usage")?;
        Ok(())
    }

    pub fn accumulate_task_usage(
        &self,
        task_id: i64,
        input_tokens: i64,
        output_tokens: i64,
        cost_usd: f64,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET \
             total_input_tokens = COALESCE(total_input_tokens, 0) + ?1, \
             total_output_tokens = COALESCE(total_output_tokens, 0) + ?2, \
             total_cost_usd = COALESCE(total_cost_usd, 0) + ?3, \
             updated_at = ?4 WHERE id = ?5",
            params![input_tokens, output_tokens, cost_usd, now_str(), task_id],
        )
        .context("accumulate_task_usage")?;
        Ok(())
    }

    pub fn get_usage_summary(
        &self,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
    ) -> Result<UsageSummary> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;

        let mut where_clauses = Vec::new();
        let mut params_vec: Vec<Box<dyn pg::types::ToSql>> = Vec::new();

        if let Some(from) = from {
            where_clauses.push("timestamp >= ?".to_string());
            params_vec.push(Box::new(from.format("%Y-%m-%d %H:%M:%S").to_string()));
        }
        if let Some(to) = to {
            where_clauses.push("timestamp <= ?".to_string());
            params_vec.push(Box::new(to.format("%Y-%m-%d %H:%M:%S").to_string()));
        }
        where_clauses.push("input_tokens IS NOT NULL".to_string());

        let where_sql = format!(" WHERE {}", where_clauses.join(" AND "));
        let param_refs: Vec<&dyn pg::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();

        let msg_sql = format!(
            "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), \
             COALESCE(SUM(cost_usd), 0), COUNT(*) FROM messages{where_sql}"
        );
        let (msg_input, msg_output, msg_cost, msg_count): (i64, i64, f64, i64) = conn
            .query_row(&msg_sql, param_refs.as_slice(), |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .context("get_usage_summary messages")?;

        let mut task_where = Vec::new();
        let mut task_params: Vec<Box<dyn pg::types::ToSql>> = Vec::new();
        if let Some(from) = from {
            task_where.push("created_at >= ?".to_string());
            task_params.push(Box::new(from.format("%Y-%m-%d %H:%M:%S").to_string()));
        }
        if let Some(to) = to {
            task_where.push("created_at <= ?".to_string());
            task_params.push(Box::new(to.format("%Y-%m-%d %H:%M:%S").to_string()));
        }
        task_where.push("total_input_tokens > 0".to_string());

        let task_where_sql = format!(" WHERE {}", task_where.join(" AND "));
        let task_param_refs: Vec<&dyn pg::types::ToSql> =
            task_params.iter().map(|p| p.as_ref()).collect();

        let task_sql = format!(
            "SELECT COALESCE(SUM(total_input_tokens), 0), COALESCE(SUM(total_output_tokens), 0), \
             COALESCE(SUM(total_cost_usd), 0), COUNT(*) FROM pipeline_tasks{task_where_sql}"
        );
        let (task_input, task_output, task_cost, task_count): (i64, i64, f64, i64) = conn
            .query_row(&task_sql, task_param_refs.as_slice(), |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .context("get_usage_summary tasks")?;

        Ok(UsageSummary {
            total_input_tokens: msg_input + task_input,
            total_output_tokens: msg_output + task_output,
            total_cost_usd: msg_cost + task_cost,
            message_count: msg_count,
            task_count,
        })
    }

    // ── Tool Call Tracking ────────────────────────────────────────────────

    pub fn insert_tool_call(
        &self,
        run_id: &str,
        tool_name: &str,
        task_id: Option<i64>,
        chat_key: Option<&str>,
        input_summary: Option<&str>,
    ) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let id = conn.execute_returning_id(
            "INSERT INTO tool_calls (run_id, tool_name, task_id, chat_key, input_summary) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run_id, tool_name, task_id, chat_key, input_summary],
        )
        .context("insert_tool_call")?;
        Ok(id)
    }

    pub fn complete_tool_call(
        &self,
        id: i64,
        output_summary: Option<&str>,
        duration_ms: i64,
        success: bool,
        error: Option<&str>,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE tool_calls SET output_summary = ?1, duration_ms = ?2, \
             success = ?3, error = ?4 WHERE id = ?5",
            params![output_summary, duration_ms, success, error, id],
        )
        .context("complete_tool_call")?;
        Ok(())
    }

    pub fn list_tool_calls_by_task(
        &self,
        task_id: i64,
        limit: i64,
    ) -> Result<Vec<crate::tool_calls::ToolCallEvent>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, chat_key, run_id, tool_name, input_summary, \
             output_summary, started_at, duration_ms, success, error \
             FROM tool_calls WHERE task_id = ?1 \
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![task_id, limit], row_to_tool_call)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_tool_calls_by_task")?;
        Ok(rows)
    }

    pub fn list_tool_calls_by_chat(
        &self,
        chat_key: &str,
        limit: i64,
    ) -> Result<Vec<crate::tool_calls::ToolCallEvent>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, chat_key, run_id, tool_name, input_summary, \
             output_summary, started_at, duration_ms, success, error \
             FROM tool_calls WHERE chat_key = ?1 \
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![chat_key, limit], row_to_tool_call)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_tool_calls_by_chat")?;
        Ok(rows)
    }

    pub fn list_tool_calls_by_run(
        &self,
        run_id: &str,
        limit: i64,
    ) -> Result<Vec<crate::tool_calls::ToolCallEvent>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, chat_key, run_id, tool_name, input_summary, \
             output_summary, started_at, duration_ms, success, error \
             FROM tool_calls WHERE run_id = ?1 \
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![run_id, limit], row_to_tool_call)?
            .collect::<pg::Result<Vec<_>>>()
            .context("list_tool_calls_by_run")?;
        Ok(rows)
    }
}
