mod auth;
mod backup;
mod ingestion;
mod instrumentation;
mod logging;
mod messaging_progress;
mod proxy;
mod routes;
mod routes_modes;
mod search;
mod storage;
mod user_bots;
mod vespa;

use std::{
    collections::{HashMap, VecDeque},
    sync::{atomic::AtomicU64, Arc},
    time::Instant,
};

use axum::{
    extract::DefaultBodyLimit,
    http::{HeaderName, HeaderValue},
    middleware,
    routing::{delete, get, post, put},
    Router,
};
use borg_agent::{
    claude::ClaudeBackend, codex::CodexBackend, gemini::GeminiBackend, ollama::OllamaBackend,
};
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
use messaging_progress::{new_chat_run_id, spawn_chat_progress_forwarder, MessagingProgressSink};
use tokio::sync::{broadcast, Mutex as TokioMutex, Semaphore};
use tower_http::{
    cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer},
    services::ServeDir,
};
use tracing::info;

// ── AppState ──────────────────────────────────────────────────────────────

pub struct AppState {
    pub db: Arc<Db>,
    pub config: Arc<Config>,
    pub ai_request_count: Arc<AtomicU64>,
    pub api_token: String,
    pub jwt_secret: String,
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
    pub embed_registry: Arc<borg_core::knowledge::EmbeddingRegistry>,
    pub file_storage: Arc<storage::FileStorage>,
    pub ingestion_queue: Arc<ingestion::IngestionQueue>,
    pub search: Option<Arc<search::SearchClient>>,
    pub brave_search: Option<Arc<borg_core::knowledge::BraveSearchClient>>,
    pub upload_processing_sem: Arc<Semaphore>,
    pub upload_processing_limit: usize,
    pub login_attempts: Arc<std::sync::Mutex<HashMap<String, (u32, std::time::Instant)>>>,
    pub(crate) linked_credential_sessions:
        Arc<TokioMutex<HashMap<String, routes::LinkedCredentialConnectSession>>>,
    pub(crate) linked_credential_stdins:
        Arc<TokioMutex<HashMap<String, tokio::process::ChildStdin>>>,
}

impl AppState {
    pub fn default_backend(&self, name: &str) -> Option<Arc<dyn borg_core::agent::AgentBackend>> {
        self.backends.get(name).map(Arc::clone)
    }
}

// ── Sidecar helpers ───────────────────────────────────────────────────────

fn sidecar_source_prefix(chat_key: &str) -> String {
    chat_key.splitn(2, ':').next().unwrap_or("discord").to_string()
}

/// Parse a chat_key like "discord:u42:channel_id" into (user_id, channel_id).
/// For non-user keys like "discord:channel_id", returns (None, "channel_id").
fn parse_user_chat_id(chat_key: &str) -> (Option<i64>, String) {
    let rest = chat_key.splitn(2, ':').nth(1).unwrap_or("");
    if let Some(stripped) = rest.strip_prefix('u') {
        if let Some(colon) = stripped.find(':') {
            if let Ok(uid) = stripped[..colon].parse::<i64>() {
                return (Some(uid), stripped[colon + 1..].to_string());
            }
        }
    }
    (None, rest.to_string())
}

fn make_progress_sink(
    source: &str,
    sidecar: Arc<borg_core::sidecar::Sidecar>,
    chat_id: String,
    reply_to: Option<String>,
) -> messaging_progress::MessagingProgressSink {
    match source {
        "slack" => messaging_progress::MessagingProgressSink::Slack { sidecar, chat_id, reply_to },
        "whatsapp" => messaging_progress::MessagingProgressSink::WhatsApp {
            sidecar,
            chat_id,
            quote_id: reply_to,
        },
        _ => messaging_progress::MessagingProgressSink::Discord { sidecar, chat_id, reply_to },
    }
}

fn send_sidecar_reply_with_user(
    sidecar: &borg_core::sidecar::Sidecar,
    source: &str,
    user_id: Option<i64>,
    chat_id: &str,
    text: &str,
    reply_to: Option<&str>,
) {
    match source {
        "slack" => sidecar.send_slack(chat_id, text, reply_to),
        "whatsapp" => sidecar.send_whatsapp(chat_id, text, reply_to),
        _ => {
            if let Some(uid) = user_id {
                sidecar.send_user_discord(uid, chat_id, text, reply_to);
            } else {
                sidecar.send_discord(chat_id, text, reply_to);
            }
        },
    }
}

// ── Background task functions ─────────────────────────────────────────────

fn spawn_pipeline_ticker(pipeline: Arc<borg_core::pipeline::Pipeline>, tick_secs: u64) {
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
                },
                Err(join_err) => {
                    consecutive_panics += 1;
                    tracing::error!(
                        "Pipeline tick panicked ({consecutive_panics}/5): {join_err}"
                    );
                    if consecutive_panics >= 5 {
                        tracing::error!("5 consecutive tick panics — exiting for restart");
                        std::process::exit(1);
                    }
                },
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(tick_secs)).await;
        }
    });
}

