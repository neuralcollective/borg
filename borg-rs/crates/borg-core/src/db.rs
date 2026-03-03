use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json;

use crate::types::{Proposal, QueueEntry, Task};

const SCHEMA_SQL: &str = include_str!("../../../schema.sql");

pub struct Db {
    conn: Mutex<Connection>,
}

// ── Auxiliary types ───────────────────────────────────────────────────────

pub struct TaskOutput {
    pub id: i64,
    pub task_id: i64,
    pub phase: String,
    pub output: String,
    pub raw_stream: String,
    pub exit_code: i64,
    pub created_at: DateTime<Utc>,
}

pub struct TaskMessage {
    pub id: i64,
    pub task_id: i64,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub delivered_phase: Option<String>,
}

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
}

#[derive(serde::Serialize)]
pub struct ApiKeyEntry {
    pub id: i64,
    pub owner: String,
    pub provider: String,
    pub key_name: String,
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

#[derive(serde::Serialize)]
pub struct ProjectRow {
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
    pub created_at: DateTime<Utc>,
}

#[derive(serde::Serialize, Clone)]
pub struct ProjectFileRow {
    pub id: i64,
    pub project_id: i64,
    pub file_name: String,
    pub stored_path: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub extracted_text: String,
    pub content_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct KnowledgeFile {
    pub id: i64,
    pub file_name: String,
    pub description: String,
    pub size_bytes: i64,
    pub inline: bool,
    pub tags: String,
    pub category: String,
    pub jurisdiction: String,
    pub project_id: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, serde::Serialize, Clone)]
pub struct ConflictHit {
    pub project_id: i64,
    pub project_name: String,
    pub party_name: String,
    pub party_role: String,
    pub matched_field: String,
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
pub struct DeadlineRow {
    pub id: i64,
    pub project_id: i64,
    pub label: String,
    pub due_date: String,
    pub rule_basis: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
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

fn row_to_knowledge(row: &rusqlite::Row<'_>) -> rusqlite::Result<KnowledgeFile> {
    let inline_int: i64 = row.get(4)?;
    Ok(KnowledgeFile {
        id: row.get(0)?,
        file_name: row.get(1)?,
        description: row.get(2)?,
        size_bytes: row.get(3)?,
        inline: inline_int != 0,
        created_at: row.get(5)?,
        tags: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        category: row.get::<_, Option<String>>(7)?.unwrap_or_else(|| "general".to_string()),
        jurisdiction: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
        project_id: row.get::<_, Option<i64>>(9)?,
    })
}

fn normalize_party_name(name: &str) -> String {
    let lower = name.to_lowercase();
    let stripped: String = lower
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' })
        .collect();
    let tokens: Vec<&str> = stripped.split_whitespace()
        .filter(|t| !matches!(*t, "inc" | "llc" | "ltd" | "corp" | "co" | "plc" | "the" | "of" | "and"))
        .collect();
    tokens.join(" ")
}

fn is_stopword(token: &str) -> bool {
    matches!(
        token,
        "a" | "an" | "and" | "are" | "as" | "at" | "be" | "been" | "being" | "but" | "by"
            | "can" | "could" | "did" | "do" | "does" | "for" | "from" | "had" | "has"
            | "have" | "if" | "in" | "into" | "is" | "it" | "its" | "may" | "might" | "must"
            | "not" | "of" | "on" | "or" | "our" | "shall" | "should" | "that" | "the"
            | "their" | "there" | "these" | "they" | "this" | "those" | "to" | "under"
            | "upon" | "was" | "were" | "will" | "with" | "would" | "you" | "your"
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

fn push_theme_term(
    out: &mut Vec<ThemeTerm>,
    term: String,
    occurrences: i64,
    document_count: i64,
) {
    out.push(ThemeTerm {
        term,
        occurrences,
        document_count,
    });
}

fn row_to_cloud_connection(row: &rusqlite::Row<'_>) -> rusqlite::Result<CloudConnection> {
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

fn row_to_upload_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<UploadSession> {
    let is_zip: i64 = row.get(8)?;
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
        status: row.get(9)?,
        stored_path: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
        error: row.get::<_, Option<String>>(11)?.unwrap_or_default(),
        created_at: row.get::<_, Option<String>>(12)?.unwrap_or_default(),
        updated_at: row.get::<_, Option<String>>(13)?.unwrap_or_default(),
    })
}

// ── Row mappers ───────────────────────────────────────────────────────────

const TASK_COLS: &str = "id, title, description, repo_path, branch, status, attempt, \
    max_attempts, last_error, created_by, notify_chat, created_at, \
    session_id, mode, backend, project_id, task_type, started_at, completed_at, duration_secs, \
    review_status, revision_count";

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let created_at_str: String = row.get(11)?;
    let started_at: Option<String> = row.get(17)?;
    let completed_at: Option<String> = row.get(18)?;
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
        session_id: row.get(12)?,
        mode: row.get(13)?,
        backend: row.get::<_, Option<String>>(14)?.unwrap_or_default(),
        project_id: row.get::<_, Option<i64>>(15)?.unwrap_or(0),
        task_type: row.get::<_, Option<String>>(16)?.unwrap_or_default(),
        started_at: started_at.map(|s| parse_ts(&s)),
        completed_at: completed_at.map(|s| parse_ts(&s)),
        duration_secs: row.get(19)?,
        review_status: row.get(20)?,
        revision_count: row.get::<_, Option<i64>>(21)?.unwrap_or(0),
    })
}

fn row_to_proposal(row: &rusqlite::Row<'_>) -> rusqlite::Result<Proposal> {
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

fn row_to_queue_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<QueueEntry> {
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

fn row_to_task_output(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskOutput> {
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

fn row_to_task_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskMessage> {
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

fn row_to_repo(row: &rusqlite::Row<'_>) -> rusqlite::Result<RepoRow> {
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

fn row_to_chat_agent_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatAgentRun> {
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

fn row_to_legacy_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<LegacyEvent> {
    Ok(LegacyEvent {
        id: row.get(0)?,
        ts: row.get(1)?,
        level: row.get(2)?,
        category: row.get(3)?,
        message: row.get(4)?,
        metadata: row.get(5)?,
    })
}

const PROJECT_COLS: &str = "id, name, mode, repo_path, client_name, case_number, jurisdiction, \
    matter_type, opposing_counsel, deadline, privilege_level, status, default_template_id, created_at";

fn row_to_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRow> {
    let created_at_str: String = row.get(13)?;
    Ok(ProjectRow {
        id: row.get(0)?,
        name: row.get(1)?,
        mode: row.get(2)?,
        repo_path: row.get(3)?,
        client_name: row.get(4)?,
        case_number: row.get(5)?,
        jurisdiction: row.get(6)?,
        matter_type: row.get(7)?,
        opposing_counsel: row.get(8)?,
        deadline: row.get(9)?,
        privilege_level: row.get(10)?,
        status: row.get(11)?,
        default_template_id: row.get(12)?,
        created_at: parse_ts(&created_at_str),
    })
}

fn row_to_project_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectFileRow> {
    let created_at_str: String = row.get(8)?;
    Ok(ProjectFileRow {
        id: row.get(0)?,
        project_id: row.get(1)?,
        file_name: row.get(2)?,
        stored_path: row.get(3)?,
        mime_type: row.get(4)?,
        size_bytes: row.get(5)?,
        extracted_text: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        content_hash: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
        created_at: parse_ts(&created_at_str),
    })
}

// ── Db impl ───────────────────────────────────────────────────────────────

impl Db {
    pub fn raw_conn(&self) -> &std::sync::Mutex<Connection> {
        &self.conn
    }

    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open SQLite database at {path:?}"))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .context("failed to set PRAGMAs")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn migrate(&mut self) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute_batch(SCHEMA_SQL)
            .context("failed to apply schema migrations")?;
        // Idempotent column additions for DBs created before these columns existed.
        // ALTER TABLE fails if the column already exists; ignore that error.
        let alters = [
            "ALTER TABLE pipeline_tasks ADD COLUMN repo_id INTEGER REFERENCES repos(id)",
            "ALTER TABLE pipeline_tasks ADD COLUMN backend TEXT",
            "ALTER TABLE pipeline_tasks ADD COLUMN project_id INTEGER REFERENCES projects(id)",
            "ALTER TABLE proposals ADD COLUMN repo_id INTEGER REFERENCES repos(id)",
            "ALTER TABLE repos ADD COLUMN repo_slug TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE projects ADD COLUMN repo_path TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE projects ADD COLUMN client_name TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE projects ADD COLUMN case_number TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE projects ADD COLUMN jurisdiction TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE projects ADD COLUMN matter_type TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE projects ADD COLUMN opposing_counsel TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE projects ADD COLUMN deadline TEXT",
            "ALTER TABLE projects ADD COLUMN privilege_level TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE projects ADD COLUMN status TEXT NOT NULL DEFAULT 'active'",
            "ALTER TABLE pipeline_tasks ADD COLUMN task_type TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE pipeline_tasks ADD COLUMN structured_data TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE pipeline_events ADD COLUMN project_id INTEGER REFERENCES projects(id)",
            "ALTER TABLE pipeline_events ADD COLUMN actor TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE knowledge_files ADD COLUMN tags TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE knowledge_files ADD COLUMN category TEXT NOT NULL DEFAULT 'general'",
            "ALTER TABLE knowledge_files ADD COLUMN jurisdiction TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE knowledge_files ADD COLUMN project_id INTEGER",
            "ALTER TABLE pipeline_tasks ADD COLUMN started_at TEXT",
            "ALTER TABLE pipeline_tasks ADD COLUMN completed_at TEXT",
            "ALTER TABLE pipeline_tasks ADD COLUMN duration_secs INTEGER",
            "ALTER TABLE pipeline_tasks ADD COLUMN review_status TEXT",
            "ALTER TABLE pipeline_tasks ADD COLUMN revision_count INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE project_files ADD COLUMN extracted_text TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE project_files ADD COLUMN content_hash TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE projects ADD COLUMN default_template_id INTEGER",
            "CREATE TABLE IF NOT EXISTS cloud_connections (\
              id INTEGER PRIMARY KEY AUTOINCREMENT, \
              project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE, \
              provider TEXT NOT NULL, \
              access_token TEXT NOT NULL DEFAULT '', \
              refresh_token TEXT NOT NULL DEFAULT '', \
              token_expiry TEXT NOT NULL DEFAULT '', \
              account_email TEXT NOT NULL DEFAULT '', \
              account_id TEXT NOT NULL DEFAULT '', \
              created_at TEXT NOT NULL DEFAULT (datetime('now')))",
            "CREATE INDEX IF NOT EXISTS idx_cloud_connections_project ON cloud_connections(project_id)",
        ];
        for sql in alters {
            let _ = conn.execute(sql, []);
        }

        // Indexes on columns that may have been added via ALTER TABLE above.
        // CREATE INDEX IF NOT EXISTS is safe to run unconditionally.
        let post_alter_indexes = [
            "CREATE INDEX IF NOT EXISTS idx_pipeline_project ON pipeline_tasks(project_id)",
            "CREATE INDEX IF NOT EXISTS idx_pipeline_repo_status ON pipeline_tasks(repo_id, status)",
            "CREATE INDEX IF NOT EXISTS idx_pipeline_events_project ON pipeline_events(project_id)",
        ];
        for sql in post_alter_indexes {
            let _ = conn.execute(sql, []);
        }

        // Backfill deadlines from legacy projects.deadline column
        let needs_deadline_backfill: bool = conn
            .query_row("SELECT COUNT(*) = 0 FROM deadlines", [], |r| r.get(0))
            .unwrap_or(true);
        if needs_deadline_backfill {
            let rows: Vec<(i64, String)> = conn
                .prepare("SELECT id, deadline FROM projects WHERE deadline IS NOT NULL AND deadline != ''")
                .ok()
                .and_then(|mut s| {
                    s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                        .ok()
                        .map(|iter| iter.filter_map(|r| r.ok()).collect())
                })
                .unwrap_or_default();
            let ts = now_str();
            for (pid, due) in rows {
                let _ = conn.execute(
                    "INSERT INTO deadlines (project_id, label, due_date, created_at) VALUES (?1, 'Primary Deadline', ?2, ?3)",
                    params![pid, due, ts],
                );
            }
        }

        // Backfill parties table from existing projects (idempotent)
        let needs_backfill: bool = conn
            .query_row("SELECT COUNT(*) = 0 FROM parties", [], |r| r.get(0))
            .unwrap_or(true);
        if needs_backfill {
            let rows: Vec<(i64, String, String)> = conn
                .prepare(
                    "SELECT id, client_name, opposing_counsel FROM projects \
                     WHERE client_name != '' OR opposing_counsel != ''"
                )
                .ok()
                .and_then(|mut s| {
                    s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                        .ok()
                        .map(|iter| iter.filter_map(|r| r.ok()).collect())
                })
                .unwrap_or_default();
            let created_at = now_str();
            for (pid, client, opposing) in rows {
                for (name, role) in [(&client, "client"), (&opposing, "opposing_counsel")] {
                    let trimmed = name.trim();
                    if trimmed.is_empty() { continue; }
                    let normalized = normalize_party_name(trimmed);
                    let _ = conn.execute(
                        "INSERT INTO parties (project_id, name, normalized_name, role, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![pid, trimmed, normalized, role, created_at],
                    );
                }
            }
        }

        Ok(())
    }

    // ── Pipeline Tasks ────────────────────────────────────────────────────

    pub fn get_task(&self, id: i64) -> Result<Option<Task>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = format!(
            "SELECT {TASK_COLS} FROM pipeline_tasks \
             WHERE status NOT IN ('done', 'merged', 'failed', 'blocked', 'pending_review') \
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
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_active_tasks")?;
        Ok(tasks)
    }

    pub fn insert_task(&self, task: &Task) -> Result<i64> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let created_at = task.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
        let project_id = if task.project_id == 0 { None } else { Some(task.project_id) };
        conn.execute(
            "INSERT INTO pipeline_tasks \
             (title, description, repo_path, branch, status, attempt, max_attempts, \
              last_error, created_by, notify_chat, created_at, session_id, mode, backend, project_id, task_type) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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
                project_id,
                &task.task_type,
            ],
        )
        .context("insert_task")?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_task_status(&self, id: i64, status: &str, error: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let now = now_str();
        conn.execute(
            "UPDATE pipeline_tasks SET started_at = COALESCE(started_at, ?1) WHERE id = ?2",
            params![now, id],
        )
        .context("mark_task_started")?;
        Ok(())
    }

    pub fn mark_task_completed(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let now = now_str();
        conn.execute(
            "UPDATE pipeline_tasks SET completed_at = ?1, \
             duration_secs = CASE WHEN started_at IS NOT NULL \
               THEN CAST((julianday(?1) - julianday(started_at)) * 86400 AS INTEGER) \
               ELSE NULL END \
             WHERE id = ?2",
            params![now, id],
        )
        .context("mark_task_completed")?;
        Ok(())
    }

    pub fn set_review_status(&self, id: i64, status: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET review_status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status, now_str(), id],
        )
        .context("set_review_status")?;
        Ok(())
    }

    pub fn increment_revision_count(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
            |r: &rusqlite::Row| r.get(0),
        )
        .unwrap_or(0)
    }

    pub fn update_task_branch(&self, id: i64, branch: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET branch = ?1 WHERE id = ?2",
            params![branch, id],
        )
        .context("update_task_branch")?;
        Ok(())
    }

    pub fn update_task_session(&self, id: i64, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET session_id = ?1 WHERE id = ?2",
            params![session_id, id],
        )
        .context("update_task_session")?;
        Ok(())
    }

    pub fn update_task_description(&self, id: i64, title: &str, description: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET title = ?1, description = ?2 WHERE id = ?3",
            params![title, description, id],
        )
        .context("update_task_description")?;
        Ok(())
    }

