use std::{
    collections::HashMap,
    sync::{
        atomic::Ordering,
        Arc,
    },
};

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Json,
    },
};
use borg_core::{
    linked_credentials::{PROVIDER_CLAUDE, PROVIDER_OPENAI},
    types::{PhaseConfig, PhaseContext, RepoConfig, Task},
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};

use super::internal;
use crate::AppState;

pub(crate) const SETTINGS_KEYS: &[&str] = &[
    "continuous_mode",
    "release_interval_mins",
    "pipeline_max_backlog",
    "agent_timeout_s",
    "pipeline_seed_cooldown_s",
    "pipeline_tick_s",
    "model",
    "container_memory_mb",
    "assistant_name",
    "pipeline_max_agents",
    "pipeline_agent_cooldown_s",
    "proposal_promote_threshold",
    "backend",
    "git_claude_coauthor",
    "git_user_coauthor",
    "chat_disallowed_tools",
    "pipeline_disallowed_tools",
    "public_url",
    "dropbox_client_id",
    "dropbox_client_secret",
    "google_client_id",
    "google_client_secret",
    "ms_client_id",
    "ms_client_secret",
    "storage_backend",
    "s3_bucket",
    "s3_region",
    "s3_endpoint",
    "s3_prefix",
    "backup_backend",
    "backup_mode",
    "backup_bucket",
    "backup_region",
    "backup_endpoint",
    "backup_prefix",
    "backup_poll_interval_s",
    "project_max_bytes",
    "knowledge_max_bytes",
    "cloud_import_max_batch_files",
    "ingestion_queue_backend",
    "sqs_queue_url",
    "sqs_region",
    "search_backend",
    "vespa_url",
    "vespa_namespace",
    "vespa_document_type",
    "experimental_domains",
    "visible_categories",
    "model_override",
    "dashboard_mode",
];

pub(crate) const SETTINGS_DEFAULTS: &[(&str, &str)] = &[
    ("continuous_mode", "false"),
    ("release_interval_mins", "180"),
    ("pipeline_max_backlog", "5"),
    ("agent_timeout_s", "600"),
    ("pipeline_seed_cooldown_s", "3600"),
    ("pipeline_tick_s", "10"),
    ("model", "claude-sonnet-4-6"),
    ("container_memory_mb", "2048"),
    ("assistant_name", "Borg"),
    ("pipeline_max_agents", "2"),
    ("pipeline_agent_cooldown_s", "120"),
    ("proposal_promote_threshold", "70"),
    ("backend", "claude"),
    ("git_claude_coauthor", "false"),
    ("git_user_coauthor", ""),
    ("chat_disallowed_tools", ""),
    ("pipeline_disallowed_tools", ""),
    ("public_url", ""),
    ("dropbox_client_id", ""),
    ("dropbox_client_secret", ""),
    ("google_client_id", ""),
    ("google_client_secret", ""),
    ("ms_client_id", ""),
    ("ms_client_secret", ""),
    ("storage_backend", "local"),
    ("s3_bucket", ""),
    ("s3_region", "us-east-1"),
    ("s3_endpoint", ""),
    ("s3_prefix", "borg/"),
    ("backup_backend", "disabled"),
    ("backup_mode", "active_work_only"),
    ("backup_bucket", ""),
    ("backup_region", "us-east-1"),
    ("backup_endpoint", ""),
    ("backup_prefix", "borg-backups/"),
    ("backup_poll_interval_s", "300"),
    ("project_max_bytes", "214748364800"),
    ("knowledge_max_bytes", "536870912000"),
    ("cloud_import_max_batch_files", "1000"),
    ("ingestion_queue_backend", "disabled"),
    ("sqs_queue_url", ""),
    ("sqs_region", "us-east-1"),
    ("search_backend", "vespa"),
    ("vespa_url", "http://127.0.0.1:8080"),
    ("vespa_namespace", "borg"),
    ("vespa_document_type", "project_file"),
    ("experimental_domains", "false"),
    ("visible_categories", "Professional Services"),
    ("model_override", ""),
    ("dashboard_mode", "general"),
];

