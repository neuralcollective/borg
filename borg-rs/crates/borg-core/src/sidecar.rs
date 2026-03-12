use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::mpsc,
};
use tracing::{info, warn};

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

/// Client for the unified Discord+WhatsApp bridge.js sidecar process.
/// Uses a shared cmd_tx that is replaced on restart, so callers hold a stable Arc<Sidecar>.
pub struct Sidecar {
    cmd_tx: Arc<Mutex<Option<mpsc::UnboundedSender<String>>>>,
}

fn sidecar_bridge_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../sidecar/bridge.js")
}

impl Sidecar {
    /// Spawn `bun sidecar/bridge.js` with automatic restart on exit.
    /// Returns `(Arc<Sidecar>, event_rx)` where event_rx is a persistent channel
    /// that receives events from all sidecar lifetimes.
    pub async fn spawn(
        assistant_name: &str,
        discord_token: &str,
        wa_auth_dir: &str,
        wa_disabled: bool,
        slack_bot_token: &str,
        slack_app_token: &str,
    ) -> Result<(Self, mpsc::UnboundedReceiver<SidecarEvent>)> {
        let cmd_tx_arc: Arc<Mutex<Option<mpsc::UnboundedSender<String>>>> =
            Arc::new(Mutex::new(None));
        let (event_tx, event_rx) = mpsc::unbounded_channel::<SidecarEvent>();

        let cmd_tx_arc2 = Arc::clone(&cmd_tx_arc);
        let event_tx2 = event_tx.clone();
        let assistant_name = assistant_name.to_string();
        let discord_token = discord_token.to_string();
        let wa_auth_dir = wa_auth_dir.to_string();
        let slack_bot_token = slack_bot_token.to_string();
        let slack_app_token = slack_app_token.to_string();

        tokio::spawn(async move {
            let mut attempt = 0u32;
            loop {
                match spawn_once(
                    &assistant_name,
                    &discord_token,
                    &wa_auth_dir,
                    wa_disabled,
                    &slack_bot_token,
                    &slack_app_token,
                    event_tx2.clone(),
                )
                .await
                {
                    Ok((cmd_tx, died_rx)) => {
                        attempt = 0;
                        *cmd_tx_arc2.lock().unwrap_or_else(|e| e.into_inner()) = Some(cmd_tx);
                        died_rx.await.ok();
                        *cmd_tx_arc2.lock().unwrap_or_else(|e| e.into_inner()) = None;
                        warn!("Sidecar process exited, restarting in 5s");
                    },
                    Err(e) => {
                        attempt += 1;
                        let delay = (5u64 * attempt as u64).min(60);
                        warn!(
                            "Sidecar spawn failed (attempt {attempt}): {e}, retrying in {delay}s"
                        );
                        tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                        continue;
                    },
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        });

        Ok((Self { cmd_tx: cmd_tx_arc }, event_rx))
    }

    pub fn send_discord(&self, channel_id: &str, text: &str, reply_to: Option<&str>) {
        let mut obj = serde_json::json!({
            "target": "discord", "cmd": "send",
            "channel_id": channel_id, "text": text,
        });
        if let Some(id) = reply_to {
            obj["reply_to"] = serde_json::Value::String(id.to_string());
        }
        self.send_raw(obj.to_string());
    }

    pub fn send_whatsapp(&self, jid: &str, text: &str, quote_id: Option<&str>) {
        let mut obj = serde_json::json!({
            "target": "whatsapp", "cmd": "send",
            "jid": jid, "text": text,
        });
        if let Some(id) = quote_id {
            obj["quote_id"] = serde_json::Value::String(id.to_string());
        }
        self.send_raw(obj.to_string());
    }

    pub fn send_discord_typing(&self, channel_id: &str) {
        let cmd =
            serde_json::json!({"target": "discord", "cmd": "typing", "channel_id": channel_id});
        self.send_raw(cmd.to_string());
    }

    pub fn send_whatsapp_typing(&self, jid: &str) {
        let cmd = serde_json::json!({"target": "whatsapp", "cmd": "typing", "jid": jid});
        self.send_raw(cmd.to_string());
    }

    pub fn send_slack(&self, channel_id: &str, text: &str, reply_to: Option<&str>) {
        let mut obj = serde_json::json!({
            "target": "slack", "cmd": "send",
            "channel_id": channel_id, "text": text,
        });
        if let Some(ts) = reply_to {
            obj["reply_to"] = serde_json::Value::String(ts.to_string());
        }
        self.send_raw(obj.to_string());
    }

    pub fn send_slack_typing(&self, _channel_id: &str) {
        // Slack Socket Mode doesn't support typing indicators
    }

    pub fn add_user_discord_bot(&self, user_id: i64, token: &str) {
        let cmd = serde_json::json!({
            "target": "discord", "cmd": "add_user_bot",
            "user_id": user_id, "token": token,
        });
        self.send_raw(cmd.to_string());
    }

    pub fn remove_user_discord_bot(&self, user_id: i64) {
        let cmd = serde_json::json!({
            "target": "discord", "cmd": "remove_user_bot",
            "user_id": user_id,
        });
        self.send_raw(cmd.to_string());
    }

    pub fn send_user_discord(
        &self,
        user_id: i64,
        channel_id: &str,
        text: &str,
        reply_to: Option<&str>,
    ) {
        let mut obj = serde_json::json!({
            "target": "discord", "cmd": "send",
            "user_id": user_id, "channel_id": channel_id, "text": text,
        });
        if let Some(id) = reply_to {
            obj["reply_to"] = serde_json::Value::String(id.to_string());
        }
        self.send_raw(obj.to_string());
    }

    pub fn send_user_discord_typing(&self, user_id: i64, channel_id: &str) {
        let cmd = serde_json::json!({
            "target": "discord", "cmd": "typing",
            "user_id": user_id, "channel_id": channel_id,
        });
        self.send_raw(cmd.to_string());
    }

    fn send_raw(&self, cmd: String) {
        if let Ok(guard) = self.cmd_tx.lock() {
            if let Some(tx) = guard.as_ref() {
                tx.send(cmd).ok();
            }
        }
    }
}

/// Spawn one sidecar process. Returns (cmd_tx, died_rx).
/// `event_tx` receives all parsed events. `died_rx` fires when the process exits.
async fn spawn_once(
    assistant_name: &str,
    discord_token: &str,
    wa_auth_dir: &str,
    wa_disabled: bool,
    slack_bot_token: &str,
    slack_app_token: &str,
    event_tx: mpsc::UnboundedSender<SidecarEvent>,
) -> Result<(
    mpsc::UnboundedSender<String>,
    tokio::sync::oneshot::Receiver<()>,
)> {
    let mut cmd = Command::new("bun");
    let bridge_path = sidecar_bridge_path();
    cmd.arg(&bridge_path);
    cmd.arg(assistant_name);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::inherit());
    if !discord_token.is_empty() {
        cmd.env("DISCORD_TOKEN", discord_token);
    }
    if !wa_auth_dir.is_empty() {
        cmd.env("WA_AUTH_DIR", wa_auth_dir);
    }
    if wa_disabled {
        cmd.env("WA_DISABLED", "true");
    }
    if !slack_bot_token.is_empty() {
        cmd.env("SLACK_BOT_TOKEN", slack_bot_token);
    }
    if !slack_app_token.is_empty() {
        cmd.env("SLACK_APP_TOKEN", slack_app_token);
    }

    let mut child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn {}", bridge_path.display()))?;
    let stdin = child.stdin.take().context("sidecar stdin unavailable")?;
    let stdout = child.stdout.take().context("sidecar stdout unavailable")?;

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<String>();
    let (died_tx, died_rx) = tokio::sync::oneshot::channel::<()>();

    // Stdout reader → events
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.is_empty() {
                continue;
            }
            let Some(event) = parse_event(&line) else {
                continue;
            };
            match &event {
                SidecarEvent::DiscordReady { bot_id } => {
                    info!("Discord connected as bot {}", bot_id);
                },
                SidecarEvent::SlackReady { bot_name, .. } => {
                    info!("Slack connected as @{}", bot_name);
                },
                SidecarEvent::WaConnected { jid } => {
                    info!("WhatsApp connected as {}", jid);
                },
                SidecarEvent::WaQr { .. } => {
                    info!("WhatsApp QR code generated - scan with phone");
                },
                SidecarEvent::Disconnected { source, reason } => {
                    warn!("Sidecar {:?} disconnected: {}", source, reason);
                },
                SidecarEvent::Error { source, message } => {
                    warn!("Sidecar {:?} error: {}", source, message);
                },
                _ => {},
            }
            let _ = event_tx.send(event);
        }
    });

    // Stdin writer ← commands
    tokio::spawn(async move {
        let mut stdin = stdin;
        while let Some(line) = cmd_rx.recv().await {
            let payload = format!("{line}\n");
            if let Err(e) = stdin.write_all(payload.as_bytes()).await {
                warn!("Sidecar stdin write error: {}", e);
                break;
            }
            let _ = stdin.flush().await;
        }
    });

    // Monitor process exit; fire died_tx when done
    tokio::spawn(async move {
        let _ = child.wait().await;
        let _ = died_tx.send(());
    });

    info!("Sidecar process started");
    Ok((cmd_tx, died_rx))
}

