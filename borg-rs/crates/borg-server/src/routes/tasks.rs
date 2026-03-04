use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use borg_core::{
    pipeline::PipelineEvent,
    types::{PhaseType, PipelineMode, Task},
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::AppState;

use super::{internal, TaskMessageJson, TaskOutputJson};

#[derive(Deserialize)]
pub(crate) struct CreateTaskBody {
    pub title: String,
    pub description: Option<String>,
    pub mode: Option<String>,
    pub repo: Option<String>,
    pub project_id: Option<i64>,
    pub task_type: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct PatchTaskBody {
    pub title: Option<String>,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CreateMessageBody {
    pub role: String,
    pub content: String,
}

#[derive(Deserialize)]
pub(crate) struct TasksQuery {
    pub repo: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct TaskDiagnosticsQuery {
    pub limit: Option<i64>,
}

pub(crate) async fn list_tasks(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TasksQuery>,
) -> Result<Json<Value>, StatusCode> {
    let tasks = state
        .db
        .list_all_tasks(q.repo.as_deref())
        .map_err(internal)?;
    Ok(Json(json!(tasks)))
}

pub(crate) async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    match state.db.get_task_with_outputs(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some((task, outputs)) => {
            let outputs_json: Vec<TaskOutputJson> =
                outputs.into_iter().map(TaskOutputJson::from).collect();
            let structured = state.db.get_task_structured_data(task.id).unwrap_or_default();
            let mut v = serde_json::to_value(&task).map_err(internal)?;
            if let Some(obj) = v.as_object_mut() {
                obj.insert(
                    "outputs".into(),
                    serde_json::to_value(outputs_json).map_err(internal)?,
                );
                if !structured.is_empty() {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&structured) {
                        obj.insert("structured_data".into(), parsed);
                    }
                }
            }
            Ok(Json(v))
        },
    }
}

fn normalize_failure_signature(text: &str) -> String {
    let mut out = String::with_capacity(256);
    let mut prev_space = false;
    for ch in text.chars().flat_map(|c| c.to_lowercase()) {
        let mapped = if ch.is_ascii_digit() {
            '#'
        } else if ch.is_ascii_alphanumeric() {
            ch
        } else {
            ' '
        };
        if mapped == ' ' {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(mapped);
            prev_space = false;
        }
        if out.len() >= 220 {
            break;
        }
    }
    out.trim().to_string()
}

pub(crate) async fn get_task_diagnostics(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(q): Query<TaskDiagnosticsQuery>,
) -> Result<Json<Value>, StatusCode> {
    let task = state.db.get_task(id).map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;
    let limit = q.limit.unwrap_or(40).clamp(10, 200);
    let outputs = state.db.get_task_outputs(id).map_err(internal)?;
    let queue_entries = state
        .db
        .get_queue_entries_for_task(id)
        .map_err(internal)?;
    let events = state.db.list_task_events(id, limit).map_err(internal)?;

    let mut same_failure_streak = 0u32;
    if outputs.len() >= 3 {
        let mut iter = outputs
            .iter()
            .rev()
            .filter(|o| o.exit_code != 0)
            .take(3)
            .map(|o| (o.phase.as_str(), normalize_failure_signature(&o.output)));
        if let Some((phase0, sig0)) = iter.next() {
            same_failure_streak = 1;
            for (phase, sig) in iter {
                if phase == phase0 && sig == sig0 {
                    same_failure_streak += 1;
                }
            }
        }
    }

    let recent_outputs: Vec<TaskOutputJson> = outputs
        .into_iter()
        .rev()
        .take(limit as usize)
        .map(TaskOutputJson::from)
        .collect();

    Ok(Json(json!({
        "task": task,
        "summary": {
            "attempt": task.attempt,
            "max_attempts": task.max_attempts,
            "status": task.status,
            "review_status": task.review_status,
            "started_at": task.started_at.map(|ts| ts.to_rfc3339()),
            "completed_at": task.completed_at.map(|ts| ts.to_rfc3339()),
            "duration_secs": task.duration_secs,
            "stuck_suspected": same_failure_streak >= 3,
            "same_failure_streak": same_failure_streak,
            "has_queue_entry": !queue_entries.is_empty(),
        },
        "queue_entries": queue_entries,
        "recent_outputs": recent_outputs,
        "recent_events": events,
    })))
}

pub(crate) async fn create_task(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateTaskBody>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let repo = if let Some(r) = body.repo {
        r
    } else if let Some(pid) = body.project_id {
        // Resolve project's dedicated repo
        state
            .db
            .get_project(pid)
            .map_err(internal)?
            .and_then(|p| if p.repo_path.is_empty() { None } else { Some(p.repo_path) })
            .unwrap_or_else(|| state.config.pipeline_repo.clone())
    } else {
        state.config.pipeline_repo.clone()
    };
    let mode = body.mode.unwrap_or_else(|| "sweborg".into());
    let task = Task {
        id: 0,
        title: body.title,
        description: body.description.unwrap_or_default(),
        repo_path: repo,
        branch: String::new(),
        status: "backlog".into(),
        attempt: 0,
        max_attempts: 5,
        last_error: String::new(),
        created_by: "api".into(),
        notify_chat: String::new(),
        created_at: Utc::now(),
        session_id: String::new(),
        mode,
        backend: String::new(),
        project_id: body.project_id.unwrap_or(0),
        task_type: body.task_type.unwrap_or_default(),
        started_at: None,
        completed_at: None,
        duration_secs: None,
        review_status: None,
        revision_count: 0,
    };
    let id = state.db.insert_task(&task).map_err(internal)?;
    let _ = state.db.log_event_full(
        Some(id),
        None,
        Some(task.project_id).filter(|&p| p > 0),
        "api",
        "task.created",
        &json!({ "title": task.title }),
    );
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub(crate) async fn patch_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<PatchTaskBody>,
) -> Result<StatusCode, StatusCode> {
    let task = state.db.get_task(id).map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;
    let title = body.title.as_deref().unwrap_or(&task.title);
    let desc = body.description.as_deref().unwrap_or(&task.description);
    state.db.update_task_description(id, title, desc).map_err(internal)?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
pub(crate) struct ReviewAction {
    #[serde(default)]
    feedback: Option<String>,
}

pub(crate) async fn approve_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let task = state.db.get_task(id).map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;
    let mode = borg_core::modes::get_mode(&task.mode)
        .or_else(|| {
            state.db.get_config("custom_modes").ok().flatten()
                .and_then(|raw| serde_json::from_str::<Vec<PipelineMode>>(&raw).ok())
                .and_then(|modes| modes.into_iter().find(|m| m.name == task.mode))
        })
        .ok_or(StatusCode::BAD_REQUEST)?;
    let phase = mode.get_phase(&task.status).ok_or(StatusCode::BAD_REQUEST)?;
    if phase.phase_type != PhaseType::HumanReview {
        return Err(StatusCode::BAD_REQUEST);
    }
    let next = phase.next.clone();
    state.db.set_review_status(id, "approved").map_err(internal)?;
    state.db.update_task_status(id, &next, None).map_err(internal)?;
    let pid = if task.project_id > 0 { Some(task.project_id) } else { None };
    let _ = state.db.log_event_full(Some(id), None, pid, "reviewer", "task.approved", &json!({}));
    Ok(Json(json!({ "ok": true, "next_phase": next })))
}

pub(crate) async fn reject_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<ReviewAction>,
) -> Result<Json<Value>, StatusCode> {
    let task = state.db.get_task(id).map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;
    let reason = body.feedback.unwrap_or_else(|| "Rejected by reviewer".into());
    state.db.set_review_status(id, "rejected").map_err(internal)?;
    state.db.update_task_status(id, "failed", Some(&reason)).map_err(internal)?;
    let pid = if task.project_id > 0 { Some(task.project_id) } else { None };
    let _ = state.db.log_event_full(Some(id), None, pid, "reviewer", "task.rejected", &json!({ "reason": reason }));
    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn request_revision(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<ReviewAction>,
) -> Result<Json<Value>, StatusCode> {
    let task = state.db.get_task(id).map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;
    let feedback = body.feedback.unwrap_or_else(|| "Revision requested".into());

    let mode = borg_core::modes::get_mode(&task.mode)
        .or_else(|| {
            state.db.get_config("custom_modes").ok().flatten()
                .and_then(|raw| serde_json::from_str::<Vec<PipelineMode>>(&raw).ok())
                .and_then(|modes| modes.into_iter().find(|m| m.name == task.mode))
        })
        .ok_or(StatusCode::BAD_REQUEST)?;

    let mut target_phase = "implement".to_string();
    for p in &mode.phases {
        if p.name == task.status {
            break;
        }
        if p.phase_type == PhaseType::Agent {
            target_phase = p.name.clone();
        }
    }

    state.db.set_review_status(id, "revision_requested").map_err(internal)?;
    state.db.increment_revision_count(id).map_err(internal)?;
    state.db.insert_task_message(id, "user", &feedback).map_err(internal)?;
    state.db.update_task_status(id, &target_phase, None).map_err(internal)?;
    let pid = if task.project_id > 0 { Some(task.project_id) } else { None };
    let _ = state.db.log_event_full(Some(id), None, pid, "reviewer", "task.revision_requested", &json!({ "feedback": feedback }));
    Ok(Json(json!({ "ok": true, "target_phase": target_phase })))
}

pub(crate) async fn get_revision_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let task = state.db.get_task(id).map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;
    let messages = state.db.get_task_messages(id).map_err(internal)?;
    let outputs = state.db.get_task_outputs(id).map_err(internal)?;

    let mut rounds: Vec<Value> = Vec::new();
    let mut current_round = 0;
    let mut round_outputs: Vec<&borg_core::db::TaskOutput> = Vec::new();
    let mut round_feedback: Option<String> = None;
    let mut round_feedback_at: Option<String> = None;
    let mut msg_iter = messages.iter().filter(|m| m.role == "user").peekable();

    for output in &outputs {
        while let Some(msg) = msg_iter.peek() {
            if msg.created_at <= output.created_at {
                if !round_outputs.is_empty() || round_feedback.is_some() {
                    rounds.push(json!({
                        "round": current_round,
                        "feedback": round_feedback,
                        "feedback_at": round_feedback_at,
                        "phases": round_outputs.iter().map(|o| json!({
                            "phase": o.phase,
                            "exit_code": o.exit_code,
                            "output_preview": if o.output.len() > 500 { format!("{}…", &o.output[..500]) } else { o.output.clone() },
                            "created_at": o.created_at.to_rfc3339(),
                        })).collect::<Vec<_>>(),
                    }));
                    round_outputs.clear();
                }
                current_round += 1;
                round_feedback = Some(msg.content.clone());
                round_feedback_at = Some(msg.created_at.to_rfc3339());
                msg_iter.next();
            } else {
                break;
            }
        }
        round_outputs.push(output);
    }

    if !round_outputs.is_empty() || round_feedback.is_some() {
        rounds.push(json!({
            "round": current_round,
            "feedback": round_feedback,
            "feedback_at": round_feedback_at,
            "phases": round_outputs.iter().map(|o| json!({
                "phase": o.phase,
                "exit_code": o.exit_code,
                "output_preview": if o.output.len() > 500 { format!("{}…", &o.output[..500]) } else { o.output.clone() },
                "created_at": o.created_at.to_rfc3339(),
            })).collect::<Vec<_>>(),
        }));
    }

    for msg in msg_iter {
        current_round += 1;
        rounds.push(json!({
            "round": current_round,
            "feedback": msg.content,
            "feedback_at": msg.created_at.to_rfc3339(),
            "phases": [],
        }));
    }

    Ok(Json(json!({
        "task_id": id,
        "revision_count": task.revision_count,
        "review_status": task.review_status,
        "rounds": rounds,
    })))
}

pub(crate) async fn get_task_citations(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    state.db.get_task(id).map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;
    let citations = state.db.get_task_citations(id).map_err(internal)?;
    Ok(Json(json!(citations)))
}

pub(crate) async fn verify_task_citations(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let task = state.db.get_task(id).map_err(internal)?.ok_or(StatusCode::NOT_FOUND)?;

    state.db.delete_task_citations(id).map_err(internal)?;

    let repo_path = task.repo_path.clone();
    let branch = task.branch.clone();
    let markdown_content = tokio::task::spawn_blocking(move || {
        let git = borg_core::git::Git::new(&repo_path);
        let tree_result = git.exec(&repo_path, &["ls-tree", "-r", "--name-only", &branch]);
        let files: Vec<String> = tree_result
            .map(|r| r.stdout.lines().map(String::from).collect())
            .unwrap_or_default();
        files
            .into_iter()
            .filter(|f| f.ends_with(".md"))
            .filter_map(|f| {
                let ref_path = format!("{branch}:{f}");
                git.exec(&repo_path, &["show", &ref_path])
                    .ok()
                    .map(|r| r.stdout)
            })
            .collect::<Vec<String>>()
            .join("\n\n")
    })
    .await
    .map_err(|e| { tracing::error!("spawn_blocking: {e}"); StatusCode::INTERNAL_SERVER_ERROR })?;

    if markdown_content.is_empty() {
        return Ok(Json(json!({ "verified": 0, "citations": [] })));
    }

    let citations = borg_domains::legal::citations::extract_citations(&markdown_content);
    if citations.is_empty() {
        return Ok(Json(json!({ "verified": 0, "citations": [] })));
    }

    let cl = borg_domains::legal::courtlistener::CourtListenerClient::new();
    let results = borg_domains::legal::citations::verify_citations(&citations, &cl).await;

    for r in &results {
        let _ = state.db.insert_citation_verification(
            id,
            &r.citation_text,
            &r.citation_type,
            &r.status,
            &r.source,
            &r.treatment,
            &r.checked_at,
        );
    }

    let verified = results.iter().filter(|r| r.status == "verified").count();
    Ok(Json(json!({ "verified": verified, "total": results.len(), "citations": results })))
}

pub(crate) async fn retry_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    match state.db.get_task(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some(_) => {
            state.db.requeue_task(id).map_err(internal)?;
            Ok(StatusCode::OK)
        },
    }
}