#[derive(Deserialize)]
pub(crate) struct EventsQuery {
    pub category: Option<String>,
    pub level: Option<String>,
    pub since: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct RepoQuery {
    pub repo: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CreateWorkspaceBody {
    pub name: String,
    pub kind: Option<String>,
    pub set_default: Option<bool>,
}

#[derive(Deserialize)]
pub(crate) struct AddWorkspaceMemberBody {
    pub username: String,
    pub role: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CreateUserBody {
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
    pub is_admin: Option<bool>,
}

#[derive(Deserialize)]
pub(crate) struct ChangePasswordBody {
    pub password: String,
}

#[derive(Deserialize)]
pub(crate) struct StoreKeyBody {
    pub provider: String,
    pub key_name: Option<String>,
    pub key_value: String,
    #[serde(rename = "owner")]
    pub _owner: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ConversationDumpQuery {
    thread: String,
    #[serde(default = "default_conv_limit")]
    limit: i64,
}
fn default_conv_limit() -> i64 {
    200
}

fn mcp_service_specs() -> [(&'static str, &'static str); 9] {
    [
        ("lexisnexis", "LexisNexis"),
        ("westlaw", "Westlaw"),
        ("clio", "Clio"),
        ("imanage", "iManage"),
        ("netdocuments", "NetDocuments"),
        ("congress", "Congress.gov"),
        ("openstates", "OpenStates"),
        ("canlii", "CanLII"),
        ("regulations_gov", "Regulations.gov"),
    ]
}

fn linked_credential_status_item(
    key: &str,
    label: &str,
    entry: Option<&borg_core::db::LinkedCredentialEntry>,
) -> Value {
    let Some(entry) = entry else {
        return mcp_status_item(
            key,
            label,
            "missing",
            format!("No linked {label} account for this user"),
            Some("user"),
            None,
        );
    };

    let expiry_suffix = if !entry.expires_at.is_empty() {
        if let Ok(exp) = chrono::DateTime::parse_from_rfc3339(&entry.expires_at) {
            let now = Utc::now();
            let until = exp.with_timezone(&Utc).signed_duration_since(now);
            if until.num_seconds() <= 0 {
                " — token expired".to_string()
            } else if until.num_hours() < 1 {
                format!(" — expires in {}m", until.num_minutes())
            } else if until.num_hours() < 24 {
                format!(" — expires in {}h", until.num_hours())
            } else {
                format!(" — expires in {}d", until.num_days())
            }
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    let is_expiring_soon = entry.expires_at.parse::<chrono::DateTime<chrono::FixedOffset>>().ok()
        .is_some_and(|exp| exp.with_timezone(&Utc).signed_duration_since(Utc::now()).num_hours() < 2);

    if entry.status == "connected" && !is_expiring_soon {
        let detail = if entry.account_email.is_empty() {
            format!("Linked and validated{expiry_suffix}")
        } else {
            format!("{} linked and validated{expiry_suffix}", entry.account_email)
        };
        mcp_status_item(key, label, "verified", detail, Some("user"), Some(entry.last_validated_at.clone()))
    } else if entry.status == "connected" && is_expiring_soon {
        let detail = if entry.account_email.is_empty() {
            format!("Token expiring soon{expiry_suffix}")
        } else {
            format!("{}{expiry_suffix}", entry.account_email)
        };
        mcp_status_item(key, label, "degraded", detail, Some("user"), Some(entry.last_validated_at.clone()))
    } else {
        let detail = if !entry.last_error.is_empty() {
            format!("{}{expiry_suffix}", entry.last_error)
        } else {
            format!("Linked account needs reconnect{expiry_suffix}")
        };
        mcp_status_item(key, label, "degraded", detail, Some("user"), Some(entry.last_validated_at.clone()))
    }
}

fn mcp_status_item(
    key: &str,
    label: &str,
    status: &str,
    detail: impl Into<String>,
    source: Option<&str>,
    checked_at: Option<String>,
) -> Value {
    json!({
        "key": key,
        "label": label,
        "status": status,
        "detail": detail.into(),
        "source": source.unwrap_or(""),
        "checked_at": checked_at.unwrap_or_default(),
    })
}

pub(crate) async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    let storage_result = state.file_storage.healthcheck().await;
    let search_result = if let Some(search) = &state.search {
        search.healthcheck().await
    } else {
        Ok(())
    };
    let backup = crate::backup::backup_status_snapshot(&state.db, &state.config).await;
    let ok = storage_result.is_ok() && search_result.is_ok();
    let mut search_info = serde_json::json!({
        "backend": state.search.as_ref().map(|s| s.backend_name()).unwrap_or("none"),
        "target": state.search.as_ref().map(|s| s.target()).unwrap_or_default(),
        "healthy": search_result.is_ok(),
        "error": search_result.err().map(|e| e.to_string()),
    });
    if let Some(search) = &state.search {
        let files = search.document_count("project_file").await.unwrap_or(-1);
        let chunks = search.document_count("project_chunk").await.unwrap_or(-1);
        search_info["documents"] = json!(files);
        search_info["chunks"] = json!(chunks);
    }
    Json(json!({
        "status": if ok { "ok" } else { "degraded" },
        "storage": {
            "backend": state.file_storage.backend_name(),
            "target": state.file_storage.target(),
            "healthy": storage_result.is_ok(),
            "error": storage_result.err().map(|e| e.to_string()),
        },
        "search": search_info,
        "backup": backup,
    }))
}

pub(crate) async fn get_mcp_status(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    let linked_credentials = state
        .db
        .list_user_linked_credentials(user.id)
        .map_err(internal)?;
    let linked_by_provider: HashMap<_, _> = linked_credentials
        .into_iter()
        .map(|entry| (entry.provider.clone(), entry))
        .collect();
    let available_keys = state
        .db
        .list_api_keys(&format!("workspace:{}", workspace.id))
        .map_err(internal)?;
    let mut effective_key_by_provider = HashMap::new();
    for entry in available_keys {
        let provider = entry.provider.clone();
        let replace = effective_key_by_provider.get(&provider).is_none_or(
            |current: &borg_core::db::ApiKeyEntry| {
                current.owner == "global" && entry.owner != "global"
            },
        );
        if replace {
            effective_key_by_provider.insert(provider, entry);
        }
    }

    let search_result = if let Some(search) = &state.search {
        search.healthcheck().await
    } else {
        Ok(())
    };
    let search_backend = state
        .search
        .as_ref()
        .map(|s| s.backend_name())
        .unwrap_or("none");
    let search_target = state
        .search
        .as_ref()
        .map(|s| s.target())
        .unwrap_or_default();

    let borg_mcp_path = if let Ok(path) = std::env::var("BORG_MCP_SERVER") {
        path
    } else {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../sidecar/borg-mcp/server.js")
            .to_string_lossy()
            .to_string()
    };
    let lawborg_mcp_path = if let Ok(path) = std::env::var("LAWBORG_MCP_SERVER") {
        path
    } else {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../sidecar/lawborg-mcp/server.js")
            .to_string_lossy()
            .to_string()
    };

    let agent_access = vec![
        linked_credential_status_item("claude", "Claude Code", linked_by_provider.get(PROVIDER_CLAUDE)),
        linked_credential_status_item("openai", "Codex / ChatGPT", linked_by_provider.get(PROVIDER_OPENAI)),
    ];

    let runtime = vec![
        if std::path::Path::new(&borg_mcp_path).exists() {
            mcp_status_item(
                "borg_mcp",
                "Borg MCP",
                "verified",
                format!("Sidecar present at {borg_mcp_path}"),
                Some("filesystem"),
                Some(Utc::now().to_rfc3339()),
            )
        } else {
            mcp_status_item(
                "borg_mcp",
                "Borg MCP",
                "missing",
                format!("Sidecar missing at {borg_mcp_path}"),
                Some("filesystem"),
                None,
            )
        },
        if search_backend == "none" {
            mcp_status_item(
                "borgsearch",
                "BorgSearch Tools",
                "missing",
                "No search backend configured",
                Some("runtime"),
                None,
            )
        } else if search_result.is_ok() {
            mcp_status_item(
                "borgsearch",
                "BorgSearch Tools",
                "verified",
                format!("{search_backend} healthy at {search_target}"),
                Some("endpoint"),
                Some(Utc::now().to_rfc3339()),
            )
        } else {
            mcp_status_item(
                "borgsearch",
                "BorgSearch Tools",
                "degraded",
                search_result
                    .err()
                    .map(|err| err.to_string())
                    .unwrap_or_else(|| "Search healthcheck failed".to_string()),
                Some("endpoint"),
                Some(Utc::now().to_rfc3339()),
            )
        },
        if std::path::Path::new(&lawborg_mcp_path).exists() {
            mcp_status_item(
                "lawborg_mcp",
                "Lawborg MCP",
                "verified",
                format!("Sidecar present at {lawborg_mcp_path}"),
                Some("filesystem"),
                Some(Utc::now().to_rfc3339()),
            )
        } else {
            mcp_status_item(
                "lawborg_mcp",
                "Lawborg MCP",
                "missing",
                format!("Sidecar missing at {lawborg_mcp_path}"),
                Some("filesystem"),
                None,
            )
        },
    ];

    let services: Vec<Value> = mcp_service_specs()
        .into_iter()
        .map(|(provider, label)| {
            if let Some(entry) = effective_key_by_provider.get(provider) {
                let source = if entry.owner == "global" {
                    "global"
                } else {
                    "workspace"
                };
                mcp_status_item(
                    provider,
                    label,
                    "configured",
                    format!("Credential configured via {source} scope"),
                    Some(source),
                    Some(entry.created_at.clone()),
                )
            } else {
                mcp_status_item(
                    provider,
                    label,
                    "missing",
                    "No credential configured via workspace or global scope",
                    None,
                    None,
                )
            }
        })
        .collect();

    let mut verified = 0;
    let mut configured = 0;
    let mut degraded = 0;
    let mut missing = 0;
    for item in agent_access
        .iter()
        .chain(runtime.iter())
        .chain(services.iter())
    {
        match item
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("missing")
        {
            "verified" => verified += 1,
            "configured" => configured += 1,
            "degraded" => degraded += 1,
            _ => missing += 1,
        }
    }

    let mut service_counts: HashMap<&str, i64> = HashMap::new();
    service_counts.insert("verified", 0);
    service_counts.insert("configured", 0);
    service_counts.insert("degraded", 0);
    service_counts.insert("missing", 0);
    for item in &services {
        let status = item
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("missing");
        *service_counts.entry(status).or_insert(0) += 1;
    }

    Ok(Json(json!({
        "generated_at": Utc::now().to_rfc3339(),
        "summary": {
            "verified": verified,
            "configured": configured,
            "degraded": degraded,
            "missing": missing,
        },
        "agent_access": agent_access,
        "runtime": runtime,
        "services": services,
        "workspace": {
            "id": workspace.id,
            "name": workspace.name,
        },
        "service_rollup": {
            "verified": service_counts.get("verified").copied().unwrap_or(0),
            "configured": service_counts.get("configured").copied().unwrap_or(0),
            "degraded": service_counts.get("degraded").copied().unwrap_or(0),
            "missing": service_counts.get("missing").copied().unwrap_or(0),
        }
    })))
}

pub(crate) async fn get_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let uptime_s = state.start_time.elapsed().as_secs();
    let now = chrono::Utc::now().timestamp();

    let watched_repos: Vec<Value> = state
        .config
        .watched_repos
        .iter()
        .map(|r| {
            json!({
                "path": r.path,
                "test_cmd": r.test_cmd,
                "is_self": r.is_self,
                "auto_merge": r.auto_merge,
                "mode": r.mode,
            })
        })
        .collect();

    let (active, merged, failed, total) = state.db.task_stats().map_err(internal)?;

    let model = state
        .db
        .get_config("model")
        .map_err(internal)?
        .unwrap_or_else(|| "claude-sonnet-4-6".into());

    let release_interval_mins: i64 = state
        .db
        .get_config("release_interval_mins")
        .map_err(internal)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(180);

    let continuous_mode: bool = state
        .db
        .get_config("continuous_mode")
        .map_err(internal)?
        .map(|v| v == "true")
        .unwrap_or(false);

    let assistant_name = state
        .db
        .get_config("assistant_name")
        .map_err(internal)?
        .unwrap_or_else(|| "Borg".into());

    let rebase_count = state
        .db
        .count_tasks_with_status("rebase")
        .map_err(internal)?;
    let queued_count = state
        .db
        .count_queue_with_status("queued")
        .map_err(internal)?
        + state
            .db
            .count_queue_with_status("merging")
            .map_err(internal)?;
    let last_merge_ts = state.db.get_ts("last_release_ts");
    let no_merge_mins = if last_merge_ts > 0 {
        ((now - last_merge_ts).max(0)) / 60
    } else {
        0
    };
    let rebase_backlog_alert = rebase_count >= 50;
    let no_merge_alert = queued_count > 0 && last_merge_ts > 0 && (now - last_merge_ts) >= 60 * 60;
    let guardrail_alert = rebase_backlog_alert || no_merge_alert;
    let ai_requests = state.ai_request_count.load(Ordering::Relaxed);
    state.db.set_ts("ai_request_count", ai_requests as i64);
    let backup = crate::backup::backup_status_snapshot(&state.db, &state.config).await;

    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_s": uptime_s,
        "model": model,
        "watched_repos": watched_repos,
        "release_interval_mins": release_interval_mins,
        "continuous_mode": continuous_mode,
        "assistant_name": assistant_name,
        "active_tasks": active,
        "merged_tasks": merged,
        "ai_requests": ai_requests,
        "failed_tasks": failed,
        "total_tasks": total,
        "dispatched_agents": 0,
        "guardrail_alert": guardrail_alert,
        "guardrail_rebase_count": rebase_count,
        "guardrail_queued_count": queued_count,
        "guardrail_no_merge_mins": no_merge_mins,
        "storage": {
            "backend": state.file_storage.backend_name(),
            "target": state.file_storage.target(),
        },
        "search": {
            "backend": state.search.as_ref().map(|s| s.backend_name()).unwrap_or("none"),
            "target": state.search.as_ref().map(|s| s.target()).unwrap_or_default(),
        },
        "backup": backup,
    })))
}

