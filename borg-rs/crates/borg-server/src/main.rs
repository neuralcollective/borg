use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        Json,
    },
    routing::{delete, get, post, put},
    Router,
};
use borg_agent::claude::ClaudeBackend;
use borg_agent::codex::CodexBackend;
use borg_agent::ollama::OllamaBackend;
use borg_core::{
    chat::ChatCollector,
    config::Config,
    db::{Db, TaskMessage, TaskOutput},
    modes::all_modes,
    observer::Observer,
    pipeline::{Pipeline, PipelineEvent},
    sandbox::Sandbox,
    sidecar::{Sidecar, SidecarEvent, Source},
    types::Task,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, Mutex as TokioMutex};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tower_http::{cors::CorsLayer, services::ServeDir};
use tracing::info;

// ── AppState ──────────────────────────────────────────────────────────────

pub struct AppState {
    pub db: Arc<Db>,
    pub start_time: Instant,
    pub log_tx: broadcast::Sender<String>,
    pub pipeline_event_tx: broadcast::Sender<PipelineEvent>,
    pub backends: std::collections::HashMap<String, Arc<dyn borg_core::agent::AgentBackend>>,
}

impl AppState {
    fn default_backend(&self, name: &str) -> Arc<dyn borg_core::agent::AgentBackend> {
        self.backends
            .get(name)
            .or_else(|| self.backends.values().next())
            .map(Arc::clone)
            .expect("no backends configured")
    }
}

// ── Error helper ──────────────────────────────────────────────────────────

fn internal(e: impl std::fmt::Display) -> StatusCode {
    tracing::error!("internal error: {e}");
    StatusCode::INTERNAL_SERVER_ERROR
}

// ── Request body types ────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateTaskBody {
    title: String,
    description: Option<String>,
    mode: Option<String>,
    repo: Option<String>,
}

#[derive(Deserialize)]
struct CreateMessageBody {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct FocusBody {
    text: String,
}

#[derive(Deserialize)]
struct RepoQuery {
    repo: Option<String>,
}

#[derive(Deserialize)]
struct TasksQuery {
    repo: Option<String>,
}

// ── Serializable wrappers ─────────────────────────────────────────────────

#[derive(Serialize)]
struct TaskOutputJson {
    id: i64,
    task_id: i64,
    phase: String,
    output: String,
    exit_code: i64,
    created_at: String,
}

impl From<TaskOutput> for TaskOutputJson {
    fn from(o: TaskOutput) -> Self {
        Self {
            id: o.id,
            task_id: o.task_id,
            phase: o.phase,
            output: o.output,
            exit_code: o.exit_code,
            created_at: o.created_at.to_rfc3339(),
        }
    }
}

#[derive(Serialize)]
struct TaskMessageJson {
    id: i64,
    task_id: i64,
    role: String,
    content: String,
    created_at: String,
    delivered_phase: Option<String>,
}

impl From<TaskMessage> for TaskMessageJson {
    fn from(m: TaskMessage) -> Self {
        Self {
            id: m.id,
            task_id: m.task_id,
            role: m.role,
            content: m.content,
            created_at: m.created_at.to_rfc3339(),
            delivered_phase: m.delivered_phase,
        }
    }
}

// ── BroadcastLayer: forwards tracing events to the SSE /api/logs stream ──

struct BroadcastLayer {
    tx: broadcast::Sender<String>,
}

struct MessageVisitor<'a> {
    message: &'a mut String,
}

impl tracing::field::Visit for MessageVisitor<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            *self.message = value.to_string();
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message.clear();
            use std::fmt::Write;
            let _ = write!(self.message, "{value:?}");
            // Strip surrounding quotes added by Debug on &str
            if self.message.starts_with('"') && self.message.ends_with('"') {
                *self.message = self.message[1..self.message.len() - 1].to_string();
            }
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for BroadcastLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let level = match *event.metadata().level() {
            tracing::Level::ERROR => "err",
            tracing::Level::WARN => "warn",
            tracing::Level::INFO => "info",
            tracing::Level::DEBUG => "debug",
            tracing::Level::TRACE => return,
        };

        let target = event.metadata().target();
        let category = if target.contains("pipeline") {
            "pipeline"
        } else if target.contains("agent") || target.contains("claude") || target.contains("codex") {
            "agent"
        } else {
            "system"
        };

        let mut message = String::new();
        event.record(&mut MessageVisitor { message: &mut message });

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let json = serde_json::json!({
            "ts": ts,
            "level": level,
            "message": message,
            "category": category,
        })
        .to_string();

        let _ = self.tx.send(json);
    }
}