fn spawn_telegram_poller(
    token: String,
    db: Arc<Db>,
    config: Arc<Config>,
    repos: Vec<borg_core::types::RepoConfig>,
    file_storage: Arc<storage::FileStorage>,
    search: Option<Arc<search::SearchClient>>,
    chat_event_tx: broadcast::Sender<String>,
    ai_request_count: Arc<AtomicU64>,
) {
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
                        if !msg.mentions_bot && !msg.reply_to_bot && msg.chat_type != "private" {
                            continue;
                        }
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
                                updated_at: Utc::now(),
                                session_id: String::new(),
                                mode,
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
                            let task_title = task.title.clone();
                            let tg2 = Arc::clone(&tg);
                            match db.insert_task(&task) {
                                Ok(id) => {
                                    let reply = format!("Task #{id} created: {task_title}");
                                    let _ = tg2
                                        .send_message(msg.chat_id, &reply, Some(msg.message_id))
                                        .await;
                                },
                                Err(e) => tracing::error!("insert_task from telegram: {e}"),
                            }
                        } else {
                            let chat_key = format!("telegram:{}", msg.chat_id);
                            let _ = tg.send_typing(msg.chat_id).await;
                            let tg2 = Arc::clone(&tg);
                            let sessions2 = Arc::clone(&tg_sessions);
                            let config2 = Arc::clone(&config);
                            let db2 = Arc::clone(&db);
                            let search2 = search.clone();
                            let storage2 = Arc::clone(&file_storage);
                            let chat_tx2 = chat_event_tx.clone();
                            let ai_request_count2 = Arc::clone(&ai_request_count);
                            let sender_name = msg.sender_name.clone();
                            let chat_id = msg.chat_id;
                            let message_id = msg.message_id;
                            tokio::spawn(async move {
                                let run_id = new_chat_run_id();
                                let progress = spawn_chat_progress_forwarder(
                                    &chat_tx2,
                                    chat_key.clone(),
                                    run_id.clone(),
                                    MessagingProgressSink::Telegram {
                                        client: Arc::clone(&tg2),
                                        chat_id,
                                        reply_to: Some(message_id),
                                    },
                                );
                                match routes::run_chat_agent(
                                    &chat_key,
                                    &run_id,
                                    &sender_name,
                                    &[text],
                                    &sessions2,
                                    &config2,
                                    &db2,
                                    search2.clone(),
                                    &storage2,
                                    &chat_tx2,
                                    &ai_request_count2,
                                )
                                .await
                                {
                                    Ok(reply) if !reply.is_empty() => {
                                        progress.stop().await;
                                        let _ = tg2
                                            .send_message(chat_id, &reply, Some(message_id))
                                            .await;
                                    },
                                    Ok(_) => {
                                        progress.stop().await;
                                    },
                                    Err(e) => {
                                        progress.stop().await;
                                        let _ = tg2
                                            .send_plain_message(
                                                chat_id,
                                                "I hit an error while working on that.",
                                                Some(message_id),
                                            )
                                            .await;
                                        tracing::warn!("Telegram chat agent error: {e}");
                                    },
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

fn spawn_user_bot_manager(
    db: Arc<Db>,
    config: Arc<Config>,
    search: Option<Arc<search::SearchClient>>,
    file_storage: Arc<storage::FileStorage>,
    chat_event_tx: broadcast::Sender<String>,
    ai_request_count: Arc<AtomicU64>,
    sidecar_slot: Arc<TokioMutex<Option<Arc<Sidecar>>>>,
) {
    let mgr = Arc::new(user_bots::UserBotManager::new(
        db,
        config,
        search,
        file_storage,
        chat_event_tx,
        ai_request_count,
        sidecar_slot,
    ));
    tokio::spawn(async move { mgr.run().await });
}

fn spawn_self_repo_watcher(
    self_repo: borg_core::types::RepoConfig,
    check_interval: u64,
    force_restart: Arc<std::sync::atomic::AtomicBool>,
    build_cmd: String,
) {
    tokio::spawn(async move {
        let git = borg_core::git::Git::new(&self_repo.path);
        let mut last_head = git.rev_parse_head().unwrap_or_default();

        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(check_interval)).await;

            if force_restart.load(std::sync::atomic::Ordering::Relaxed) {
                tracing::info!("Force restart requested via /api/release, rebuilding...");
                if routes::rebuild_and_exec(&self_repo.path, &build_cmd).await {
                    force_restart.store(false, std::sync::atomic::Ordering::Relaxed);
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
                last_head = remote_head;
            } else {
                tracing::warn!(
                    "Self-update rebuild failed; keeping previous last_head for retry"
                );
            }
        }
    });
}

async fn spawn_sidecar_manager(
    config: Arc<Config>,
    db: Arc<Db>,
    file_storage: Arc<storage::FileStorage>,
    search: Option<Arc<search::SearchClient>>,
    chat_event_tx: broadcast::Sender<String>,
    ai_request_count: Arc<AtomicU64>,
    sidecar_slot: Arc<TokioMutex<Option<Arc<Sidecar>>>>,
) {
    match Sidecar::spawn(
        &config.assistant_name,
        &config.discord_token,
        &config.wa_auth_dir,
        config.wa_disabled,
        &config.slack_bot_token,
        &config.slack_app_token,
    )
    .await
    {
        Err(e) => tracing::warn!("Sidecar spawn failed: {e}"),
        Ok((sidecar, mut event_rx)) => {
            let sidecar = Arc::new(sidecar);
            *sidecar_slot.lock().await = Some(Arc::clone(&sidecar));
            let sc_sessions: Arc<TokioMutex<HashMap<String, String>>> =
                Arc::new(TokioMutex::new(HashMap::new()));
            let collector = Arc::new(ChatCollector::new(
                config.chat_collection_window_ms.max(0) as u64,
                config.max_chat_agents,
                config.chat_cooldown_ms.max(0) as u64,
            ));

            // Flush expired collection windows periodically
            {
                let collector_flush = Arc::clone(&collector);
                let sidecar_flush = Arc::clone(&sidecar);
                let sessions_flush = Arc::clone(&sc_sessions);
                let config_flush = Arc::clone(&config);
                let db_flush = Arc::clone(&db);
                let storage_flush = Arc::clone(&file_storage);
                let search_flush = search.clone();
                let chat_tx_flush = chat_event_tx.clone();
                let flush_ai_request_count = Arc::clone(&ai_request_count);
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                        for batch in collector_flush.flush_expired().await {
                            let sidecar2 = Arc::clone(&sidecar_flush);
                            let sessions2 = Arc::clone(&sessions_flush);
                            let config2 = Arc::clone(&config_flush);
                            let db2 = Arc::clone(&db_flush);
                            let search2 = search_flush.clone();
                            let storage2 = Arc::clone(&storage_flush);
                            let chat_tx2 = chat_tx_flush.clone();
                            let ai_request_count2 = Arc::clone(&flush_ai_request_count);
                            let collector2 = Arc::clone(&collector_flush);
                            let chat_source = sidecar_source_prefix(&batch.chat_key);
                            let (user_id, chat_id) = parse_user_chat_id(&batch.chat_key);
                            let sender_name = batch.sender_name.clone();
                            tokio::spawn(async move {
                                let run_id = new_chat_run_id();
                                let sink = make_progress_sink(
                                    &chat_source,
                                    Arc::clone(&sidecar2),
                                    chat_id.clone(),
                                    None,
                                );
                                let progress = spawn_chat_progress_forwarder(
                                    &chat_tx2,
                                    batch.chat_key.clone(),
                                    run_id.clone(),
                                    sink,
                                );
                                match routes::run_chat_agent(
                                    &batch.chat_key,
                                    &run_id,
                                    &sender_name,
                                    &batch.messages,
                                    &sessions2,
                                    &config2,
                                    &db2,
                                    search2.clone(),
                                    &storage2,
                                    &chat_tx2,
                                    &ai_request_count2,
                                )
                                .await
                                {
                                    Ok(reply) if !reply.is_empty() => {
                                        progress.stop().await;
                                        send_sidecar_reply_with_user(
                                            &sidecar2,
                                            &chat_source,
                                            user_id,
                                            &chat_id,
                                            &reply,
                                            None,
                                        );
                                    },
                                    Ok(_) => {
                                        progress.stop().await;
                                    },
                                    Err(e) => {
                                        progress.stop().await;
                                        send_sidecar_reply_with_user(
                                            &sidecar2,
                                            &chat_source,
                                            user_id,
                                            &chat_id,
                                            "I hit an error while working on that.",
                                            None,
                                        );
                                        tracing::warn!("Sidecar flush agent error: {e}");
                                    },
                                }
                                collector2.mark_done(&batch.chat_key).await;
                            });
                        }
                    }
                });
            }

            // Process incoming sidecar events
            {
                let db_events = Arc::clone(&db);
                let storage_events = Arc::clone(&file_storage);
                let search_events = search.clone();
                let chat_tx_events = chat_event_tx.clone();
                let events_ai_request_count = Arc::clone(&ai_request_count);
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
                        let source_prefix = match msg.source {
                            Source::Discord => "discord",
                            Source::WhatsApp => "whatsapp",
                            Source::Slack => "slack",
                        };
                        let chat_key = if let Some(uid) = msg.user_id {
                            format!("{}:u{}:{}", source_prefix, uid, msg.chat_id)
                        } else {
                            format!("{}:{}", source_prefix, msg.chat_id)
                        };
                        let mut msg_text = msg.text.clone();
                        if !msg.attachments.is_empty() {
                            let att_base =
                                format!("{}/attachments", config.data_dir);
                            for att in &msg.attachments {
                                let att_dir =
                                    format!("{}/discord-{}", att_base, &att.filename);
                                std::fs::create_dir_all(&att_dir).ok();
                                let path = format!("{}/{}", att_dir, att.filename);
                                match reqwest::get(&att.url).await {
                                    Ok(resp) => {
                                        if let Ok(bytes) = resp.bytes().await {
                                            std::fs::write(&path, &bytes).ok();
                                            let size_kb = bytes.len() / 1024;
                                            msg_text.push_str(&format!(
                                                "\n[Attached file: {} ({}KB)] Path: {}",
                                                att.filename, size_kb, path
                                            ));
                                        }
                                    },
                                    Err(e) => {
                                        tracing::warn!(
                                            "discord attachment download failed: {e}"
                                        )
                                    },
                                }
                            }
                        }

                        let incoming = borg_core::chat::IncomingMessage {
                            chat_key: chat_key.clone(),
                            sender_name: msg.sender_name.clone(),
                            text: msg_text,
                            timestamp: msg.timestamp,
                            reply_to_message_id: None,
                        };
                        if let Some(batch) = collector.process(incoming).await {
                            let sidecar2 = Arc::clone(&sidecar);
                            let sessions2 = Arc::clone(&sc_sessions);
                            let config2 = Arc::clone(&config);
                            let db2 = Arc::clone(&db_events);
                            let search2 = search_events.clone();
                            let storage2 = Arc::clone(&storage_events);
                            let chat_tx2 = chat_tx_events.clone();
                            let ai_request_count2 = Arc::clone(&events_ai_request_count);
                            let collector2 = Arc::clone(&collector);
                            let chat_source = source_prefix.to_string();
                            let chat_id = msg.chat_id.clone();
                            let msg_user_id = msg.user_id;
                            let msg_id = msg.id.clone();
                            let sender_name = msg.sender_name.clone();
                            match (&msg.source, msg_user_id) {
                                (Source::Discord, Some(uid)) => sidecar.send_user_discord_typing(uid, &chat_id),
                                (Source::Discord, None) => sidecar.send_discord_typing(&chat_id),
                                (Source::WhatsApp, _) => sidecar.send_whatsapp_typing(&chat_id),
                                (Source::Slack, _) => sidecar.send_slack_typing(&chat_id),
                            }
                            tokio::spawn(async move {
                                let run_id = new_chat_run_id();
                                let sink = make_progress_sink(
                                    &chat_source,
                                    Arc::clone(&sidecar2),
                                    chat_id.clone(),
                                    Some(msg_id.clone()),
                                );
                                let progress = spawn_chat_progress_forwarder(
                                    &chat_tx2,
                                    batch.chat_key.clone(),
                                    run_id.clone(),
                                    sink,
                                );
                                match routes::run_chat_agent(
                                    &batch.chat_key,
                                    &run_id,
                                    &sender_name,
                                    &batch.messages,
                                    &sessions2,
                                    &config2,
                                    &db2,
                                    search2.clone(),
                                    &storage2,
                                    &chat_tx2,
                                    &ai_request_count2,
                                )
                                .await
                                {
                                    Ok(reply) if !reply.is_empty() => {
                                        progress.stop().await;
                                        send_sidecar_reply_with_user(
                                            &sidecar2,
                                            &chat_source,
                                            msg_user_id,
                                            &chat_id,
                                            &reply,
                                            Some(&msg_id),
                                        );
                                    },
                                    Ok(_) => {
                                        progress.stop().await;
                                    },
                                    Err(e) => {
                                        progress.stop().await;
                                        send_sidecar_reply_with_user(
                                            &sidecar2,
                                            &chat_source,
                                            msg_user_id,
                                            &chat_id,
                                            "I hit an error while working on that.",
                                            Some(&msg_id),
                                        );
                                        tracing::warn!("Sidecar chat agent error: {e}");
                                    },
                                }
                                collector2.mark_done(&batch.chat_key).await;
                            });
                        }
                    }
                });
            }
        },
    }
}

