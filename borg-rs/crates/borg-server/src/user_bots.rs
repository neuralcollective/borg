use std::{
    collections::HashMap,
    sync::{atomic::AtomicU64, Arc},
};

use borg_core::{config::Config, db::Db, sidecar::Sidecar, telegram::Telegram};
use tokio::{
    sync::{broadcast, Mutex as TokioMutex},
    task::JoinHandle,
};
use tracing::{info, warn};

use crate::{
    messaging_progress::{new_chat_run_id, spawn_chat_progress_forwarder, MessagingProgressSink},
    routes,
    search::SearchClient,
    storage::FileStorage,
};

struct RunningBot {
    token_hash: u64,
    handle: JoinHandle<()>,
}

struct RunningDiscordBot {
    token_hash: u64,
}

struct RunningSlackBot {
    token_hash: u64,
}

/// Manages per-user Telegram bot polling loops and Discord/Slack bots (via sidecar).
pub struct UserBotManager {
    bots: TokioMutex<HashMap<i64, RunningBot>>,
    discord_bots: TokioMutex<HashMap<i64, RunningDiscordBot>>,
    slack_bots: TokioMutex<HashMap<i64, RunningSlackBot>>,
    db: Arc<Db>,
    config: Arc<Config>,
    search: Option<Arc<SearchClient>>,
    storage: Arc<FileStorage>,
    chat_event_tx: broadcast::Sender<String>,
    ai_request_count: Arc<AtomicU64>,
    sidecar_slot: Arc<TokioMutex<Option<Arc<Sidecar>>>>,
}

fn hash_token(token: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut h);
    h.finish()
}

impl UserBotManager {
    pub fn new(
        db: Arc<Db>,
        config: Arc<Config>,
        search: Option<Arc<SearchClient>>,
        storage: Arc<FileStorage>,
        chat_event_tx: broadcast::Sender<String>,
        ai_request_count: Arc<AtomicU64>,
        sidecar_slot: Arc<TokioMutex<Option<Arc<Sidecar>>>>,
    ) -> Self {
        Self {
            bots: TokioMutex::new(HashMap::new()),
            discord_bots: TokioMutex::new(HashMap::new()),
            slack_bots: TokioMutex::new(HashMap::new()),
            db,
            config,
            search,
            storage,
            chat_event_tx,
            ai_request_count,
            sidecar_slot,
        }
    }