// ── Chat agent ────────────────────────────────────────────────────────────

/// Sanitize a chat key into a safe filesystem component.
fn sanitize_chat_key(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .collect()
}

/// Run claude as a conversational chat agent with session continuity.
/// `sessions` maps chat_key → claude session_id for resume.
async fn run_chat_agent(
    chat_key: &str,
    sender_name: &str,
    messages: &[String],
    sessions: &Arc<TokioMutex<HashMap<String, String>>>,
    config: &Config,
) -> anyhow::Result<String> {
    let session_dir = format!("{}/sessions/chat-{}", config.data_dir, sanitize_chat_key(chat_key));
    std::fs::create_dir_all(&session_dir)?;

    let prompt = if messages.len() == 1 {
        format!("{} says: {}", sender_name, messages[0])
    } else {
        let joined: Vec<String> = messages.iter().map(|m| format!("- {m}")).collect();
        format!("{} says:\n{}", sender_name, joined.join("\n"))
    };

    let mut args = vec![
        "--model".to_string(),
        config.model.clone(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--allowedTools".to_string(),
        "none".to_string(),
        "--max-turns".to_string(),
        "10".to_string(),
    ];

    let session_id = sessions.lock().await.get(chat_key).cloned();
    if let Some(ref sid) = session_id {
        args.push("--resume".to_string());
        args.push(sid.clone());
    }

    args.push("--print".to_string());
    args.push(prompt);

    let out = tokio::process::Command::new("claude")
        .args(&args)
        .current_dir(&session_dir)
        .env("HOME", &session_dir)
        .env("ANTHROPIC_API_KEY", &config.oauth_token)
        .env("CLAUDE_CODE_OAUTH_TOKEN", &config.oauth_token)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .await?;

    let raw = String::from_utf8_lossy(&out.stdout).into_owned();
    let (text, new_session_id) = borg_agent::event::parse_stream(&raw);

    if let Some(sid) = new_session_id {
        sessions.lock().await.insert(chat_key.to_string(), sid);
    }

    Ok(text)
}

// ── main ──────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let (log_tx, _log_rx) = broadcast::channel::<String>(1024);

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "borg_server=info,borg_core=info,borg_agent=info,tower_http=warn".into());

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(BroadcastLayer { tx: log_tx.clone() })
        .init();

    let config = Config::from_env()?;

    std::fs::create_dir_all(&config.data_dir)?;
    let db_path = format!("{}/borg.db", config.data_dir);
    let mut db = Db::open(&db_path)?;
    db.migrate()?;

    // Upsert repos from config into DB
    for repo in &config.watched_repos {
        let name = std::path::Path::new(&repo.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&repo.path);
        db.upsert_repo(
            &repo.path,
            name,
            &repo.mode,
            &repo.test_cmd,
            &repo.prompt_file,
            repo.auto_merge,
            None,
        )
        .ok();
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
        backends.insert(
            "codex".into(),
            Arc::new(CodexBackend::new(config.codex_api_key.clone(), "gpt-5.3-codex")),
        );
    }
    // Local model via Ollama — enabled by setting OLLAMA_URL or LOCAL_MODEL
    if std::env::var("OLLAMA_URL").is_ok() || std::env::var("LOCAL_MODEL").is_ok() {
        let url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into());
        let model = std::env::var("LOCAL_MODEL").unwrap_or_else(|_| "llama3.2".into());
        backends.insert("local".into(), Arc::new(OllamaBackend::new(url, model).with_timeout(300)));
        info!("local backend registered (Ollama)");
    }

    let (pipeline, pipeline_rx) = Pipeline::new(Arc::clone(&db), backends.clone(), Arc::clone(&config));
    let pipeline_event_tx = pipeline.event_tx.clone();
    let pipeline = Arc::new(pipeline);

    // Spawn pipeline tick loop
    let tick_secs = config.pipeline_tick_s;
    {
        let pipeline = Arc::clone(&pipeline);
        tokio::spawn(async move {
            loop {
                if let Err(e) = Arc::clone(&pipeline).tick().await {
                    tracing::error!("Pipeline tick error: {e}");
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
        let tg_sessions: Arc<TokioMutex<HashMap<String, String>>> =
            Arc::new(TokioMutex::new(HashMap::new()));
        tokio::spawn(async move {
            let mut tg = borg_core::telegram::Telegram::new(token);
            if let Err(e) = tg.connect().await {
                tracing::warn!("Telegram connect failed: {e}");
                return;
            }
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
                                text.splitn(2, ' ')
                                    .nth(1)
                                    .unwrap_or("")
                                    .trim()
                                    .to_string()
                            } else {
                                text
                            };
                            let text_lower = text.to_lowercase();

                            if text_lower.starts_with("task:") || text_lower.starts_with("task ") {
                                let title_part = text[5..].trim().to_string();
                                let (title, desc) = if let Some(nl) = title_part.find('\n') {
                                    (title_part[..nl].to_string(), title_part[nl + 1..].to_string())
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
                                match db_tg.insert_task(&task) {
                                    Ok(id) => {
                                        let reply = format!("Task #{id} created: {task_title}");
                                        let _ = tg
                                            .send_message(msg.chat_id, &reply, Some(msg.message_id))
                                            .await;
                                    }
                                    Err(e) => tracing::error!("insert_task from telegram: {e}"),
                                }
                            } else {
                                // Run chat agent
                                let chat_key = format!("telegram:{}", msg.chat_id);
                                let _ = tg.send_typing(msg.chat_id).await;
                                match run_chat_agent(
                                    &chat_key,
                                    &msg.sender_name,
                                    &[text],
                                    &tg_sessions,
                                    &config_tg,
                                )
                                .await
                                {
                                    Ok(reply) if !reply.is_empty() => {
                                        let _ = tg
                                            .send_message(
                                                msg.chat_id,
                                                &reply,
                                                Some(msg.message_id),
                                            )
                                            .await;
                                    }
                                    Ok(_) => {}
                                    Err(e) => tracing::warn!("Telegram chat agent error: {e}"),
                                }
                            }
                        }
                    }
                    Err(e) => tracing::warn!("Telegram poll error: {e}"),
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
        });
    }

    // Self-update detection loop
    if let Some(self_repo) = config.watched_repos.iter().find(|r| r.is_self).cloned() {
        let check_interval = config.remote_check_interval_s as u64;
        tokio::spawn(async move {
            let git = borg_core::git::Git::new(&self_repo.path);
            let mut last_head = git.rev_parse_head().unwrap_or_default();

            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(check_interval)).await;

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
                last_head = remote_head;
                tracing::info!("Self-update: rebuilding...");
                let build = tokio::process::Command::new("cargo")
                    .args(["build", "--release"])
                    .current_dir(&self_repo.path)
                    .status()
                    .await;
                match build {
                    Ok(s) if s.success() => {
                        tracing::info!("Self-update: build done, restarting");
                        let bin = format!("{}/target/release/borg-server", self_repo.path);
                        use std::os::unix::process::CommandExt;
                        let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
                        let err = std::process::Command::new(&bin).args(&args[1..]).exec();
                        tracing::error!("execve failed: {err}");
                    }
                    Ok(_) => tracing::error!("Self-update: cargo build failed"),
                    Err(e) => tracing::error!("Self-update: failed to run cargo: {e}"),
                }
            }
        });
    }

    // Sidecar (Discord + WhatsApp bridge)
    if !config.discord_token.is_empty() || !config.wa_auth_dir.is_empty() {
        let config_sc = Arc::clone(&config);
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
                ));

                // Flush expired collection windows periodically
                let collector_flush = Arc::clone(&collector);
                let sidecar_flush = Arc::clone(&sidecar);
                let sessions_flush = Arc::clone(&sc_sessions);
                let config_flush = Arc::clone(&config_sc);
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                        for batch in collector_flush.flush_expired().await {
                            let sidecar2 = Arc::clone(&sidecar_flush);
                            let sessions2 = Arc::clone(&sessions_flush);
                            let config2 = Arc::clone(&config_flush);
                            let collector2 = Arc::clone(&collector_flush);
                            // chat_key format: "discord:<channel_id>" or "whatsapp:<jid>"
                            let is_discord = batch.chat_key.starts_with("discord:");
                            let chat_id = batch.chat_key
                                .splitn(2, ':')
                                .nth(1)
                                .unwrap_or("")
                                .to_string();
                            let sender_name = batch.messages.first().cloned().unwrap_or_default();
                            tokio::spawn(async move {
                                match run_chat_agent(
                                    &batch.chat_key,
                                    &sender_name,
                                    &batch.messages,
                                    &sessions2,
                                    &config2,
                                )
                                .await
                                {
                                    Ok(reply) if !reply.is_empty() => {
                                        if is_discord {
                                            sidecar2.send_discord(&chat_id, &reply, None);
                                        } else {
                                            sidecar2.send_whatsapp(&chat_id, &reply, None);
                                        }
                                    }
                                    Ok(_) => {}
                                    Err(e) => tracing::warn!("Sidecar flush agent error: {e}"),
                                }
                                collector2.mark_done(&batch.chat_key).await;
                            });
                        }
                    }
                });

                // Process incoming sidecar events
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
                        let prefix = if msg.source == Source::Discord { "discord" } else { "whatsapp" };
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
                                match run_chat_agent(
                                    &batch.chat_key,
                                    &sender_name,
                                    &batch.messages,
                                    &sessions2,
                                    &config2,
                                )
                                .await
                                {
                                    Ok(reply) if !reply.is_empty() => {
                                        if is_discord {
                                            sidecar2.send_discord(&chat_id, &reply, Some(&msg_id));
                                        } else {
                                            sidecar2.send_whatsapp(&chat_id, &reply, Some(&msg_id));
                                        }
                                    }
                                    Ok(_) => {}
                                    Err(e) => tracing::warn!("Sidecar chat agent error: {e}"),
                                }
                                collector2.mark_done(&batch.chat_key).await;
                            });
                        }
                    }
                });
            }
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

    // Integration queue processor
    {
        let db_iq = Arc::clone(&db);
        let config_iq = Arc::clone(&config);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
                let entries = match db_iq.list_queue() {
                    Ok(e) => e,
                    Err(e) => { tracing::error!("list_queue: {e}"); continue; }
                };
                for entry in entries {
                    let auto_merge = config_iq
                        .watched_repos
                        .iter()
                        .find(|r| r.path == entry.repo_path)
                        .map(|r| r.auto_merge)
                        .unwrap_or(false);

                    let wt_path = format!("{}/.worktrees/task-{}", entry.repo_path, entry.task_id);
                    let git = borg_core::git::Git::new(&entry.repo_path);

                    if let Err(e) = git.push_branch(&wt_path, &entry.branch) {
                        tracing::warn!("push_branch task #{}: {e}", entry.task_id);
                        db_iq.update_queue_status(entry.id, "failed").ok();
                        continue;
                    }

                    let pr_out = tokio::process::Command::new("gh")
                        .args([
                            "pr", "create",
                            "--base", "main",
                            "--head", &entry.branch,
                            "--title", &format!("task-{}", entry.task_id),
                            "--body", &format!("Automated task #{}", entry.task_id),
                        ])
                        .current_dir(&entry.repo_path)
                        .output()
                        .await;

                    match pr_out {
                        Err(e) => {
                            tracing::warn!("gh pr create task #{}: {e}", entry.task_id);
                            db_iq.update_queue_status(entry.id, "failed").ok();
                        }
                        Ok(out) if !out.status.success() => {
                            let stderr = String::from_utf8_lossy(&out.stderr);
                            tracing::warn!("gh pr create failed task #{}: {stderr}", entry.task_id);
                            db_iq.update_queue_status(entry.id, "failed").ok();
                        }
                        Ok(out) => {
                            let pr_url = String::from_utf8_lossy(&out.stdout).trim().to_string();
                            tracing::info!("task #{} PR: {pr_url}", entry.task_id);

                            if auto_merge {
                                let merge_out = tokio::process::Command::new("gh")
                                    .args(["pr", "merge", "--squash", "--auto", &pr_url])
                                    .current_dir(&entry.repo_path)
                                    .output()
                                    .await;
                                if let Ok(m) = merge_out {
                                    if m.status.success() {
                                        tracing::info!("task #{} auto-merge queued", entry.task_id);
                                        db_iq.update_task_status(entry.task_id, "merged", None).ok();
                                    } else {
                                        let stderr = String::from_utf8_lossy(&m.stderr);
                                        tracing::warn!("gh pr merge failed: {stderr}");
                                    }
                                }
                            }
                            db_iq.update_queue_status(entry.id, "merging").ok();
                        }
                    }
                }
            }
        });
    }

    // Forward pipeline events to the SSE log stream
    {
        let log_tx_fwd = log_tx.clone();
        tokio::spawn(async move {
            let mut rx = pipeline_rx;
            loop {
                match rx.recv().await {
                    Ok(evt) => {
                        let data = serde_json::json!({
                            "type": evt.kind,
                            "task_id": evt.task_id,
                            "message": evt.message,
                        })
                        .to_string();
                        let _ = log_tx_fwd.send(data);
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
        });
    }

    let state = Arc::new(AppState {
        db,
        start_time: Instant::now(),
        log_tx,
        pipeline_event_tx,
        backends,
    });

    let dashboard_dir = config
        .dashboard_dist_dir
        .clone();

    let serve_dir = ServeDir::new(&dashboard_dir)
        .fallback(tower_http::services::ServeFile::new(
            format!("{dashboard_dir}/index.html"),
        ));

    let app = Router::new()
        // Health
        .route("/api/health", get(health))
        // Tasks
        .route("/api/tasks", get(list_tasks))
        .route("/api/tasks/create", post(create_task))
        .route("/api/tasks/:id", get(get_task))
        .route("/api/tasks/:id/retry", post(retry_task))
        // Task messages
        .route("/api/tasks/:id/messages", get(get_task_messages))
        .route("/api/tasks/:id/messages", post(post_task_message))
        // Queue
        .route("/api/queue", get(list_queue))
        // Status
        .route("/api/status", get(get_status))
        // Proposals
        .route("/api/proposals", get(list_proposals))
        .route("/api/proposals/triage", post(triage_proposals))
        .route("/api/proposals/:id/approve", post(approve_proposal))
        .route("/api/proposals/:id/dismiss", post(dismiss_proposal))
        .route("/api/proposals/:id/reopen", post(reopen_proposal))
        // Modes
        .route("/api/modes", get(get_modes))
        // Settings
        .route("/api/settings", get(get_settings))
        .route("/api/settings", put(put_settings))
        // Focus
        .route("/api/focus", get(get_focus))
        .route("/api/focus", post(post_focus))
        .route("/api/focus", delete(delete_focus))
        // SSE logs
        .route("/api/logs", get(sse_logs))
        // Backend overrides
        .route("/api/tasks/:id/backend", put(put_task_backend))
        .route("/api/repos", get(list_repos_handler))
        .route("/api/repos/:id/backend", put(put_repo_backend))
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

// ── Handlers ──────────────────────────────────────────────────────────────

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

// Tasks

async fn list_tasks(
    State(state): State<Arc<AppState>>,
    Query(q): Query<TasksQuery>,
) -> Result<Json<Value>, StatusCode> {
    let tasks = state
        .db
        .list_all_tasks(q.repo.as_deref())
        .map_err(internal)?;
    Ok(Json(json!(tasks)))
}

async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    match state.db.get_task_with_outputs(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some((task, outputs)) => {
            let outputs_json: Vec<TaskOutputJson> =
                outputs.into_iter().map(TaskOutputJson::from).collect();
            let mut v = serde_json::to_value(&task).map_err(internal)?;
            if let Some(obj) = v.as_object_mut() {
                obj.insert(
                    "outputs".into(),
                    serde_json::to_value(outputs_json).map_err(internal)?,
                );
            }
            Ok(Json(v))
        }
    }
}

async fn create_task(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateTaskBody>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let repo = body
        .repo
        .or_else(|| std::env::var("PIPELINE_REPO").ok())
        .unwrap_or_default();
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
    };
    let id = state.db.insert_task(&task).map_err(internal)?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

async fn retry_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    match state.db.get_task(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some(_) => {
            state
                .db
                .update_task_status(id, "backlog", None)
                .map_err(internal)?;
            Ok(StatusCode::OK)
        }
    }
}

// Task messages

async fn get_task_messages(
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
        }
    }
}

