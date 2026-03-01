mod logging;
mod routes;

use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Instant,
};

use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post, put},
    Router,
};
use borg_agent::{claude::ClaudeBackend, codex::CodexBackend, ollama::OllamaBackend};
use borg_core::{
    chat::ChatCollector,
    config::Config,
    db::Db,
    observer::Observer,
    pipeline::{Pipeline, PipelineEvent},
    sandbox::Sandbox,
    sidecar::{Sidecar, SidecarEvent, Source},
    stream::TaskStreamManager,
    types::Task,
};
use chrono::Utc;
use serde_json::json;
use tokio::sync::{broadcast, Mutex as TokioMutex};
use tower_http::{cors::CorsLayer, services::ServeDir};
use tracing::info;

// ── AppState ──────────────────────────────────────────────────────────────

pub struct AppState {
    pub db: Arc<Db>,
    pub config: Arc<Config>,
    pub start_time: Instant,
    pub log_tx: broadcast::Sender<String>,
    pub log_ring: Arc<std::sync::Mutex<VecDeque<String>>>,
    pub pipeline_event_tx: broadcast::Sender<PipelineEvent>,
    pub stream_manager: Arc<TaskStreamManager>,
    pub chat_event_tx: broadcast::Sender<String>,
    pub web_sessions: Arc<TokioMutex<HashMap<String, String>>>,
    pub backends: std::collections::HashMap<String, Arc<dyn borg_core::agent::AgentBackend>>,
    pub force_restart: Arc<std::sync::atomic::AtomicBool>,
}

impl AppState {
    pub fn default_backend(&self, name: &str) -> Arc<dyn borg_core::agent::AgentBackend> {
        self.backends
            .get(name)
            .or_else(|| self.backends.values().next())
            .map(Arc::clone)
            .expect("no backends configured")
    }
}