    /// Scan user settings and start/stop bot loops as needed.
    pub async fn sync(&self) {
        let all_users = match self.db.list_users() {
            Ok(u) => u,
            Err(e) => {
                warn!("user_bots sync: list_users failed: {e}");
                return;
            },
        };

        // Telegram bots
        let mut desired_tg: HashMap<i64, String> = HashMap::new();
        let mut desired_dc: HashMap<i64, String> = HashMap::new();
        let mut desired_slack: HashMap<i64, (String, String)> = HashMap::new();
        for (user_id, _, _, _, _) in &all_users {
            if let Ok(Some(token)) = self.db.get_user_setting(*user_id, "telegram_bot_token") {
                if !token.is_empty() {
                    desired_tg.insert(*user_id, token);
                }
            }
            if let Ok(Some(token)) = self.db.get_user_setting(*user_id, "discord_bot_token") {
                if !token.is_empty() {
                    desired_dc.insert(*user_id, token);
                }
            }
            if let (Ok(Some(bot_token)), Ok(Some(app_token))) = (
                self.db.get_user_setting(*user_id, "slack_bot_token"),
                self.db.get_user_setting(*user_id, "slack_app_token"),
            ) {
                if !bot_token.is_empty() && !app_token.is_empty() {
                    desired_slack.insert(*user_id, (bot_token, app_token));
                }
            }
        }

        // Sync Telegram bots
        {
            let mut bots = self.bots.lock().await;

            let to_remove: Vec<i64> = bots
                .keys()
                .filter(|uid| {
                    desired_tg
                        .get(uid)
                        .map(|t| hash_token(t) != bots[uid].token_hash)
                        .unwrap_or(true)
                })
                .copied()
                .collect();
            for uid in to_remove {
                if let Some(bot) = bots.remove(&uid) {
                    info!(user_id = uid, "stopping user telegram bot");
                    bot.handle.abort();
                }
            }

            for (user_id, token) in &desired_tg {
                let th = hash_token(token);
                if bots.get(user_id).map(|b| b.token_hash == th).unwrap_or(false) {
                    continue;
                }
                let handle = self.spawn_telegram_bot(*user_id, token.clone()).await;
                if let Some(handle) = handle {
                    bots.insert(*user_id, RunningBot { token_hash: th, handle });
                }
            }
        }

        // Sync Discord bots via sidecar
        {
            let sidecar_guard = self.sidecar_slot.lock().await;
            let Some(sidecar) = sidecar_guard.as_ref() else {
                return;
            };

            let mut dc_bots = self.discord_bots.lock().await;

            let to_remove: Vec<i64> = dc_bots
                .keys()
                .filter(|uid| {
                    desired_dc
                        .get(uid)
                        .map(|t| hash_token(t) != dc_bots[uid].token_hash)
                        .unwrap_or(true)
                })
                .copied()
                .collect();
            for uid in to_remove {
                if dc_bots.remove(&uid).is_some() {
                    info!(user_id = uid, "stopping user discord bot");
                    sidecar.remove_user_discord_bot(uid);
                }
            }

            for (user_id, token) in &desired_dc {
                let th = hash_token(token);
                if dc_bots.get(user_id).map(|b| b.token_hash == th).unwrap_or(false) {
                    continue;
                }
                info!(user_id, "starting user discord bot");
                sidecar.add_user_discord_bot(*user_id, token);
                dc_bots.insert(*user_id, RunningDiscordBot { token_hash: th });
            }

            // Sync Slack bots via sidecar
            let mut slack_bots = self.slack_bots.lock().await;

            let to_remove: Vec<i64> = slack_bots
                .keys()
                .filter(|uid| {
                    desired_slack
                        .get(uid)
                        .map(|(bt, at)| {
                            hash_token(&format!("{bt}{at}")) != slack_bots[uid].token_hash
                        })
                        .unwrap_or(true)
                })
                .copied()
                .collect();
            for uid in to_remove {
                if slack_bots.remove(&uid).is_some() {
                    info!(user_id = uid, "stopping user slack bot");
                    sidecar.remove_user_slack_bot(uid);
                }
            }

            for (user_id, (bot_token, app_token)) in &desired_slack {
                let th = hash_token(&format!("{bot_token}{app_token}"));
                if slack_bots
                    .get(user_id)
                    .map(|b| b.token_hash == th)
                    .unwrap_or(false)
                {
                    continue;
                }
                info!(user_id, "starting user slack bot");
                sidecar.add_user_slack_bot(*user_id, bot_token, app_token);
                slack_bots.insert(*user_id, RunningSlackBot { token_hash: th });
            }
        }
    }

    async fn spawn_telegram_bot(&self, user_id: i64, token: String) -> Option<JoinHandle<()>> {
        let mut tg = Telegram::new(&token);
        if let Err(e) = tg.connect().await {
            warn!(user_id, "user telegram bot connect failed: {e}");
            return None;
        }
        let bot_username = tg.bot_username.clone();
        info!(user_id, bot = %bot_username, "starting user telegram bot");

        let tg = Arc::new(tg);
        let db = Arc::clone(&self.db);
        let config = Arc::clone(&self.config);
        let search = self.search.clone();
        let storage = Arc::clone(&self.storage);
        let chat_event_tx = self.chat_event_tx.clone();
        let ai_request_count = Arc::clone(&self.ai_request_count);
        let sessions: Arc<TokioMutex<HashMap<String, String>>> =
            Arc::new(TokioMutex::new(HashMap::new()));

        let handle = tokio::spawn(async move {
            poll_loop(
                user_id,
                tg,
                db,
                config,
                search,
                storage,
                chat_event_tx,
                ai_request_count,
                sessions,
            )
            .await;
        });

        Some(handle)
    }