async fn post_task_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<CreateMessageBody>,
) -> Result<StatusCode, StatusCode> {
    match state.db.get_task(id).map_err(internal)? {
        None => Err(StatusCode::NOT_FOUND),
        Some(_) => {
            state
                .db
                .insert_task_message(id, &body.role, &body.content)
                .map_err(internal)?;
            let _ = state.pipeline_event_tx.send(PipelineEvent {
                kind: "task_message".into(),
                task_id: Some(id),
                message: body.content.clone(),
            });
            Ok(StatusCode::CREATED)
        }
    }
}

// Queue

async fn list_queue(State(state): State<Arc<AppState>>) -> Result<Json<Value>, StatusCode> {
    let entries = state.db.list_queue().map_err(internal)?;
    Ok(Json(json!(entries)))
}

// Status

async fn get_status(State(state): State<Arc<AppState>>) -> Result<Json<Value>, StatusCode> {
    let uptime_s = state.start_time.elapsed().as_secs();

    let repos = state.db.list_repos().map_err(internal)?;
    let primary_repo = std::env::var("PIPELINE_REPO").unwrap_or_default();
    let watched_repos: Vec<Value> = repos
        .iter()
        .map(|r| {
            json!({
                "path": r.path,
                "test_cmd": r.test_cmd,
                "is_self": r.path == primary_repo,
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
        "failed_tasks": failed,
        "total_tasks": total,
        "dispatched_agents": 0,
    })))
}

