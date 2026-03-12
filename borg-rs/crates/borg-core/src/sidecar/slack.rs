use std::{collections::HashMap, path::PathBuf, sync::Arc};

use anyhow::Result;
use slack_morphism::{
    hyper_tokio::{SlackClientHyperConnector, SlackClientHyperHttpsConnector},
    listener::{SlackClientEventsListenerEnvironment, SlackClientEventsUserState},
    prelude::*,
};
use tokio::sync::{mpsc, Mutex as TokioMutex};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::{attachment, SidecarEvent, SidecarMessage, Source};

type HyperConnector = SlackClientHyperHttpsConnector;

/// Context stored in SlackClientEventsUserState for callback access.
#[derive(Clone)]
struct SlackContext {
    event_tx: mpsc::UnboundedSender<SidecarEvent>,
    assistant_name: String,
    bot_user_id: Option<String>,
    data_dir: PathBuf,
    bot_token_value: String,
    user_id: Option<i64>,
}

pub(crate) struct SlackManager {
    client: Arc<SlackClient<HyperConnector>>,
    token: SlackApiToken,
    user_bots: Arc<TokioMutex<HashMap<i64, SlackUserBot>>>,
    event_tx: mpsc::UnboundedSender<SidecarEvent>,
    assistant_name: String,
    data_dir: PathBuf,
    cancel: CancellationToken,
}

struct SlackUserBot {
    client: Arc<SlackClient<HyperConnector>>,
    token: SlackApiToken,
    cancel: CancellationToken,
}