fn spawn_observer(observer_config: String, api_key: String, telegram_token: String) {
    let observer = Observer::load(&observer_config, &api_key, &telegram_token);
    tokio::spawn(async move { observer.run().await });
}

fn spawn_pipeline_event_forwarder(
    pipeline_rx: broadcast::Receiver<PipelineEvent>,
    log_tx: broadcast::Sender<String>,
    telegram_token: String,
    chat_event_tx: broadcast::Sender<String>,
) {
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
                        if let Some(raw_id) = chat_id.strip_prefix("tg:") {
                            if !telegram_token.is_empty() {
                                if let Ok(chat_id_i64) = raw_id.parse::<i64>() {
                                    let tg =
                                        borg_core::telegram::Telegram::new(telegram_token.clone());
                                    let _ = tg.send_message(chat_id_i64, message, None).await;
                                }
                            }
                        } else {
                            let event = serde_json::json!({
                                "role": "system",
                                "sender": "pipeline",
                                "text": message,
                                "ts": chrono::Utc::now().timestamp(),
                                "thread": chat_id,
                                "type": "task_notification",
                            })
                            .to_string();
                            let _ = chat_event_tx.send(event);
                        }
                    }
                    let data = serde_json::json!({
                        "type": evt.kind(),
                        "task_id": evt.task_id(),
                        "message": evt.message(),
                    })
                    .to_string();
                    let _ = log_tx.send(data);
                },
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
}

