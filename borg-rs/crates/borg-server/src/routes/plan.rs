use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::AppState;

use super::internal;

#[derive(Deserialize)]
pub(crate) struct PlanTodosQuery {
    pub status: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CreatePlanTodoBody {
    pub title: String,
    #[serde(default)]
    pub details: String,
    #[serde(default = "default_todo_status")]
    pub status: String,
    #[serde(default = "default_priority")]
    pub priority: i64,
}

#[derive(Deserialize)]
pub(crate) struct PatchPlanTodoBody {
    pub title: Option<String>,
    pub details: Option<String>,
    pub status: Option<String>,
    pub priority: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct BulkUpsertPlanTodosBody {
    pub items: Vec<CreatePlanTodoBody>,
}

fn default_todo_status() -> String {
    "todo".to_string()
}

fn default_priority() -> i64 {
    100
}

fn valid_status(status: &str) -> bool {
    matches!(status, "todo" | "doing" | "blocked" | "done")
}

pub(crate) async fn list_plan_todos(
    State(state): State<Arc<AppState>>,
    Query(q): Query<PlanTodosQuery>,
) -> Result<Json<Value>, StatusCode> {
    let status = q.status.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = status {
        if !valid_status(s) {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    let todos = state.db.list_plan_todos(status).map_err(internal)?;
    Ok(Json(json!(todos)))
}

pub(crate) async fn create_plan_todo(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreatePlanTodoBody>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let title = body.title.trim();
    if title.is_empty() || !valid_status(&body.status) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let id = state
        .db
        .upsert_plan_todo(title, body.details.trim(), body.status.trim(), body.priority)
        .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub(crate) async fn bulk_upsert_plan_todos(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BulkUpsertPlanTodosBody>,
) -> Result<Json<Value>, StatusCode> {
    if body.items.is_empty() {
        return Ok(Json(json!({ "upserted": 0, "ids": [] })));
    }
    let mut ids = Vec::with_capacity(body.items.len());
    for item in body.items {
        let title = item.title.trim();
        let status = item.status.trim();
        if title.is_empty() || !valid_status(status) {
            return Err(StatusCode::BAD_REQUEST);
        }
        let id = state
            .db
            .upsert_plan_todo(title, item.details.trim(), status, item.priority)
            .map_err(internal)?;
        ids.push(id);
    }
    Ok(Json(json!({ "upserted": ids.len(), "ids": ids })))
}

pub(crate) async fn patch_plan_todo(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<PatchPlanTodoBody>,
) -> Result<StatusCode, StatusCode> {
    let status = body.status.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = status {
        if !valid_status(s) {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    let updated = state
        .db
        .update_plan_todo(
            id,
            body.title.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            body.details.as_deref().map(str::trim),
            status,
            body.priority,
        )
        .map_err(internal)?;
    if updated {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

pub(crate) async fn delete_plan_todo(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    let deleted = state.db.delete_plan_todo(id).map_err(internal)?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}