// Proposals

async fn list_proposals(
    State(state): State<Arc<AppState>>,
    Query(q): Query<RepoQuery>,
) -> Result<Json<Value>, StatusCode> {
    let proposals = state
        .db
        .list_all_proposals(q.repo.as_deref())
        .map_err(internal)?;
    Ok(Json(json!(proposals)))
}

async fn approve_proposal(
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
        session_id: String::new(),
        mode: "sweborg".into(),
        backend: String::new(),
    };
    let task_id = state.db.insert_task(&task).map_err(internal)?;
    Ok(Json(json!({ "task_id": task_id })))
}

async fn dismiss_proposal(
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
        }
    }
}

async fn reopen_proposal(
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
        }
    }
}

async fn triage_proposals(
    State(state): State<Arc<AppState>>,
) -> Json<Value> {
    let proposals = match state.db.list_untriaged_proposals() {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("list_untriaged_proposals: {e}");
            return Json(json!({ "scored": 0 }));
        }
    };
    let count = proposals.len();
    if count == 0 {
        return Json(json!({ "scored": 0 }));
    }

    let db = Arc::clone(&state.db);
    let backend = state.default_backend("claude");
    let model = db.get_config("model").ok().flatten().unwrap_or_else(|| "claude-sonnet-4-6".into());
    let oauth = std::env::var("ANTHROPIC_API_KEY")
        .or_else(|_| std::env::var("CLAUDE_CODE_OAUTH_TOKEN"))
        .unwrap_or_default();

    tokio::spawn(async move {
        for proposal in proposals {
            let prompt = format!(
                "Score this software proposal as JSON.\n\nTitle: {}\nDescription: {}\nRationale: {}\n\nRespond ONLY with valid JSON:\n{{\"score\":0-100,\"impact\":0-100,\"feasibility\":0-100,\"risk\":0-100,\"effort\":0-100,\"reasoning\":\"...\"}}",
                proposal.title, proposal.description, proposal.rationale
            );

            let task = borg_core::types::Task {
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
                session_id: String::new(),
                mode: "sweborg".into(),
                backend: String::new(),
            };

            let phase = borg_core::types::PhaseConfig {
                name: "triage".into(),
                label: "Triage".into(),
                instruction: prompt,
                fresh_session: true,
                allowed_tools: String::new(),
                ..Default::default()
            };

            let ctx = borg_core::types::PhaseContext {
                task: task.clone(),
                repo_config: borg_core::types::RepoConfig {
                    path: proposal.repo_path.clone(),
                    test_cmd: String::new(),
                    prompt_file: String::new(),
                    mode: "sweborg".into(),
                    is_self: false,
                    auto_merge: false,
                    lint_cmd: String::new(),
                    backend: String::new(),
                },
                session_dir: format!("store/sessions/triage-{}", proposal.id),
                worktree_path: proposal.repo_path.clone(),
                oauth_token: oauth.clone(),
                model: model.clone(),
                pending_messages: Vec::new(),
            };

            std::fs::create_dir_all(&ctx.session_dir).ok();

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
                                    proposal.id, score, impact, feasibility, risk, effort, &reasoning,
                                ) {
                                    tracing::error!("update_proposal_triage #{}: {e}", proposal.id);
                                } else {
                                    tracing::info!("triaged proposal #{}: score={score}", proposal.id);
                                }
                            }
                        }
                    }
                }
                Err(e) => tracing::error!("triage agent for proposal #{}: {e}", proposal.id),
            }
        }
    });

    Json(json!({ "scored": count }))
}

