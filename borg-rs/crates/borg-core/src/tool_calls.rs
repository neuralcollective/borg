use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A recorded tool call for observability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallEvent {
    pub id: i64,
    pub task_id: Option<i64>,
    pub chat_key: Option<String>,
    pub run_id: String,
    pub tool_name: String,
    pub input_summary: Option<String>,
    pub output_summary: Option<String>,
    pub started_at: DateTime<Utc>,
    pub duration_ms: Option<i64>,
    pub success: Option<bool>,
    pub error: Option<String>,
}

/// Insert params (no id, started_at auto-set).
#[derive(Debug, Clone)]
pub struct InsertToolCall {
    pub task_id: Option<i64>,
    pub chat_key: Option<String>,
    pub run_id: String,
    pub tool_name: String,
    pub input_summary: Option<String>,
}

/// Update params after tool completes.
#[derive(Debug, Clone)]
pub struct CompleteToolCall {
    pub output_summary: Option<String>,
    pub duration_ms: i64,
    pub success: bool,
    pub error: Option<String>,
}

pub const CREATE_TOOL_CALLS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS tool_calls (
    id BIGSERIAL PRIMARY KEY,
    task_id BIGINT,
    chat_key TEXT,
    run_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    input_summary TEXT,
    output_summary TEXT,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    duration_ms BIGINT,
    success BOOLEAN,
    error TEXT
)"#;

pub const CREATE_TOOL_CALLS_INDEXES: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_tool_calls_task ON tool_calls(task_id)",
    "CREATE INDEX IF NOT EXISTS idx_tool_calls_chat ON tool_calls(chat_key)",
    "CREATE INDEX IF NOT EXISTS idx_tool_calls_run ON tool_calls(run_id)",
];