pub(crate) async fn list_queue(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let entries = state.db.list_queue().map_err(internal)?;
    Ok(Json(json!(entries)))
}

pub(crate) async fn list_proposals(
    State(state): State<Arc<AppState>>,
    Query(q): Query<RepoQuery>,
) -> Result<Json<Value>, StatusCode> {
    let proposals = state
        .db
        .list_all_proposals(q.repo.as_deref())
        .map_err(internal)?;
    Ok(Json(json!(proposals)))
}

pub(crate) async fn approve_proposal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let proposal = state
        .db
        .get_proposal(id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;

    state
        .db
        .update_proposal_status(id, "approved")
        .map_err(internal)?;

    let task = Task {
        id: 0,
        title: proposal.title.clone(),
        description: proposal.description.clone(),
        repo_path: proposal.repo_path.clone(),
        branch: String::new(),
        status: "backlog".into(),
        attempt: 0,
        max_attempts: 5,
        last_error: String::new(),
        created_by: "proposal".into(),
        notify_chat: String::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        session_id: String::new(),
        mode: state
            .config
            .watched_repos
            .iter()
            .find(|r| r.path == proposal.repo_path)
            .map(|r| r.mode.clone())
            .unwrap_or_else(|| "sweborg".into()),
        backend: String::new(),
        workspace_id: 0,
        project_id: 0,
        task_type: String::new(),
        requires_exhaustive_corpus_review: false,
        started_at: None,
        completed_at: None,
        duration_secs: None,
        review_status: None,
        revision_count: 0,
        chat_thread: String::new(),
    };
    let task_id = state.db.insert_task(&task).map_err(internal)?;
    Ok(Json(json!({ "task_id": task_id })))
}

