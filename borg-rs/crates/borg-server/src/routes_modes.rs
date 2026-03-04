use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use borg_core::{modes::all_modes, types::PipelineMode};
use serde_json::{json, Value};

use crate::{routes::internal, AppState};

fn get_custom_modes(db: &borg_core::db::Db) -> Vec<PipelineMode> {
    let raw = match db.get_config("custom_modes") {
        Ok(Some(v)) => v,
        _ => return Vec::new(),
    };
    serde_json::from_str::<Vec<PipelineMode>>(&raw).unwrap_or_default()
}

fn save_custom_modes(db: &borg_core::db::Db, modes: &[PipelineMode]) -> Result<(), StatusCode> {
    let serialized = serde_json::to_string(modes).map_err(internal)?;
    db.set_config("custom_modes", &serialized)
        .map_err(internal)?;
    Ok(())
}

fn valid_mode_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn is_experimental_mode(name: &str) -> bool {
    !matches!(name, "sweborg" | "swe" | "lawborg" | "legal")
}

pub(crate) async fn get_modes(State(state): State<Arc<AppState>>) -> Json<Value> {
    let mut merged_modes = all_modes();
    merged_modes.extend(get_custom_modes(&state.db));
    let modes: Vec<Value> = merged_modes
        .into_iter()
        .map(|m| {
            let phases: Vec<Value> = m
                .phases
                .iter()
                .map(|p| json!({ "name": p.name, "label": p.label }))
                .collect();
            json!({
                "name": m.name,
                "label": m.label,
                "category": m.category,
                "phases": phases,
                "experimental": is_experimental_mode(&m.name),
            })
        })
        .collect();
    Json(json!(modes))
}

pub(crate) async fn get_full_modes(State(state): State<Arc<AppState>>) -> Json<Value> {
    let mut merged_modes = all_modes();
    merged_modes.extend(get_custom_modes(&state.db));
    Json(json!(merged_modes))
}

pub(crate) async fn list_custom_modes(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!(get_custom_modes(&state.db)))
}

pub(crate) async fn upsert_custom_mode(
    State(state): State<Arc<AppState>>,
    Json(mode): Json<PipelineMode>,
) -> Result<Json<Value>, StatusCode> {
    let name = mode.name.trim();
    if !valid_mode_name(name) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if !state.config.experimental_domains && is_experimental_mode(name) {
        return Err(StatusCode::FORBIDDEN);
    }
    if all_modes().iter().any(|m| m.name == name) {
        return Err(StatusCode::CONFLICT);
    }
    if mode.phases.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut custom = get_custom_modes(&state.db);
    custom.retain(|m| m.name != name);
    custom.push(mode);
    save_custom_modes(&state.db, &custom)?;
    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn delete_custom_mode(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    if all_modes().iter().any(|m| m.name == name) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut custom = get_custom_modes(&state.db);
    let before = custom.len();
    custom.retain(|m| m.name != name);
    if before == custom.len() {
        return Err(StatusCode::NOT_FOUND);
    }
    save_custom_modes(&state.db, &custom)?;
    Ok(Json(json!({ "ok": true })))
}
