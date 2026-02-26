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

            let entities: &[Value] = msg
                .get("entities")
                .and_then(|e| e.as_array())
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let mentions_bot = check_mentions_bot(&self.bot_username, &text, entities);

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

    pub async fn send_typing(&self, chat_id: i64) -> Result<()> {
        let body = serde_json::json!({
            "chat_id": chat_id,
            "action": "typing",
        });
        self.client
            .post(self.api_url("sendChatAction"))
            .json(&body)
            .send()
            .await?;
        Ok(())
    }
}

/// Returns true if the message text or its Telegram entities indicate the bot
/// was mentioned.
///
/// - Text fallback: checks whether `text` (lowercased) contains `@bot_username`.
/// - Entity path: for each entity with `type == "mention"`, slices the mention
///   text using UTF-16 code-unit `offset`/`length` (as Telegram specifies),
///   strips the leading `@`, and compares case-insensitively to `bot_username`.
pub fn check_mentions_bot(bot_username: &str, text: &str, entities: &[Value]) -> bool {
    let bot_name = format!("@{}", bot_username).to_lowercase();
    if text.to_lowercase().contains(&bot_name) {
        return true;
    }
    if bot_username.is_empty() {
        return false;
    }
    let utf16: Vec<u16> = text.encode_utf16().collect();
    entities
        .iter()
        .filter(|e| e["type"] == "mention")
        .any(|e| {
            let offset = e["offset"].as_u64().unwrap_or(0) as usize;
            let length = e["length"].as_u64().unwrap_or(0) as usize;
            if length == 0 || offset >= utf16.len() {
                return false;
            }
            let end = (offset + length).min(utf16.len());
            let mention = String::from_utf16_lossy(&utf16[offset..end]);
            mention.trim_start_matches('@').to_lowercase() == bot_username.to_lowercase()
        })
}

