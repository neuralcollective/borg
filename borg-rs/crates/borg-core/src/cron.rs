use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;

use crate::db::Db;
use crate::pgcompat as pg;

// ── Types ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CronJob {
    pub id: i64,
    pub name: String,
    pub schedule: String,
    pub job_type: CronJobType,
    pub config: serde_json::Value,
    pub project_id: Option<i64>,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CronJobType {
    AgentTask,
    Shell,
}

impl CronJobType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AgentTask => "agent_task",
            Self::Shell => "shell",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "shell" => Self::Shell,
            _ => Self::AgentTask,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CronRun {
    pub id: i64,
    pub job_id: i64,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: CronRunStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub task_id: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CronRunStatus {
    Running,
    Success,
    Error,
}

impl CronRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Success => "success",
            Self::Error => "error",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "success" => Self::Success,
            "error" => Self::Error,
            _ => Self::Running,
        }
    }
}

// ── Row mappers (called from db.rs) ──────────────────────────────────────

fn parse_ts(s: &str) -> DateTime<Utc> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .map(|ndt| ndt.and_utc())
        .unwrap_or_else(|_| Utc::now())
}

pub fn row_to_cron_job(row: &pg::Row<'_>) -> pg::Result<CronJob> {
    let enabled_int: i64 = row.get(6)?;
    let config_str: String = row.get(4)?;
    let last_run: Option<String> = row.get(7)?;
    let next_run: Option<String> = row.get(8)?;
    let created_at_str: String = row.get(9)?;
    let job_type_str: String = row.get(3)?;
    Ok(CronJob {
        id: row.get(0)?,
        name: row.get(1)?,
        schedule: row.get(2)?,
        job_type: CronJobType::from_str_lossy(&job_type_str),
        config: serde_json::from_str(&config_str).unwrap_or(serde_json::Value::Object(Default::default())),
        project_id: row.get(5)?,
        enabled: enabled_int != 0,
        last_run: last_run.map(|s| parse_ts(&s)),
        next_run: next_run.map(|s| parse_ts(&s)),
        created_at: parse_ts(&created_at_str),
    })
}

pub fn row_to_cron_run(row: &pg::Row<'_>) -> pg::Result<CronRun> {
    let started_at_str: String = row.get(2)?;
    let finished_at: Option<String> = row.get(3)?;
    let status_str: String = row.get(4)?;
    Ok(CronRun {
        id: row.get(0)?,
        job_id: row.get(1)?,
        started_at: parse_ts(&started_at_str),
        finished_at: finished_at.map(|s| parse_ts(&s)),
        status: CronRunStatus::from_str_lossy(&status_str),
        result: row.get(5)?,
        error: row.get(6)?,
        task_id: row.get(7)?,
    })
}

// ── Cron expression helpers ──────────────────────────────────────────────

/// Parse a 5-field cron expression ("min hour dom month dow") and compute the
/// next fire time after `from`. The `cron` crate expects 7 fields
/// (sec min hour dom month dow year), so we prepend "0" and append "*".
pub fn compute_next_run(schedule: &str, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let expr = normalize_cron_expr(schedule);
    let sched = cron::Schedule::from_str(&expr).ok()?;
    sched.after(&from).next()
}

/// Convert a 5-field user cron expression to the 7-field format the cron crate expects.
/// Standard cron uses 0=Sun,1=Mon..6=Sat for day-of-week, but the cron crate uses
/// 1=Sun,2=Mon..7=Sat. We shift numeric DOW values by +1 when converting 5-field input.
fn normalize_cron_expr(expr: &str) -> String {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    match fields.len() {
        5 => {
            let dow = shift_dow_field(fields[4]);
            format!("0 {} {} {} {} {} *", fields[0], fields[1], fields[2], fields[3], dow)
        }
        6 => format!("0 {}", expr),
        _ => expr.to_string(),
    }
}