pub(crate) async fn dismiss_proposal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    match state.db.get_proposal(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some(_) => {
            state
                .db
                .update_proposal_status(id, "dismissed")
                .map_err(internal)?;
            Ok(StatusCode::OK)
        },
    }
}

pub(crate) async fn reopen_proposal(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    match state.db.get_proposal(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some(_) => {
            state
                .db
                .update_proposal_status(id, "proposed")
                .map_err(internal)?;
            Ok(StatusCode::OK)
        },
    }
}

pub(crate) async fn triage_proposals(State(state): State<Arc<AppState>>) -> Json<Value> {
    if state
        .triage_running
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return Json(json!({ "scored": 0, "error": "triage already running" }));
    }

    let proposals = match state.db.list_untriaged_proposals() {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("list_untriaged_proposals: {e}");
            return Json(json!({ "scored": 0 }));
        },
    };
    let count = proposals.len();
    if count == 0 {
        return Json(json!({ "scored": 0 }));
    }

    let db = Arc::clone(&state.db);
    let Some(backend) = state.default_backend("claude") else {
        tracing::error!("triage_proposals: no backends configured");
        return Json(json!({ "scored": 0 }));
    };
    let model = db
        .get_config("model")
        .ok()
        .flatten()
        .unwrap_or_else(|| "claude-sonnet-4-6".into());
    let oauth = state.config.oauth_token.clone();

    let triage_flag = Arc::clone(&state.triage_running);
    tokio::spawn(async move {
        for proposal in proposals {
            let prompt = format!(
                "Score this software proposal as JSON.\n\nTitle: {}\nDescription: {}\nRationale: {}\n\nRespond ONLY with valid JSON:\n{{\"score\":0-100,\"impact\":0-100,\"feasibility\":0-100,\"risk\":0-100,\"effort\":0-100,\"reasoning\":\"...\"}}",
                proposal.title, proposal.description, proposal.rationale
            );

            let task = Task {
                id: proposal.id,
                title: format!("triage:{}", proposal.id),
                description: String::new(),
                repo_path: proposal.repo_path.clone(),
                branch: String::new(),
                status: "triage".into(),
                attempt: 0,
                max_attempts: 1,
                last_error: String::new(),
                created_by: "triage".into(),
                notify_chat: String::new(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                session_id: String::new(),
                mode: "sweborg".into(),
                backend: String::new(),
                workspace_id: 0,
                project_id: 0,
                task_type: String::new(),
                requires_exhaustive_corpus_review: false,
                started_at: None,
                completed_at: None,
                duration_secs: None,
                review_status: None,
                revision_count: 0,
                chat_thread: String::new(),
            };

            let phase = PhaseConfig {
                name: "triage".into(),
                label: "Triage".into(),
                instruction: prompt,
                fresh_session: true,
                allowed_tools: String::new(),
                ..Default::default()
            };

            let ctx = PhaseContext {
                task: task.clone(),
                repo_config: RepoConfig {
                    path: proposal.repo_path.clone(),
                    test_cmd: String::new(),
                    prompt_file: String::new(),
                    mode: "sweborg".into(),
                    is_self: false,
                    auto_merge: false,
                    lint_cmd: String::new(),
                    backend: String::new(),
                    repo_slug: String::new(),
                },
                data_dir: state.config.data_dir.clone(),
                session_dir: format!("{}/sessions/triage-{}", state.config.data_dir, proposal.id),
                work_dir: proposal.repo_path.clone(),
                oauth_token: oauth.clone(),
                model: model.clone(),
                pending_messages: Vec::new(),
                phase_attempt: task.attempt,
                phase_gate_token: format!(
                    "triage:{}:{}",
                    task.id,
                    chrono::Utc::now()
                        .timestamp_nanos_opt()
                        .unwrap_or_else(|| chrono::Utc::now().timestamp_micros() * 1_000)
                ),
                system_prompt_suffix: String::new(),
                user_coauthor: String::new(),
                stream_tx: None,
                setup_script: String::new(),
                api_keys: std::collections::HashMap::new(),
                disallowed_tools: String::new(),
                knowledge_files: Vec::new(),
                knowledge_dir: String::new(),
                knowledge_repo_paths: Vec::new(),
                agent_network: None,
                prior_research: Vec::new(),
                revision_count: 0,
                experimental_domains: state.config.experimental_domains,
                isolated: true,
                borg_api_url: format!("http://127.0.0.1:{}", state.config.web_port),
                borg_api_token: state.api_token.clone(),
                chat_context: Vec::new(),
                github_token: state.config.github_token.clone(),
            };

            tokio::fs::create_dir_all(&ctx.session_dir).await.ok();

            state.ai_request_count.fetch_add(1, Ordering::Relaxed);
            match backend.run_phase(&task, &phase, ctx).await {
                Ok(result) => {
                    if let Some(json_start) = result.output.find('{') {
                        if let Some(json_end) = result.output[json_start..].rfind('}') {
                            let json_str = &result.output[json_start..json_start + json_end + 1];
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                                let score = v["score"].as_i64().unwrap_or(0);
                                let impact = v["impact"].as_i64().unwrap_or(0);
                                let feasibility = v["feasibility"].as_i64().unwrap_or(0);
                                let risk = v["risk"].as_i64().unwrap_or(0);
                                let effort = v["effort"].as_i64().unwrap_or(0);
                                let reasoning = v["reasoning"].as_str().unwrap_or("").to_string();
                                if let Err(e) = db.update_proposal_triage(
                                    proposal.id,
                                    score,
                                    impact,
                                    feasibility,
                                    risk,
                                    effort,
                                    &reasoning,
                                ) {
                                    tracing::error!("update_proposal_triage #{}: {e}", proposal.id);
                                } else {
                                    tracing::info!(
                                        "triaged proposal #{}: score={score}",
                                        proposal.id
                                    );
                                }
                            }
                        }
                    }
                },
                Err(e) => tracing::error!("triage agent for proposal #{}: {e}", proposal.id),
            }
        }
        triage_flag.store(false, std::sync::atomic::Ordering::SeqCst);
    });

    Json(json!({ "scored": count }))
}

