use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
};

use anyhow::Result;
use serenity::{
    all::{
        ChannelId, Context, CreateMessage, EventHandler, GatewayIntents, Message, Ready,
    },
    async_trait, Client,
};
use tokio::sync::{mpsc, Mutex as TokioMutex};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::{
    attachment, SidecarEvent, SidecarMessage, Source,
};

pub(crate) struct DiscordManager {
    http: Arc<serenity::http::Http>,
    shard_manager: Arc<serenity::gateway::ShardManager>,
    user_bots: Arc<TokioMutex<HashMap<i64, DiscordUserBot>>>,
    event_tx: mpsc::UnboundedSender<SidecarEvent>,
    assistant_name: String,
    data_dir: PathBuf,
    cancel: CancellationToken,
}

struct DiscordUserBot {
    http: Arc<serenity::http::Http>,
    shard_manager: Arc<serenity::gateway::ShardManager>,
}

struct BorgEventHandler {
    event_tx: mpsc::UnboundedSender<SidecarEvent>,
    assistant_name: String,
    data_dir: PathBuf,
    user_id: Option<i64>,
}

#[async_trait]
impl EventHandler for BorgEventHandler {
    async fn message(&self, _ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }
        if msg.content.is_empty() && msg.attachments.is_empty() {
            return;
        }

        let bot_user = _ctx.cache.current_user().clone();
        let mentions_bot = msg.mentions_user(&bot_user)
            || msg
                .content
                .to_lowercase()
                .contains(&format!("@{}", self.assistant_name));

        let mut attachments = Vec::new();
        let http_client = reqwest::Client::new();
        for att in &msg.attachments {
            let content_type = att
                .content_type
                .as_deref()
                .unwrap_or("application/octet-stream");
            match att.download().await {
                Ok(bytes) => {
                    match attachment::save_bytes(
                        &bytes,
                        "discord",
                        &att.filename,
                        content_type,
                        &self.data_dir,
                    )
                    .await
                    {
                        Ok(sa) => attachments.push(sa),
                        Err(e) => warn!("Failed to save Discord attachment: {e}"),
                    }
                }
                Err(e) => warn!("Failed to download Discord attachment: {e}"),
            }
        }
        let _ = http_client; // reqwest client used by attachment::download_and_save if needed

        let sidecar_msg = SidecarMessage {
            source: Source::Discord,
            id: msg.id.to_string(),
            chat_id: msg.channel_id.to_string(),
            sender: msg.author.id.to_string(),
            sender_name: msg
                .author_nick(&_ctx.http)
                .await
                .unwrap_or_else(|| msg.author.name.clone()),
            text: msg.content.clone(),
            attachments,
            timestamp: msg.timestamp.unix_timestamp(),
            is_group: msg.guild_id.is_some(),
            mentions_bot,
            user_id: self.user_id,
        };

        let _ = self.event_tx.send(SidecarEvent::Message(sidecar_msg));
    }

    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!("Discord connected as {}", ready.user.name);
        let _ = self.event_tx.send(SidecarEvent::DiscordReady {
            bot_id: ready.user.id.to_string(),
        });
    }
}