fn spawn_ingestion_workers(
    ingestion_queue: Arc<ingestion::IngestionQueue>,
    db: Arc<Db>,
    file_storage: Arc<storage::FileStorage>,
    search: Option<Arc<search::SearchClient>>,
    embed_registry: Arc<borg_core::knowledge::EmbeddingRegistry>,
    worker_loops: usize,
) {
    for _ in 0..worker_loops {
        let queue = Arc::clone(&ingestion_queue);
        let db = Arc::clone(&db);
        let storage = Arc::clone(&file_storage);
        let search = search.clone();
        let er = Arc::clone(&embed_registry);
        tokio::spawn(async move {
            queue.run_worker(db, storage, search, er).await;
        });
    }
}

fn spawn_backup_loop(
    db: Arc<Db>,
    config: Arc<Config>,
    file_storage: Arc<storage::FileStorage>,
) {
    tokio::spawn(async move {
        backup::run_backup_loop(db, config, file_storage).await;
    });
}

fn spawn_imap_poller(
    imap_cfg: borg_core::email::ImapConfig,
    db: Arc<Db>,
    config: Arc<Config>,
    file_storage: Arc<storage::FileStorage>,
    search: Option<Arc<search::SearchClient>>,
    chat_event_tx: broadcast::Sender<String>,
    ai_request_count: Arc<AtomicU64>,
) {
    let imap_sessions = Arc::new(TokioMutex::new(HashMap::new()));
    tokio::spawn(async move {
        borg_core::email::run_imap_poller(imap_cfg, 60, move |emails| {
            let db = Arc::clone(&db);
            let config = Arc::clone(&config);
            let sessions = Arc::clone(&imap_sessions);
            let search = search.clone();
            let storage = Arc::clone(&file_storage);
            let chat_tx = chat_event_tx.clone();
            let ai_count = Arc::clone(&ai_request_count);
            async move {
                for email in emails {
                    let user = db.get_user_by_email(&email.from).ok().flatten();
                    let sender_name = match &user {
                        Some((_, _, dn, _)) if !dn.is_empty() => dn.clone(),
                        _ => {
                            if !email.from_name.is_empty() {
                                email.from_name.clone()
                            } else {
                                email.from.clone()
                            }
                        },
                    };
                    let att_dir = format!(
                        "{}/attachments/email-{}",
                        config.data_dir,
                        chrono::Utc::now().timestamp_millis()
                    );
                    let att_paths = borg_core::email::save_attachments(
                        &email.attachments,
                        std::path::Path::new(&att_dir),
                    )
                    .unwrap_or_default();
                    let mut msgs = vec![format!(
                        "Email from {} <{}>: {}\n\n{}",
                        sender_name, email.from, email.subject, email.body
                    )];
                    for p in &att_paths {
                        let size_kb =
                            std::fs::metadata(p).map(|m| m.len() / 1024).unwrap_or(0);
                        let fname = p.file_name().unwrap_or_default().to_string_lossy();
                        msgs.push(format!(
                            "[Attached file: {} ({}KB)] Path: {}",
                            fname,
                            size_kb,
                            p.display()
                        ));
                    }
                    let chat_key = format!("email:{}", email.from);
                    let run_id = messaging_progress::new_chat_run_id();
                    let from_email = email.from.clone();
                    let reply_subject = format!("Re: {}", email.subject);
                    match routes::run_chat_agent(
                        &chat_key,
                        &run_id,
                        &sender_name,
                        &msgs,
                        &sessions,
                        &config,
                        &db,
                        search.clone(),
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
                        Err(e) => tracing::warn!("imap email agent error: {e}"),
                    }
                }
            }
        })
        .await;
    });
}

