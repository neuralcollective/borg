use anyhow::Result;
use reqwest::Client;
use serde_json::Value;
use tracing::{info, warn};

const HAIKU: &str = "claude-haiku-4-5-20251001";
const MAX_LOG_BYTES: usize = 50_000;

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    fn from_str(s: &str) -> Self {
        match s {
            "low" => Self::Low,
            "high" => Self::High,
            "critical" => Self::Critical,
            _ => Self::Medium,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone)]
enum Source {
    Journalctl { unit: String },
    FileTail { path: String },
    Command { cmd: String },
}

#[derive(Debug, Clone)]
enum Action {
    Alert { chat_id: String },
    Command { cmd: String },
    Webhook { url: String },
}

#[derive(Debug, Clone)]
struct Entry {
    name: String,
    source: Source,
    window_lines: u32,
    interval_s: i64,
    prompt: String,
    cooldown_s: i64,
    severity_threshold: Severity,
    actions: Vec<Action>,
    last_run: i64,
    last_triggered: i64,
}

struct AnalysisResult {
    triggered: bool,
    severity: Severity,
    summary: String,
    recommendation: String,
}

pub struct Observer {
    entries: Vec<Entry>,
    api_key: String,
    telegram_token: String,
    client: Client,
}

impl Observer {
    /// Load observer config from a JSON file. Returns an empty observer if the file is missing.
    pub fn load(config_path: &str, api_key: &str, telegram_token: &str) -> Self {
        let entries = load_entries(config_path);
        if !entries.is_empty() {
            info!("Observer: loaded {} entry/entries from {}", entries.len(), config_path);
        }
        Self {
            entries,
            api_key: api_key.to_string(),
            telegram_token: telegram_token.to_string(),
            client: Client::new(),
        }
    }

