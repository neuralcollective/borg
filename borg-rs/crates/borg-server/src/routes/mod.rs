use axum::http::StatusCode;
use borg_core::db::ProjectRow;
use borg_core::types::Task;

pub(crate) mod admin;
pub(crate) use admin::*;

pub(crate) mod chat;
pub(crate) use chat::*;

pub(crate) mod cloud;
pub(crate) use cloud::*;

pub(crate) mod knowledge;
pub(crate) use knowledge::*;

pub(crate) mod linked_credentials;
pub(crate) use linked_credentials::*;

pub(crate) mod projects;
pub(crate) use projects::*;

pub(crate) mod search;
pub(crate) use search::*;

pub(crate) mod tasks;
pub(crate) use tasks::*;

pub(crate) mod utils;

pub(crate) use crate::routes_modes::{
    delete_custom_mode, get_full_modes, get_modes, list_custom_modes, upsert_custom_mode,
};

pub(crate) fn internal(e: impl std::fmt::Debug + std::fmt::Display) -> StatusCode {
    tracing::error!("internal error: {e:#}");
    tracing::debug!("internal error detail: {e:?}");
    StatusCode::INTERNAL_SERVER_ERROR
}

pub(crate) fn require_project_access(
    state: &crate::AppState,
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
    state: &crate::AppState,
    workspace: &crate::auth::WorkspaceContext,
    task_id: i64,
) -> Result<Task, StatusCode> {
    state
        .db
        .get_task_in_workspace(workspace.id, task_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)
}