fn spawn_knowledge_repo_sync(db: Arc<Db>, data_dir: String) {
    tokio::spawn(async move {
        let repos = db.list_all_knowledge_repos().unwrap_or_default();
        for repo in repos {
            let db2 = Arc::clone(&db);
            let url = repo.url.clone();
            let dd = data_dir.clone();
            tokio::spawn(async move {
                routes::clone_knowledge_repo(repo.id, &url, &dd, &db2).await;
            });
        }
    });
}

// ── Setup helpers ─────────────────────────────────────────────────────────

fn init_tracing(
    log_tx: broadcast::Sender<String>,
    log_ring: Arc<std::sync::Mutex<VecDeque<String>>>,
) {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        "borg_server=info,borg_core=info,borg_agent=info,tower_http=warn".into()
    });
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(logging::BroadcastLayer { tx: log_tx, ring: log_ring })
        .init();
}

fn write_api_token(data_dir: &str, token: &str) -> anyhow::Result<()> {
    let token_path = format!("{data_dir}/.api-token");
    std::fs::write(&token_path, token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600))?;
    }
    info!("API token written to {token_path}");
    Ok(())
}

fn init_db(env_config: &Config) -> anyhow::Result<(Db, Config)> {
    let mut db = Db::open(&env_config.database_url)?;
    db.migrate()?;
    env_config.seed_db(&db)?;
    let config = env_config.load_from_db(&db);
    let builtin_modes = borg_domains::modes_for_focus(config.experimental_domains);
    borg_core::modes::register_modes(builtin_modes);

    if let Err(e) = db.abandon_running_agents() {
        tracing::error!("abandon_running_agents failed: {e}");
    }

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

    if let Ok(active) = db.list_active_tasks() {
        let resumable = active
            .iter()
            .filter(|t| !t.session_id.is_empty())
            .filter(|t| {
                matches!(
                    t.status.as_str(),
                    "implement"
                        | "validate"
                        | "review"
                        | "lint_fix"
                        | "rebase"
                        | "spec"
                        | "qa"
                        | "qa_fix"
                        | "impl"
                        | "retry"
                )
            })
            .count();
        if resumable > 0 {
            tracing::info!(
                "restart recovery: {resumable} active pipeline tasks have resumable sessions"
            );
        }
    }

    Ok((db, config))
}

async fn init_sandbox(config: &Config) -> (borg_core::sandbox::SandboxMode, bool) {
    let sandbox_mode = Sandbox::detect(&config.sandbox_backend).await;
    let agent_network_available = if sandbox_mode == borg_core::sandbox::SandboxMode::Docker {
        Sandbox::prune_orphan_containers().await;
        let net_ok = Sandbox::ensure_agent_network().await;
        let _ = Sandbox::ensure_isolated_network().await;
        if net_ok && !Sandbox::install_network_rules().await {
            tracing::warn!(
                "sandbox: iptables rules not installed — agent containers have unrestricted network access"
            );
        }
        net_ok
    } else {
        false
    };
    (sandbox_mode, agent_network_available)
}