pub(crate) async fn get_settings(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
) -> Result<Json<Value>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let mut obj = serde_json::Map::new();
    for key in SETTINGS_KEYS {
        let val = state.db.get_config(key).map_err(internal)?;
        let default = SETTINGS_DEFAULTS
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| *v)
            .unwrap_or("");
        let s = val.as_deref().unwrap_or(default);
        let json_val = if matches!(
            *key,
            "continuous_mode" | "git_claude_coauthor" | "experimental_domains"
        ) {
            json!(s == "true")
        } else if matches!(
            *key,
            "release_interval_mins"
                | "pipeline_max_backlog"
                | "agent_timeout_s"
                | "pipeline_seed_cooldown_s"
                | "pipeline_tick_s"
                | "container_memory_mb"
                | "pipeline_max_agents"
                | "pipeline_agent_cooldown_s"
                | "proposal_promote_threshold"
                | "project_max_bytes"
                | "knowledge_max_bytes"
                | "cloud_import_max_batch_files"
                | "backup_poll_interval_s"
        ) {
            s.parse::<i64>().map(|n| json!(n)).unwrap_or(json!(s))
        } else {
            json!(s)
        };
        obj.insert(key.to_string(), json_val);
    }
    Ok(Json(Value::Object(obj)))
}

pub(crate) async fn put_settings(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let map = body.as_object().ok_or(StatusCode::BAD_REQUEST)?;
    let mut updated = 0usize;
    for (key, val) in map {
        if !SETTINGS_KEYS.contains(&key.as_str()) {
            continue;
        }
        let s = match val {
            Value::String(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            _ => continue,
        };
        state.db.set_config(key, &s).map_err(internal)?;
        updated += 1;
    }
    Ok(Json(json!({ "updated": updated })))
}

pub(crate) async fn list_users(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
) -> Result<Json<Value>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let users = state.db.list_users().map_err(internal)?;
    let arr: Vec<Value> = users
        .into_iter()
        .map(|(id, username, display_name, is_admin, created_at)| {
            json!({ "id": id, "username": username, "display_name": display_name, "is_admin": is_admin, "created_at": created_at })
        })
        .collect();
    Ok(Json(json!(arr)))
}

