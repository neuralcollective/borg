use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use borg_core::cron::{compute_next_run, CronJobType};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use super::internal;
use crate::AppState;

#[derive(Deserialize)]
pub(crate) struct CreateCronJobBody {
    pub name: String,
    pub schedule: String,
    pub job_type: CronJobType,
    pub config: Value,
    pub project_id: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct UpdateCronJobBody {
    pub name: Option<String>,
    pub schedule: Option<String>,
    pub job_type: Option<CronJobType>,
    pub config: Option<Value>,
    pub project_id: Option<Option<i64>>,
    pub enabled: Option<bool>,
}

pub(crate) async fn list_cron_jobs(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let jobs = state.db.list_cron_jobs().map_err(internal)?;
    Ok(Json(json!(jobs)))
}

pub(crate) async fn create_cron_job(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateCronJobBody>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    if compute_next_run(&body.schedule, Utc::now()).is_none() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let id = state
        .db
        .insert_cron_job(
            &body.name,
            &body.schedule,
            &body.job_type,
            &body.config,
            body.project_id,
        )
        .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub(crate) async fn update_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateCronJobBody>,
) -> Result<StatusCode, StatusCode> {
    if let Some(ref schedule) = body.schedule {
        if compute_next_run(schedule, Utc::now()).is_none() {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    state
        .db
        .get_cron_job(id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    state
        .db
        .update_cron_job(
            id,
            body.name.as_deref(),
            body.schedule.as_deref(),
            body.job_type.as_ref(),
            body.config.as_ref(),
            body.project_id,
            body.enabled,
        )
        .map_err(internal)?;
    Ok(StatusCode::OK)
}

pub(crate) async fn delete_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    let deleted = state.db.delete_cron_job(id).map_err(internal)?;
    if deleted {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

pub(crate) async fn trigger_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let job = state
        .db
        .get_cron_job(id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let run_id = state.db.insert_cron_run(job.id).map_err(internal)?;

    let db = Arc::clone(&state.db);
    let job_clone = job.clone();
    tokio::spawn(async move {
        if let Err(e) = borg_core::cron::execute_job(&db, &job_clone).await {
            tracing::error!(job_id = job_clone.id, err = %e, "cron: manual trigger failed");
        }
    });

    let now = Utc::now();
    let next = compute_next_run(&job.schedule, now);
    let _ = state.db.update_cron_job_after_run(job.id, &now, next.as_ref());

    Ok(Json(json!({ "run_id": run_id })))
}

pub(crate) async fn list_cron_runs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    state
        .db
        .get_cron_job(id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let runs = state.db.list_cron_runs(id, 50).map_err(internal)?;
    Ok(Json(json!(runs)))
}