fn split_text(text: &str, limit: usize) -> Vec<String> {
    if limit == 0 {
        return vec![text.to_string()];
    }
    if text.len() <= limit {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while remaining.len() > limit {
        let safe_limit = remaining.floor_char_boundary(limit);
        let cut = remaining[..safe_limit].rfind('\n').unwrap_or(safe_limit);
        chunks.push(remaining[..cut].to_string());
        remaining = &remaining[cut..];
        remaining = remaining.trim_start_matches('\n');
    }
    if !remaining.is_empty() {
        chunks.push(remaining.to_string());
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::split_text;

    // --- AC5: ASCII-only behaviour ---

    #[test]
    fn split_text_ascii_short() {
        let chunks = split_text("hello world", 4000);
        assert_eq!(chunks, vec!["hello world"]);
    }

    #[test]
    fn split_text_ascii_exact_limit() {
        let s = "a".repeat(4000);
        let chunks = split_text(&s, 4000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 4000);
    }

    #[test]
    fn split_text_ascii_splits_at_newline() {
        // Two 3000-char lines separated by \n → total 6001 bytes; should split at \n.
        let line = "a".repeat(3000);
        let text = format!("{}\n{}", line, line);
        let chunks = split_text(&text, 4000);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], line);
        assert_eq!(chunks[1], line);
    }

    #[test]
    fn split_text_ascii_no_newline_no_loss() {
        // AC3 + AC4 for ASCII: join of chunks must equal original.
        let s = "a".repeat(8000);
        let chunks = split_text(&s, 4000);
        assert_eq!(chunks.len(), 2);
        for c in &chunks {
            assert!(c.len() <= 4000, "chunk len {} > 4000", c.len());
        }
        assert_eq!(chunks.join(""), s);
    }

    // --- Edge case: limit = 0 ---

    #[test]
    fn split_text_limit_zero() {
        let chunks = split_text("hello", 0);
        assert_eq!(chunks, vec!["hello"]);
    }

    // --- Edge case: text just over limit → exactly 2 chunks ---

    #[test]
    fn split_text_just_over_limit() {
        let s = "a".repeat(4001);
        let chunks = split_text(&s, 4000);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 4000);
        assert_eq!(chunks[1].len(), 1);
    }

    // --- Edge case: newline immediately before boundary → clean split ---

    #[test]
    fn split_text_newline_before_boundary() {
        let before = "x".repeat(3999);
        let after = "y".repeat(100);
        let text = format!("{}\n{}", before, after);
        let chunks = split_text(&text, 4000);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], before);
        assert_eq!(chunks[1], after);
    }

    // --- Edge case: consecutive newlines at boundary ---

    #[test]
    fn split_text_consecutive_newlines_at_boundary() {
        // Newlines at bytes 3998 and 3999; rfind picks the later one.
        let before = "a".repeat(3998);
        let after = "b".repeat(100);
        let text = format!("{}\n\n{}", before, after);
        let chunks = split_text(&text, 4000);
        for c in &chunks {
            assert!(c.len() <= 4000, "chunk len {} > 4000", c.len());
        }
        let joined = chunks.join("\n");
        assert!(joined.contains(&before));
        assert!(joined.contains(after.as_str()));
    }

    // === Tests that PANIC with the buggy code ===

    // AC1 + AC7: emoji starting at byte 3998 straddles the 4000-byte boundary.
    // floor_char_boundary(4000) must retreat to 3998.
    // Buggy code: remaining[..4000] panics (byte 4000 is inside the 4-byte emoji).
    #[test]
    fn split_text_emoji_at_boundary() {
        // 3998 ASCII bytes + U+1F600 (4 bytes: starts at 3998, ends at 4002) + 100 ASCII
        let prefix = "a".repeat(3998);
        let emoji = "\u{1F600}";
        let suffix = "b".repeat(100);
        let text = format!("{}{}{}", prefix, emoji, suffix);
        assert_eq!(text.len(), 3998 + 4 + 100);

        let chunks = split_text(&text, 4000);

        // AC3: every chunk within limit
        for c in &chunks {
            assert!(c.len() <= 4000, "chunk len {} > 4000", c.len());
        }
        // AC4: no data loss (no newlines, so direct join)
        assert_eq!(chunks.join(""), text);
        // First chunk ends before the emoji (safe_limit = 3998, no newline → cut = 3998)
        assert_eq!(chunks[0].len(), 3998);
        assert_eq!(chunks[1], format!("{}{}", emoji, suffix));
    }

    // AC2 + AC6: entire string is CJK (3-byte chars); byte 4000 is a continuation byte.
    // "中" = U+4E2D = 3 bytes. 2000 × 3 = 6000 bytes.
    // 4000 = 1333×3 + 1 → byte 4000 is the 2nd byte of char #1334 → PANIC with buggy code.
    #[test]
    fn split_text_multibyte() {
        let text: String = "中".repeat(2000);
        assert_eq!(text.len(), 6000);

        let chunks = split_text(&text, 4000);

        // AC3: every chunk within limit
        for c in &chunks {
            assert!(c.len() <= 4000, "chunk len {} > 4000", c.len());
        }
        // AC4: no data loss (no newlines)
        assert_eq!(chunks.join(""), text);
        // floor_char_boundary(4000) = 1333×3 = 3999 bytes = 1333 chars for first chunk.
        // Remaining: 667 chars = 2001 bytes ≤ 4000 → exactly 2 chunks.
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].chars().count(), 1333);
        assert_eq!(chunks[1].chars().count(), 667);
    }

    // AC1: Arabic text (2-byte chars) with a 1-byte ASCII prefix to misalign the boundary.
    // "ا" (U+0627) = 2 bytes. "a" + "ا"×3000 = 1 + 6000 = 6001 bytes.
    // Byte 4000: (4000-1)=3999 is odd → continuation byte of 2-byte Arabic char → PANIC.
    #[test]
    fn split_text_arabic_no_panic() {
        let text: String = "a".to_string() + &"ا".repeat(3000);
        assert_eq!(text.len(), 6001);

        let chunks = split_text(&text, 4000);

        for c in &chunks {
            assert!(c.len() <= 4000, "chunk len {} > 4000", c.len());
        }
        // AC4: no data loss
        assert_eq!(chunks.join(""), text);
    }

    // AC2: 4-byte emoji with a 3-byte ASCII prefix misaligns the boundary.
    // "aaa" + U+1F600×2000 = 3 + 8000 = 8003 bytes.
    // Byte 4000: (4000-3)=3997; 3997%4=1 → continuation byte → PANIC with buggy code.
    #[test]
    fn split_text_all_emoji_no_newline() {
        let text: String = "aaa".to_string() + &"\u{1F600}".repeat(2000);
        assert_eq!(text.len(), 3 + 8000);

        let chunks = split_text(&text, 4000);

        for c in &chunks {
            assert!(c.len() <= 4000, "chunk len {} > 4000", c.len());
        }
        // AC4: no data loss
        assert_eq!(chunks.join(""), text);
    }

    // AC3 + AC4: mixed CJK and ASCII newlines; verify no data loss via non-newline char count.
    // Each segment: "日"×1000 + "\n" = 3001 bytes. Three segments = 9003 bytes.
    // Byte 4000 falls inside a "日" (3 bytes) in the second segment → PANIC with buggy code.
    #[test]
    fn split_text_mixed_multibyte_newlines_no_loss() {
        let segment = format!("{}\n", "日".repeat(1000));
        let text = segment.repeat(3);
        assert!(text.len() > 4000);

        let chunks = split_text(&text, 4000);

        for c in &chunks {
            assert!(c.len() <= 4000, "chunk len {} > 4000", c.len());
        }
        // Newlines at split boundaries are consumed; count non-newline chars.
        let orig_non_nl: usize = text.chars().filter(|&c| c != '\n').count();
        let chunk_non_nl: usize = chunks
            .iter()
            .flat_map(|c| c.chars())
            .filter(|&c| c != '\n')
            .count();
        assert_eq!(chunk_non_nl, orig_non_nl, "non-newline char count mismatch");
    }
}
