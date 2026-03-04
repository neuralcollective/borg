mod auth;
mod ingestion;
mod logging;
mod opensearch;
mod routes;
mod routes_modes;
mod storage;

use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Instant,
};

use axum::{
    extract::DefaultBodyLimit,
    middleware,
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
use tokio::sync::{broadcast, Mutex as TokioMutex, Semaphore};
use axum::http::HeaderValue;
use tower_http::{
    cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer},
    services::ServeDir,
};
use tracing::info;

// ── AppState ──────────────────────────────────────────────────────────────

pub struct AppState {
    pub db: Arc<Db>,
    pub config: Arc<Config>,
    pub api_token: String,
    pub start_time: Instant,
    pub log_tx: broadcast::Sender<String>,
    pub log_ring: Arc<std::sync::Mutex<VecDeque<String>>>,
    pub pipeline_event_tx: broadcast::Sender<PipelineEvent>,
    pub stream_manager: Arc<TaskStreamManager>,
    pub chat_event_tx: broadcast::Sender<String>,
    pub web_sessions: Arc<TokioMutex<HashMap<String, String>>>,
    pub backends: std::collections::HashMap<String, Arc<dyn borg_core::agent::AgentBackend>>,
    pub force_restart: Arc<std::sync::atomic::AtomicBool>,
    pub chat_rate: Arc<std::sync::Mutex<HashMap<String, std::time::Instant>>>,
    pub triage_running: Arc<std::sync::atomic::AtomicBool>,
    pub embed_client: borg_core::knowledge::EmbeddingClient,
    pub file_storage: Arc<storage::FileStorage>,
    pub ingestion_queue: Arc<ingestion::IngestionQueue>,
    pub opensearch: Option<Arc<opensearch::OpenSearchClient>>,
    pub upload_processing_sem: Arc<Semaphore>,
    pub upload_processing_limit: usize,
}