    /// Run the sync loop forever (call from a spawned task).
    pub async fn run(self: Arc<Self>) {
        // Initial sync
        self.sync().await;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            self.sync().await;
        }
    }
}

async fn poll_loop(
    user_id: i64,
    tg: Arc<Telegram>,
    db: Arc<Db>,
    config: Arc<Config>,
    search: Option<Arc<SearchClient>>,
    storage: Arc<FileStorage>,
    chat_event_tx: broadcast::Sender<String>,
    ai_request_count: Arc<AtomicU64>,
    sessions: Arc<TokioMutex<HashMap<String, String>>>,
) {
    let repos = config.watched_repos.clone();
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
                        handle_task_message(&msg, &text, &repos, &db, &tg).await;
                    } else {
                        let chat_key = format!("tg:u{}:{}", user_id, msg.chat_id);
                        let _ = tg.send_typing(msg.chat_id).await;
                        let tg2 = Arc::clone(&tg);
                        let sessions2 = Arc::clone(&sessions);
                        let config2 = Arc::clone(&config);
                        let db2 = Arc::clone(&db);
                        let search2 = search.clone();
                        let storage2 = Arc::clone(&storage);
                        let chat_tx2 = chat_event_tx.clone();
                        let ai_count2 = Arc::clone(&ai_request_count);
                        let sender_name = msg.sender_name.clone();
                        let chat_id = msg.chat_id;
                        let message_id = msg.message_id;
                        let files = msg.files.clone();
                        tokio::spawn(async move {
                            let run_id = new_chat_run_id();

                            // Build messages: text + downloaded attachments
                            let mut agent_messages: Vec<String> = Vec::new();
                            if !text.is_empty() {
                                agent_messages.push(text);
                            }
                            for file in &files {
                                match tg2.download_file(&file.file_id).await {
                                    Ok((bytes, dl_name)) => {
                                        let att_dir = format!(
                                            "{}/attachments/{}",
                                            config2.data_dir, file.file_id
                                        );
                                        std::fs::create_dir_all(&att_dir).ok();
                                        let save_name = if !file.filename.is_empty() {
                                            file.filename.clone()
                                        } else {
                                            dl_name
                                        };
                                        let save_path =
                                            format!("{}/{}", att_dir, save_name);
                                        if std::fs::write(&save_path, &bytes).is_ok() {
                                            let size_kb = bytes.len() / 1024;
                                            agent_messages.push(format!(
                                                "[Attached file: {} ({}KB)] Path: {}",
                                                save_name, size_kb, save_path
                                            ));
                                        }
                                    },
                                    Err(e) => {
                                        warn!(user_id, "download file {}: {e}", file.file_id)
                                    },
                                }
                            }

                            if agent_messages.is_empty() {
                                return;
                            }

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
                                &agent_messages,
                                &sessions2,
                                &config2,
                                &db2,
                                search2,
                                &storage2,
                                &chat_tx2,
                                &ai_count2,
                                None,
                                None,
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
                                    warn!(user_id, "user bot chat agent error: {e}");
                                },
                            }
                        });
                    }
                }
            },
            Err(e) => warn!(user_id, "user bot poll error: {e}"),
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }
}

async fn handle_task_message(
    msg: &borg_core::telegram::TgMessage,
    text: &str,
    repos: &[borg_core::types::RepoConfig],
    db: &Arc<Db>,
    tg: &Arc<Telegram>,
) {
    use chrono::Utc;
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
    let task = borg_core::types::Task {
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
    match db.insert_task(&task) {
        Ok(id) => {
            let reply = format!("Task #{id} created: {task_title}");
            let _ = tg.send_message(msg.chat_id, &reply, Some(msg.message_id)).await;
        },
        Err(e) => tracing::error!("insert_task from user telegram bot: {e}"),
    }
}