// Modes

async fn get_modes() -> Json<Value> {
    let modes: Vec<Value> = all_modes()
        .into_iter()
        .map(|m| {
            let phases: Vec<Value> = m
                .phases
                .iter()
                .map(|p| {
                    json!({
                        "name": p.name,
                        "label": p.label,
                        "priority": p.priority,
                    })
                })
                .collect();
            json!({
                "name": m.name,
                "label": m.label,
                "phases": phases,
            })
        })
        .collect();
    Json(json!(modes))
}

// Settings

const SETTINGS_KEYS: &[&str] = &[
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
    "proposal_promote_threshold",
    "backend",
];

const SETTINGS_DEFAULTS: &[(&str, &str)] = &[
    ("continuous_mode", "false"),
    ("release_interval_mins", "180"),
    ("pipeline_max_backlog", "5"),
    ("agent_timeout_s", "600"),
    ("pipeline_seed_cooldown_s", "3600"),
    ("pipeline_tick_s", "30"),
    ("model", "claude-sonnet-4-6"),
    ("container_memory_mb", "2048"),
    ("assistant_name", "Borg"),
    ("pipeline_max_agents", "3"),
    ("proposal_promote_threshold", "70"),
    ("backend", "claude"),
];

async fn get_settings(State(state): State<Arc<AppState>>) -> Result<Json<Value>, StatusCode> {
    let mut obj = serde_json::Map::new();
    for key in SETTINGS_KEYS {
        let val = state.db.get_config(key).map_err(internal)?;
        let default = SETTINGS_DEFAULTS
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| *v)
            .unwrap_or("");
        let s = val.as_deref().unwrap_or(default);
        let json_val = if *key == "continuous_mode" {
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
                | "proposal_promote_threshold"
        ) {
            s.parse::<i64>().map(|n| json!(n)).unwrap_or(json!(s))
        } else {
            json!(s)
        };
        obj.insert(key.to_string(), json_val);
    }
    Ok(Json(Value::Object(obj)))
}