impl DiscordManager {
    pub(crate) async fn start(
        token: &str,
        assistant_name: &str,
        data_dir: PathBuf,
        event_tx: mpsc::UnboundedSender<SidecarEvent>,
        cancel: CancellationToken,
    ) -> Result<Arc<Self>> {
        let intents = GatewayIntents::GUILDS
            | GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::DIRECT_MESSAGES;

        let handler = BorgEventHandler {
            event_tx: event_tx.clone(),
            assistant_name: assistant_name.to_lowercase(),
            data_dir: data_dir.clone(),
            user_id: None,
        };

        let mut client = Client::builder(token, intents)
            .event_handler(handler)
            .await?;

        let http = client.http.clone();
        let shard_manager = client.shard_manager.clone();

        let cancel2 = cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                result = client.start() => {
                    if let Err(e) = result {
                        warn!("Discord client error: {e}");
                    }
                }
                _ = cancel2.cancelled() => {
                    client.shard_manager.shutdown_all().await;
                }
            }
        });

        let manager = Arc::new(Self {
            http,
            shard_manager,
            user_bots: Arc::new(TokioMutex::new(HashMap::new())),
            event_tx,
            assistant_name: assistant_name.to_lowercase(),
            data_dir,
            cancel,
        });

        Ok(manager)
    }

    pub(crate) async fn send(
        &self,
        channel_id: &str,
        text: &str,
        reply_to: Option<&str>,
        http: Option<&serenity::http::Http>,
    ) {
        let http = http.unwrap_or(&self.http);
        let Ok(cid) = channel_id.parse::<u64>() else {
            warn!("Invalid Discord channel_id: {channel_id}");
            return;
        };
        let channel = ChannelId::new(cid);

        let chunks = split_text(text, 2000);
        for (i, chunk) in chunks.iter().enumerate() {
            let mut msg = CreateMessage::new().content(chunk);
            if i == 0 {
                if let Some(ref_id) = reply_to {
                    if let Ok(mid) = ref_id.parse::<u64>() {
                        msg = msg.reference_message((channel, serenity::all::MessageId::new(mid)));
                    }
                }
            }
            if let Err(e) = channel.send_message(http, msg).await {
                warn!("Discord send error: {e}");
            }
        }
    }

    pub(crate) async fn send_typing(&self, channel_id: &str, http: Option<&serenity::http::Http>) {
        let http = http.unwrap_or(&self.http);
        let Ok(cid) = channel_id.parse::<u64>() else { return };
        let _ = ChannelId::new(cid).broadcast_typing(http).await;
    }

    pub(crate) async fn add_user_bot(&self, user_id: i64, token: &str) {
        self.remove_user_bot(user_id).await;

        let intents = GatewayIntents::GUILDS
            | GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::DIRECT_MESSAGES;

        let handler = BorgEventHandler {
            event_tx: self.event_tx.clone(),
            assistant_name: self.assistant_name.clone(),
            data_dir: self.data_dir.clone(),
            user_id: Some(user_id),
        };

        let client_result = Client::builder(token, intents)
            .event_handler(handler)
            .await;

        let mut client = match client_result {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to create user Discord bot {user_id}: {e}");
                let _ = self.event_tx.send(SidecarEvent::Error {
                    source: Source::Discord,
                    message: e.to_string(),
                });
                return;
            }
        };

        let http = client.http.clone();
        let shard_manager = client.shard_manager.clone();

        let cancel = self.cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                result = client.start() => {
                    if let Err(e) = result {
                        warn!("User Discord bot error: {e}");
                    }
                }
                _ = cancel.cancelled() => {
                    client.shard_manager.shutdown_all().await;
                }
            }
        });

        self.user_bots.lock().await.insert(
            user_id,
            DiscordUserBot {
                http,
                shard_manager,
            },
        );
    }

    pub(crate) async fn remove_user_bot(&self, user_id: i64) {
        if let Some(bot) = self.user_bots.lock().await.remove(&user_id) {
            bot.shard_manager.shutdown_all().await;
            let _ = self.event_tx.send(SidecarEvent::Error {
                source: Source::Discord,
                message: format!("User bot {user_id} removed"),
            });
        }
    }

    pub(crate) async fn send_user(
        &self,
        user_id: i64,
        channel_id: &str,
        text: &str,
        reply_to: Option<&str>,
    ) {
        let bots = self.user_bots.lock().await;
        if let Some(bot) = bots.get(&user_id) {
            self.send(channel_id, text, reply_to, Some(&bot.http)).await;
        }
    }

    pub(crate) async fn send_user_typing(&self, user_id: i64, channel_id: &str) {
        let bots = self.user_bots.lock().await;
        if let Some(bot) = bots.get(&user_id) {
            self.send_typing(channel_id, Some(&bot.http)).await;
        }
    }

    pub(crate) async fn shutdown(&self) {
        self.cancel.cancel();
        self.shard_manager.shutdown_all().await;
        let mut bots = self.user_bots.lock().await;
        for (_, bot) in bots.drain() {
            bot.shard_manager.shutdown_all().await;
        }
    }
}

fn split_text(text: &str, limit: usize) -> Vec<String> {
    if text.len() <= limit {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while remaining.len() > limit {
        let cut = remaining[..limit]
            .rfind('\n')
            .filter(|&pos| pos > 0)
            .unwrap_or(limit);
        chunks.push(remaining[..cut].to_string());
        remaining = remaining[cut..].trim_start_matches('\n');
    }
    if !remaining.is_empty() {
        chunks.push(remaining.to_string());
    }
    chunks
}
