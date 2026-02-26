use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;
use std::sync::atomic::{AtomicI64, Ordering};
use tracing::{info, warn};

/// An incoming Telegram message.
#[derive(Debug, Clone)]
pub struct TgMessage {
    pub message_id: i64,
    pub chat_id: i64,
    pub chat_type: String,
    pub chat_title: String,
    pub sender_id: i64,
    pub sender_name: String,
    pub text: String,
    pub date: i64,
    pub mentions_bot: bool,
    pub reply_to_text: Option<String>,
    pub reply_to_author: Option<String>,
}

pub struct Telegram {
    pub token: String,
    pub bot_username: String,
    client: Client,
    last_update_id: AtomicI64,
}

impl Telegram {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            bot_username: String::new(),
            client: Client::new(),
            last_update_id: AtomicI64::new(0),
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.token, method)
    }

    /// Fetch bot info and set bot_username.
    pub async fn connect(&mut self) -> Result<()> {
        let resp: Value = self
            .client
            .get(self.api_url("getMe"))
            .send()
            .await?
            .json()
            .await?;

        if let Some(username) = resp["result"]["username"].as_str() {
            self.bot_username = username.to_string();
            info!("Telegram bot connected: @{}", username);
        }
        Ok(())
    }

    /// Long-poll for new messages with timeout=2s.
    pub async fn get_updates(&self) -> Result<Vec<TgMessage>> {
        let offset = self.last_update_id.load(Ordering::Relaxed) + 1;
        let url = format!(
            "{}?timeout=2&offset={}&allowed_updates=[\"message\"]",
            self.api_url("getUpdates"),
            offset
        );

        let resp: Value = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .context("getUpdates request")?
            .json()
            .await
            .context("getUpdates parse")?;

        let updates = match resp["result"].as_array() {
            Some(a) => a,
            None => return Ok(vec![]),
        };

        let mut messages = Vec::new();

        for update in updates {
            let update_id = update["update_id"].as_i64().unwrap_or(0);
            if update_id > self.last_update_id.load(Ordering::Relaxed) {
                self.last_update_id.store(update_id, Ordering::Relaxed);
            }

            let msg = match update["message"].as_object() {
                Some(m) => m,
                None => continue,
            };

            let text = match msg["text"].as_str() {
                Some(t) => t.to_string(),
                None => continue,
            };

            let chat_id = msg["chat"]["id"].as_i64().unwrap_or(0);
            let chat_type = msg["chat"]["type"]
                .as_str()
                .unwrap_or("private")
                .to_string();
            let chat_title = msg["chat"]["title"]
                .as_str()
                .or_else(|| msg["chat"]["username"].as_str())
                .unwrap_or("")
                .to_string();
            let sender_id = msg["from"]["id"].as_i64().unwrap_or(0);
            let first = msg["from"]["first_name"].as_str().unwrap_or("");
            let last = msg["from"]["last_name"].as_str().unwrap_or("");
            let sender_name = if last.is_empty() {
                first.to_string()
            } else {
                format!("{} {}", first, last)
            };

            let bot_name = format!("@{}", self.bot_username).to_lowercase();
            let text_lower = text.to_lowercase();
            let mentions_bot = text_lower.contains(&bot_name)
                || msg
                    .get("entities")
                    .and_then(|e| e.as_array())
                    .map(|ents| ents.iter().any(|e| e["type"] == "mention"))
                    .unwrap_or(false);

            let reply_to_text = msg
                .get("reply_to_message")
                .and_then(|r| r["text"].as_str())
                .map(|s| s.to_string());
            let reply_to_author = msg
                .get("reply_to_message")
                .and_then(|r| r["from"]["first_name"].as_str())
                .map(|s| s.to_string());

            messages.push(TgMessage {
                message_id: msg["message_id"].as_i64().unwrap_or(0),
                chat_id,
                chat_type,
                chat_title,
                sender_id,
                sender_name,
                text,
                date: msg["date"].as_i64().unwrap_or(0),
                mentions_bot,
                reply_to_text,
                reply_to_author,
            });
        }

        Ok(messages)
    }

    /// Send a text message to a chat.
    pub async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        reply_to: Option<i64>,
    ) -> Result<()> {
        for chunk in split_text(text, 4000) {
            let mut body = serde_json::json!({
                "chat_id": chat_id,
                "text": chunk,
                "parse_mode": "Markdown",
            });
            if let Some(id) = reply_to {
                body["reply_to_message_id"] = serde_json::json!(id);
            }

            let resp: Value = self
                .client
                .post(self.api_url("sendMessage"))
                .json(&body)
                .send()
                .await?
                .json()
                .await?;

            if resp["ok"] != true {
                warn!("Telegram sendMessage failed: {:?}", resp["description"]);
            }
        }
        Ok(())
    }
}

fn split_text(text: &str, limit: usize) -> Vec<String> {
    if text.len() <= limit {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while remaining.len() > limit {
        let cut = remaining[..limit].rfind('\n').unwrap_or(limit);
        chunks.push(remaining[..cut].to_string());
        remaining = remaining[cut..].trim_start_matches('\n');
    }
    if !remaining.is_empty() {
        chunks.push(remaining.to_string());
    }
    chunks
}
