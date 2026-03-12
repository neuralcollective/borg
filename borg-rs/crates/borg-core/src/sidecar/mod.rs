mod attachment;
mod discord;
mod slack;
mod whatsapp;

use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use self::{discord::DiscordManager, slack::SlackManager, whatsapp::WhatsAppManager};

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Discord,
    WhatsApp,
    Slack,
}

#[derive(Debug, Clone)]
pub struct SidecarAttachment {
    pub url: String,
    pub filename: String,
    pub content_type: String,
}

#[derive(Debug, Clone)]
pub struct SidecarMessage {
    pub source: Source,
    pub id: String,
    pub chat_id: String,
    pub sender: String,
    pub sender_name: String,
    pub text: String,
    pub attachments: Vec<SidecarAttachment>,
    pub timestamp: i64,
    pub is_group: bool,
    pub mentions_bot: bool,
    /// Set for per-user bot messages (e.g. user Discord bots)
    pub user_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub enum SidecarEvent {
    Message(SidecarMessage),
    DiscordReady { bot_id: String },
    SlackReady { bot_id: String, bot_name: String },
    WaConnected { jid: String },
    WaQr { data: String },
    Disconnected { source: Source, reason: String },
    Error { source: Source, message: String },
}

/// Native chat integration manager for Discord, WhatsApp, and Slack.
pub struct Sidecar {
    discord: Option<Arc<DiscordManager>>,
    whatsapp: Option<Arc<WhatsAppManager>>,
    slack: Option<Arc<SlackManager>>,
    cancel: CancellationToken,
}

impl Sidecar {
    pub async fn spawn(
        assistant_name: &str,
        discord_token: &str,
        wa_auth_dir: &str,
        wa_disabled: bool,
        slack_bot_token: &str,
        slack_app_token: &str,
        data_dir: &str,
    ) -> Result<(Self, mpsc::UnboundedReceiver<SidecarEvent>)> {
        let (event_tx, event_rx) = mpsc::unbounded_channel::<SidecarEvent>();
        let cancel = CancellationToken::new();
        let data_dir = PathBuf::from(data_dir);

        let discord = if !discord_token.is_empty() {
            match DiscordManager::start(
                discord_token,
                assistant_name,
                data_dir.clone(),
                event_tx.clone(),
                cancel.clone(),
            )
            .await
            {
                Ok(mgr) => Some(mgr),
                Err(e) => {
                    warn!("Failed to start Discord: {e}");
                    let _ = event_tx.send(SidecarEvent::Error {
                        source: Source::Discord,
                        message: e.to_string(),
                    });
                    None
                },
            }
        } else {
            None
        };

        let whatsapp = if !wa_disabled && !wa_auth_dir.is_empty() {
            match WhatsAppManager::start(
                wa_auth_dir,
                assistant_name,
                data_dir.clone(),
                event_tx.clone(),
                cancel.clone(),
            )
            .await
            {
                Ok(mgr) => Some(mgr),
                Err(e) => {
                    warn!("Failed to start WhatsApp: {e}");
                    let _ = event_tx.send(SidecarEvent::Error {
                        source: Source::WhatsApp,
                        message: e.to_string(),
                    });
                    None
                },
            }
        } else {
            None
        };

        let slack = if !slack_bot_token.is_empty() && !slack_app_token.is_empty() {
            match SlackManager::start(
                slack_bot_token,
                slack_app_token,
                assistant_name,
                data_dir.clone(),
                event_tx.clone(),
                cancel.clone(),
            )
            .await
            {
                Ok(mgr) => Some(mgr),
                Err(e) => {
                    warn!("Failed to start Slack: {e}");
                    let _ = event_tx.send(SidecarEvent::Error {
                        source: Source::Slack,
                        message: e.to_string(),
                    });
                    None
                },
            }
        } else {
            None
        };

        Ok((
            Self {
                discord,
                whatsapp,
                slack,
                cancel,
            },
            event_rx,
        ))
    }

    pub fn send_discord(&self, channel_id: &str, text: &str, reply_to: Option<&str>) {
        if let Some(ref mgr) = self.discord {
            let mgr = Arc::clone(mgr);
            let channel_id = channel_id.to_string();
            let text = text.to_string();
            let reply_to = reply_to.map(|s| s.to_string());
            tokio::spawn(async move {
                mgr.send(&channel_id, &text, reply_to.as_deref(), None)
                    .await;
            });
        }
    }

    pub fn send_whatsapp(&self, jid: &str, text: &str, quote_id: Option<&str>) {
        if let Some(ref mgr) = self.whatsapp {
            let mgr = Arc::clone(mgr);
            let jid = jid.to_string();
            let text = text.to_string();
            let quote_id = quote_id.map(|s| s.to_string());
            tokio::spawn(async move {
                mgr.send(&jid, &text, quote_id.as_deref()).await;
            });
        }
    }

    pub fn send_slack(&self, channel_id: &str, text: &str, reply_to: Option<&str>) {
        if let Some(ref mgr) = self.slack {
            let mgr = Arc::clone(mgr);
            let channel_id = channel_id.to_string();
            let text = text.to_string();
            let reply_to = reply_to.map(|s| s.to_string());
            tokio::spawn(async move {
                mgr.send(&channel_id, &text, reply_to.as_deref(), None, None)
                    .await;
            });
        }
    }