fn build_backends(
    config: &Config,
    db: &Db,
    sandbox_mode: borg_core::sandbox::SandboxMode,
) -> anyhow::Result<std::collections::HashMap<String, Arc<dyn borg_core::agent::AgentBackend>>> {
    let mut backends: std::collections::HashMap<String, Arc<dyn borg_core::agent::AgentBackend>> =
        std::collections::HashMap::new();

    backends.insert(
        "claude".into(),
        Arc::new(
            ClaudeBackend::new("claude", sandbox_mode.clone(), &config.container_image)
                .with_timeout(config.agent_timeout_s as u64)
                .with_resource_limits(config.container_memory_mb, config.container_cpus)
                .with_git_author(&config.git_author_name, &config.git_author_email)
                .with_base_url("http://127.0.0.1:3132".to_string()),
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

    if !config.gemini_api_key.is_empty() {
        backends.insert(
            "gemini".into(),
            Arc::new(
                GeminiBackend::new(config.gemini_api_key.clone())
                    .with_timeout(config.agent_timeout_s as u64),
            ),
        );
    }

    if std::env::var("OLLAMA_URL").is_ok() || std::env::var("LOCAL_MODEL").is_ok() {
        let url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into());
        let model = std::env::var("LOCAL_MODEL").unwrap_or_else(|_| "llama3.2".into());
        backends.insert(
            "local".into(),
            Arc::new(OllamaBackend::new(url, model)?.with_timeout(300)?),
        );
        info!("local backend registered (Ollama)");
    }

    Ok(backends)
}

fn spawn_post_state_tasks(state: &Arc<AppState>, config: &Arc<Config>, db: &Arc<Db>) {
    routes::spawn_linked_credential_maintenance(Arc::clone(state));

    let worker_loops: usize = std::env::var("INGEST_WORKER_LOOPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    spawn_ingestion_workers(
        Arc::clone(&state.ingestion_queue),
        Arc::clone(&state.db),
        Arc::clone(&state.file_storage),
        state.search.clone(),
        Arc::clone(&state.embed_registry),
        worker_loops,
    );

    spawn_backup_loop(Arc::clone(db), Arc::clone(config), Arc::clone(&state.file_storage));

    if !config.imap_host.is_empty() {
        let imap_cfg = borg_core::email::ImapConfig {
            host: config.imap_host.clone(),
            port: config.imap_port,
            user: config.imap_user.clone(),
            pass: config.imap_pass.clone(),
            mailbox: config.imap_mailbox.clone(),
        };
        spawn_imap_poller(
            imap_cfg,
            Arc::clone(&state.db),
            Arc::clone(&state.config),
            Arc::clone(&state.file_storage),
            state.search.clone(),
            state.chat_event_tx.clone(),
            Arc::clone(&state.ai_request_count),
        );
    }

    spawn_knowledge_repo_sync(Arc::clone(db), config.data_dir.clone());
}

fn build_cors_layer(config: &Config) -> CorsLayer {
    let mut allowed_origins = vec![
        "http://localhost:5173".to_string(),
        "http://localhost:3131".to_string(),
    ];
    let public_origin = config.get_base_url();
    if !allowed_origins.iter().any(|o| o == &public_origin) {
        allowed_origins.push(public_origin);
    }
    let origins: Vec<HeaderValue> = allowed_origins
        .iter()
        .filter_map(|o| o.parse().ok())
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
            HeaderName::from_static(auth::WORKSPACE_HEADER),
        ]))
        .allow_credentials(true)
}