/// Shift day-of-week values from standard cron (0-6, 0=Sun) to cron-crate (1-7, 1=Sun).
fn shift_dow_field(field: &str) -> String {
    if field == "*" || field.contains(|c: char| c.is_ascii_alphabetic()) {
        return field.to_string();
    }
    field
        .split(',')
        .map(|part| {
            if part.contains('-') {
                let mut range_parts = part.splitn(2, '-');
                let start = range_parts.next().unwrap_or("0");
                let rest = range_parts.next().unwrap_or("0");
                let (end, step) = if let Some((e, s)) = rest.split_once('/') {
                    (e, Some(s))
                } else {
                    (rest, None)
                };
                let s = start.parse::<u8>().map(|v| v + 1).unwrap_or(1);
                let e = end.parse::<u8>().map(|v| v + 1).unwrap_or(1);
                match step {
                    Some(st) => format!("{s}-{e}/{st}"),
                    None => format!("{s}-{e}"),
                }
            } else if let Some((val, step)) = part.split_once('/') {
                let v = val.parse::<u8>().map(|v| v + 1).unwrap_or(1);
                format!("{v}/{step}")
            } else {
                part.parse::<u8>()
                    .map(|v| (v + 1).to_string())
                    .unwrap_or_else(|_| part.to_string())
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

// ── Scheduler ────────────────────────────────────────────────────────────

pub struct CronScheduler {
    db: Arc<Db>,
    poll_interval: Duration,
}

impl CronScheduler {
    pub fn new(db: Arc<Db>, poll_interval: Duration) -> Self {
        Self { db, poll_interval }
    }

    pub async fn run(&self, cancel: CancellationToken) {
        tracing::info!(
            poll_secs = self.poll_interval.as_secs(),
            "cron scheduler started"
        );
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("cron scheduler shutting down");
                    break;
                }
                _ = tokio::time::sleep(self.poll_interval) => {
                    if let Err(e) = self.check_due_jobs().await {
                        tracing::error!(err = %e, "cron: error checking due jobs");
                    }
                }
            }
        }
    }

    async fn check_due_jobs(&self) -> Result<()> {
        let jobs = self.db.list_due_cron_jobs()?;
        for job in jobs {
            tracing::info!(job_id = job.id, name = %job.name, "cron: executing due job");
            let db = Arc::clone(&self.db);
            let job_clone = job.clone();
            tokio::spawn(async move {
                if let Err(e) = execute_job(&db, &job_clone).await {
                    tracing::error!(job_id = job_clone.id, err = %e, "cron: job execution failed");
                }
            });

            let now = Utc::now();
            let next = compute_next_run(&job.schedule, now);
            if let Err(e) = self.db.update_cron_job_after_run(job.id, &now, next.as_ref()) {
                tracing::error!(job_id = job.id, err = %e, "cron: failed to update job timestamps");
            }
        }
        Ok(())
    }
}

pub async fn execute_job(db: &Db, job: &CronJob) -> Result<()> {
    let run_id = db.insert_cron_run(job.id)?;

    match job.job_type {
        CronJobType::AgentTask => {
            match execute_agent_task(db, job) {
                Ok(task_id) => {
                    db.update_cron_run(
                        run_id,
                        "success",
                        Some(&format!("created task {task_id}")),
                        None,
                        Some(task_id),
                    )?;
                }
                Err(e) => {
                    db.update_cron_run(
                        run_id,
                        "error",
                        None,
                        Some(&e.to_string()),
                        None,
                    )?;
                    return Err(e);
                }
            }
        }
        CronJobType::Shell => {
            match execute_shell(job).await {
                Ok(output) => {
                    db.update_cron_run(run_id, "success", Some(&output), None, None)?;
                }
                Err(e) => {
                    db.update_cron_run(
                        run_id,
                        "error",
                        None,
                        Some(&e.to_string()),
                        None,
                    )?;
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

fn execute_agent_task(db: &Db, job: &CronJob) -> Result<i64> {
    let config = &job.config;
    let title = config["title"]
        .as_str()
        .unwrap_or(&job.name)
        .to_string();
    let description = config["description"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let repo_path = config["repo_path"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let mode = config["mode"]
        .as_str()
        .unwrap_or("sweborg")
        .to_string();
    let backend = config["backend"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let task_type = config["task_type"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let task = crate::types::Task {
        id: 0,
        title,
        description,
        repo_path,
        branch: String::new(),
        status: "backlog".into(),
        attempt: 0,
        max_attempts: config["max_attempts"].as_i64().unwrap_or(3),
        last_error: String::new(),
        created_by: format!("cron:{}", job.id),
        notify_chat: config["notify_chat"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        session_id: String::new(),
        mode,
        backend,
        workspace_id: config["workspace_id"].as_i64().unwrap_or(0),
        project_id: job.project_id.unwrap_or(0),
        task_type,
        requires_exhaustive_corpus_review: config["requires_exhaustive_corpus_review"]
            .as_bool()
            .unwrap_or(false),
        started_at: None,
        completed_at: None,
        duration_secs: None,
        review_status: None,
        revision_count: 0,
        chat_thread: String::new(),
    };
    db.insert_task(&task).context("cron: insert agent task")
}

async fn execute_shell(job: &CronJob) -> Result<String> {
    let command = job.config["command"]
        .as_str()
        .context("shell cron job missing 'command' in config")?;

    let output = tokio::time::timeout(
        Duration::from_secs(300),
        tokio::process::Command::new("bash")
            .arg("-c")
            .arg(command)
            .output(),
    )
    .await
    .context("shell command timed out after 300s")?
    .context("failed to spawn shell command")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        anyhow::bail!(
            "command exited with {}: {}",
            output.status,
            if stderr.is_empty() {
                stdout.into_owned()
            } else {
                stderr.into_owned()
            }
        );
    }

    let mut result = stdout.into_owned();
    if !stderr.is_empty() {
        result.push_str("\nSTDERR: ");
        result.push_str(&stderr);
    }
    // Truncate to prevent storing huge outputs
    if result.len() > 64_000 {
        result.truncate(64_000);
        result.push_str("\n... (truncated)");
    }
    Ok(result)
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_next_run_daily_2am() {
        let from = DateTime::parse_from_rfc3339("2026-03-14T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let next = compute_next_run("0 2 * * *", from).unwrap();
        assert_eq!(next.format("%Y-%m-%d %H:%M").to_string(), "2026-03-14 02:00");
    }

    #[test]
    fn test_compute_next_run_after_fire_time() {
        let from = DateTime::parse_from_rfc3339("2026-03-14T03:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let next = compute_next_run("0 2 * * *", from).unwrap();
        assert_eq!(next.format("%Y-%m-%d %H:%M").to_string(), "2026-03-15 02:00");
    }

    #[test]
    fn test_compute_next_run_every_5_min() {
        let from = DateTime::parse_from_rfc3339("2026-03-14T12:03:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let next = compute_next_run("*/5 * * * *", from).unwrap();
        assert_eq!(next.format("%Y-%m-%d %H:%M").to_string(), "2026-03-14 12:05");
    }

    #[test]
    fn test_compute_next_run_weekly_monday() {
        // 2026-03-14 is a Saturday; standard cron: 1=Monday
        let from = DateTime::parse_from_rfc3339("2026-03-14T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let next = compute_next_run("0 9 * * 1", from).unwrap();
        assert_eq!(next.format("%Y-%m-%d %H:%M").to_string(), "2026-03-16 09:00");
    }

    #[test]
    fn test_compute_next_run_invalid_expression() {
        let from = Utc::now();
        assert!(compute_next_run("not a cron expression", from).is_none());
    }

    #[test]
    fn test_normalize_5_fields() {
        assert_eq!(normalize_cron_expr("0 2 * * *"), "0 0 2 * * * *");
    }

    #[test]
    fn test_normalize_5_fields_dow_shift() {
        // Standard cron 0=Sun becomes cron-crate 1=Sun
        assert_eq!(normalize_cron_expr("0 9 * * 0"), "0 0 9 * * 1 *");
        // Standard cron 1=Mon becomes cron-crate 2=Mon
        assert_eq!(normalize_cron_expr("0 9 * * 1"), "0 0 9 * * 2 *");
        // Named days pass through unchanged
        assert_eq!(normalize_cron_expr("0 9 * * Mon"), "0 0 9 * * Mon *");
    }

    #[test]
    fn test_normalize_6_fields() {
        assert_eq!(normalize_cron_expr("0 0 2 * * *"), "0 0 0 2 * * *");
    }

    #[test]
    fn test_normalize_7_fields_passthrough() {
        assert_eq!(normalize_cron_expr("0 0 2 * * * *"), "0 0 2 * * * *");
    }

    #[test]
    fn test_cron_job_type_roundtrip() {
        let agent = CronJobType::AgentTask;
        let shell = CronJobType::Shell;
        assert_eq!(CronJobType::from_str_lossy(agent.as_str()), CronJobType::AgentTask);
        assert_eq!(CronJobType::from_str_lossy(shell.as_str()), CronJobType::Shell);
    }

    #[test]
    fn test_cron_job_type_serde_roundtrip() {
        let agent = CronJobType::AgentTask;
        let json = serde_json::to_string(&agent).unwrap();
        assert_eq!(json, "\"agent_task\"");
        let deserialized: CronJobType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, CronJobType::AgentTask);

        let shell = CronJobType::Shell;
        let json = serde_json::to_string(&shell).unwrap();
        assert_eq!(json, "\"shell\"");
        let deserialized: CronJobType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, CronJobType::Shell);
    }

    #[test]
    fn test_cron_run_status_roundtrip() {
        for status in [CronRunStatus::Running, CronRunStatus::Success, CronRunStatus::Error] {
            let s = status.as_str();
            assert_eq!(CronRunStatus::from_str_lossy(s), status);
        }
    }

    #[test]
    fn test_cron_job_type_unknown_defaults_to_agent() {
        assert_eq!(CronJobType::from_str_lossy("unknown"), CronJobType::AgentTask);
        assert_eq!(CronJobType::from_str_lossy(""), CronJobType::AgentTask);
    }

    #[test]
    fn test_cron_run_status_unknown_defaults_to_running() {
        assert_eq!(CronRunStatus::from_str_lossy("unknown"), CronRunStatus::Running);
    }
}