    pub async fn run(mut self) {
        let api_key = self.api_key.clone();
        let telegram_token = self.telegram_token.clone();
        let client = self.client.clone();

        loop {
            let now = chrono::Utc::now().timestamp();
            for entry in self.entries.iter_mut() {
                if now - entry.last_run < entry.interval_s {
                    continue;
                }
                entry.last_run = now;
                let last_triggered = entry.last_triggered;
                match run_entry(&client, entry, &api_key, &telegram_token, now, last_triggered).await {
                    Ok(triggered) => {
                        if triggered {
                            entry.last_triggered = now;
                        }
                    }
                    Err(e) => warn!("Observer [{}]: {}", entry.name, e),
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        }
    }
}

async fn run_entry(
    client: &Client,
    entry: &Entry,
    api_key: &str,
    telegram_token: &str,
    now: i64,
    last_triggered: i64,
) -> Result<bool> {
    let logs = collect_logs(entry).await?;
    if logs.is_empty() {
        return Ok(false);
    }

    let result = analyze(client, entry, &logs, api_key).await?;
    if !result.triggered {
        return Ok(false);
    }
    if result.severity < entry.severity_threshold {
        return Ok(false);
    }
    if now - last_triggered < entry.cooldown_s {
        return Ok(false);
    }

    warn!(
        "Observer [{}] triggered ({}): {}",
        entry.name,
        result.severity.as_str(),
        result.summary
    );

    for action in &entry.actions {
        if let Err(e) = execute_action(client, &entry.name, action, &result, telegram_token).await {
            warn!("Observer [{}] action failed: {}", entry.name, e);
        }
    }
    Ok(true)
}

async fn analyze(client: &Client, entry: &Entry, logs: &str, api_key: &str) -> Result<AnalysisResult> {
    let log_slice = if logs.len() > MAX_LOG_BYTES {
        &logs[logs.len() - MAX_LOG_BYTES..]
    } else {
        logs
    };

    let user_content = format!(
        "You are a log monitor.\n\n{}\n\nRecent log output:\n```\n{}\n```\n\n\
         Respond ONLY with JSON. If something concerning is found: \
         {{\"triggered\":true,\"severity\":\"low|medium|high|critical\",\"summary\":\"one sentence\",\"recommendation\":\"one sentence\"}}. \
         If nothing to flag: {{\"triggered\":false}}",
        entry.prompt, log_slice
    );

    let body = serde_json::json!({
        "model": HAIKU,
        "max_tokens": 256,
        "messages": [{"role": "user", "content": user_content}]
    });

    let resp: Value = client
        .post("https://api.anthropic.com/v1/messages")
        .header("content-type", "application/json")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await?
        .json()
        .await?;

    let text = resp["content"][0]["text"].as_str().unwrap_or("{}");
    let inner: Value = serde_json::from_str(strip_fences(text)).unwrap_or(Value::Null);

    let triggered = inner["triggered"].as_bool().unwrap_or(false);
    if !triggered {
        return Ok(AnalysisResult {
            triggered: false,
            severity: Severity::Low,
            summary: String::new(),
            recommendation: String::new(),
        });
    }

    Ok(AnalysisResult {
        triggered: true,
        severity: Severity::from_str(inner["severity"].as_str().unwrap_or("medium")),
        summary: inner["summary"].as_str().unwrap_or("").to_string(),
        recommendation: inner["recommendation"].as_str().unwrap_or("").to_string(),
    })
}

async fn execute_action(
    client: &Client,
    name: &str,
    action: &Action,
    result: &AnalysisResult,
    telegram_token: &str,
) -> Result<()> {
    match action {
        Action::Alert { chat_id } => {
            if telegram_token.is_empty() {
                return Ok(());
            }
            let raw_id = chat_id.strip_prefix("tg:").unwrap_or(chat_id);
            let msg = format!(
                "[Observer: {}] {}\n\n{}\n\nRecommendation: {}",
                name,
                result.severity.as_str(),
                result.summary,
                result.recommendation
            );
            let url = format!("https://api.telegram.org/bot{}/sendMessage", telegram_token);
            client
                .post(&url)
                .json(&serde_json::json!({"chat_id": raw_id, "text": msg}))
                .send()
                .await?;
        }
        Action::Command { cmd } => {
            tokio::process::Command::new("/bin/sh")
                .args(["-c", cmd])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?
                .wait()
                .await?;
        }
        Action::Webhook { url } => {
            let body = serde_json::json!({
                "observer": name,
                "severity": result.severity.as_str(),
                "summary": result.summary,
                "recommendation": result.recommendation,
            });
            client.post(url).json(&body).send().await?;
        }
    }
    Ok(())
}

async fn collect_logs(entry: &Entry) -> Result<String> {
    let lines_str = entry.window_lines.to_string();
    let mut cmd = match &entry.source {
        Source::Journalctl { unit } => {
            let mut c = tokio::process::Command::new("journalctl");
            c.args(["-u", unit, "-n", &lines_str, "--no-pager", "--output=short-precise"]);
            c
        }
        Source::FileTail { path } => {
            let mut c = tokio::process::Command::new("tail");
            c.args(["-n", &lines_str, path]);
            c
        }
        Source::Command { cmd } => {
            let mut c = tokio::process::Command::new("/bin/sh");
            c.args(["-c", cmd]);
            c
        }
    };
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());

    let output = cmd.output().await?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn strip_fences(text: &str) -> &str {
    let t = text.trim();
    if !t.starts_with("```") {
        return t;
    }
    let nl = match t.find('\n') {
        Some(i) => i,
        None => return t,
    };
    let inner = &t[nl + 1..];
    if inner.ends_with("```") {
        inner[..inner.len() - 3].trim_end()
    } else {
        inner
    }
}

fn load_entries(path: &str) -> Vec<Entry> {
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) => {
            warn!("Observer: can't read {}: {}", path, e);
            return vec![];
        }
    };
    let v: Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(e) => {
            warn!("Observer: invalid JSON in {}: {}", path, e);
            return vec![];
        }
    };
    let arr = match v.as_array() {
        Some(a) => a,
        None => {
            warn!("Observer: config must be a JSON array");
            return vec![];
        }
    };

    arr.iter()
        .filter_map(|item| match parse_entry(item) {
            Ok(e) => Some(e),
            Err(err) => {
                warn!("Observer: skipping invalid entry: {}", err);
                None
            }
        })
        .collect()
}