pub(crate) async fn create_user(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Json(body): Json<CreateUserBody>,
) -> Result<Json<Value>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    if body.username.trim().is_empty() || body.password.len() < 4 {
        return Ok(Json(
            json!({"error": "username required, password min 4 chars"}),
        ));
    }
    let hash = crate::auth::hash_password(&body.password).map_err(|e| {
        tracing::error!("hash_password: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let display = body.display_name.as_deref().unwrap_or(&body.username);
    let is_admin = body.is_admin.unwrap_or(false);
    let id = state
        .db
        .create_user(&body.username, display, &hash, is_admin)
        .map_err(internal)?;
    Ok(Json(
        json!({ "id": id, "username": body.username, "display_name": display, "is_admin": is_admin }),
    ))
}

pub(crate) async fn delete_user(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    if id == user.id {
        return Ok(Json(json!({"error": "cannot delete yourself"})));
    }
    state.db.delete_user(id).map_err(internal)?;
    Ok(Json(json!({ "deleted": id })))
}

pub(crate) async fn change_password(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Path(id): Path<i64>,
    Json(body): Json<ChangePasswordBody>,
) -> Result<Json<Value>, StatusCode> {
    if !user.is_admin && user.id != id {
        return Err(StatusCode::FORBIDDEN);
    }
    if body.password.len() < 4 {
        return Ok(Json(json!({"error": "password min 4 chars"})));
    }
    let hash = crate::auth::hash_password(&body.password).map_err(|e| {
        tracing::error!("hash_password: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    state.db.update_user_password(id, &hash).map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

fn workspace_role_can_manage(role: &str) -> bool {
    matches!(role, "owner" | "admin")
}

pub(crate) async fn list_workspaces(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    if user.id == 0 {
        return Ok(Json(json!({
            "workspaces": [{
                "workspace_id": workspace.id,
                "name": workspace.name,
                "slug": "",
                "kind": workspace.kind,
                "role": workspace.role,
                "is_default": workspace.is_default,
                "created_at": "",
            }],
            "default_workspace_id": workspace.id,
        })));
    }
    let workspaces = state.db.list_user_workspaces(user.id).map_err(internal)?;
    Ok(Json(json!({
        "workspaces": workspaces,
        "default_workspace_id": user.default_workspace_id,
    })))
}

pub(crate) async fn create_workspace(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Json(body): Json<CreateWorkspaceBody>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    if user.id == 0 {
        return Err(StatusCode::FORBIDDEN);
    }
    let name = body.name.trim();
    if name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let kind = body.kind.as_deref().unwrap_or("shared");
    if !matches!(kind, "shared" | "org") {
        return Err(StatusCode::BAD_REQUEST);
    }
    let workspace_id = state
        .db
        .create_workspace(name, kind, Some(user.id))
        .map_err(internal)?;
    state
        .db
        .add_workspace_member(workspace_id, user.id, "owner")
        .map_err(internal)?;
    if body.set_default.unwrap_or(false) {
        state
            .db
            .set_user_default_workspace_id(user.id, workspace_id)
            .map_err(internal)?;
    }
    let workspace = state
        .db
        .get_workspace(workspace_id)
        .map_err(internal)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "workspace": workspace,
            "default_workspace_id": if body.set_default.unwrap_or(false) { workspace_id } else { user.default_workspace_id },
        })),
    ))
}

pub(crate) async fn select_workspace(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    if user.id == 0 {
        return Err(StatusCode::FORBIDDEN);
    }
    let membership = state
        .db
        .get_user_workspace_membership(user.id, id)
        .map_err(internal)?
        .ok_or(StatusCode::FORBIDDEN)?;
    state
        .db
        .set_user_default_workspace_id(user.id, id)
        .map_err(internal)?;
    Ok(Json(json!({
        "ok": true,
        "workspace_id": membership.workspace_id,
    })))
}

pub(crate) async fn add_workspace_member(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Path(id): Path<i64>,
    Json(body): Json<AddWorkspaceMemberBody>,
) -> Result<Json<Value>, StatusCode> {
    let membership = state
        .db
        .get_user_workspace_membership(user.id, id)
        .map_err(internal)?
        .ok_or(StatusCode::FORBIDDEN)?;
    if !user.is_admin && !workspace_role_can_manage(&membership.role) {
        return Err(StatusCode::FORBIDDEN);
    }
    let role = body.role.as_deref().unwrap_or("member");
    if !matches!(role, "owner" | "admin" | "member" | "viewer") {
        return Err(StatusCode::BAD_REQUEST);
    }
    let target = state
        .db
        .get_user_by_username(body.username.trim())
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;
    state
        .db
        .add_workspace_member(id, target.0, role)
        .map_err(internal)?;
    Ok(Json(json!({
        "ok": true,
        "workspace_id": id,
        "user_id": target.0,
        "role": role,
    })))
}

const USER_SETTINGS_KEYS: &[&str] = &[
    "model",
    "backend",
    "github_token",
    "gitlab_token",
    "codeberg_token",
    "telegram_bot_token",
    "telegram_bot_username",
    "contact_email",
    "discord_bot_token",
    "discord_bot_username",
    "dashboard_mode",
];
const USER_SETTINGS_PROTECTED: &[&str] = &[
    "telegram_bot_token",
    "telegram_bot_username",
    "discord_bot_token",
    "discord_bot_username",
];

pub(crate) async fn get_user_settings(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
) -> Result<Json<Value>, StatusCode> {
    let settings = state.db.get_all_user_settings(user.id).map_err(internal)?;

    let model_override = state.db.get_config("model_override").map_err(internal)?;
    let has_override = model_override.as_ref().map_or(false, |v| !v.is_empty());

    let mut obj = serde_json::Map::new();
    for key in USER_SETTINGS_KEYS {
        let val = settings.get(*key).cloned().unwrap_or_default();
        match *key {
            "github_token" => {
                obj.insert("github_token_set".to_string(), json!(!val.is_empty()));
            },
            "gitlab_token" => {
                obj.insert("gitlab_token_set".to_string(), json!(!val.is_empty()));
            },
            "codeberg_token" => {
                obj.insert("codeberg_token_set".to_string(), json!(!val.is_empty()));
            },
            "telegram_bot_token" => {
                // Exposed via dedicated telegram-bot endpoints, not here
            },
            _ => {
                obj.insert(key.to_string(), json!(val));
            },
        }
    }
    obj.insert(
        "model_override".to_string(),
        json!(model_override.unwrap_or_default()),
    );
    obj.insert("model_override_active".to_string(), json!(has_override));

    let tg_username = settings.get("telegram_bot_username").cloned().unwrap_or_default();
    let tg_connected = !settings
        .get("telegram_bot_token")
        .map(|t| t.is_empty())
        .unwrap_or(true);
    obj.insert("telegram_bot_connected".to_string(), json!(tg_connected));
    obj.insert("telegram_bot_username".to_string(), json!(tg_username));

    let dc_username = settings.get("discord_bot_username").cloned().unwrap_or_default();
    let dc_connected = !settings
        .get("discord_bot_token")
        .map(|t| t.is_empty())
        .unwrap_or(true);
    obj.insert("discord_bot_connected".to_string(), json!(dc_connected));
    obj.insert("discord_bot_username".to_string(), json!(dc_username));

    Ok(Json(Value::Object(obj)))
}

pub(crate) async fn put_user_settings(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let map = body.as_object().ok_or(StatusCode::BAD_REQUEST)?;
    let mut updated = 0usize;
    for (key, val) in map {
        if !USER_SETTINGS_KEYS.contains(&key.as_str())
            || USER_SETTINGS_PROTECTED.contains(&key.as_str())
        {
            continue;
        }
        let s = match val {
            Value::String(s) => s.clone(),
            _ => continue,
        };
        if s.is_empty() {
            state
                .db
                .delete_user_setting(user.id, key)
                .map_err(internal)?;
        } else {
            state
                .db
                .set_user_setting(user.id, key, &s)
                .map_err(internal)?;
        }
        updated += 1;
    }
    Ok(Json(json!({ "updated": updated })))
}

pub(crate) async fn connect_telegram_bot(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let token = body["token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let client = reqwest::Client::new();
    let resp: Value = client
        .get(format!("https://api.telegram.org/bot{token}/getMe"))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        .json()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    let username = resp["result"]["username"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?
        .to_string();

    state
        .db
        .set_user_setting(user.id, "telegram_bot_token", token)
        .map_err(internal)?;
    state
        .db
        .set_user_setting(user.id, "telegram_bot_username", &username)
        .map_err(internal)?;

    tracing::info!(
        user_id = user.id,
        bot = %username,
        "user connected telegram bot"
    );

    Ok(Json(json!({
        "ok": true,
        "bot_username": username,
    })))
}

pub(crate) async fn disconnect_telegram_bot(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
) -> Result<Json<Value>, StatusCode> {
    state
        .db
        .delete_user_setting(user.id, "telegram_bot_token")
        .map_err(internal)?;
    state
        .db
        .delete_user_setting(user.id, "telegram_bot_username")
        .map_err(internal)?;

    tracing::info!(user_id = user.id, "user disconnected telegram bot");

    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn connect_discord_bot(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let token = body["token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let client = reqwest::Client::new();
    let resp: Value = client
        .get("https://discord.com/api/v10/users/@me")
        .header("Authorization", format!("Bot {token}"))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        .json()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;

    let username = resp["username"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?
        .to_string();

    state
        .db
        .set_user_setting(user.id, "discord_bot_token", token)
        .map_err(internal)?;
    state
        .db
        .set_user_setting(user.id, "discord_bot_username", &username)
        .map_err(internal)?;

    tracing::info!(
        user_id = user.id,
        bot = %username,
        "user connected discord bot"
    );

    Ok(Json(json!({
        "ok": true,
        "bot_username": username,
    })))
}

pub(crate) async fn disconnect_discord_bot(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
) -> Result<Json<Value>, StatusCode> {
    state
        .db
        .delete_user_setting(user.id, "discord_bot_token")
        .map_err(internal)?;
    state
        .db
        .delete_user_setting(user.id, "discord_bot_username")
        .map_err(internal)?;

    tracing::info!(user_id = user.id, "user disconnected discord bot");

    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn sse_logs(
    State(state): State<Arc<AppState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let live_rx = state.log_tx.subscribe();
    let history: Vec<String> = state
        .log_ring
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect();
    tokio::spawn(async move {
        for line in history {
            if tx.send(line).is_err() {
                return;
            }
        }
        let mut live_rx = live_rx;
        loop {
            match live_rx.recv().await {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        return;
                    }
                },
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => {
                    tracing::debug!("log SSE broadcast closed: {e}");
                    break;
                },
            }
        }
    });
    let stream = UnboundedReceiverStream::new(rx)
        .map(|data| Ok::<_, std::convert::Infallible>(Event::default().data(data)));
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    )
}

pub(crate) async fn sse_task_stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let (history, live_rx) = state.stream_manager.subscribe(id).await;

        let history = if history.is_empty() && live_rx.is_none() {
            let mut lines = Vec::new();
            if let Ok(outputs) = state.db.get_task_outputs(id) {
                for output in outputs {
                    for line in output.raw_stream.lines() {
                        if !line.is_empty() {
                            lines.push(line.to_string());
                        }
                    }
                }
            }
            if !lines.is_empty() {
                lines.push(r#"{"type":"stream_end"}"#.to_string());
            }
            lines
        } else {
            history
        };

        for line in history {
            if tx.send(line).is_err() {
                return;
            }
        }

        if let Some(mut live_rx) = live_rx {
            loop {
                match live_rx.recv().await {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            return;
                        }
                    },
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(e) => {
                        tracing::debug!("task SSE broadcast closed: {e}");
                        break;
                    },
                }
            }
        }
    });
    let stream = UnboundedReceiverStream::new(rx)
        .map(|data| Ok::<_, std::convert::Infallible>(Event::default().data(data)));
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    )
}