impl AppState {
    pub fn default_backend(&self, name: &str) -> Option<Arc<dyn borg_core::agent::AgentBackend>> {
        self.backends
            .get(name)
            .or_else(|| self.backends.values().next())
            .map(Arc::clone)
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

    std::fs::create_dir_all(&env_config.data_dir)?;

    // Generate per-startup API token and write to disk (0600)
    let api_token = auth::generate_token();
    let token_path = format!("{}/.api-token", env_config.data_dir);
    std::fs::write(&token_path, &api_token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600))?;
    }
    info!("API token written to {token_path}");

    let db_path = format!("{}/borg.db", env_config.data_dir);
    let mut db = Db::open(&db_path)?;
    db.migrate()?;

    // Seed DB from env on first run, then load DB values (DB wins over env)
    env_config.seed_db(&db)?;
    let config = env_config.load_from_db(&db);
    let builtin_modes = borg_domains::modes_for_focus(config.experimental_domains);
    borg_core::modes::register_modes(builtin_modes);

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
            &repo.repo_slug,
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
                    "implement" | "validate" | "review" | "lint_fix" | "rebase"
                        | "spec" | "qa" | "qa_fix" | "impl" | "retry"
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
    let file_storage = Arc::new(storage::FileStorage::from_config(&config).await?);
    let ingestion_queue = Arc::new(ingestion::IngestionQueue::from_config(&config).await?);
    let opensearch = opensearch::OpenSearchClient::from_config(&config).map(Arc::new);

    match &*ingestion_queue {
        ingestion::IngestionQueue::Disabled => info!("ingestion queue backend: disabled"),
        ingestion::IngestionQueue::Sqs { queue_url, .. } => {
            info!("ingestion queue backend: sqs ({queue_url})");
        }
    }

    // Detect sandbox backend (bwrap preferred, docker fallback, configurable via SANDBOX_BACKEND)
    let sandbox_mode = Sandbox::detect(&config.sandbox_backend).await;

    // Remove any orphaned borg-agent containers left over from a previous crash,
    // and ensure the agent bridge network exists.
    let agent_network_available = if sandbox_mode == borg_core::sandbox::SandboxMode::Docker {
        Sandbox::prune_orphan_containers().await;
        let net_ok = Sandbox::ensure_agent_network().await;
        if net_ok {
            if !Sandbox::install_network_rules().await {
                tracing::warn!("sandbox: iptables rules not installed — agent containers have unrestricted network access");
            }
        }
        net_ok
    } else {
        false
    };

    // Build backends map
    let mut backends: std::collections::HashMap<String, Arc<dyn borg_core::agent::AgentBackend>> =
        std::collections::HashMap::new();
    backends.insert(
        "claude".into(),
        Arc::new(
            ClaudeBackend::new("claude", sandbox_mode.clone(), &config.container_image)
                .with_timeout(config.agent_timeout_s as u64)
                .with_resource_limits(config.container_memory_mb, config.container_cpus)
                .with_git_author(&config.git_author_name, &config.git_author_email),
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
                    .with_reasoning_effort(codex_reasoning_effort)
                    .with_timeout(config.agent_timeout_s as u64)
                    .with_git_identity(
                        &config.git_author_name,
                        &config.git_author_email,
                        &config.git_committer_name,
                        &config.git_committer_email,
                    ),
            ),
        );
    }
    // Local model via Ollama — enabled by setting OLLAMA_URL or LOCAL_MODEL
    if std::env::var("OLLAMA_URL").is_ok() || std::env::var("LOCAL_MODEL").is_ok() {
        let url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into());
        let model = std::env::var("LOCAL_MODEL").unwrap_or_else(|_| "llama3.2".into());
        backends.insert(
            "local".into(),
            Arc::new(OllamaBackend::new(url, model)?.with_timeout(300)?),
        );
        info!("local backend registered (Ollama)");
    }

    let force_restart = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let (pipeline, pipeline_rx) = Pipeline::new(
        Arc::clone(&db),
        backends.clone(),
        Arc::clone(&config),
        sandbox_mode.clone(),
        Arc::clone(&force_restart),
        agent_network_available,
    );
    let pipeline_event_tx = pipeline.event_tx.clone();
    let pipeline = Arc::new(pipeline);

    // Pipeline tick loop — inner spawn catches panics so the loop never dies.
    // If tick panics repeatedly, exit and let systemd restart with a clean process.
    let tick_secs = config.pipeline_tick_s;
    {
        let pipeline = Arc::clone(&pipeline);
        tokio::spawn(async move {
            let mut consecutive_panics = 0u32;
            loop {
                let p = Arc::clone(&pipeline);
                let handle = tokio::spawn(async move { p.tick().await });
                match handle.await {
                    Ok(Ok(())) => consecutive_panics = 0,
                    Ok(Err(e)) => {
                        tracing::error!("Pipeline tick error: {e}");
                        consecutive_panics = 0;
                    }
                    Err(join_err) => {
                        consecutive_panics += 1;
                        tracing::error!(
                            "Pipeline tick panicked ({consecutive_panics}/5): {join_err}"
                        );
                        if consecutive_panics >= 5 {
                            tracing::error!("5 consecutive tick panics — exiting for restart");
                            std::process::exit(1);
                        }
                    }
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
        let file_storage_tg = Arc::clone(&file_storage);
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
                                    project_id: 0,
                                    task_type: String::new(),
                                    started_at: None,
                                    completed_at: None,
                                    duration_secs: None,
                                    review_status: None,
                                    revision_count: 0,
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
                                let storage2 = Arc::clone(&file_storage_tg);
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
                                        &storage2,
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
        let build_cmd = config.build_cmd.clone();
        tokio::spawn(async move {
            let git = borg_core::git::Git::new(&self_repo.path);
            let mut last_head = git.rev_parse_head().unwrap_or_default();

            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(check_interval)).await;

                if force_restart_check.load(std::sync::atomic::Ordering::Relaxed) {
                    tracing::info!("Force restart requested via /api/release, rebuilding...");
                    if routes::rebuild_and_exec(&self_repo.path, &build_cmd).await {
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
                if routes::rebuild_and_exec(&self_repo.path, &build_cmd).await {
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
        let storage_sc = Arc::clone(&file_storage);
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
                    config.chat_collection_window_ms.max(0) as u64,
                    config.max_chat_agents,
                    config.chat_cooldown_ms.max(0) as u64,
                ));

                // Flush expired collection windows periodically
                let collector_flush = Arc::clone(&collector);
                let sidecar_flush = Arc::clone(&sidecar);
                let sessions_flush = Arc::clone(&sc_sessions);
                let config_flush = Arc::clone(&config_sc);
                let db_flush = Arc::clone(&db_sc);
                let storage_flush = Arc::clone(&storage_sc);
                let chat_tx_flush = sc_chat_event_tx.clone();
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                        for batch in collector_flush.flush_expired().await {
                            let sidecar2 = Arc::clone(&sidecar_flush);
                            let sessions2 = Arc::clone(&sessions_flush);
                            let config2 = Arc::clone(&config_flush);
                            let db2 = Arc::clone(&db_flush);
                            let storage2 = Arc::clone(&storage_flush);
                            let chat_tx2 = chat_tx_flush.clone();
                            let collector2 = Arc::clone(&collector_flush);
                            let is_discord = batch.chat_key.starts_with("discord:");
                            let chat_id = batch
                                .chat_key
                                .splitn(2, ':')
                                .nth(1)
                                .unwrap_or("")
                                .to_string();
                            let sender_name = batch.sender_name.clone();
                            tokio::spawn(async move {
                                match routes::run_chat_agent(
                                    &batch.chat_key,
                                    &sender_name,
                                    &batch.messages,
                                    &sessions2,
                                    &config2,
                                    &db2,
                                    &storage2,
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
                let storage_events = Arc::clone(&storage_sc);
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
                            let storage2 = Arc::clone(&storage_events);
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
                                    &storage2,
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
        let observer_api_key = std::env::var("ANTHROPIC_API_KEY")
            .unwrap_or_else(|_| config.oauth_token.clone());
        let observer = Observer::load(
            &config.observer_config,
            &observer_api_key,
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

    let upload_processing_limit = std::env::var("UPLOAD_PROCESSING_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(2);

    let state = Arc::new(AppState {
        db,
        config: Arc::clone(&config),
        api_token,
        start_time: Instant::now(),
        log_tx,
        log_ring,
        pipeline_event_tx,
        stream_manager,
        chat_event_tx,
        web_sessions: Arc::new(TokioMutex::new(HashMap::new())),
        backends,
        force_restart,
        chat_rate: Arc::new(std::sync::Mutex::new(HashMap::new())),
        triage_running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        embed_client: borg_core::knowledge::EmbeddingClient::from_env(),
        file_storage: Arc::clone(&file_storage),
        ingestion_queue: Arc::clone(&ingestion_queue),
        opensearch: opensearch.clone(),
        upload_processing_sem: Arc::new(Semaphore::new(upload_processing_limit)),
        upload_processing_limit,
    });

    {
        let queue = Arc::clone(&state.ingestion_queue);
        let db = Arc::clone(&state.db);
        let storage = Arc::clone(&state.file_storage);
        let search = state.opensearch.clone();
        tokio::spawn(async move {
            queue.run_worker(db, storage, search).await;
        });
    }

    let dashboard_dir = config.dashboard_dist_dir.clone();
    let serve_dir = ServeDir::new(&dashboard_dir).fallback(tower_http::services::ServeFile::new(
        format!("{dashboard_dir}/index.html"),
    ));

    let app = Router::new()
        // Health (unauthenticated)
        .route("/api/health", get(routes::health))
        // Token endpoint (localhost-only, no bearer required)
        .route("/api/auth/token", get(auth::get_token))
        // Tasks
        .route("/api/tasks", get(routes::list_tasks))
        .route("/api/tasks/create", post(routes::create_task))
        .route("/api/tasks/:id", get(routes::get_task).patch(routes::patch_task))
        .route("/api/tasks/:id/approve", post(routes::approve_task))
        .route("/api/tasks/:id/reject", post(routes::reject_task))
        .route("/api/tasks/:id/request-revision", post(routes::request_revision))
        .route("/api/tasks/:id/revisions", get(routes::get_revision_history))
        .route("/api/tasks/:id/citations", get(routes::get_task_citations))
        .route("/api/tasks/:id/verify-citations", post(routes::verify_task_citations))
        .route("/api/tasks/:id/retry", post(routes::retry_task))
        .route("/api/tasks/:id/unblock", post(routes::unblock_task))
        .route("/api/tasks/:id/diagnostics", get(routes::get_task_diagnostics))
        .route("/api/tasks/retry-all-failed", post(routes::retry_all_failed))
        .route(
            "/api/tasks/:id/outputs",
            get(routes::get_task_outputs_handler),
        )
        .route("/api/tasks/:id/stream", get(routes::sse_task_stream))
        .route("/api/tasks/:id/container", get(routes::get_task_container))
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
        .route("/api/projects/search", get(routes::search_projects))
        .route("/api/projects/conflicts", get(routes::check_conflicts))
        .route("/api/projects/:id/files", get(routes::list_project_files))
        .route(
            "/api/projects/:id/files/:file_id/content",
            get(routes::get_project_file_content),
        )
        .route(
            "/api/projects/:id/files/:file_id/text",
            get(routes::get_project_file_text),
        )
        .route(
            "/api/projects/:id/files/:file_id/reextract",
            post(routes::reextract_project_file),
        )
        .route(
            "/api/projects/:id/files/upload",
            post(routes::upload_project_files).layer(DefaultBodyLimit::max(110 * 1024 * 1024)),
        )
        .route(
            "/api/projects/:id/uploads/sessions",
            post(routes::create_upload_session).get(routes::list_project_upload_sessions),
        )
        .route(
            "/api/projects/:id/uploads/sessions/:session_id",
            get(routes::get_upload_session_status),
        )
        .route(
            "/api/projects/:id/uploads/sessions/:session_id/retry",
            post(routes::retry_upload_session),
        )
        .route(
            "/api/projects/:id/uploads/sessions/:session_id/complete",
            post(routes::complete_upload_session),
        )
        .route(
            "/api/projects/:id/uploads/sessions/:session_id/chunks/:chunk_index",
            put(routes::upload_session_chunk),
        )
        .route("/api/uploads/overview", get(routes::get_upload_overview))
        .route(
            "/api/projects/:id/chat/messages",
            get(routes::get_project_chat_messages),
        )
        .route("/api/projects/:id/chat", post(routes::post_project_chat))
        .route("/api/projects/:id", get(routes::get_project).put(routes::update_project).delete(routes::delete_project))
        .route("/api/projects/:id/tasks", get(routes::list_project_tasks))
        .route("/api/projects/:id/deadlines", get(routes::list_project_deadlines).post(routes::create_deadline))
        .route("/api/projects/:id/deadlines/:did", put(routes::update_deadline).delete(routes::delete_deadline))
        .route("/api/deadlines", get(routes::list_upcoming_deadlines))
        .route("/api/search", get(routes::search_documents))
        .route("/api/themes", get(routes::summarize_workspace_themes))
        .route("/api/projects/:id/themes", get(routes::summarize_project_themes))
        .route("/api/projects/:id/audit", get(routes::list_project_audit))
        .route("/api/projects/:id/documents", get(routes::list_project_documents))
        .route(
            "/api/projects/:id/documents/:task_id/content",
            get(routes::get_project_document_content),
        )
        .route(
            "/api/projects/:id/documents/:task_id/versions",
            get(routes::get_project_document_versions),
        )
        .route(
            "/api/projects/:id/documents/:task_id/export",
            get(routes::export_project_document),
        )
        .route(
            "/api/projects/:id/export-all",
            get(routes::export_all_project_documents),
        )
        .route(
            "/api/projects/:id/documents/:task_id",
            delete(routes::delete_project_document),
        )
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
        // API keys (BYOK)
        .route("/api/keys", get(routes::list_api_keys))
        .route("/api/keys", post(routes::store_api_key))
        .route("/api/keys/:id", delete(routes::delete_api_key))
        // Cache volumes
        .route("/api/cache", get(routes::list_cache_volumes))
        .route("/api/cache/:name", delete(routes::delete_cache_volume))
        // Cloud storage OAuth
        .route("/api/cloud/:provider/auth", get(routes::cloud_auth_init))
        .route("/api/cloud/:provider/callback", get(routes::cloud_auth_callback))
        .route("/api/projects/:id/cloud", get(routes::list_cloud_connections))
        .route("/api/projects/:id/cloud/:conn_id", delete(routes::delete_cloud_connection))
        .route("/api/projects/:id/cloud/:conn_id/browse", get(routes::browse_cloud_files))
        .route("/api/projects/:id/cloud/:conn_id/import", post(routes::import_cloud_files))
        // Knowledge base
        .route("/api/knowledge", get(routes::list_knowledge))
        .route("/api/knowledge/templates", get(routes::list_templates))
        .route(
            "/api/knowledge/upload",
            post(routes::upload_knowledge).layer(DefaultBodyLimit::max(55 * 1024 * 1024)),
        )
        .route("/api/knowledge/:id", put(routes::update_knowledge))
        .route("/api/knowledge/:id", delete(routes::delete_knowledge))
        .route("/api/knowledge/:id/content", get(routes::get_knowledge_content))
        // Static dashboard
        .fallback_service(serve_dir)
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            auth::auth_middleware,
        ))
        .layer({
            let origins: Vec<HeaderValue> = [
                "https://borg.legal",
                "https://app.borg.legal",
                "https://borg.neuralcollective.ai",
                "http://localhost:5173",
                "http://localhost:3131",
            ]
            .iter()
            .map(|o| o.parse().unwrap())
            .collect();
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(origins))
                .allow_methods(AllowMethods::list([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::PUT,
                    axum::http::Method::DELETE,
                    axum::http::Method::OPTIONS,
                ]))
                .allow_headers(AllowHeaders::list([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::AUTHORIZATION,
                ]))
                .allow_credentials(true)
        })
        .with_state(state);

    let bind = config.web_bind.clone();
    let port = config.web_port;
    let addr = format!("{bind}:{port}");

    info!("Listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    );

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        res = server => { res?; }
        _ = tokio::signal::ctrl_c() => {
            info!("shutdown signal received (SIGINT)");
        }
        _ = sigterm.recv() => {
            info!("shutdown signal received (SIGTERM)");
        }
    }

    if agent_network_available {
        Sandbox::remove_network_rules().await;
        Sandbox::remove_agent_network().await;
    }

    Ok(())
}
