use axum::http::StatusCode;
use borg_core::{db::ProjectRow, types::Task};

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
    tracing::error!("internal error: {e:?}");
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

/// Like `require_project_access` but also checks per-project shares as fallback.
/// Returns the project and the effective role ("owner"/"admin"/"member"/"viewer"/"editor").
pub(crate) fn require_project_access_with_shares(
    state: &crate::AppState,
    user: &crate::auth::AuthUser,
    workspace: &crate::auth::WorkspaceContext,
    project_id: i64,
) -> Result<(ProjectRow, String), StatusCode> {
    if let Some(project) = state
        .db
        .get_project_in_workspace(workspace.id, project_id)
        .map_err(internal)?
    {
        return Ok((project, workspace.role.clone()));
    }
    if user.id > 0 {
        if let Some(share) = state
            .db
            .get_user_project_share(project_id, user.id)
            .map_err(internal)?
        {
            let project = state
                .db
                .get_project(project_id)
                .map_err(internal)?
                .ok_or(StatusCode::NOT_FOUND)?;
            return Ok((project, share.role));
        }
    }
    Err(StatusCode::NOT_FOUND)
}

pub(crate) fn role_level(role: &str) -> u8 {
    match role {
        "owner" | "admin" => 3,
        "editor" | "member" => 2,
        "viewer" => 1,
        _ => 0,
    }
}

pub(crate) fn require_min_role(role: &str, minimum: &str) -> Result<(), StatusCode> {
    if role_level(role) >= role_level(minimum) {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
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