    pub fn requeue_task(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET attempt = attempt + 1 WHERE id = ?1",
            params![id],
        )
        .context("increment_attempt")?;
        Ok(())
    }

    pub fn update_task_backend(&self, id: i64, backend: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE pipeline_tasks SET structured_data = ?1 WHERE id = ?2",
            params![data, id],
        )
        .context("update_task_structured_data")?;
        Ok(())
    }

    pub fn get_task_structured_data(&self, id: i64) -> Result<String> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, repo_path, title, description, rationale, status, created_at, \
             triage_score, triage_impact, triage_feasibility, triage_risk, triage_effort, \
             triage_reasoning \
             FROM proposals WHERE repo_path = ?1 ORDER BY id ASC",
        )?;
        let proposals = stmt
            .query_map(params![repo_path], row_to_proposal)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_proposals")?;
        Ok(proposals)
    }

    pub fn list_all_proposals(&self, repo_path: Option<&str>) -> Result<Vec<Proposal>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, repo_path, title, description, rationale, status, created_at, \
             triage_score, triage_impact, triage_feasibility, triage_risk, triage_effort, \
             triage_reasoning \
             FROM proposals \
             WHERE (?1 IS NULL OR repo_path = ?1) \
             ORDER BY id DESC",
        )?;
        let proposals = stmt
            .query_map(params![repo_path], row_to_proposal)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_all_proposals")?;
        Ok(proposals)
    }

    pub fn get_proposal(&self, id: i64) -> Result<Option<Proposal>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM pipeline_tasks", [], |r| r.get(0))
            .context("task_stats total")?;
        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pipeline_tasks WHERE status NOT IN ('done','merged','failed','blocked','pending_review')",
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let created_at = proposal.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
        conn.execute(
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
        Ok(conn.last_insert_rowid())
    }

    pub fn update_proposal_status(&self, id: i64, status: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = format!("SELECT {PROJECT_COLS} FROM projects ORDER BY id DESC");
        let mut stmt = conn.prepare(&sql)?;
        let projects = stmt
            .query_map([], row_to_project)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_projects")?;
        Ok(projects)
    }

    pub fn search_projects(&self, query: &str) -> Result<Vec<ProjectRow>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("search_projects")?;
        Ok(projects)
    }

    pub fn get_project(&self, id: i64) -> Result<Option<ProjectRow>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = format!("SELECT {PROJECT_COLS} FROM projects WHERE id=?1");
        let project = conn
            .query_row(&sql, params![id], row_to_project)
            .optional()
            .context("get_project")?;
        Ok(project)
    }

    pub fn insert_project(
        &self,
        name: &str,
        mode: &str,
        repo_path: &str,
        client_name: &str,
        jurisdiction: &str,
        matter_type: &str,
        privilege_level: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let created_at = now_str();
        conn.execute(
            "INSERT INTO projects (name, mode, repo_path, client_name, jurisdiction, matter_type, \
             privilege_level, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![name, mode, repo_path, client_name, jurisdiction, matter_type, privilege_level, created_at],
        )
        .context("insert_project")?;
        Ok(conn.last_insert_rowid())
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut sets = Vec::new();
        let mut vals: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
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
        let params: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|v| v.as_ref()).collect();
        conn.execute(&sql, params.as_slice())
            .context("update_project")?;
        Ok(())
    }

    pub fn delete_project(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let _ = conn.execute("DELETE FROM parties WHERE project_id=?1", params![id]);
        let _ = conn.execute("DELETE FROM project_files WHERE project_id=?1", params![id]);
        let affected = conn
            .execute("DELETE FROM projects WHERE id=?1", params![id])
            .context("delete_project")?;
        Ok(affected > 0)
    }

    // ── Parties / Conflict Checking ────────────────────────────────────────

    pub fn sync_project_parties(&self, project_id: i64, client_name: &str, opposing_counsel: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute("DELETE FROM parties WHERE project_id=?1", params![project_id])?;
        let created_at = now_str();
        for (name, role) in [
            (client_name, "client"),
            (opposing_counsel, "opposing_counsel"),
        ] {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }
            let normalized = normalize_party_name(trimmed);
            conn.execute(
                "INSERT INTO parties (project_id, name, normalized_name, role, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![project_id, trimmed, normalized, role, created_at],
            )?;
        }
        Ok(())
    }

    pub fn check_conflicts(
        &self,
        exclude_project_id: Option<i64>,
        client_name: &str,
        opposing_counsel: &str,
    ) -> Result<Vec<ConflictHit>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut hits = Vec::new();

        for (name, field) in [
            (client_name, "client_name"),
            (opposing_counsel, "opposing_counsel"),
        ] {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }
            let normalized = normalize_party_name(trimmed);
            let pattern = format!("%{normalized}%");
            let mut stmt = conn.prepare(
                "SELECT p.project_id, pr.name, p.name, p.role \
                 FROM parties p \
                 JOIN projects pr ON pr.id = p.project_id \
                 WHERE p.normalized_name LIKE ?1 \
                 AND (?2 IS NULL OR p.project_id != ?2) \
                 LIMIT 20",
            )?;
            let rows = stmt.query_map(params![pattern, exclude_project_id], |row| {
                Ok(ConflictHit {
                    project_id: row.get(0)?,
                    project_name: row.get(1)?,
                    party_name: row.get(2)?,
                    party_role: row.get(3)?,
                    matched_field: field.to_string(),
                })
            })?;
            for r in rows {
                hits.push(r?);
            }
        }
        Ok(hits)
    }

    // ── Deadlines ──────────────────────────────────────────────────────────

    pub fn list_project_deadlines(&self, project_id: i64) -> Result<Vec<DeadlineRow>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, label, due_date, rule_basis, status, created_at \
             FROM deadlines WHERE project_id = ?1 ORDER BY due_date ASC"
        )?;
        let rows = stmt.query_map(params![project_id], |r| {
            let ts: String = r.get(6)?;
            Ok(DeadlineRow {
                id: r.get(0)?,
                project_id: r.get(1)?,
                label: r.get(2)?,
                due_date: r.get(3)?,
                rule_basis: r.get(4)?,
                status: r.get(5)?,
                created_at: parse_ts(&ts),
            })
        })?.collect::<rusqlite::Result<Vec<_>>>().context("list_project_deadlines")?;
        Ok(rows)
    }

    pub fn list_upcoming_deadlines(&self, limit: i64) -> Result<Vec<(DeadlineRow, String)>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT d.id, d.project_id, d.label, d.due_date, d.rule_basis, d.status, d.created_at, p.name \
             FROM deadlines d JOIN projects p ON d.project_id = p.id \
             WHERE d.status = 'pending' ORDER BY d.due_date ASC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit], |r| {
            let ts: String = r.get(6)?;
            Ok((DeadlineRow {
                id: r.get(0)?,
                project_id: r.get(1)?,
                label: r.get(2)?,
                due_date: r.get(3)?,
                rule_basis: r.get(4)?,
                status: r.get(5)?,
                created_at: parse_ts(&ts),
            }, r.get::<_, String>(7)?))
        })?.collect::<rusqlite::Result<Vec<_>>>().context("list_upcoming_deadlines")?;
        Ok(rows)
    }

    pub fn insert_deadline(&self, project_id: i64, label: &str, due_date: &str, rule_basis: &str) -> Result<i64> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "INSERT INTO deadlines (project_id, label, due_date, rule_basis, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project_id, label, due_date, rule_basis, now_str()],
        ).context("insert_deadline")?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_deadline(&self, id: i64, label: Option<&str>, due_date: Option<&str>, rule_basis: Option<&str>, status: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        if let Some(v) = label { conn.execute("UPDATE deadlines SET label = ?1 WHERE id = ?2", params![v, id])?; }
        if let Some(v) = due_date { conn.execute("UPDATE deadlines SET due_date = ?1 WHERE id = ?2", params![v, id])?; }
        if let Some(v) = rule_basis { conn.execute("UPDATE deadlines SET rule_basis = ?1 WHERE id = ?2", params![v, id])?; }
        if let Some(v) = status { conn.execute("UPDATE deadlines SET status = ?1 WHERE id = ?2", params![v, id])?; }
        Ok(())
    }

    pub fn delete_deadline(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute("DELETE FROM deadlines WHERE id = ?1", params![id]).context("delete_deadline")?;
        Ok(())
    }

    // ── FTS5 ──────────────────────────────────────────────────────────────

    pub fn fts_index_document(&self, project_id: i64, task_id: i64, file_path: &str, title: &str, content: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute("DELETE FROM legal_fts WHERE task_id = ?1", params![task_id])?;
        Ok(())
    }

    pub fn fts_search(&self, query: &str, project_id: Option<i64>, limit: i64) -> Result<Vec<FtsResult>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = if project_id.is_some() {
            "SELECT project_id, task_id, file_path, \
                    snippet(legal_fts, 3, '<b>', '</b>', '…', 48) as title_snip, \
                    snippet(legal_fts, 4, '<b>', '</b>', '…', 80) as content_snip, \
                    rank \
             FROM legal_fts WHERE legal_fts MATCH ?1 AND project_id = ?2 \
             ORDER BY rank LIMIT ?3"
        } else {
            "SELECT project_id, task_id, file_path, \
                    snippet(legal_fts, 3, '<b>', '</b>', '…', 48) as title_snip, \
                    snippet(legal_fts, 4, '<b>', '</b>', '…', 80) as content_snip, \
                    rank \
             FROM legal_fts WHERE legal_fts MATCH ?1 \
             ORDER BY rank LIMIT ?3"
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
            })?.collect::<rusqlite::Result<Vec<_>>>().context("fts_search")?
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
            })?.collect::<rusqlite::Result<Vec<_>>>().context("fts_search")?
        };
        Ok(results)
    }

    pub fn list_project_tasks(&self, project_id: i64) -> Result<Vec<Task>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = format!("SELECT {TASK_COLS} FROM pipeline_tasks WHERE project_id = ?1 ORDER BY id DESC");
        let mut stmt = conn.prepare(&sql)?;
        let tasks = stmt
            .query_map(params![project_id], row_to_task)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_project_tasks")?;
        Ok(tasks)
    }

    pub fn list_project_files(&self, project_id: i64) -> Result<Vec<ProjectFileRow>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, file_name, stored_path, mime_type, size_bytes, extracted_text, content_hash, created_at \
             FROM project_files WHERE project_id=?1 ORDER BY id ASC",
        )?;
        let files = stmt
            .query_map(params![project_id], row_to_project_file)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_project_files")?;
        Ok(files)
    }

    pub fn get_project_file(
        &self,
        project_id: i64,
        file_id: i64,
    ) -> Result<Option<ProjectFileRow>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.query_row(
            "SELECT id, project_id, file_name, stored_path, mime_type, size_bytes, extracted_text, content_hash, created_at \
             FROM project_files WHERE id=?1 AND project_id=?2",
            params![file_id, project_id],
            row_to_project_file,
        )
        .optional()
        .context("get_project_file")
    }

    pub fn insert_project_file(
        &self,
        project_id: i64,
        file_name: &str,
        stored_path: &str,
        mime_type: &str,
        size_bytes: i64,
        content_hash: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let created_at = now_str();
        conn.execute(
            "INSERT INTO project_files \
             (project_id, file_name, stored_path, mime_type, size_bytes, content_hash, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                project_id,
                file_name,
                stored_path,
                mime_type,
                size_bytes,
                content_hash,
                created_at
            ],
        )
        .context("insert_project_file")?;
        Ok(conn.last_insert_rowid())
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
            "SELECT id, project_id, file_name, stored_path, mime_type, size_bytes, extracted_text, content_hash, created_at \
             FROM project_files WHERE project_id=?1 AND content_hash=?2 ORDER BY id ASC LIMIT 1",
            params![project_id, content_hash],
            row_to_project_file,
        )
        .optional()
        .context("find_project_file_by_hash")
    }

    pub fn update_project_file_text(&self, file_id: i64, text: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE project_files SET extracted_text = ?1 WHERE id = ?2",
            params![text, file_id],
        )?;
        Ok(())
    }

    pub fn total_project_file_bytes(&self, project_id: i64) -> Result<i64> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let total = conn
            .query_row(
                "SELECT COALESCE(SUM(size_bytes), 0) FROM project_files WHERE project_id=?1",
                params![project_id],
                |r| r.get(0),
            )
            .context("total_project_file_bytes")?;
        Ok(total)
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
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_str();
        conn.execute(
            "INSERT INTO upload_sessions \
             (project_id, file_name, mime_type, file_size, chunk_size, total_chunks, uploaded_bytes, is_zip, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, 'uploading', ?8, ?8)",
            params![
                project_id,
                file_name,
                mime_type,
                file_size,
                chunk_size,
                total_chunks,
                if is_zip { 1 } else { 0 },
                now
            ],
        )
        .context("create_upload_session")?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_upload_session(&self, session_id: i64) -> Result<Option<UploadSession>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT id, project_id, file_name, mime_type, file_size, chunk_size, total_chunks, \
                    uploaded_bytes, is_zip, status, stored_path, error, created_at, updated_at \
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
                    is_zip, status, stored_path, error, created_at, updated_at \
             FROM upload_sessions WHERE project_id=?1 ORDER BY id DESC LIMIT ?2"
        } else {
            "SELECT id, project_id, file_name, mime_type, file_size, chunk_size, total_chunks, uploaded_bytes, \
                    is_zip, status, stored_path, error, created_at, updated_at \
             FROM upload_sessions ORDER BY id DESC LIMIT ?1"
        };
        let mut stmt = conn.prepare(sql).context("list_upload_sessions prepare")?;
        let out = if let Some(pid) = project_id {
            stmt.query_map(params![pid, lim], row_to_upload_session)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("list_upload_sessions map")?
        } else {
            stmt.query_map(params![lim], row_to_upload_session)?
                .collect::<rusqlite::Result<Vec<_>>>()
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
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_uploaded_chunks")?;
        Ok(rows)
    }

    pub fn upsert_upload_chunk(&self, session_id: i64, chunk_index: i64, size_bytes: i64) -> Result<()> {
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut keyword_counts: HashMap<String, i64> = HashMap::new();
        let mut keyword_docs: HashMap<String, i64> = HashMap::new();
        let mut phrase_counts: HashMap<String, i64> = HashMap::new();
        let mut phrase_docs: HashMap<String, i64> = HashMap::new();
        let mut documents_scanned = 0i64;
        let mut tokens_scanned = 0i64;

        let sql = if project_id.is_some() {
            "SELECT extracted_text FROM project_files WHERE project_id = ?1 AND extracted_text != ''"
        } else {
            "SELECT extracted_text FROM project_files WHERE extracted_text != ''"
        };
        let mut stmt = conn.prepare(sql).context("summarize_themes prepare")?;
        if let Some(pid) = project_id {
            let rows = stmt.query_map(params![pid], |r| r.get::<_, String>(0))?;
            for row in rows {
                let text = row?;
                documents_scanned += 1;
                let tokens = tokenize_for_themes(&text);
                tokens_scanned += tokens.len() as i64;
                if tokens.is_empty() {
                    continue;
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
            }
        } else {
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            for row in rows {
                let text = row?;
                documents_scanned += 1;
                let tokens = tokenize_for_themes(&text);
                tokens_scanned += tokens.len() as i64;
                if tokens.is_empty() {
                    continue;
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

    // ── Cloud connections ─────────────────────────────────────────────────

    pub fn insert_cloud_connection(
        &self, project_id: i64, provider: &str, access_token: &str,
        refresh_token: &str, token_expiry: &str, account_email: &str, account_id: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO cloud_connections \
             (project_id, provider, access_token, refresh_token, token_expiry, account_email, account_id, created_at) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![project_id, provider, access_token, refresh_token, token_expiry,
                    account_email, account_id, now_str()],
        ).context("insert_cloud_connection")?;
        Ok(conn.last_insert_rowid())
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
        for r in rows { out.push(r?); }
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
        ).optional().context("get_cloud_connection")
    }

    pub fn update_cloud_connection_tokens(
        &self, id: i64, access_token: &str, refresh_token: &str, token_expiry: &str,
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
                "SELECT COALESCE(SUM(size_bytes), 0) FROM knowledge_files",
                [],
                |r| r.get(0),
            )
            .context("total_knowledge_file_bytes")?;
        Ok(total)
    }

    pub fn list_knowledge_files(&self) -> Result<Vec<KnowledgeFile>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, file_name, description, size_bytes, \"inline\", created_at, \
                    tags, category, jurisdiction, project_id \
             FROM knowledge_files ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], row_to_knowledge)?;
        let mut out = Vec::new();
        for r in rows { out.push(r?); }
        Ok(out)
    }

    pub fn get_knowledge_file(&self, id: i64) -> Result<Option<KnowledgeFile>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.query_row(
            "SELECT id, file_name, description, size_bytes, \"inline\", created_at, \
                    tags, category, jurisdiction, project_id \
             FROM knowledge_files WHERE id=?1",
            params![id],
            row_to_knowledge,
        )
        .optional()
        .context("get_knowledge_file")
    }

    pub fn list_templates(&self, category: Option<&str>, jurisdiction: Option<&str>) -> Result<Vec<KnowledgeFile>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, file_name, description, size_bytes, \"inline\", created_at, \
                    tags, category, jurisdiction, project_id \
             FROM knowledge_files \
             WHERE (?1 IS NULL OR category = ?1) AND (?2 IS NULL OR jurisdiction = ?2 OR jurisdiction = '') \
             ORDER BY category, file_name",
        )?;
        let rows = stmt.query_map(params![category, jurisdiction], row_to_knowledge)?;
        let mut out = Vec::new();
        for r in rows { out.push(r?); }
        Ok(out)
    }

    pub fn insert_knowledge_file(
        &self,
        file_name: &str,
        description: &str,
        size_bytes: i64,
        inline: bool,
    ) -> Result<i64> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "INSERT INTO knowledge_files (file_name, description, size_bytes, \"inline\") \
             VALUES (?1, ?2, ?3, ?4)",
            params![file_name, description, size_bytes, inline as i64],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn delete_knowledge_file(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute("DELETE FROM knowledge_files WHERE id=?1", params![id])?;
        Ok(())
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        if let Some(d) = description { conn.execute("UPDATE knowledge_files SET description=?1 WHERE id=?2", params![d, id])?; }
        if let Some(i) = inline { conn.execute("UPDATE knowledge_files SET \"inline\"=?1 WHERE id=?2", params![i as i64, id])?; }
        if let Some(t) = tags { conn.execute("UPDATE knowledge_files SET tags=?1 WHERE id=?2", params![t, id])?; }
        if let Some(c) = category { conn.execute("UPDATE knowledge_files SET category=?1 WHERE id=?2", params![c, id])?; }
        if let Some(j) = jurisdiction { conn.execute("UPDATE knowledge_files SET jurisdiction=?1 WHERE id=?2", params![j, id])?; }
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match project_id {
            Some(pid) => (
                format!("SELECT id, project_id, task_id, chunk_text, file_path, embedding FROM embeddings WHERE project_id = ?1 ORDER BY rowid DESC LIMIT {cap}"),
                vec![Box::new(pid) as Box<dyn rusqlite::types::ToSql>],
            ),
            None => (
                format!("SELECT id, project_id, task_id, chunk_text, file_path, embedding FROM embeddings ORDER BY rowid DESC LIMIT {cap}"),
                vec![],
            ),
        };
        let mut stmt = conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(params_refs.as_slice(), |row: &rusqlite::Row| {
                Ok((
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("search_embeddings query")?;

        let mut results: Vec<crate::knowledge::EmbeddingSearchResult> = rows
            .into_iter()
            .map(|(pid, tid, text, path, blob)| {
                let emb = crate::knowledge::bytes_to_embedding(&blob);
                let score = crate::knowledge::cosine_similarity(query_embedding, &emb);
                crate::knowledge::EmbeddingSearchResult {
                    chunk_text: text,
                    file_path: path,
                    project_id: pid,
                    task_id: tid,
                    score,
                }
            })
            .collect();

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);
        Ok(results)
    }

    pub fn embedding_count(&self) -> i64 {
        let Ok(conn) = self.conn.lock() else { return 0 };
        conn.query_row("SELECT COUNT(*) FROM embeddings", [], |r: &rusqlite::Row| r.get(0))
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "INSERT INTO citation_verifications (task_id, citation_text, citation_type, status, source, treatment, checked_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![task_id, citation_text, citation_type, status, source, treatment, checked_at],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_task_citations(&self, task_id: i64) -> Result<Vec<CitationVerification>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, citation_text, citation_type, status, source, treatment, checked_at, created_at \
             FROM citation_verifications WHERE task_id = ?1 ORDER BY id"
        )?;
        let rows = stmt
            .query_map(params![task_id], |r: &rusqlite::Row| {
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "DELETE FROM citation_verifications WHERE task_id = ?1",
            params![task_id],
        )?;
        Ok(())
    }

    pub fn get_top_scored_proposals(&self, threshold: i64, limit: i64) -> Result<Vec<Proposal>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, repo_path, title, description, rationale, status, created_at, \
             triage_score, triage_impact, triage_feasibility, triage_risk, triage_effort, \
             triage_reasoning \
             FROM proposals WHERE status='proposed' AND triage_score >= ?1 \
             ORDER BY triage_score DESC LIMIT ?2",
        )?;
        let proposals = stmt
            .query_map(params![threshold, limit], row_to_proposal)?
            .collect::<rusqlite::Result<Vec<_>>>()
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, repo_path, title, description, rationale, status, created_at, \
             triage_score, triage_impact, triage_feasibility, triage_risk, triage_effort, \
             triage_reasoning \
             FROM proposals WHERE status='proposed' AND triage_score=0 ORDER BY id ASC",
        )?;
        let proposals = stmt
            .query_map([], row_to_proposal)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_untriaged_proposals")?;
        Ok(proposals)
    }

    // ── Merge Queue ───────────────────────────────────────────────────────

    pub fn list_queue(&self) -> Result<Vec<QueueEntry>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, branch, repo_path, status, queued_at, pr_number \
             FROM integration_queue WHERE status = 'queued' ORDER BY id ASC",
        )?;
        let entries = stmt
            .query_map([], row_to_queue_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let queued_at = now_str();
        conn.execute(
            "INSERT INTO integration_queue (task_id, branch, repo_path, status, queued_at, pr_number) \
             VALUES (?1, ?2, ?3, 'queued', ?4, ?5)",
            params![task_id, branch, repo_path, queued_at, pr_number],
        )
        .context("enqueue")?;
        Ok(conn.last_insert_rowid())
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

        conn.execute(
            "INSERT INTO integration_queue (task_id, branch, repo_path, status, queued_at, pr_number) \
             VALUES (?1, ?2, ?3, 'queued', ?4, ?5)",
            params![task_id, branch, repo_path, queued_at, pr_number],
        )
        .context("enqueue_or_requeue insert")?;
        Ok(conn.last_insert_rowid())
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE integration_queue SET status = ?1, error_msg = ?2 WHERE id = ?3",
            params![status, error_msg, id],
        )
        .context("update_queue_status_with_error")?;
        Ok(())
    }

    pub fn get_queued_branches_for_repo(&self, repo_path: &str) -> Result<Vec<QueueEntry>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, branch, repo_path, status, queued_at, pr_number \
             FROM integration_queue WHERE repo_path = ?1 AND status = 'queued' ORDER BY task_id ASC",
        )?;
        let entries = stmt
            .query_map(params![repo_path], row_to_queue_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("get_queued_branches_for_repo")?;
        Ok(entries)
    }

    pub fn get_queue_entries_for_task(&self, task_id: i64) -> Result<Vec<QueueEntry>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, branch, repo_path, status, queued_at, pr_number \
             FROM integration_queue WHERE task_id = ?1 ORDER BY id ASC",
        )?;
        let entries = stmt
            .query_map(params![task_id], row_to_queue_entry)?
            .collect::<rusqlite::Result<Vec<_>>>()
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE integration_queue SET unknown_retries = unknown_retries + 1 WHERE id = ?1",
            params![id],
        )
        .context("increment_unknown_retries")?;
        Ok(())
    }

    pub fn reset_unknown_retries(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let created_at = now_str();
        conn.execute(
            "INSERT INTO task_outputs (task_id, phase, output, raw_stream, exit_code, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![task_id, phase, output, raw_stream, exit_code, created_at],
        )
        .context("insert_task_output")?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_task_outputs(&self, task_id: i64) -> Result<Vec<TaskOutput>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, phase, output, raw_stream, exit_code, created_at \
             FROM task_outputs WHERE task_id = ?1 ORDER BY id ASC",
        )?;
        let outputs = stmt
            .query_map(params![task_id], row_to_task_output)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("get_task_outputs")?;
        Ok(outputs)
    }

    // ── Task Messages ─────────────────────────────────────────────────────

    pub fn insert_task_message(&self, task_id: i64, role: &str, content: &str) -> Result<i64> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let created_at = now_str();
        conn.execute(
            "INSERT INTO task_messages (task_id, role, content, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![task_id, role, content, created_at],
        )
        .context("insert_task_message")?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_task_messages(&self, task_id: i64) -> Result<Vec<TaskMessage>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, role, content, created_at, delivered_phase \
             FROM task_messages WHERE task_id = ?1 ORDER BY id ASC",
        )?;
        let messages = stmt
            .query_map(params![task_id], row_to_task_message)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("get_task_messages")?;
        Ok(messages)
    }

    pub fn get_pending_task_messages(&self, task_id: i64) -> Result<Vec<TaskMessage>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, role, content, created_at, delivered_phase \
             FROM task_messages WHERE task_id = ?1 AND delivered_phase IS NULL ORDER BY id ASC",
        )?;
        let messages = stmt
            .query_map(params![task_id], row_to_task_message)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("get_pending_task_messages")?;
        Ok(messages)
    }

    pub fn mark_messages_delivered(&self, task_id: i64, phase: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE task_messages SET delivered_phase = ?1 \
             WHERE task_id = ?2 AND delivered_phase IS NULL",
            params![phase, task_id],
        )
        .context("mark_messages_delivered")?;
        Ok(())
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, path, name, mode, backend, test_cmd, prompt_file, auto_merge, repo_slug \
             FROM repos ORDER BY id ASC",
        )?;
        let repos = stmt
            .query_map([], row_to_repo)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_repos")?;
        Ok(repos)
    }

    pub fn get_repo_by_path(&self, path: &str) -> Result<Option<RepoRow>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let payload_str = payload.to_string();
        let created_at = now_str();
        conn.execute(
            "INSERT INTO pipeline_events (task_id, repo_id, project_id, actor, kind, payload, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![task_id, repo_id, project_id, actor, kind, payload_str, created_at],
        )
        .context("log_event")?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_project_events(&self, project_id: i64, limit: i64) -> Result<Vec<AuditEvent>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, project_id, actor, kind, payload, created_at \
             FROM pipeline_events WHERE project_id = ?1 \
             ORDER BY created_at DESC, id DESC LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![project_id, limit], |r| {
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
        })?.collect::<rusqlite::Result<Vec<_>>>().context("list_project_events")?;
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
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_task_events")?;
        Ok(rows)
    }

    // ── Config ────────────────────────────────────────────────────────────

    pub fn get_config(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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

    pub fn set_config(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let ts = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO events (ts, level, category, message, metadata) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ts, level, category, message, metadata],
        )
        .context("log_legacy_event")?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_recent_events(&self, limit: i64) -> Result<Vec<LegacyEvent>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, ts, level, category, message, metadata \
             FROM events ORDER BY ts DESC, id DESC LIMIT ?1",
        )?;
        let events = stmt
            .query_map(params![limit], row_to_legacy_event)?
            .collect::<rusqlite::Result<Vec<_>>>()
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "INSERT INTO pipeline_tasks \
             (title, description, repo_path, status, attempt, max_attempts, last_error, \
              created_by, notify_chat, created_at, session_id, mode, backend) \
             VALUES (?1, ?2, ?3, 'backlog', 0, 5, '', ?4, ?5, ?6, '', ?7, '')",
            params![
                title,
                description,
                repo_path,
                source,
                notify_chat,
                now_str(),
                mode
            ],
        )
        .context("create_pipeline_task")?;
        Ok(conn.last_insert_rowid())
    }

    /// Return "done" tasks that have no integration_queue entry (orphaned after restart).
    pub fn list_done_tasks_without_queue(&self) -> Result<Vec<Task>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_done_tasks_without_queue")?;
        Ok(tasks)
    }

    /// Reset integration_queue entries stuck in "merging" where the task is not yet merged.
    pub fn reset_stale_merging_queue(&self) -> Result<usize> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
            "SELECT COUNT(*) FROM pipeline_tasks WHERE status NOT IN ('done','merged','failed','blocked','pending_review')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0)
    }

    pub fn get_recent_merged_tasks(&self, limit: i64) -> Result<Vec<Task>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = format!(
            "SELECT {TASK_COLS} FROM pipeline_tasks WHERE status = 'merged' ORDER BY id DESC LIMIT ?1"
        );
        let mut stmt = conn.prepare(&sql)?;
        let tasks = stmt
            .query_map(params![limit], row_to_task)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("get_recent_merged_tasks")?;
        Ok(tasks)
    }

    pub fn recycle_failed_tasks(&self, repo_path: &str) -> Result<usize> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let sql = format!(
            "SELECT {TASK_COLS} FROM pipeline_tasks \
             WHERE (?1 IS NULL OR repo_path = ?1) \
             ORDER BY id DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let tasks = stmt
            .query_map(params![repo_path], row_to_task)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_all_tasks")?;
        Ok(tasks)
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let ts = now_str();
        conn.execute(
            "INSERT OR IGNORE INTO messages (id, chat_jid, sender, sender_name, content, timestamp, is_from_me, is_bot_message) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, chat_jid, sender, sender_name, content, ts,
                    if is_from_me { 1i32 } else { 0 },
                    if is_bot_message { 1i32 } else { 0 }],
        )
        .context("insert_chat_message")?;
        Ok(())
    }

    /// List all chat threads (distinct chat_jid values) with msg count and last timestamp.
    pub fn get_chat_threads(&self) -> Result<Vec<(String, i64, String)>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("get_chat_threads")?;
        Ok(rows)
    }

    /// Get messages for a specific chat thread, newest last.
    pub fn get_chat_messages(&self, chat_jid: &str, limit: i64) -> Result<Vec<ChatMessage>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, chat_jid, sender, sender_name, content, timestamp, is_from_me, is_bot_message \
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
                    is_from_me: row.get::<_, i32>(6)? != 0,
                    is_bot_message: row.get::<_, i32>(7)? != 0,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("get_chat_messages")?;
        Ok(rows)
    }

    // ── Registered groups ─────────────────────────────────────────────────

    pub fn get_all_groups(&self) -> Result<Vec<RegisteredGroup>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
                    requires_trigger: row.get::<_, i32>(4)? != 0,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "INSERT INTO registered_groups (jid, name, folder, trigger_pattern, requires_trigger) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(jid) DO UPDATE SET name=excluded.name, folder=excluded.folder, \
               trigger_pattern=excluded.trigger_pattern, requires_trigger=excluded.requires_trigger",
            params![jid, name, folder, trigger_pattern, if requires_trigger { 1i32 } else { 0 }],
        )
        .context("register_group")?;
        Ok(())
    }

    pub fn unregister_group(&self, jid: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute("DELETE FROM registered_groups WHERE jid = ?1", params![jid])
            .context("unregister_group")?;
        Ok(())
    }

    // ── Chat sessions ─────────────────────────────────────────────────────

    pub fn get_session(&self, folder: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.query_row(
            "SELECT session_id FROM sessions WHERE folder = ?1",
            params![folder],
            |r| r.get(0),
        )
        .optional()
        .context("get_session")
    }

    pub fn set_session(&self, folder: &str, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "INSERT INTO sessions (folder, session_id) VALUES (?1, ?2) \
             ON CONFLICT(folder) DO UPDATE SET session_id=excluded.session_id, created_at=datetime('now')",
            params![folder, session_id],
        )
        .context("set_session")?;
        Ok(())
    }

    pub fn get_seed_cooldowns(&self) -> Result<HashMap<(String, String), i64>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let n = conn
            .execute(
                "DELETE FROM sessions WHERE created_at < datetime('now', ?1)",
                params![format!("-{max_age_hours} hours")],
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "INSERT INTO chat_agent_runs (jid, status, transport, original_id, trigger_msg_id, folder) \
             VALUES (?1, 'running', ?2, ?3, ?4, ?5)",
            params![jid, transport, original_id, trigger_msg_id, folder],
        )
        .context("create_chat_agent_run")?;
        Ok(conn.last_insert_rowid())
    }

    pub fn complete_chat_agent_run(
        &self,
        id: i64,
        output: &str,
        new_session_id: &str,
        last_msg_timestamp: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE chat_agent_runs SET status='completed', output=?1, new_session_id=?2, \
             last_msg_timestamp=?3, completed_at=datetime('now') WHERE id=?4",
            params![output, new_session_id, last_msg_timestamp, id],
        )
        .context("complete_chat_agent_run")?;
        Ok(())
    }

    pub fn mark_chat_agent_run_delivered(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "UPDATE chat_agent_runs SET status='delivered' WHERE id=?1",
            params![id],
        )
        .context("mark_chat_agent_run_delivered")?;
        Ok(())
    }

    pub fn get_undelivered_runs(&self, jid: &str) -> Result<Vec<ChatAgentRun>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, jid, status, transport, original_id, trigger_msg_id, folder, \
             output, new_session_id, last_msg_timestamp, started_at, completed_at \
             FROM chat_agent_runs WHERE jid=?1 AND status='completed' ORDER BY id ASC",
        )?;
        let runs = stmt
            .query_map(params![jid], row_to_chat_agent_run)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("get_undelivered_runs")?;
        Ok(runs)
    }

    pub fn abandon_running_agents(&self) -> Result<usize> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, chat_jid, sender, sender_name, content, timestamp, is_from_me, is_bot_message \
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
                    is_from_me: row.get::<_, i32>(6)? != 0,
                    is_bot_message: row.get::<_, i32>(7)? != 0,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
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
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, ts, level, category, message, metadata FROM events \
             WHERE (?1 IS NULL OR category = ?1) \
             AND (?2 IS NULL OR level = ?2) \
             AND (?3 IS NULL OR ts >= ?3) \
             ORDER BY ts DESC, id DESC LIMIT ?4",
        )?;
        let events = stmt
            .query_map(
                params![category, level, since_ts, limit],
                row_to_legacy_event,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("get_events_filtered")?;
        Ok(events)
    }

    // ── API Keys (BYOK) ──────────────────────────────────────────────────

    // ── API Keys (BYOK) ──────────────────────────────────────────────────

    pub fn store_api_key(
        &self,
        owner: &str,
        provider: &str,
        key_name: &str,
        key_value: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute(
            "INSERT INTO api_keys (owner, provider, key_name, key_value) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(owner, provider) DO UPDATE SET key_name=excluded.key_name, key_value=excluded.key_value",
            params![owner, provider, key_name, key_value],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_api_key(&self, owner: &str, provider: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        // Try owner-specific first, then fall back to global
        let result = conn
            .query_row(
                "SELECT key_value FROM api_keys WHERE owner = ?1 AND provider = ?2",
                params![owner, provider],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("get_api_key")?;
        if result.is_some() {
            return Ok(result);
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
            return Ok(global);
        }
        Ok(None)
    }

    pub fn list_api_keys(&self, owner: &str) -> Result<Vec<ApiKeyEntry>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT id, owner, provider, key_name, created_at FROM api_keys \
             WHERE owner = ?1 OR owner = 'global' ORDER BY provider",
        )?;
        let keys = stmt
            .query_map(params![owner], |row| {
                Ok(ApiKeyEntry {
                    id: row.get(0)?,
                    owner: row.get(1)?,
                    provider: row.get(2)?,
                    key_name: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_api_keys")?;
        Ok(keys)
    }

    pub fn delete_api_key(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("db mutex poisoned"))?;
        conn.execute("DELETE FROM api_keys WHERE id = ?1", params![id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_party_name;

    #[test]
    fn punctuation_and_hyphens_stripped() {
        assert_eq!(normalize_party_name("Smith-Jones, LLC"), "smith jones");
    }

    #[test]
    fn all_stop_words_removed() {
        // Each legal stop-word must be filtered out on its own.
        for word in &["inc", "llc", "ltd", "corp", "co", "plc", "the", "of", "and"] {
            let result = normalize_party_name(&format!("Acme {}", word));
            assert_eq!(result, "acme", "stop word '{word}' was not removed");
        }
    }

    #[test]
    fn mixed_case_normalised() {
        assert_eq!(normalize_party_name("ACME CORP"), "acme");
        assert_eq!(normalize_party_name("Smith And Jones"), "smith jones");
    }

    #[test]
    fn multiple_spaces_collapsed() {
        assert_eq!(normalize_party_name("Foo   Bar   Inc"), "foo bar");
    }

    #[test]
    fn fully_empty_after_stop_word_removal() {
        // A name composed solely of stop words should yield an empty string.
        assert_eq!(normalize_party_name("The Co LLC Inc"), "");
    }
}