fn parse_source(s: &str) -> Option<Source> {
    match s {
        "discord" => Some(Source::Discord),
        "whatsapp" => Some(Source::WhatsApp),
        "slack" => Some(Source::Slack),
        _ => None,
    }
}

fn parse_event(line: &str) -> Option<SidecarEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    let source_str = v["source"].as_str()?;
    let event_type = v["event"].as_str()?;
    let source = parse_source(source_str)?;

    let user_id = v["user_id"].as_i64();

    let ev = match event_type {
        "message" => {
            let attachments = v["attachments"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|a| {
                            let url = a["url"].as_str()?.to_string();
                            Some(SidecarAttachment {
                                url,
                                filename: a["filename"].as_str().unwrap_or("file").to_string(),
                                content_type: a["content_type"]
                                    .as_str()
                                    .unwrap_or("application/octet-stream")
                                    .to_string(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let msg = match source {
                Source::Discord => SidecarMessage {
                    source: Source::Discord,
                    id: str_val(&v, "message_id"),
                    chat_id: str_val(&v, "channel_id"),
                    sender: str_val(&v, "sender_id"),
                    sender_name: str_val(&v, "sender_name"),
                    text: str_val(&v, "text"),
                    attachments,
                    timestamp: v["timestamp"].as_i64().unwrap_or(0),
                    is_group: !v["is_dm"].as_bool().unwrap_or(false),
                    mentions_bot: v["mentions_bot"].as_bool().unwrap_or(false),
                    user_id,
                },
                Source::Slack => SidecarMessage {
                    source: Source::Slack,
                    id: str_val(&v, "message_id"),
                    chat_id: str_val(&v, "channel_id"),
                    sender: str_val(&v, "sender_id"),
                    sender_name: str_val(&v, "sender_name"),
                    text: str_val(&v, "text"),
                    attachments: vec![],
                    timestamp: v["timestamp"].as_i64().unwrap_or(0),
                    is_group: !v["is_dm"].as_bool().unwrap_or(false),
                    mentions_bot: v["mentions_bot"].as_bool().unwrap_or(false),
                    user_id: None,
                },
                Source::WhatsApp => SidecarMessage {
                    source: Source::WhatsApp,
                    id: str_val(&v, "id"),
                    chat_id: str_val(&v, "jid"),
                    sender: str_val(&v, "sender"),
                    sender_name: str_val(&v, "sender_name"),
                    text: str_val(&v, "text"),
                    attachments: vec![],
                    timestamp: v["timestamp"].as_i64().unwrap_or(0),
                    is_group: v["is_group"].as_bool().unwrap_or(false),
                    mentions_bot: v["mentions_bot"].as_bool().unwrap_or(false),
                    user_id: None,
                },
            };
            SidecarEvent::Message(msg)
        },
        "ready" if source == Source::Discord => SidecarEvent::DiscordReady {
            bot_id: str_val(&v, "bot_id"),
        },
        "ready" if source == Source::Slack => SidecarEvent::SlackReady {
            bot_id: str_val(&v, "bot_id"),
            bot_name: str_val(&v, "bot_name"),
        },
        "connected" => SidecarEvent::WaConnected {
            jid: str_val(&v, "jid"),
        },
        "qr" => SidecarEvent::WaQr {
            data: str_val(&v, "data"),
        },
        "disconnected" => SidecarEvent::Disconnected {
            source,
            reason: str_val(&v, "reason"),
        },
        "error" => SidecarEvent::Error {
            source,
            message: str_val(&v, "message"),
        },
        _ => return None,
    };
    Some(ev)
}

fn str_val(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or("").to_string()
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_bridge_path_from_repo_root() {
        let bridge_path = sidecar_bridge_path();

        assert!(bridge_path.ends_with("sidecar/bridge.js"));
        assert!(bridge_path.is_file());
    }

    #[test]
    fn parse_discord_message() {
        let line = r#"{"source":"discord","event":"message","message_id":"m1","channel_id":"ch1","sender_id":"u1","sender_name":"Alice","text":"hello","timestamp":1234,"is_dm":false,"mentions_bot":true}"#;
        let ev = parse_event(line).unwrap();
        let SidecarEvent::Message(msg) = ev else {
            panic!("expected Message")
        };
        assert_eq!(msg.source, Source::Discord);
        assert_eq!(msg.chat_id, "ch1");
        assert_eq!(msg.sender_name, "Alice");
        assert!(msg.mentions_bot);
        assert!(msg.is_group);
    }

    #[test]
    fn parse_whatsapp_message() {
        let line = r#"{"source":"whatsapp","event":"message","id":"wa1","jid":"12345@g.us","sender":"56789","sender_name":"Bob","text":"yo","timestamp":5678,"is_group":true,"mentions_bot":false}"#;
        let ev = parse_event(line).unwrap();
        let SidecarEvent::Message(msg) = ev else {
            panic!("expected Message")
        };
        assert_eq!(msg.source, Source::WhatsApp);
        assert_eq!(msg.chat_id, "12345@g.us");
        assert_eq!(msg.text, "yo");
    }

    #[test]
    fn parse_discord_ready() {
        let line = r#"{"source":"discord","event":"ready","bot_id":"bot123"}"#;
        let ev = parse_event(line).unwrap();
        let SidecarEvent::DiscordReady { bot_id } = ev else {
            panic!()
        };
        assert_eq!(bot_id, "bot123");
    }

    #[test]
    fn parse_wa_connected() {
        let line = r#"{"source":"whatsapp","event":"connected","jid":"me@s.whatsapp.net"}"#;
        let ev = parse_event(line).unwrap();
        let SidecarEvent::WaConnected { jid } = ev else {
            panic!()
        };
        assert_eq!(jid, "me@s.whatsapp.net");
    }

    #[test]
    fn parse_slack_message() {
        let line = r#"{"source":"slack","event":"message","message_id":"1234567890.123456","channel_id":"C12345","sender_id":"U99999","sender_name":"Alice","text":"hello borg","timestamp":1234567890,"is_dm":false,"mentions_bot":true}"#;
        let ev = parse_event(line).unwrap();
        let SidecarEvent::Message(msg) = ev else {
            panic!("expected Message")
        };
        assert_eq!(msg.source, Source::Slack);
        assert_eq!(msg.chat_id, "C12345");
        assert_eq!(msg.sender_name, "Alice");
        assert!(msg.mentions_bot);
        assert!(msg.is_group);
    }

    #[test]
    fn parse_slack_ready() {
        let line = r#"{"source":"slack","event":"ready","bot_id":"U0BOT","bot_name":"borg"}"#;
        let ev = parse_event(line).unwrap();
        let SidecarEvent::SlackReady { bot_id, bot_name } = ev else {
            panic!()
        };
        assert_eq!(bot_id, "U0BOT");
        assert_eq!(bot_name, "borg");
    }

    #[test]
    fn parse_unknown_source_returns_none() {
        let line = r#"{"source":"signal","event":"message","text":"hi"}"#;
        assert!(parse_event(line).is_none());
    }
}