impl SlackManager {
    pub(crate) async fn start(
        bot_token: &str,
        app_token: &str,
        assistant_name: &str,
        data_dir: PathBuf,
        event_tx: mpsc::UnboundedSender<SidecarEvent>,
        cancel: CancellationToken,
    ) -> Result<Arc<Self>> {
        let connector = SlackClientHyperConnector::new()?;
        let client = Arc::new(SlackClient::new(connector));
        let token = SlackApiToken::new(bot_token.into());

        // Get bot user ID
        let session = client.open_session(&token);
        let auth_resp = session.auth_test().await?;
        let bot_user_id_str = auth_resp.user_id.to_string();
        let bot_name = auth_resp.user.unwrap_or_default();

        info!("Slack connected as @{bot_name}");
        let _ = event_tx.send(SidecarEvent::SlackReady {
            bot_id: bot_user_id_str.clone(),
            bot_name,
        });

        // Clone data_dir before it moves into the manager
        let dd = data_dir.clone();

        let manager = Arc::new(Self {
            client,
            token,
            user_bots: Arc::new(TokioMutex::new(HashMap::new())),
            event_tx,
            assistant_name: assistant_name.to_lowercase(),
            data_dir,
            cancel: cancel.clone(),
        });

        // Start Socket Mode listener
        let mgr = Arc::clone(&manager);
        let app_token_str = app_token.to_string();
        let bot_token_str = bot_token.to_string();
        let bot_uid = bot_user_id_str.clone();
        let asst = assistant_name.to_lowercase();
        let etx = mgr.event_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = run_socket_mode(
                Arc::clone(&mgr.client),
                &app_token_str,
                &bot_token_str,
                &bot_uid,
                &asst,
                dd,
                etx,
                None,
                cancel,
            )
            .await
            {
                warn!("Slack Socket Mode error: {e}");
                let _ = mgr.event_tx.send(SidecarEvent::Error {
                    source: Source::Slack,
                    message: e.to_string(),
                });
            }
        });

        Ok(manager)
    }

    pub(crate) async fn send(
        &self,
        channel_id: &str,
        text: &str,
        reply_to: Option<&str>,
        client: Option<&SlackClient<HyperConnector>>,
        token: Option<&SlackApiToken>,
    ) {
        let client = client.unwrap_or(&self.client);
        let token = token.unwrap_or(&self.token);
        let session = client.open_session(token);

        let chunks = split_text(text, 3000);
        for (i, chunk) in chunks.iter().enumerate() {
            let mut req = SlackApiChatPostMessageRequest::new(
                channel_id.into(),
                SlackMessageContent::new().with_text(chunk.clone()),
            );
            if i == 0 {
                if let Some(ts) = reply_to {
                    req = req.with_thread_ts(ts.into());
                }
            }
            if let Err(e) = session.chat_post_message(&req).await {
                warn!("Slack send error: {e}");
            }
        }
    }

    pub(crate) async fn add_user_bot(
        self: &Arc<Self>,
        user_id: i64,
        bot_token: &str,
        app_token: &str,
    ) {
        self.remove_user_bot(user_id).await;

        let connector = match SlackClientHyperConnector::new() {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to create Slack connector for user {user_id}: {e}");
                return;
            },
        };
        let user_client = Arc::new(SlackClient::new(connector));
        let user_token = SlackApiToken::new(bot_token.into());
        let user_cancel = CancellationToken::new();

        // Verify token
        let session = user_client.open_session(&user_token);
        let bot_uid = match session.auth_test().await {
            Ok(resp) => {
                let bot_id = resp.user_id.to_string();
                let bot_name = resp.user.unwrap_or_default();
                let _ = self.event_tx.send(SidecarEvent::SlackReady {
                    bot_id: bot_id.clone(),
                    bot_name,
                });
                bot_id
            },
            Err(e) => {
                warn!("Slack user bot auth failed for {user_id}: {e}");
                let _ = self.event_tx.send(SidecarEvent::Error {
                    source: Source::Slack,
                    message: e.to_string(),
                });
                return;
            },
        };

        let event_tx = self.event_tx.clone();
        let assistant_name = self.assistant_name.clone();
        let data_dir = self.data_dir.clone();
        let user_client2 = Arc::clone(&user_client);
        let user_cancel2 = user_cancel.clone();
        let app_token_val = app_token.to_string();
        let bot_token_val = bot_token.to_string();

        tokio::spawn(async move {
            if let Err(e) = run_socket_mode(
                user_client2,
                &app_token_val,
                &bot_token_val,
                &bot_uid,
                &assistant_name,
                data_dir,
                event_tx,
                Some(user_id),
                user_cancel2,
            )
            .await
            {
                warn!("User Slack bot Socket Mode error: {e}");
            }
        });

        self.user_bots.lock().await.insert(
            user_id,
            SlackUserBot {
                client: user_client,
                token: user_token,
                cancel: user_cancel,
            },
        );
    }

    pub(crate) async fn remove_user_bot(&self, user_id: i64) {
        if let Some(bot) = self.user_bots.lock().await.remove(&user_id) {
            bot.cancel.cancel();
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
            self.send(
                channel_id,
                text,
                reply_to,
                Some(&bot.client),
                Some(&bot.token),
            )
            .await;
        }
    }

    pub(crate) async fn shutdown(&self) {
        self.cancel.cancel();
        let mut bots = self.user_bots.lock().await;
        for (_, bot) in bots.drain() {
            bot.cancel.cancel();
        }
    }
}

async fn run_socket_mode(
    client: Arc<SlackClient<HyperConnector>>,
    app_token: &str,
    bot_token: &str,
    bot_user_id: &str,
    assistant_name: &str,
    data_dir: PathBuf,
    event_tx: mpsc::UnboundedSender<SidecarEvent>,
    user_id: Option<i64>,
    cancel: CancellationToken,
) -> Result<()> {
    let config = SlackClientSocketModeConfig::new();
    let app_token = SlackApiToken::new(app_token.into());

    let ctx = SlackContext {
        event_tx,
        assistant_name: assistant_name.to_string(),
        bot_user_id: Some(bot_user_id.to_string()),
        data_dir,
        bot_token_value: bot_token.to_string(),
        user_id,
    };

    let listener_env = Arc::new(
        SlackClientEventsListenerEnvironment::new(Arc::clone(&client)).with_user_state(ctx),
    );

    let callbacks = SlackSocketModeListenerCallbacks::new().with_push_events(push_events_handler);

    let listener = SlackClientSocketModeListener::new(&config, listener_env, callbacks);

    tokio::select! {
        result = listener.listen_for(&app_token) => {
            result?;
            // After listen_for, serve (blocks until signal)
            listener.serve().await;
        }
        _ = cancel.cancelled() => {}
    }

    Ok(())
}

