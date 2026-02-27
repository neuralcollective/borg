use std::sync::{Arc, Mutex};

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
}

#[derive(Debug, Clone)]
pub struct SidecarMessage {
    pub source: Source,
    pub id: String,
    pub chat_id: String,
    pub sender: String,
    pub sender_name: String,
    pub text: String,
    pub timestamp: i64,
    pub is_group: bool,
    pub mentions_bot: bool,
}

#[derive(Debug, Clone)]
pub enum SidecarEvent {
    Message(SidecarMessage),
    DiscordReady { bot_id: String },
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

impl Sidecar {
    /// Spawn `bun sidecar/bridge.js` with automatic restart on exit.
    /// Returns `(Arc<Sidecar>, event_rx)` where event_rx is a persistent channel
    /// that receives events from all sidecar lifetimes.
    pub async fn spawn(
        assistant_name: &str,
        discord_token: &str,
        wa_auth_dir: &str,
        wa_disabled: bool,
    ) -> Result<(Self, mpsc::UnboundedReceiver<SidecarEvent>)> {
        let cmd_tx_arc: Arc<Mutex<Option<mpsc::UnboundedSender<String>>>> =
            Arc::new(Mutex::new(None));
        let (event_tx, event_rx) = mpsc::unbounded_channel::<SidecarEvent>();

        let cmd_tx_arc2 = Arc::clone(&cmd_tx_arc);
        let event_tx2 = event_tx.clone();
        let assistant_name = assistant_name.to_string();
        let discord_token = discord_token.to_string();
        let wa_auth_dir = wa_auth_dir.to_string();

        tokio::spawn(async move {
            let mut attempt = 0u32;
            loop {
                match spawn_once(
                    &assistant_name,
                    &discord_token,
                    &wa_auth_dir,
                    wa_disabled,
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
        let escaped = json_escape(text);
        let mut cmd = format!(
            r#"{{"target":"discord","cmd":"send","channel_id":"{channel_id}","text":"{escaped}""#
        );
        if let Some(id) = reply_to {
            cmd.push_str(&format!(r#","reply_to":"{id}""#));
        }
        cmd.push('}');
        self.send_raw(cmd);
    }

    pub fn send_whatsapp(&self, jid: &str, text: &str, quote_id: Option<&str>) {
        let escaped = json_escape(text);
        let mut cmd =
            format!(r#"{{"target":"whatsapp","cmd":"send","jid":"{jid}","text":"{escaped}""#);
        if let Some(id) = quote_id {
            cmd.push_str(&format!(r#","quote_id":"{id}""#));
        }
        cmd.push('}');
        self.send_raw(cmd);
    }

    pub fn send_discord_typing(&self, channel_id: &str) {
        let cmd = format!(r#"{{"target":"discord","cmd":"typing","channel_id":"{channel_id}"}}"#);
        self.send_raw(cmd);
    }

    pub fn send_whatsapp_typing(&self, jid: &str) {
        let cmd = format!(r#"{{"target":"whatsapp","cmd":"typing","jid":"{jid}"}}"#);
        self.send_raw(cmd);
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
    event_tx: mpsc::UnboundedSender<SidecarEvent>,
) -> Result<(
    mpsc::UnboundedSender<String>,
    tokio::sync::oneshot::Receiver<()>,
)> {
    let mut cmd = Command::new("bun");
    cmd.args(["sidecar/bridge.js", assistant_name]);
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

    let mut child = cmd.spawn().context("failed to spawn sidecar/bridge.js")?;
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
        _ => None,
    }
}

fn parse_event(line: &str) -> Option<SidecarEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    let source_str = v["source"].as_str()?;
    let event_type = v["event"].as_str()?;
    let source = parse_source(source_str)?;

    let ev = match event_type {
        "message" => {
            let msg = if source == Source::Discord {
                SidecarMessage {
                    source: Source::Discord,
                    id: str_val(&v, "message_id"),
                    chat_id: str_val(&v, "channel_id"),
                    sender: str_val(&v, "sender_id"),
                    sender_name: str_val(&v, "sender_name"),
                    text: str_val(&v, "text"),
                    timestamp: v["timestamp"].as_i64().unwrap_or(0),
                    is_group: !v["is_dm"].as_bool().unwrap_or(false),
                    mentions_bot: v["mentions_bot"].as_bool().unwrap_or(false),
                }
            } else {
                SidecarMessage {
                    source: Source::WhatsApp,
                    id: str_val(&v, "id"),
                    chat_id: str_val(&v, "jid"),
                    sender: str_val(&v, "sender"),
                    sender_name: str_val(&v, "sender_name"),
                    text: str_val(&v, "text"),
                    timestamp: v["timestamp"].as_i64().unwrap_or(0),
                    is_group: v["is_group"].as_bool().unwrap_or(false),
                    mentions_bot: v["mentions_bot"].as_bool().unwrap_or(false),
                }
            };
            SidecarEvent::Message(msg)
        },
        "ready" => SidecarEvent::DiscordReady {
            bot_id: str_val(&v, "bot_id"),
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

/// Escape a string for embedding inside a JSON string literal.
fn json_escape(s: &str) -> String {
    // serde_json::to_string produces `"..."` including quotes; strip them
    let quoted = serde_json::to_string(s).unwrap_or_default();
    if quoted.len() < 2 {
        return String::new();
    }
    quoted[1..quoted.len() - 1].to_string()
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parse_unknown_source_returns_none() {
        let line = r#"{"source":"signal","event":"message","text":"hi"}"#;
        assert!(parse_event(line).is_none());
    }

    #[test]
    fn json_escape_special_chars() {
        let s = json_escape("hello\nworld\t\"quote\"");
        assert!(s.contains("\\n"));
        assert!(s.contains("\\t"));
        assert!(s.contains("\\\""));
        assert!(!s.starts_with('"'));
    }

    #[test]
    fn json_escape_empty() {
        assert_eq!(json_escape(""), "");
    }
}