fn build_app_router(state: Arc<AppState>, dashboard_dir: &str) -> Router {
    let serve_dir = ServeDir::new(dashboard_dir)
        .fallback(tower_http::services::ServeFile::new(format!("{dashboard_dir}/index.html")));
    let cors = build_cors_layer(&state.config);

    Router::new()
        // Email inbound webhook (unauthenticated — verified by api_token param)
        .route(
            "/api/email/inbound",
            post(routes::email_inbound).layer(DefaultBodyLimit::max(25 * 1024 * 1024)),
        )
        // Health (unauthenticated)
        .route("/api/health", get(routes::health))
        // Auth endpoints (unauthenticated)
        .route("/api/auth/token", get(auth::get_token))
        .route("/api/auth/status", get(auth::auth_status))
        .route("/api/auth/setup", post(auth::setup))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/sso/:provider/start", get(auth::sso_start))
        .route("/api/auth/sso/:provider/callback", get(auth::sso_callback))
        .route("/api/auth/me", get(auth::get_me))
        .route("/api/workspaces", get(routes::list_workspaces))
        .route("/api/workspaces", post(routes::create_workspace))
        .route("/api/workspaces/:id/select", put(routes::select_workspace))
        .route(
            "/api/workspaces/:id/members",
            post(routes::add_workspace_member),
        )
        // User management (admin-only, enforced in handlers)
        .route("/api/users", get(routes::list_users))
        .route("/api/users", post(routes::create_user))
        .route("/api/users/:id", delete(routes::delete_user))
        .route("/api/users/:id/password", put(routes::change_password))
        // Per-user settings
        .route("/api/user/settings", get(routes::get_user_settings))
        .route("/api/user/settings", put(routes::put_user_settings))
        .route(
            "/api/user/telegram-bot",
            post(routes::connect_telegram_bot).delete(routes::disconnect_telegram_bot),
        )
        .route(
            "/api/user/discord-bot",
            post(routes::connect_discord_bot).delete(routes::disconnect_discord_bot),
        )
        .route(
            "/api/user/linked-credentials",
            get(routes::list_user_linked_credentials),
        )
        .route(
            "/api/user/linked-credentials/:provider/connect",
            post(routes::start_linked_credential_connect),
        )
        .route(
            "/api/user/linked-credentials/connect/:id",
            get(routes::get_linked_credential_connect_session),
        )
        .route(
            "/api/user/linked-credentials/connect/:id/code",
            post(routes::submit_credential_connect_code),
        )
        .route(
            "/api/user/linked-credentials/:provider",
            delete(routes::delete_user_linked_credential),
        )
        // Tasks
        .route("/api/tasks", get(routes::list_tasks))
        .route("/api/tasks/create", post(routes::create_task))
        .route(
            "/api/tasks/:id",
            get(routes::get_task).patch(routes::patch_task),
        )
        .route("/api/tasks/:id/approve", post(routes::approve_task))
        .route("/api/tasks/:id/reject", post(routes::reject_task))
        .route(
            "/api/tasks/:id/request-revision",
            post(routes::request_revision),
        )
        .route(
            "/api/tasks/:id/revisions",
            get(routes::get_revision_history),
        )
        .route("/api/tasks/:id/citations", get(routes::get_task_citations))
        .route(
            "/api/tasks/:id/verify-citations",
            post(routes::verify_task_citations),
        )
        .route("/api/tasks/:id/retry", post(routes::retry_task))
        .route("/api/tasks/:id/unblock", post(routes::unblock_task))
        .route(
            "/api/tasks/:id/diagnostics",
            get(routes::get_task_diagnostics),
        )
        .route(
            "/api/tasks/retry-all-failed",
            post(routes::retry_all_failed),
        )
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
        .route(
            "/api/projects/:id/files",
            get(routes::list_project_files).delete(routes::delete_all_project_files),
        )
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
        .route(
            "/api/projects/:id",
            get(routes::get_project)
                .put(routes::update_project)
                .delete(routes::delete_project),
        )
        .route("/api/projects/:id/tasks", get(routes::list_project_tasks))
        .route("/api/search", get(routes::search_documents))
        .route("/api/themes", get(routes::summarize_workspace_themes))
        .route(
            "/api/projects/:id/themes",
            get(routes::summarize_project_themes),
        )
        .route("/api/projects/:id/audit", get(routes::list_project_audit))
        .route(
            "/api/projects/:id/documents",
            get(routes::list_project_documents),
        )
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
        .route("/api/mcp/status", get(routes::get_mcp_status))
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
        .route(
            "/api/cloud/:provider/callback",
            get(routes::cloud_auth_callback),
        )
        .route(
            "/api/projects/:id/cloud",
            get(routes::list_cloud_connections),
        )
        .route(
            "/api/projects/:id/cloud/:conn_id",
            delete(routes::delete_cloud_connection),
        )
        .route(
            "/api/projects/:id/cloud/:conn_id/browse",
            get(routes::browse_cloud_files),
        )
        .route(
            "/api/projects/:id/cloud/:conn_id/import",
            post(routes::import_cloud_files),
        )
        // Knowledge base
        .route(
            "/api/knowledge",
            get(routes::list_knowledge).delete(routes::delete_all_knowledge),
        )
        .route("/api/knowledge/templates", get(routes::list_templates))
        .route(
            "/api/knowledge/upload",
            post(routes::upload_knowledge).layer(DefaultBodyLimit::max(55 * 1024 * 1024)),
        )
        .route("/api/knowledge/:id", put(routes::update_knowledge))
        .route("/api/knowledge/:id", delete(routes::delete_knowledge))
        .route(
            "/api/knowledge/:id/content",
            get(routes::get_knowledge_content),
        )
        // User knowledge ("My Knowledge")
        .route(
            "/api/knowledge/my",
            get(routes::list_user_knowledge).delete(routes::delete_all_user_knowledge),
        )
        .route(
            "/api/knowledge/my/upload",
            post(routes::upload_user_knowledge).layer(DefaultBodyLimit::max(55 * 1024 * 1024)),
        )
        .route("/api/knowledge/my/:id", delete(routes::delete_user_knowledge))
        .route(
            "/api/knowledge/my/:id/content",
            get(routes::get_user_knowledge_content),
        )
        // Knowledge repos
        .route(
            "/api/knowledge/repos",
            get(routes::list_knowledge_repos).post(routes::add_knowledge_repo),
        )
        .route(
            "/api/knowledge/repos/:id",
            delete(routes::delete_knowledge_repo_handler),
        )
        .route(
            "/api/knowledge/my/repos",
            get(routes::list_user_knowledge_repos).post(routes::add_user_knowledge_repo),
        )
        .route(
            "/api/knowledge/my/repos/:id",
            delete(routes::delete_user_knowledge_repo_handler),
        )
        // BorgSearch
        .route("/api/borgsearch/facets", get(routes::borgsearch_facets))
        .route("/api/borgsearch/reindex", post(routes::borgsearch_reindex))
        .route("/api/borgsearch/query", get(routes::agent_search))
        .route("/api/borgsearch/file/:id", get(routes::agent_get_file))
        .route("/api/borgsearch/files", get(routes::agent_list_files))
        .route("/api/borgsearch/coverage", get(routes::agent_coverage))
        // Admin / debugging
        .route(
            "/api/admin/conversation",
            get(routes::admin_conversation_dump),
        )
        // Static dashboard
        .fallback_service(serve_dir)
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            instrumentation::request_telemetry_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            auth::workspace_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            auth::auth_middleware,
        ))
        .layer(cors)
        .with_state(state)
}