async fn push_events_handler(
    event: SlackPushEventCallback,
    _client: Arc<SlackClient<HyperConnector>>,
    states: SlackClientEventsUserState,
) -> UserCallbackResult<()> {
    let ctx: SlackContext = {
        let state = states.read().await;
        match state.get_user_state::<SlackContext>() {
            Some(ctx) => ctx.clone(),
            None => return Ok(()),
        }
    };

    if let SlackEventCallbackBody::Message(msg_event) = event.event {
        handle_slack_message(msg_event, &ctx).await;
    }

    Ok(())
}

async fn handle_slack_message(msg_event: SlackMessageEvent, ctx: &SlackContext) {
    // Skip bot messages and subtypes
    if msg_event.sender.bot_id.is_some() || msg_event.subtype.is_some() {
        return;
    }

    let text = msg_event
        .content
        .as_ref()
        .and_then(|c| c.text.as_ref())
        .map(|t| t.to_string())
        .unwrap_or_default();

    if text.trim().is_empty() {
        return;
    }

    let sender_id = msg_event
        .sender
        .user
        .as_ref()
        .map(|u| u.to_string())
        .unwrap_or_default();

    let channel_id = msg_event
        .origin
        .channel
        .as_ref()
        .map(|c| c.to_string())
        .unwrap_or_default();

    let message_id = msg_event.origin.ts.to_string();

    let is_dm = msg_event
        .origin
        .channel_type
        .as_ref()
        .map(|ct| ct.to_string() == "im")
        .unwrap_or(false);

    let mentions_bot = ctx
        .bot_user_id
        .as_ref()
        .map(|uid| text.contains(&format!("<@{uid}>")))
        .unwrap_or(false)
        || text
            .to_lowercase()
            .contains(&format!("@{}", ctx.assistant_name));

    // Download file attachments if present
    let mut attachments = Vec::new();
    if let Some(ref content) = msg_event.content {
        if let Some(ref files) = content.files {
            let http_client = reqwest::Client::new();
            for file in files {
                if let Some(ref url) = file.url_private_download {
                    let filename = file.name.as_deref().unwrap_or("file");
                    let content_type = file
                        .mimetype
                        .as_ref()
                        .map(|m| m.to_string())
                        .unwrap_or_else(|| "application/octet-stream".to_string());
                    match http_client
                        .get(url.as_str())
                        .header("Authorization", format!("Bearer {}", ctx.bot_token_value))
                        .send()
                        .await
                    {
                        Ok(resp) => match resp.bytes().await {
                            Ok(bytes) => {
                                match attachment::save_bytes(
                                    &bytes,
                                    "slack",
                                    filename,
                                    &content_type,
                                    &ctx.data_dir,
                                )
                                .await
                                {
                                    Ok(sa) => attachments.push(sa),
                                    Err(e) => warn!("Failed to save Slack attachment: {e}"),
                                }
                            },
                            Err(e) => warn!("Failed to download Slack file bytes: {e}"),
                        },
                        Err(e) => warn!("Failed to download Slack file: {e}"),
                    }
                }
            }
        }
    }

    let timestamp = message_id
        .split('.')
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);

    let _ = ctx.event_tx.send(SidecarEvent::Message(SidecarMessage {
        source: Source::Slack,
        id: message_id,
        chat_id: channel_id,
        sender: sender_id.clone(),
        sender_name: sender_id,
        text,
        attachments,
        timestamp,
        is_group: !is_dm,
        mentions_bot,
        user_id: ctx.user_id,
    }));
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