async fn put_settings(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
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

// Focus

async fn get_focus(State(state): State<Arc<AppState>>) -> Result<Json<Value>, StatusCode> {
    let text = state
        .db
        .get_config("focus")
        .map_err(internal)?
        .unwrap_or_default();
    let active = !text.is_empty();
    Ok(Json(json!({ "text": text, "active": active })))
}

async fn post_focus(
    State(state): State<Arc<AppState>>,
    Json(body): Json<FocusBody>,
) -> Result<StatusCode, StatusCode> {
    state.db.set_config("focus", &body.text).map_err(internal)?;
    Ok(StatusCode::OK)
}

async fn delete_focus(State(state): State<Arc<AppState>>) -> Result<StatusCode, StatusCode> {
    state.db.set_config("focus", "").map_err(internal)?;
    Ok(StatusCode::OK)
}

// SSE logs

async fn sse_logs(
    State(state): State<Arc<AppState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = state.log_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| {
        msg.ok().map(|data| Ok(Event::default().data(data)))
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    )
}

// Backend override handlers

async fn put_task_backend(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<Value>, StatusCode> {
    let backend = body["backend"].as_str().unwrap_or("").to_string();
    state.db.update_task_backend(id, &backend).map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

async fn list_repos_handler(
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
            })
        })
        .collect();
    Ok(Json(json!(arr)))
}

async fn put_repo_backend(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<Value>, StatusCode> {
    let backend = body["backend"].as_str().unwrap_or("").to_string();
    state.db.update_repo_backend(id, &backend).map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}