pub(crate) async fn retry_all_failed(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    const MAX_RETRY_BATCH: usize = 20;
    let tasks = state.db.list_all_tasks(None).map_err(internal)?;
    let mut count = 0;
    for task in &tasks {
        if task.status == "failed" {
            if count >= MAX_RETRY_BATCH {
                break;
            }
            state.db.requeue_task(task.id).map_err(internal)?;
            count += 1;
        }
    }
    Ok(Json(json!({ "requeued": count })))
}

#[derive(Deserialize)]
pub(crate) struct UnblockBody {
    pub response: String,
}

pub(crate) async fn unblock_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<UnblockBody>,
) -> Result<StatusCode, StatusCode> {
    match state.db.get_task(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some(task) if task.status != "blocked" => Err(StatusCode::CONFLICT),
        Some(task) => {
            state
                .db
                .insert_task_message(id, "user", &body.response)
                .map_err(internal)?;
            let next_phase = borg_core::modes::get_mode(&task.mode)
                .map(|m| {
                    m.phases.iter()
                        .find(|p| p.phase_type == PhaseType::Agent)
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "implement".to_string())
                })
                .unwrap_or_else(|| "implement".to_string());
            state
                .db
                .update_task_status(id, &next_phase, None)
                .map_err(internal)?;
            Ok(StatusCode::OK)
        },
    }
}

pub(crate) async fn get_task_messages(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    match state.db.get_task(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some(_) => {
            let messages = state.db.get_task_messages(id).map_err(internal)?;
            let messages_json: Vec<TaskMessageJson> =
                messages.into_iter().map(TaskMessageJson::from).collect();
            Ok(Json(json!({ "messages": messages_json })))
        },
    }
}

pub(crate) async fn post_task_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<CreateMessageBody>,
) -> Result<StatusCode, StatusCode> {
    match state.db.get_task(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some(_) => {
            if body.role != "user" && body.role != "system" {
                return Err(StatusCode::BAD_REQUEST);
            }
            state
                .db
                .insert_task_message(id, &body.role, &body.content)
                .map_err(internal)?;
            let _ = state.pipeline_event_tx.send(PipelineEvent::Output {
                task_id: Some(id),
                message: body.content.clone(),
            });
            Ok(StatusCode::CREATED)
        },
    }
}