fn parse_entry(v: &Value) -> Result<Entry> {
    let name = v["name"].as_str().ok_or_else(|| anyhow::anyhow!("missing name"))?.to_string();
    let prompt = v["prompt"].as_str().ok_or_else(|| anyhow::anyhow!("missing prompt"))?.to_string();

    let src = &v["source"];
    let src_type = src["type"].as_str().ok_or_else(|| anyhow::anyhow!("missing source.type"))?;
    let source = match src_type {
        "journalctl" => Source::Journalctl {
            unit: src["unit"].as_str().ok_or_else(|| anyhow::anyhow!("missing unit"))?.to_string(),
        },
        "file_tail" => Source::FileTail {
            path: src["path"].as_str().ok_or_else(|| anyhow::anyhow!("missing path"))?.to_string(),
        },
        "command" => Source::Command {
            cmd: src["cmd"].as_str().ok_or_else(|| anyhow::anyhow!("missing cmd"))?.to_string(),
        },
        other => anyhow::bail!("unknown source type: {}", other),
    };

    let actions = v["actions"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|av| {
            let atype = av["type"].as_str()?;
            match atype {
                "alert" => Some(Action::Alert { chat_id: av["chat_id"].as_str()?.to_string() }),
                "command" => Some(Action::Command { cmd: av["cmd"].as_str()?.to_string() }),
                "webhook" => Some(Action::Webhook { url: av["url"].as_str()?.to_string() }),
                _ => None,
            }
        })
        .collect();

    Ok(Entry {
        name,
        source,
        window_lines: v["window_lines"].as_u64().unwrap_or(200) as u32,
        interval_s: v["interval_s"].as_i64().unwrap_or(60),
        prompt,
        cooldown_s: v["cooldown_s"].as_i64().unwrap_or(300),
        severity_threshold: Severity::from_str(v["severity_threshold"].as_str().unwrap_or("medium")),
        actions,
        last_run: 0,
        last_triggered: 0,
    })
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_ordering() {
        assert!(Severity::Low < Severity::Medium);
        assert!(Severity::Medium < Severity::High);
        assert!(Severity::High < Severity::Critical);
    }

    #[test]
    fn severity_from_str() {
        assert_eq!(Severity::from_str("low"), Severity::Low);
        assert_eq!(Severity::from_str("high"), Severity::High);
        assert_eq!(Severity::from_str("critical"), Severity::Critical);
        assert_eq!(Severity::from_str("unknown"), Severity::Medium);
    }

    #[test]
    fn strip_fences_plain() {
        assert_eq!(strip_fences(r#"{"triggered":false}"#), r#"{"triggered":false}"#);
    }

    #[test]
    fn strip_fences_with_backticks() {
        let text = "```json\n{\"triggered\":false}\n```";
        assert_eq!(strip_fences(text), r#"{"triggered":false}"#);
    }

    #[test]
    fn parse_entry_valid() {
        let v: Value = serde_json::from_str(r#"{
            "name": "test",
            "source": {"type": "journalctl", "unit": "myservice"},
            "prompt": "Check for errors",
            "interval_s": 120,
            "cooldown_s": 600,
            "severity_threshold": "high",
            "window_lines": 100,
            "actions": [
                {"type": "alert", "chat_id": "-123456"},
                {"type": "webhook", "url": "https://example.com/hook"}
            ]
        }"#).unwrap();

        let entry = parse_entry(&v).unwrap();
        assert_eq!(entry.name, "test");
        assert_eq!(entry.interval_s, 120);
        assert_eq!(entry.severity_threshold, Severity::High);
        assert_eq!(entry.actions.len(), 2);
    }

    #[test]
    fn parse_entry_missing_name_errors() {
        let v: Value = serde_json::from_str(r#"{"source":{"type":"command","cmd":"ls"},"prompt":"x"}"#).unwrap();
        assert!(parse_entry(&v).is_err());
    }

    #[test]
    fn load_entries_missing_file() {
        let entries = load_entries("/nonexistent/observer.json");
        assert!(entries.is_empty());
    }

    #[test]
    fn load_entries_from_valid_json() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, r#"[{{"name":"t","source":{{"type":"command","cmd":"echo hi"}},"prompt":"p","actions":[]}}]"#).unwrap();
        let entries = load_entries(tmp.path().to_str().unwrap());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "t");
    }
}