// ── main ──────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let (log_tx, _log_rx) = broadcast::channel::<String>(1024);
    let log_ring: Arc<std::sync::Mutex<VecDeque<String>>> =
        Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(500)));
    let (chat_event_tx, _) = broadcast::channel::<String>(256);

    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        "borg_server=info,borg_core=info,borg_agent=info,tower_http=warn".into()
    });

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(logging::BroadcastLayer {
            tx: log_tx.clone(),
            ring: Arc::clone(&log_ring),
        })
        .init();

    let env_config = Config::from_env()?;

    borg_core::modes::register_modes(borg_domains::all_modes());

    std::fs::create_dir_all(&env_config.data_dir)?;
    let db_path = format!("{}/borg.db", env_config.data_dir);
    let mut db = Db::open(&db_path)?;
    db.migrate()?;

    // Seed DB from env on first run, then load DB values (DB wins over env)
    env_config.seed_db(&db)?;
    let config = env_config.load_from_db(&db);

    // Abandon any runs left in 'running' state from previous crash
    if let Err(e) = db.abandon_running_agents() {
        tracing::error!("abandon_running_agents failed: {e}");
    }

    // Upsert repos from config into DB
    for repo in &config.watched_repos {
        let name = std::path::Path::new(&repo.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&repo.path);
        if let Err(e) = db.upsert_repo(
            &repo.path,
            name,
            &repo.mode,
            &repo.test_cmd,
            &repo.prompt_file,
            repo.auto_merge,
            None,
        ) {
            tracing::error!("upsert_repo {}: {e}", repo.path);
        }
    }

    // Restart recovery: pipeline tasks are resumed from persisted task.status + session_id.
    // This makes recovery explicit and observable in logs after service restarts.
    if let Ok(active) = db.list_active_tasks() {
        let resumable = active
            .iter()
            .filter(|t| !t.session_id.is_empty())
            .filter(|t| {
                matches!(
                    t.status.as_str(),
                    "spec" | "qa" | "qa_fix" | "impl" | "retry" | "lint_fix" | "rebase"
                )
            })
            .count();
        if resumable > 0 {
            tracing::info!(
                "restart recovery: {resumable} active pipeline tasks have resumable sessions"
            );
        }
    }

    let db = Arc::new(db);
    let config = Arc::new(config);

    // Detect sandbox backend (bwrap preferred, docker fallback, configurable via SANDBOX_BACKEND)
    let sandbox_mode = Sandbox::detect(&config.sandbox_backend).await;

    // Build backends map
    let mut backends: std::collections::HashMap<String, Arc<dyn borg_core::agent::AgentBackend>> =
        std::collections::HashMap::new();
    backends.insert(
        "claude".into(),
        Arc::new(
            ClaudeBackend::new("claude", sandbox_mode.clone(), &config.container_image)
                .with_timeout(config.agent_timeout_s as u64),
        ),
    );
    if !config.codex_api_key.is_empty()
        || borg_core::config::codex_has_credentials(&config.codex_credentials_path)
    {
        let codex_model = db
            .get_config("codex_model")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "gpt-5.3-codex".to_string());
        let codex_reasoning_effort = db
            .get_config("codex_reasoning_effort")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "medium".to_string());
        backends.insert(
            "codex".into(),
            Arc::new(
                CodexBackend::new(config.codex_api_key.clone(), codex_model)
                    .with_reasoning_effort(codex_reasoning_effort),
            ),
        );
    }
    // Local model via Ollama — enabled by setting OLLAMA_URL or LOCAL_MODEL
    if std::env::var("OLLAMA_URL").is_ok() || std::env::var("LOCAL_MODEL").is_ok() {
        let url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into());
        let model = std::env::var("LOCAL_MODEL").unwrap_or_else(|_| "llama3.2".into());
        backends.insert(
            "local".into(),
            Arc::new(OllamaBackend::new(url, model).with_timeout(300)),
        );
        info!("local backend registered (Ollama)");
    }

    let force_restart = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let (pipeline, pipeline_rx) = Pipeline::new(
        Arc::clone(&db),
        backends.clone(),
        Arc::clone(&config),
        Arc::clone(&force_restart),
    );
    let pipeline_event_tx = pipeline.event_tx.clone();
    let pipeline = Arc::new(pipeline);

    // Pipeline tick loop — inner spawn catches panics so the loop never dies
    let tick_secs = config.pipeline_tick_s;
    {
        let pipeline = Arc::clone(&pipeline);
        tokio::spawn(async move {
            loop {
                let p = Arc::clone(&pipeline);
                let handle = tokio::spawn(async move { p.tick().await });
                match handle.await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => tracing::error!("Pipeline tick error: {e}"),
                    Err(join_err) => tracing::error!("Pipeline tick panicked: {join_err}"),
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(tick_secs)).await;
            }
        });
    }

    // Telegram polling loop
    if !config.telegram_token.is_empty() {
        let token = config.telegram_token.clone();
        let db_tg = Arc::clone(&db);
        let repos = config.watched_repos.clone();
        let config_tg = Arc::clone(&config);
        let tg_chat_event_tx = chat_event_tx.clone();
        let tg_sessions: Arc<TokioMutex<HashMap<String, String>>> =
            Arc::new(TokioMutex::new(HashMap::new()));
        tokio::spawn(async move {
            let mut tg = borg_core::telegram::Telegram::new(token);
            if let Err(e) = tg.connect().await {
                tracing::warn!("Telegram connect failed: {e}");
                return;
            }
            let tg = Arc::new(tg);
            loop {
                match tg.get_updates().await {
                    Ok(messages) => {
                        for msg in messages {
                            if !msg.mentions_bot && msg.chat_type != "private" {
                                continue;
                            }
                            // Strip leading @mention
                            let text = msg.text.trim().to_string();
                            let text = if text.starts_with('@') {
                                text.splitn(2, ' ').nth(1).unwrap_or("").trim().to_string()
                            } else {
                                text
                            };
                            let text_lower = text.to_lowercase();

                            if text_lower.starts_with("task:") || text_lower.starts_with("task ") {
                                let title_part = text[5..].trim().to_string();
                                let (title, desc) = if let Some(nl) = title_part.find('\n') {
                                    (
                                        title_part[..nl].to_string(),
                                        title_part[nl + 1..].to_string(),
                                    )
                                } else {
                                    (title_part.clone(), title_part.clone())
                                };
                                let repo_path = repos
                                    .iter()
                                    .find(|r| r.is_self)
                                    .or_else(|| repos.first())
                                    .map(|r| r.path.clone())
                                    .unwrap_or_default();
                                let mode = repos
                                    .iter()
                                    .find(|r| r.path == repo_path)
                                    .map(|r| r.mode.clone())
                                    .unwrap_or_else(|| "sweborg".to_string());
                                let task = Task {
                                    id: 0,
                                    title,
                                    description: desc,
                                    repo_path,
                                    branch: String::new(),
                                    status: "backlog".to_string(),
                                    attempt: 0,
                                    max_attempts: 5,
                                    last_error: String::new(),
                                    created_by: format!("telegram:{}", msg.sender_id),
                                    notify_chat: msg.chat_id.to_string(),
                                    created_at: Utc::now(),
                                    session_id: String::new(),
                                    mode,
                                    backend: String::new(),
                                };
                                let task_title = task.title.clone();
                                let tg2 = Arc::clone(&tg);
                                match db_tg.insert_task(&task) {
                                    Ok(id) => {
                                        let reply = format!("Task #{id} created: {task_title}");
                                        let _ = tg2
                                            .send_message(msg.chat_id, &reply, Some(msg.message_id))
                                            .await;
                                    },
                                    Err(e) => tracing::error!("insert_task from telegram: {e}"),
                                }
                            } else {
                                // Run chat agent in a separate task so the poll loop isn't blocked
                                let chat_key = format!("telegram:{}", msg.chat_id);
                                let _ = tg.send_typing(msg.chat_id).await;
                                let tg2 = Arc::clone(&tg);
                                let sessions2 = Arc::clone(&tg_sessions);
                                let config2 = Arc::clone(&config_tg);
                                let db2 = Arc::clone(&db_tg);
                                let chat_tx2 = tg_chat_event_tx.clone();
                                let sender_name = msg.sender_name.clone();
                                let chat_id = msg.chat_id;
                                let message_id = msg.message_id;
                                tokio::spawn(async move {
                                    match routes::run_chat_agent(
                                        &chat_key,
                                        &sender_name,
                                        &[text],
                                        &sessions2,
                                        &config2,
                                        &db2,
                                        &chat_tx2,
                                    )
                                    .await
                                    {
                                        Ok(reply) if !reply.is_empty() => {
                                            let _ = tg2
                                                .send_message(chat_id, &reply, Some(message_id))
                                                .await;
                                        },
                                        Ok(_) => {},
                                        Err(e) => tracing::warn!("Telegram chat agent error: {e}"),
                                    }
                                });
                            }
                        }
                    },
                    Err(e) => tracing::warn!("Telegram poll error: {e}"),
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        });
    }

    // Self-update detection loop
    if let Some(self_repo) = config.watched_repos.iter().find(|r| r.is_self).cloned() {
        let check_interval = config.remote_check_interval_s as u64;
        let force_restart_check = Arc::clone(&force_restart);
        tokio::spawn(async move {
            let git = borg_core::git::Git::new(&self_repo.path);
            let mut last_head = git.rev_parse_head().unwrap_or_default();

            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(check_interval)).await;

                if force_restart_check.load(std::sync::atomic::Ordering::Relaxed) {
                    tracing::info!("Force restart requested via /api/release, rebuilding...");
                    if routes::rebuild_and_exec(&self_repo.path).await {
                        force_restart_check.store(false, std::sync::atomic::Ordering::Relaxed);
                    } else {
                        tracing::warn!("Force restart rebuild failed; will retry");
                    }
                    continue;
                }

                if let Err(e) = git.fetch_origin() {
                    tracing::warn!("self-update fetch failed: {e}");
                    continue;
                }

                let remote_head = match git.exec(&self_repo.path, &["rev-parse", "origin/main"]) {
                    Ok(r) => r.stdout.trim().to_string(),
                    Err(_) => continue,
                };

                if remote_head.is_empty() || remote_head == last_head {
                    continue;
                }

                tracing::info!(
                    "Self-update: new commit on origin/main: {}",
                    &remote_head[..8.min(remote_head.len())]
                );
                tracing::info!("Self-update: rebuilding...");
                if routes::rebuild_and_exec(&self_repo.path).await {
                    // Only advance last_head after a successful execve replaces us.
                    // If build failed, we retry this same remote_head next loop.
                    last_head = remote_head;
                } else {
                    tracing::warn!(
                        "Self-update rebuild failed; keeping previous last_head for retry"
                    );
                }
            }
        });
    }

    // Sidecar (Discord + WhatsApp bridge)
    if !config.discord_token.is_empty() || !config.wa_auth_dir.is_empty() {
        let config_sc = Arc::clone(&config);
        let db_sc = Arc::clone(&db);
        let sc_chat_event_tx = chat_event_tx.clone();
        match Sidecar::spawn(
            &config.assistant_name,
            &config.discord_token,
            &config.wa_auth_dir,
            config.wa_disabled,
        )
        .await
        {
            Err(e) => tracing::warn!("Sidecar spawn failed: {e}"),
            Ok((sidecar, mut event_rx)) => {
                let sidecar = Arc::new(sidecar);
                let sc_sessions: Arc<TokioMutex<HashMap<String, String>>> =
                    Arc::new(TokioMutex::new(HashMap::new()));
                let collector = Arc::new(ChatCollector::new(
                    config.chat_collection_window_ms as u64,
                    config.max_chat_agents,
                    config.chat_cooldown_ms as u64,
                ));

                // Flush expired collection windows periodically
                let collector_flush = Arc::clone(&collector);
                let sidecar_flush = Arc::clone(&sidecar);
                let sessions_flush = Arc::clone(&sc_sessions);
                let config_flush = Arc::clone(&config_sc);
                let db_flush = Arc::clone(&db_sc);
                let chat_tx_flush = sc_chat_event_tx.clone();
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                        for batch in collector_flush.flush_expired().await {
                            let sidecar2 = Arc::clone(&sidecar_flush);
                            let sessions2 = Arc::clone(&sessions_flush);
                            let config2 = Arc::clone(&config_flush);
                            let db2 = Arc::clone(&db_flush);
                            let chat_tx2 = chat_tx_flush.clone();
                            let collector2 = Arc::clone(&collector_flush);
                            let is_discord = batch.chat_key.starts_with("discord:");
                            let chat_id = batch
                                .chat_key
                                .splitn(2, ':')
                                .nth(1)
                                .unwrap_or("")
                                .to_string();
                            let sender_name = batch.messages.first().cloned().unwrap_or_default();
                            tokio::spawn(async move {
                                match routes::run_chat_agent(
                                    &batch.chat_key,
                                    &sender_name,
                                    &batch.messages,
                                    &sessions2,
                                    &config2,
                                    &db2,
                                    &chat_tx2,
                                )
                                .await
                                {
                                    Ok(reply) if !reply.is_empty() => {
                                        if is_discord {
                                            sidecar2.send_discord(&chat_id, &reply, None);
                                        } else {
                                            sidecar2.send_whatsapp(&chat_id, &reply, None);
                                        }
                                    },
                                    Ok(_) => {},
                                    Err(e) => tracing::warn!("Sidecar flush agent error: {e}"),
                                }
                                collector2.mark_done(&batch.chat_key).await;
                            });
                        }
                    }
                });

                // Process incoming sidecar events
                let db_events = Arc::clone(&db_sc);
                let chat_tx_events = sc_chat_event_tx.clone();
                tokio::spawn(async move {
                    loop {
                        let Some(event) = event_rx.recv().await else {
                            break;
                        };
                        let SidecarEvent::Message(msg) = event else {
                            continue;
                        };
                        if msg.is_group && !msg.mentions_bot {
                            continue;
                        }
                        let prefix = if msg.source == Source::Discord {
                            "discord"
                        } else {
                            "whatsapp"
                        };
                        let chat_key = format!("{}:{}", prefix, msg.chat_id);
                        let incoming = borg_core::chat::IncomingMessage {
                            chat_key: chat_key.clone(),
                            sender_name: msg.sender_name.clone(),
                            text: msg.text.clone(),
                            timestamp: msg.timestamp,
                            reply_to_message_id: None,
                        };
                        if let Some(batch) = collector.process(incoming).await {
                            let sidecar2 = Arc::clone(&sidecar);
                            let sessions2 = Arc::clone(&sc_sessions);
                            let config2 = Arc::clone(&config_sc);
                            let db2 = Arc::clone(&db_events);
                            let chat_tx2 = chat_tx_events.clone();
                            let collector2 = Arc::clone(&collector);
                            let is_discord = msg.source == Source::Discord;
                            let chat_id = msg.chat_id.clone();
                            let msg_id = msg.id.clone();
                            let sender_name = msg.sender_name.clone();
                            if is_discord {
                                sidecar.send_discord_typing(&chat_id);
                            } else {
                                sidecar.send_whatsapp_typing(&chat_id);
                            }
                            tokio::spawn(async move {
                                match routes::run_chat_agent(
                                    &batch.chat_key,
                                    &sender_name,
                                    &batch.messages,
                                    &sessions2,
                                    &config2,
                                    &db2,
                                    &chat_tx2,
                                )
                                .await
                                {
                                    Ok(reply) if !reply.is_empty() => {
                                        if is_discord {
                                            sidecar2.send_discord(&chat_id, &reply, Some(&msg_id));
                                        } else {
                                            sidecar2.send_whatsapp(&chat_id, &reply, Some(&msg_id));
                                        }
                                    },
                                    Ok(_) => {},
                                    Err(e) => tracing::warn!("Sidecar chat agent error: {e}"),
                                }
                                collector2.mark_done(&batch.chat_key).await;
                            });
                        }
                    }
                });
            },
        }
    }

    // Observer (log monitoring)
    if !config.observer_config.is_empty() {
        let observer = Observer::load(
            &config.observer_config,
            &config.oauth_token,
            &config.telegram_token,
        );
        tokio::spawn(async move { observer.run().await });
    }

    // Forward pipeline events to SSE log stream; route Notify events to Telegram
    {
        let log_tx_fwd = log_tx.clone();
        let tg_token_notify = config.telegram_token.clone();
        tokio::spawn(async move {
            let mut rx = pipeline_rx;
            loop {
                match rx.recv().await {
                    Ok(evt) => {
                        if let PipelineEvent::Notify {
                            ref chat_id,
                            ref message,
                        } = evt
                        {
                            if !tg_token_notify.is_empty() {
                                let raw_id =
                                    chat_id.strip_prefix("tg:").unwrap_or(chat_id.as_str());
                                if let Ok(chat_id_i64) = raw_id.parse::<i64>() {
                                    let tg =
                                        borg_core::telegram::Telegram::new(tg_token_notify.clone());
                                    let _ = tg.send_message(chat_id_i64, message, None).await;
                                }
                            }
                        }
                        let data = json!({
                            "type": evt.kind(),
                            "task_id": evt.task_id(),
                            "message": evt.message(),
                        })
                        .to_string();
                        let _ = log_tx_fwd.send(data);
                    },
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }

    let stream_manager = Arc::clone(&pipeline.stream_manager);

    let state = Arc::new(AppState {
        db,
        config: Arc::clone(&config),
        start_time: Instant::now(),
        log_tx,
        log_ring,
        pipeline_event_tx,
        stream_manager,
        chat_event_tx,
        web_sessions: Arc::new(TokioMutex::new(HashMap::new())),
        backends,
        force_restart,
    });

    let dashboard_dir = config.dashboard_dist_dir.clone();
    let serve_dir = ServeDir::new(&dashboard_dir).fallback(tower_http::services::ServeFile::new(
        format!("{dashboard_dir}/index.html"),
    ));

    let app = Router::new()
        // Health
        .route("/api/health", get(routes::health))
        // Tasks
        .route("/api/tasks", get(routes::list_tasks))
        .route("/api/tasks/create", post(routes::create_task))
        .route("/api/tasks/:id", get(routes::get_task))
        .route("/api/tasks/:id/retry", post(routes::retry_task))
        .route("/api/tasks/retry-all-failed", post(routes::retry_all_failed))
        .route(
            "/api/tasks/:id/outputs",
            get(routes::get_task_outputs_handler),
        )
        .route("/api/tasks/:id/stream", get(routes::sse_task_stream))
        // Task messages
        .route("/api/tasks/:id/messages", get(routes::get_task_messages))
        .route("/api/tasks/:id/messages", post(routes::post_task_message))
        // Queue
        .route("/api/queue", get(routes::list_queue))
        // Status
        .route("/api/status", get(routes::get_status))
        // Proposals
        .route("/api/proposals", get(routes::list_proposals))
        .route("/api/proposals/triage", post(routes::triage_proposals))
        .route("/api/proposals/:id/approve", post(routes::approve_proposal))
        .route("/api/proposals/:id/dismiss", post(routes::dismiss_proposal))
        .route("/api/proposals/:id/reopen", post(routes::reopen_proposal))
        // Projects
        .route("/api/projects", get(routes::list_projects))
        .route("/api/projects", post(routes::create_project))
        .route("/api/projects/:id/files", get(routes::list_project_files))
        .route(
            "/api/projects/:id/files/upload",
            post(routes::upload_project_files).layer(DefaultBodyLimit::max(110 * 1024 * 1024)),
        )
        .route(
            "/api/projects/:id/chat/messages",
            get(routes::get_project_chat_messages),
        )
        .route("/api/projects/:id/chat", post(routes::post_project_chat))
        // Modes
        .route("/api/modes", get(routes::get_modes))
        .route("/api/modes/full", get(routes::get_full_modes))
        .route("/api/modes/custom", get(routes::list_custom_modes))
        .route("/api/modes/custom", post(routes::upsert_custom_mode))
        .route(
            "/api/modes/custom/:name",
            delete(routes::delete_custom_mode),
        )
        // Settings
        .route("/api/settings", get(routes::get_settings))
        .route("/api/settings", put(routes::put_settings))
        // Focus
        .route("/api/focus", get(routes::get_focus))
        .route("/api/focus", post(routes::post_focus))
        .route("/api/focus", delete(routes::delete_focus))
        // SSE logs
        .route("/api/logs", get(routes::sse_logs))
        // Events (queryable log)
        .route("/api/events", get(routes::get_events))
        // Chat
        .route("/api/chat/events", get(routes::sse_chat_events))
        .route("/api/chat/threads", get(routes::get_chat_threads))
        .route("/api/chat/messages", get(routes::get_chat_messages))
        .route("/api/chat", post(routes::post_chat))
        // Release / restart
        .route("/api/release", post(routes::post_release))
        // Backend overrides
        .route("/api/tasks/:id/backend", put(routes::put_task_backend))
        .route("/api/repos", get(routes::list_repos_handler))
        .route("/api/repos/:id/backend", put(routes::put_repo_backend))
        // Static dashboard
        .fallback_service(serve_dir)
        .layer(CorsLayer::permissive())
        .with_state(state);

    let bind = config.web_bind.clone();
    let port = config.web_port;
    let addr = format!("{bind}:{port}");

    info!("Listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
