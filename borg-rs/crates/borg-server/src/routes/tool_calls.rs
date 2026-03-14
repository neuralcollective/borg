use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use super::internal;
use crate::AppState;

#[derive(Deserialize)]
pub(crate) struct ToolCallsQuery {
    pub task_id: Option<i64>,
    pub chat_key: Option<String>,
    pub run_id: Option<String>,
    pub limit: Option<i64>,
}

pub(crate) async fn list_tool_calls(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ToolCallsQuery>,
) -> Result<Json<Value>, StatusCode> {
    let limit = q.limit.unwrap_or(50);
    let calls = if let Some(task_id) = q.task_id {
        state.db.list_tool_calls_by_task(task_id, limit)
    } else if let Some(ref chat_key) = q.chat_key {
        state.db.list_tool_calls_by_chat(chat_key, limit)
    } else if let Some(ref run_id) = q.run_id {
        state.db.list_tool_calls_by_run(run_id, limit)
    } else {
        return Err(StatusCode::BAD_REQUEST);
    }
    .map_err(internal)?;
    Ok(Json(json!(calls)))
}

#[derive(Deserialize)]
pub(crate) struct UsageQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

fn parse_date(s: &str) -> Option<DateTime<Utc>> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|ndt| ndt.and_utc())
}

pub(crate) async fn get_usage(
    State(state): State<Arc<AppState>>,
    Query(q): Query<UsageQuery>,
) -> Result<Json<Value>, StatusCode> {
    let from = q.from.as_deref().and_then(parse_date);
    let to = q.to.as_deref().and_then(parse_date);
    let summary = state.db.get_usage_summary(from, to).map_err(internal)?;
    Ok(Json(json!(summary)))
}