// ── main ──────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (log_tx, _log_rx) = broadcast::channel::<String>(1024);
    let log_ring: Arc<std::sync::Mutex<VecDeque<String>>> =
        Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(500)));
    let (chat_event_tx, _) = broadcast::channel::<String>(4096);
    init_tracing(log_tx.clone(), Arc::clone(&log_ring));

    let env_config = Config::from_env()?;
    std::fs::create_dir_all(&env_config.data_dir)?;

    let api_token = auth::generate_token();
    write_api_token(&env_config.data_dir, &api_token)?;

    let (db, config) = init_db(&env_config)?;

    let persisted_ai_count = db.get_ts("ai_request_count") as u64;
    let ai_request_count = Arc::new(AtomicU64::new(persisted_ai_count));
    let db = Arc::new(db);
    let config = Arc::new(config);
    let file_storage = Arc::new(storage::FileStorage::from_config(&config).await?);
    let ingestion_queue = Arc::new(ingestion::IngestionQueue::from_config(&config).await?);
    let search = search::SearchClient::from_config(&config).map(Arc::new);

    match &*ingestion_queue {
        ingestion::IngestionQueue::Disabled => info!("ingestion queue backend: disabled"),
        ingestion::IngestionQueue::Sqs { queue_url, .. } => {
            info!("ingestion queue backend: sqs ({queue_url})");
        },
    }

    let (sandbox_mode, agent_network_available) = init_sandbox(&config).await;
    let backends = build_backends(&config, &db, sandbox_mode.clone())?;
    let force_restart = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let (mut pipeline, pipeline_rx) = Pipeline::new(
        Arc::clone(&db),
        backends.clone(),
        Arc::clone(&config),
        sandbox_mode,
        Arc::clone(&force_restart),
        agent_network_available,
        Arc::clone(&ai_request_count),
    );
    pipeline.chat_event_tx = Some(chat_event_tx.clone());
    let pipeline_event_tx = pipeline.event_tx.clone();
    let pipeline = Arc::new(pipeline);

    // Pipeline tick loop — inner spawn catches panics so the loop never dies.
    // If tick panics repeatedly, exit and let systemd restart with a clean process.
    spawn_pipeline_ticker(Arc::clone(&pipeline), config.pipeline_tick_s);

    if !config.telegram_token.is_empty() {
        spawn_telegram_poller(
            config.telegram_token.clone(),
            Arc::clone(&db),
            Arc::clone(&config),
            config.watched_repos.clone(),
            Arc::clone(&file_storage),
            search.clone(),
            chat_event_tx.clone(),
            Arc::clone(&ai_request_count),
        );
    }

    let sidecar_slot: Arc<TokioMutex<Option<Arc<Sidecar>>>> = Arc::new(TokioMutex::new(None));
    spawn_user_bot_manager(
        Arc::clone(&db),
        Arc::clone(&config),
        search.clone(),
        Arc::clone(&file_storage),
        chat_event_tx.clone(),
        Arc::clone(&ai_request_count),
        Arc::clone(&sidecar_slot),
    );

    if let Some(self_repo) = config.watched_repos.iter().find(|r| r.is_self).cloned() {
        spawn_self_repo_watcher(
            self_repo,
            config.remote_check_interval_s as u64,
            Arc::clone(&force_restart),
            config.build_cmd.clone(),
        );
    }

    // Always spawn sidecar — needed for per-user Discord bots even without global tokens
    spawn_sidecar_manager(
        Arc::clone(&config),
        Arc::clone(&db),
        Arc::clone(&file_storage),
        search.clone(),
        chat_event_tx.clone(),
        Arc::clone(&ai_request_count),
        Arc::clone(&sidecar_slot),
    )
    .await;

    if !config.observer_config.is_empty() {
        let observer_api_key =
            std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| config.oauth_token.clone());
        spawn_observer(
            config.observer_config.clone(),
            observer_api_key,
            config.telegram_token.clone(),
        );
    }

    spawn_pipeline_event_forwarder(
        pipeline_rx,
        log_tx.clone(),
        config.telegram_token.clone(),
        chat_event_tx.clone(),
    );

    let stream_manager = Arc::clone(&pipeline.stream_manager);
    let upload_processing_limit = std::env::var("UPLOAD_PROCESSING_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(2);
    let brave_search = if let Ok(Some(key)) = db.get_api_key("global", "brave_search") {
        Some(Arc::new(borg_core::knowledge::BraveSearchClient::new(key)))
    } else {
        None
    };

    let state = Arc::new(AppState {
        db: Arc::clone(&db),
        config: Arc::clone(&config),
        ai_request_count,
        api_token,
        jwt_secret: auth::generate_token(),
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
        embed_registry: Arc::new(borg_core::knowledge::EmbeddingRegistry::from_env()),
        file_storage: Arc::clone(&file_storage),
        ingestion_queue: Arc::clone(&ingestion_queue),
        search: search.clone(),
        brave_search,
        upload_processing_sem: Arc::new(Semaphore::new(upload_processing_limit)),
        upload_processing_limit,
        login_attempts: Arc::new(std::sync::Mutex::new(HashMap::new())),
        linked_credential_sessions: Arc::new(TokioMutex::new(HashMap::new())),
        linked_credential_stdins: Arc::new(TokioMutex::new(HashMap::new())),
    });

    spawn_post_state_tasks(&state, &config, &db);

    let dashboard_dir = config.dashboard_dist_dir.clone();
    let app = build_app_router(Arc::clone(&state), &dashboard_dir);

    let proxy_state = Arc::new(proxy::ProxyState::new(Arc::clone(&db)).await);
    let proxy_app = Router::new().merge(proxy::proxy_routes().with_state(proxy_state));

    let bind = config.web_bind.clone();
    let addr = format!("{bind}:{}", config.web_port);
    let proxy_addr = format!("{bind}:{}", config.proxy_port);

    info!("Listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let proxy_listener = tokio::net::TcpListener::bind(&proxy_addr).await?;
    info!("Proxy listening on {proxy_addr}");

    let server =
        axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>());
    let proxy_server = axum::serve(
        proxy_listener,
        proxy_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    );

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        res = server => { res?; }
        res = proxy_server => { res?; }
        _ = tokio::signal::ctrl_c() => { info!("shutdown signal received (SIGINT)"); }
        _ = sigterm.recv() => { info!("shutdown signal received (SIGTERM)"); }
    }

    if agent_network_available {
        Sandbox::remove_network_rules().await;
        Sandbox::remove_agent_network().await;
    }

    Ok(())
}