pub(crate) async fn post_release(State(state): State<Arc<AppState>>) -> Json<Value> {
    state
        .force_restart
        .store(true, std::sync::atomic::Ordering::Relaxed);
    tracing::info!("Force restart requested via /api/release");
    Json(json!({ "ok": true }))
}

pub(crate) async fn get_events(
    State(state): State<Arc<AppState>>,
    Query(q): Query<EventsQuery>,
) -> Result<Json<Value>, StatusCode> {
    let events: Vec<borg_core::db::LegacyEvent> = state
        .db
        .get_events_filtered(
            q.category.as_deref(),
            q.level.as_deref(),
            q.since,
            q.limit.unwrap_or(100),
        )
        .map_err(internal)?;
    Ok(Json(json!(events)))
}

pub(crate) async fn put_task_backend(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<Value>, StatusCode> {
    let backend = body["backend"].as_str().unwrap_or("").to_string();
    state
        .db
        .update_task_backend(id, &backend)
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn list_repos_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let repos = state.db.list_repos().map_err(internal)?;
    let arr: Vec<_> = repos
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "path": r.path,
                "name": r.name,
                "mode": r.mode,
                "backend": r.backend,
                "test_cmd": r.test_cmd,
                "auto_merge": r.auto_merge,
                "repo_slug": r.repo_slug,
            })
        })
        .collect();
    Ok(Json(json!(arr)))
}