    pub fn send_discord_typing(&self, channel_id: &str) {
        if let Some(ref mgr) = self.discord {
            let mgr = Arc::clone(mgr);
            let channel_id = channel_id.to_string();
            tokio::spawn(async move {
                mgr.send_typing(&channel_id, None).await;
            });
        }
    }

    pub fn send_whatsapp_typing(&self, jid: &str) {
        if let Some(ref mgr) = self.whatsapp {
            let mgr = Arc::clone(mgr);
            let jid = jid.to_string();
            tokio::spawn(async move {
                mgr.send_typing(&jid).await;
            });
        }
    }

    pub fn send_slack_typing(&self, _channel_id: &str) {
        // Slack Socket Mode doesn't support typing indicators
    }

    pub fn logout_whatsapp(&self) {
        if let Some(ref mgr) = self.whatsapp {
            let mgr = Arc::clone(mgr);
            tokio::spawn(async move {
                mgr.logout().await;
            });
        }
    }

    pub fn add_user_discord_bot(&self, user_id: i64, token: &str) {
        if let Some(ref mgr) = self.discord {
            let mgr = Arc::clone(mgr);
            let token = token.to_string();
            tokio::spawn(async move {
                mgr.add_user_bot(user_id, &token).await;
            });
        }
    }

    pub fn remove_user_discord_bot(&self, user_id: i64) {
        if let Some(ref mgr) = self.discord {
            let mgr = Arc::clone(mgr);
            tokio::spawn(async move {
                mgr.remove_user_bot(user_id).await;
            });
        }
    }

    pub fn send_user_discord(
        &self,
        user_id: i64,
        channel_id: &str,
        text: &str,
        reply_to: Option<&str>,
    ) {
        if let Some(ref mgr) = self.discord {
            let mgr = Arc::clone(mgr);
            let channel_id = channel_id.to_string();
            let text = text.to_string();
            let reply_to = reply_to.map(|s| s.to_string());
            tokio::spawn(async move {
                mgr.send_user(user_id, &channel_id, &text, reply_to.as_deref())
                    .await;
            });
        }
    }

    pub fn send_user_discord_typing(&self, user_id: i64, channel_id: &str) {
        if let Some(ref mgr) = self.discord {
            let mgr = Arc::clone(mgr);
            let channel_id = channel_id.to_string();
            tokio::spawn(async move {
                mgr.send_user_typing(user_id, &channel_id).await;
            });
        }
    }

    pub fn add_user_slack_bot(&self, user_id: i64, bot_token: &str, app_token: &str) {
        if let Some(ref mgr) = self.slack {
            let mgr = Arc::clone(mgr);
            let bot_token = bot_token.to_string();
            let app_token = app_token.to_string();
            tokio::spawn(async move {
                mgr.add_user_bot(user_id, &bot_token, &app_token).await;
            });
        }
    }

    pub fn remove_user_slack_bot(&self, user_id: i64) {
        if let Some(ref mgr) = self.slack {
            let mgr = Arc::clone(mgr);
            tokio::spawn(async move {
                mgr.remove_user_bot(user_id).await;
            });
        }
    }

    pub fn send_user_slack(
        &self,
        user_id: i64,
        channel_id: &str,
        text: &str,
        reply_to: Option<&str>,
    ) {
        if let Some(ref mgr) = self.slack {
            let mgr = Arc::clone(mgr);
            let channel_id = channel_id.to_string();
            let text = text.to_string();
            let reply_to = reply_to.map(|s| s.to_string());
            tokio::spawn(async move {
                mgr.send_user(user_id, &channel_id, &text, reply_to.as_deref())
                    .await;
            });
        }
    }

    pub async fn shutdown(&self) {
        self.cancel.cancel();
        if let Some(ref mgr) = self.discord {
            mgr.shutdown().await;
        }
        if let Some(ref mgr) = self.whatsapp {
            mgr.shutdown().await;
        }
        if let Some(ref mgr) = self.slack {
            mgr.shutdown().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_equality() {
        assert_eq!(Source::Discord, Source::Discord);
        assert_ne!(Source::Discord, Source::WhatsApp);
        assert_ne!(Source::WhatsApp, Source::Slack);
    }

    #[test]
    fn sidecar_message_defaults() {
        let msg = SidecarMessage {
            source: Source::Discord,
            id: "m1".to_string(),
            chat_id: "ch1".to_string(),
            sender: "u1".to_string(),
            sender_name: "Alice".to_string(),
            text: "hello".to_string(),
            attachments: vec![],
            timestamp: 1234,
            is_group: true,
            mentions_bot: true,
            user_id: None,
        };
        assert_eq!(msg.source, Source::Discord);
        assert_eq!(msg.chat_id, "ch1");
        assert!(msg.mentions_bot);
    }

    #[test]
    fn sidecar_attachment_fields() {
        let att = SidecarAttachment {
            url: "/tmp/file.png".to_string(),
            filename: "file.png".to_string(),
            content_type: "image/png".to_string(),
        };
        assert_eq!(att.filename, "file.png");
    }

    #[test]
    fn sidecar_event_variants() {
        let ev = SidecarEvent::DiscordReady {
            bot_id: "bot123".to_string(),
        };
        assert!(matches!(ev, SidecarEvent::DiscordReady { .. }));

        let ev = SidecarEvent::WaQr {
            data: "qr-data".to_string(),
        };
        assert!(matches!(ev, SidecarEvent::WaQr { .. }));
    }
}
