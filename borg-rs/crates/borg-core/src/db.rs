use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json;
use std::sync::Mutex;

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

// ── Timestamp helpers ─────────────────────────────────────────────────────

fn parse_ts(s: &str) -> DateTime<Utc> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .map(|ndt| ndt.and_utc())
        .unwrap_or_else(|_| Utc::now())
}

fn now_str() -> String {
    Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

// ── Row mappers ───────────────────────────────────────────────────────────

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let created_at_str: String = row.get(11)?;
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute_batch(SCHEMA_SQL)
            .context("failed to apply schema migrations")?;
        // Idempotent column additions for DBs created before these columns existed.
        // ALTER TABLE fails if the column already exists; ignore that error.
        let alters = [
            "ALTER TABLE pipeline_tasks ADD COLUMN repo_id INTEGER REFERENCES repos(id)",
            "ALTER TABLE pipeline_tasks ADD COLUMN backend TEXT",
            "ALTER TABLE proposals ADD COLUMN repo_id INTEGER REFERENCES repos(id)",
        ];
        for sql in alters {
            let _ = conn.execute(sql, []);
        }
        Ok(())
    }

    // ── Pipeline Tasks ────────────────────────────────────────────────────

    pub fn get_task(&self, id: i64) -> Result<Option<Task>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let result = conn
            .query_row(
                "SELECT id, title, description, repo_path, branch, status, attempt, \
                 max_attempts, last_error, created_by, notify_chat, created_at, \
                 session_id, mode, backend \
                 FROM pipeline_tasks WHERE id = ?1",
                params![id],
                row_to_task,
            )
            .optional()
            .context("get_task")?;
        Ok(result)
    }

    pub fn list_active_tasks(&self) -> Result<Vec<Task>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, title, description, repo_path, branch, status, attempt, \
             max_attempts, last_error, created_by, notify_chat, created_at, \
             session_id, mode, backend \
             FROM pipeline_tasks \
             WHERE status NOT IN ('done', 'merged', 'failed') \
             ORDER BY CASE status \
               WHEN 'rebase' THEN 0 \
               WHEN 'impl' THEN 1 \
               WHEN 'retry' THEN 1 \
               WHEN 'qa' THEN 2 \
               WHEN 'spec' THEN 3 \
               ELSE 4 \
             END, id ASC",
        )?;
        let tasks = stmt
            .query_map([], row_to_task)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_active_tasks")?;
        Ok(tasks)
    }

    pub fn insert_task(&self, task: &Task) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let created_at = task.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
        conn.execute(
            "INSERT INTO pipeline_tasks \
             (title, description, repo_path, branch, status, attempt, max_attempts, \
              last_error, created_by, notify_chat, created_at, session_id, mode, backend) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
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
                if task.backend.is_empty() { None } else { Some(task.backend.as_str()) },
            ],
        )
        .context("insert_task")?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_task_status(&self, id: i64, status: &str, error: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let updated_at = now_str();
        conn.execute(
            "UPDATE pipeline_tasks SET status = ?1, last_error = COALESCE(?2, last_error), \
             updated_at = ?3 WHERE id = ?4",
            params![status, error, updated_at, id],
        )
        .context("update_task_status")?;
        Ok(())
    }

    pub fn update_task_branch(&self, id: i64, branch: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE pipeline_tasks SET branch = ?1 WHERE id = ?2",
            params![branch, id],
        )
        .context("update_task_branch")?;
        Ok(())
    }

    pub fn update_task_session(&self, id: i64, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE pipeline_tasks SET session_id = ?1 WHERE id = ?2",
            params![session_id, id],
        )
        .context("update_task_session")?;
        Ok(())
    }

    pub fn increment_attempt(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE pipeline_tasks SET attempt = attempt + 1 WHERE id = ?1",
            params![id],
        )
        .context("increment_attempt")?;
        Ok(())
    }

    pub fn update_task_backend(&self, id: i64, backend: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE pipeline_tasks SET backend = ?1 WHERE id = ?2",
            params![if backend.is_empty() { None } else { Some(backend) }, id],
        )
        .context("update_task_backend")?;
        Ok(())
    }

    // ── Proposals ─────────────────────────────────────────────────────────

    pub fn list_proposals(&self, repo_path: &str) -> Result<Vec<Proposal>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM pipeline_tasks", [], |r| r.get(0))
            .context("task_stats total")?;
        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pipeline_tasks WHERE status NOT IN ('done','merged','failed')",
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

    pub fn insert_proposal(&self, proposal: &Proposal) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE proposals SET triage_score=?1, triage_impact=?2, triage_feasibility=?3, \
             triage_risk=?4, triage_effort=?5, triage_reasoning=?6 WHERE id=?7",
            params![score, impact, feasibility, risk, effort, reasoning, id],
        )
        .context("update_proposal_triage")?;
        Ok(())
    }

    pub fn get_top_scored_proposals(&self, threshold: i64, limit: i64) -> Result<Vec<Proposal>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT COUNT(*) FROM proposals WHERE status='proposed' AND triage_score=0",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0)
    }

    pub fn list_untriaged_proposals(&self) -> Result<Vec<Proposal>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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

    pub fn enqueue(&self, task_id: i64, branch: &str, repo_path: &str, pr_number: i64) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let queued_at = now_str();
        conn.execute(
            "INSERT INTO integration_queue (task_id, branch, repo_path, status, queued_at, pr_number) \
             VALUES (?1, ?2, ?3, 'queued', ?4, ?5)",
            params![task_id, branch, repo_path, queued_at, pr_number],
        )
        .context("enqueue")?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_queue_status(&self, id: i64, status: &str) -> Result<()> {
        self.update_queue_status_with_error(id, status, "")
    }

    pub fn update_queue_status_with_error(&self, id: i64, status: &str, error_msg: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE integration_queue SET status = ?1, error_msg = ?2 WHERE id = ?3",
            params![status, error_msg, id],
        )
        .context("update_queue_status_with_error")?;
        Ok(())
    }

    pub fn get_queued_branches_for_repo(&self, repo_path: &str) -> Result<Vec<QueueEntry>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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

    pub fn get_unknown_retries(&self, id: i64) -> i64 {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT unknown_retries FROM integration_queue WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap_or(0)
    }

    pub fn increment_unknown_retries(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE integration_queue SET unknown_retries = unknown_retries + 1 WHERE id = ?1",
            params![id],
        )
        .context("increment_unknown_retries")?;
        Ok(())
    }

    pub fn reset_unknown_retries(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let auto_merge_int: i64 = if auto_merge { 1 } else { 0 };
        conn.execute(
            "INSERT INTO repos (path, name, mode, test_cmd, prompt_file, auto_merge, backend) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(path) DO UPDATE SET \
               name = excluded.name, \
               mode = excluded.mode, \
               test_cmd = excluded.test_cmd, \
               prompt_file = excluded.prompt_file, \
               auto_merge = excluded.auto_merge, \
               backend = excluded.backend",
            params![path, name, mode, test_cmd, prompt_file, auto_merge_int, backend],
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, path, name, mode, backend, test_cmd, prompt_file, auto_merge \
             FROM repos ORDER BY id ASC",
        )?;
        let repos = stmt
            .query_map([], row_to_repo)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_repos")?;
        Ok(repos)
    }

    pub fn get_repo_by_path(&self, path: &str) -> Result<Option<RepoRow>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let result = conn
            .query_row(
                "SELECT id, path, name, mode, backend, test_cmd, prompt_file, auto_merge \
                 FROM repos WHERE path = ?1",
                params![path],
                row_to_repo,
            )
            .optional()
            .context("get_repo_by_path")?;
        Ok(result)
    }

    pub fn update_repo_backend(&self, id: i64, backend: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE repos SET backend = ?1 WHERE id = ?2",
            params![if backend.is_empty() { None } else { Some(backend) }, id],
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let payload_str = payload.to_string();
        let created_at = now_str();
        conn.execute(
            "INSERT INTO pipeline_events (task_id, repo_id, kind, payload, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![task_id, repo_id, kind, payload_str, created_at],
        )
        .context("log_event")?;
        Ok(conn.last_insert_rowid())
    }

    // ── Config ────────────────────────────────────────────────────────────

    pub fn get_config(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO pipeline_tasks \
             (title, description, repo_path, status, attempt, max_attempts, last_error, \
              created_by, notify_chat, created_at, session_id, mode, backend) \
             VALUES (?1, ?2, ?3, 'backlog', 0, 5, '', ?4, ?5, ?6, '', ?7, '')",
            params![title, description, repo_path, source, notify_chat, Utc::now().to_rfc3339(), mode],
        )
        .context("create_pipeline_task")?;
        Ok(conn.last_insert_rowid())
    }

    /// Return "done" tasks that have no "queued" integration_queue entry (orphaned after restart).
    pub fn list_done_tasks_without_queue(&self) -> Result<Vec<Task>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, title, description, repo_path, branch, status, attempt, \
             max_attempts, last_error, created_by, notify_chat, created_at, \
             session_id, mode, backend \
             FROM pipeline_tasks \
             WHERE status = 'done' \
             AND NOT EXISTS ( \
               SELECT 1 FROM integration_queue q \
               WHERE q.task_id = pipeline_tasks.id AND q.status = 'queued' \
             )",
        )?;
        let tasks = stmt
            .query_map([], row_to_task)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("list_done_tasks_without_queue")?;
        Ok(tasks)
    }

    /// Reset integration_queue entries stuck in "merging" where the task is not yet merged.
    pub fn reset_stale_merging_queue(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let n = conn.execute(
            "UPDATE integration_queue SET status = 'queued' \
             WHERE status = 'merging' \
             AND task_id IN (SELECT id FROM pipeline_tasks WHERE status != 'merged')",
            [],
        )?;
        Ok(n)
    }

    pub fn active_task_count(&self) -> i64 {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT COUNT(*) FROM pipeline_tasks WHERE status NOT IN ('done','merged','failed')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0)
    }

    pub fn get_recent_merged_tasks(&self, limit: i64) -> Result<Vec<Task>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, title, description, repo_path, branch, status, attempt, \
             max_attempts, last_error, created_by, notify_chat, created_at, \
             session_id, mode, backend \
             FROM pipeline_tasks WHERE status = 'merged' ORDER BY id DESC LIMIT ?1",
        )?;
        let tasks = stmt
            .query_map(params![limit], row_to_task)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("get_recent_merged_tasks")?;
        Ok(tasks)
    }

    pub fn recycle_failed_tasks(&self, repo_path: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let n = conn.execute(
            "UPDATE pipeline_tasks SET status='backlog', attempt=0, last_error='' \
             WHERE status='failed' AND repo_path=?1",
            params![repo_path],
        )
        .context("recycle_failed_tasks")?;
        Ok(n)
    }

    pub fn reset_task_attempt(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let sql = "SELECT id, title, description, repo_path, branch, status, attempt, \
                   max_attempts, last_error, created_by, notify_chat, created_at, \
                   session_id, mode, backend \
                   FROM pipeline_tasks \
                   WHERE (?1 IS NULL OR repo_path = ?1) \
                   ORDER BY id DESC";
        let mut stmt = conn.prepare(sql)?;
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
            }
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT jid, name, folder, trigger_pattern, requires_trigger FROM registered_groups ORDER BY added_at ASC",
        )?;
        let groups = stmt
            .query_map([], |row| {
                Ok(RegisteredGroup {
                    jid: row.get(0)?,
                    name: row.get(1)?,
                    folder: row.get(2)?,
                    trigger_pattern: row.get::<_, Option<String>>(3)?.unwrap_or_else(|| "@Borg".into()),
                    requires_trigger: row.get::<_, i32>(4)? != 0,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("get_all_groups")?;
        Ok(groups)
    }

    pub fn register_group(&self, jid: &str, name: &str, folder: &str, trigger_pattern: &str, requires_trigger: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute("DELETE FROM registered_groups WHERE jid = ?1", params![jid])
            .context("unregister_group")?;
        Ok(())
    }

    // ── Chat sessions ─────────────────────────────────────────────────────

    pub fn get_session(&self, folder: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT session_id FROM sessions WHERE folder = ?1",
            params![folder],
            |r| r.get(0),
        )
        .optional()
        .context("get_session")
    }

    pub fn set_session(&self, folder: &str, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO sessions (folder, session_id) VALUES (?1, ?2) \
             ON CONFLICT(folder) DO UPDATE SET session_id=excluded.session_id, created_at=datetime('now')",
            params![folder, session_id],
        )
        .context("set_session")?;
        Ok(())
    }

    pub fn expire_sessions(&self, max_age_hours: i64) -> Result<usize> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let n = conn.execute(
            "DELETE FROM sessions WHERE created_at < datetime('now', ?1)",
            params![format!("-{max_age_hours} hours")],
        )
        .context("expire_sessions")?;
        Ok(n)
    }

    // ── Chat agent runs ───────────────────────────────────────────────────

    pub fn create_chat_agent_run(&self, jid: &str, transport: &str, original_id: &str, trigger_msg_id: &str, folder: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO chat_agent_runs (jid, status, transport, original_id, trigger_msg_id, folder) \
             VALUES (?1, 'running', ?2, ?3, ?4, ?5)",
            params![jid, transport, original_id, trigger_msg_id, folder],
        )
        .context("create_chat_agent_run")?;
        Ok(conn.last_insert_rowid())
    }

    pub fn complete_chat_agent_run(&self, id: i64, output: &str, new_session_id: &str, last_msg_timestamp: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE chat_agent_runs SET status='completed', output=?1, new_session_id=?2, \
             last_msg_timestamp=?3, completed_at=datetime('now') WHERE id=?4",
            params![output, new_session_id, last_msg_timestamp, id],
        )
        .context("complete_chat_agent_run")?;
        Ok(())
    }

    pub fn mark_chat_agent_run_delivered(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE chat_agent_runs SET status='delivered' WHERE id=?1",
            params![id],
        )
        .context("mark_chat_agent_run_delivered")?;
        Ok(())
    }

    pub fn get_undelivered_runs(&self, jid: &str) -> Result<Vec<ChatAgentRun>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let n = conn.execute(
            "UPDATE chat_agent_runs SET status='abandoned' WHERE status='running'",
            [],
        )
        .context("abandon_running_agents")?;
        Ok(n)
    }

    pub fn get_messages_since(&self, chat_jid: &str, since_ts: &str, limit: i64) -> Result<Vec<ChatMessage>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, ts, level, category, message, metadata FROM events \
             WHERE (?1 IS NULL OR category = ?1) \
             AND (?2 IS NULL OR level = ?2) \
             AND (?3 IS NULL OR ts >= ?3) \
             ORDER BY ts DESC, id DESC LIMIT ?4",
        )?;
        let events = stmt
            .query_map(params![category, level, since_ts, limit], row_to_legacy_event)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("get_events_filtered")?;
        Ok(events)
    }
}