pub(crate) async fn put_repo_backend(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<Value>, StatusCode> {
    let backend = body["backend"].as_str().unwrap_or("").to_string();
    state
        .db
        .update_repo_backend(id, &backend)
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn list_api_keys(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    let keys = state
        .db
        .list_workspace_api_keys(workspace.id)
        .map_err(internal)?;
    Ok(Json(json!({ "keys": keys })))
}

pub(crate) async fn store_api_key(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Json(body): Json<StoreKeyBody>,
) -> Result<Json<Value>, StatusCode> {
    let key_name = body.key_name.as_deref().unwrap_or("");
    let id = state
        .db
        .store_workspace_api_key(workspace.id, &body.provider, key_name, &body.key_value)
        .map_err(internal)?;
    Ok(Json(json!({ "id": id })))
}

pub(crate) async fn delete_api_key(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    state
        .db
        .delete_workspace_api_key(workspace.id, id)
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn list_cache_volumes(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let volumes = borg_core::sandbox::Sandbox::list_cache_volumes("borg-cache-").await;
    let arr: Vec<_> = volumes
        .into_iter()
        .map(
            |(name, size, last_used)| json!({ "name": name, "size": size, "last_used": last_used }),
        )
        .collect();
    Ok(Json(json!({ "volumes": arr })))
}

pub(crate) async fn delete_cache_volume(
    State(_state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    if !name.starts_with("borg-cache-")
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let removed = borg_core::sandbox::Sandbox::remove_volume(&name).await;
    if removed {
        Ok(Json(json!({ "ok": true })))
    } else {
        Err(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

async fn container_id_from_stream(state: &AppState, task_id: i64) -> Option<String> {
    let (history, _) = state.stream_manager.subscribe(task_id).await;
    for line in history.iter().rev() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if v.get("type").and_then(|t| t.as_str()) == Some("container_event")
                && v.get("event").and_then(|e| e.as_str()) == Some("container_id")
            {
                if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

pub(crate) async fn get_task_container(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    let container_id = container_id_from_stream(&state, task_id).await;
    match container_id {
        Some(id) => {
            let status = tokio::process::Command::new("docker")
                .args(["inspect", "--format", "{{.State.Status}}", &id])
                .output()
                .await
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            Ok(Json(
                json!({ "task_id": task_id, "container_id": id, "status": status }),
            ))
        },
        None => Err(StatusCode::NOT_FOUND),
    }
}

pub(crate) async fn admin_conversation_dump(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ConversationDumpQuery>,
) -> Result<Json<Value>, StatusCode> {
    let msgs = state
        .db
        .get_chat_messages(&query.thread, query.limit)
        .map_err(internal)?;

    let result: Vec<Value> = msgs
        .iter()
        .map(|m| {
            let mut obj = json!({
                "role": if m.is_from_me { "assistant" } else { "user" },
                "sender": m.sender_name,
                "content": m.content,
                "ts": m.timestamp,
            });

            if let Some(ref rs) = m.raw_stream {
                let mut events: Vec<Value> = Vec::new();
                for line in rs.split('\n') {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(parsed) = serde_json::from_str::<Value>(line) {
                        let event_type = parsed.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match event_type {
                            "assistant" => {
                                if let Some(msg) = parsed.get("message") {
                                    if let Some(content) = msg.get("content") {
                                        if let Some(blocks) = content.as_array() {
                                            for block in blocks {
                                                let btype = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                                match btype {
                                                    "text" => {
                                                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                                            events.push(json!({"type": "text", "content": text}));
                                                        }
                                                    }
                                                    "tool_use" => {
                                                        events.push(json!({
                                                            "type": "tool_call",
                                                            "tool": block.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                                                            "input": block.get("input"),
                                                        }));
                                                    }
                                                    "thinking" => {
                                                        if let Some(text) = block.get("thinking").and_then(|t| t.as_str()) {
                                                            let preview = if text.len() > 200 { &text[..200] } else { text };
                                                            events.push(json!({"type": "thinking", "content": preview}));
                                                        }
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            "tool_result" | "tool" => {
                                let content = parsed.get("content")
                                    .or_else(|| parsed.get("output"))
                                    .or_else(|| parsed.get("result"));
                                let text = match content {
                                    Some(Value::String(s)) => {
                                        if s.len() > 500 { format!("{}...", &s[..500]) } else { s.clone() }
                                    }
                                    Some(Value::Array(arr)) => {
                                        arr.iter()
                                            .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                                            .collect::<Vec<_>>()
                                            .join("\n")
                                    }
                                    Some(v) => {
                                        let s = v.to_string();
                                        if s.len() > 500 { format!("{}...", &s[..500]) } else { s }
                                    }
                                    None => String::new(),
                                };
                                events.push(json!({
                                    "type": "tool_result",
                                    "tool": parsed.get("tool_name").or_else(|| parsed.get("name"))
                                        .and_then(|n| n.as_str()).unwrap_or(""),
                                    "output": text,
                                }));
                            }
                            "result" => {
                                if let Some(r) = parsed.get("result").and_then(|r| r.as_str()) {
                                    events.push(json!({"type": "result", "content": r}));
                                }
                            }
                            "system" => {
                                if let Some(sub) = parsed.get("subtype").and_then(|s| s.as_str()) {
                                    if sub == "init" {
                                        let model = parsed.get("model").and_then(|m| m.as_str()).unwrap_or("?");
                                        let mcp = parsed.get("mcp_servers");
                                        events.push(json!({"type": "system_init", "model": model, "mcp_servers": mcp}));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                obj["events"] = json!(events);
                obj["raw_stream_lines"] = json!(rs.split('\n').filter(|l| !l.trim().is_empty()).count());
            }

            obj
        })
        .collect();

    Ok(Json(json!({
        "thread": query.thread,
        "message_count": result.len(),
        "messages": result,
    })))
}

pub(crate) async fn email_inbound(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    body: Bytes,
) -> impl IntoResponse {
    let provided = params
        .get("api_token")
        .cloned()
        .or_else(|| {
            headers
                .get("x-api-token")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();
    if provided != state.api_token {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let ct = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let email = match borg_core::email::parse_auto(&body, &ct) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("email_inbound: parse failed: {e}");
            return (StatusCode::BAD_REQUEST, "Bad email format").into_response();
        },
    };

    if email.from.is_empty() {
        return (StatusCode::OK, "OK").into_response();
    }

    let user = state.db.get_user_by_email(&email.from).ok().flatten();
    let (sender_name, _user_id) = match user {
        Some((id, _, display_name, _)) => {
            let name = if display_name.is_empty() { email.from_name.clone() } else { display_name };
            (name, Some(id))
        },
        None => {
            (
                if email.from_name.is_empty() { email.from.clone() } else { email.from_name.clone() },
                None,
            )
        },
    };

    let att_dir = format!(
        "{}/attachments/email-{}",
        state.config.data_dir,
        chrono::Utc::now().timestamp_millis()
    );
    let att_paths =
        borg_core::email::save_attachments(&email.attachments, std::path::Path::new(&att_dir))
            .unwrap_or_default();

    let mut agent_messages: Vec<String> = vec![format!(
        "Email from {} <{}>: {}\n\n{}",
        sender_name, email.from, email.subject, email.body
    )];
    for path in &att_paths {
        let size_kb = std::fs::metadata(path).map(|m| m.len() / 1024).unwrap_or(0);
        let filename = path.file_name().unwrap_or_default().to_string_lossy();
        agent_messages.push(format!(
            "[Attached file: {} ({}KB)] Path: {}",
            filename,
            size_kb,
            path.display()
        ));
    }

    let chat_key = format!("email:{}", email.from);
    let sessions = Arc::clone(&state.web_sessions);
    let config = Arc::clone(&state.config);
    let db = Arc::clone(&state.db);
    let search = state.search.clone();
    let storage = Arc::clone(&state.file_storage);
    let chat_tx = state.chat_event_tx.clone();
    let ai_count = Arc::clone(&state.ai_request_count);
    let from_email = email.from.clone();
    let reply_subject = format!("Re: {}", email.subject);

    tokio::spawn(async move {
        let run_id = crate::messaging_progress::new_chat_run_id();
        match super::chat::run_chat_agent(
            &chat_key,
            &run_id,
            &sender_name,
            &agent_messages,
            &sessions,
            &config,
            &db,
            search,
            &storage,
            &chat_tx,
            &ai_count,
        )
        .await
        {
            Ok(reply) if !reply.is_empty() => {
                let _ = borg_core::email::send_smtp_reply(
                    &config.smtp_host,
                    config.smtp_port,
                    &config.smtp_from,
                    &config.smtp_user,
                    &config.smtp_pass,
                    &from_email,
                    &reply_subject,
                    &reply,
                )
                .await;
            },
            Ok(_) => {},
            Err(e) => tracing::warn!("email inbound agent error: {e}"),
        }
    });

    (StatusCode::OK, "OK").into_response()
}

pub(crate) async fn rebuild_and_exec(repo_path: &str, build_cmd: &str) -> bool {
    let build_dir = format!("{repo_path}/borg-rs");
    let parts: Vec<&str> = build_cmd.split_whitespace().collect();
    let (cmd, args) = match parts.split_first() {
        Some((c, a)) => (*c, a),
        None => {
            tracing::error!("empty build_cmd");
            return false;
        },
    };
    let build = tokio::process::Command::new(cmd)
        .args(args)
        .current_dir(&build_dir)
        .status()
        .await;
    match build {
        Ok(s) if s.success() => {
            tracing::info!("Build done, restarting");
            let bin = match std::env::current_exe() {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("failed to resolve current_exe: {e}");
                    return false;
                },
            };
            use std::os::unix::process::CommandExt;
            let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
            let err = std::process::Command::new(&bin).args(&args[1..]).exec();
            tracing::error!("execve failed: {err}");
            false
        },
        Ok(_) => {
            tracing::error!("Release build failed");
            false
        },
        Err(e) => {
            tracing::error!("Failed to run cargo: {e}");
            false
        },
    }
}
