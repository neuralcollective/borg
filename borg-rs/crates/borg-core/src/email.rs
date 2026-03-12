// Email ingestion: raw MIME parsing, IMAP polling, and SMTP replies.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use mailparse::MailHeaderMap;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct ParsedEmail {
    pub from: String,
    pub from_name: String,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<EmailAttachment>,
    pub message_id: String,
    pub in_reply_to: String,
}

#[derive(Debug, Clone)]
pub struct EmailAttachment {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

/// Parse a raw RFC 2822 email message.
pub fn parse_raw(raw: &[u8]) -> Result<ParsedEmail> {
    let parsed = mailparse::parse_mail(raw).context("mailparse")?;

    let subject = parsed
        .headers
        .get_first_value("Subject")
        .unwrap_or_default();
    let from_header = parsed.headers.get_first_value("From").unwrap_or_default();
    let message_id = parsed
        .headers
        .get_first_value("Message-ID")
        .unwrap_or_default();
    let in_reply_to = parsed
        .headers
        .get_first_value("In-Reply-To")
        .unwrap_or_default();

    let (from, from_name) = parse_address(&from_header);
    let (body, attachments) = extract_body_and_attachments(&parsed);

    Ok(ParsedEmail {
        from,
        from_name,
        subject,
        body,
        attachments,
        message_id,
        in_reply_to,
    })
}

/// Parse a Postmark-style inbound webhook JSON body.
pub fn parse_postmark_json(raw: &[u8]) -> Result<ParsedEmail> {
    let v: serde_json::Value = serde_json::from_slice(raw).context("postmark json")?;

    let from_header = v["From"].as_str().unwrap_or("").to_string();
    let (from, from_name) = parse_address(&from_header);
    let subject = v["Subject"].as_str().unwrap_or("").to_string();
    let body = v["TextBody"]
        .as_str()
        .or_else(|| v["text"].as_str())
        .unwrap_or("")
        .to_string();
    let message_id = v["MessageID"]
        .as_str()
        .or_else(|| v["message_id"].as_str())
        .unwrap_or("")
        .to_string();

    let mut attachments = Vec::new();
    if let Some(atts) = v["Attachments"].as_array() {
        for att in atts {
            let name = att["Name"].as_str().unwrap_or("attachment").to_string();
            let ct = att["ContentType"]
                .as_str()
                .unwrap_or("application/octet-stream")
                .to_string();
            if let Some(content) = att["Content"].as_str() {
                use base64::Engine;
                if let Ok(data) = base64::engine::general_purpose::STANDARD.decode(content.trim()) {
                    attachments.push(EmailAttachment {
                        filename: name,
                        content_type: ct,
                        data,
                    });
                }
            }
        }
    }

    Ok(ParsedEmail {
        from,
        from_name,
        subject,
        body,
        attachments,
        message_id,
        in_reply_to: String::new(),
    })
}

/// Parse either raw RFC 2822 or Postmark JSON, auto-detecting format.
pub fn parse_auto(raw: &[u8], content_type: &str) -> Result<ParsedEmail> {
    if content_type.contains("application/json") {
        parse_postmark_json(raw)
    } else {
        parse_raw(raw)
    }
}

fn parse_address(addr: &str) -> (String, String) {
    let addr = addr.trim();
    if let (Some(lt), Some(gt)) = (addr.rfind('<'), addr.find('>')) {
        if lt < gt {
            let email = addr[lt + 1..gt].trim().to_lowercase();
            let name = addr[..lt].trim().trim_matches('"').to_string();
            return (email, name);
        }
    }
    (addr.to_lowercase(), String::new())
}

fn extract_body_and_attachments(mail: &mailparse::ParsedMail) -> (String, Vec<EmailAttachment>) {
    let mut body = String::new();
    let mut attachments = Vec::new();
    extract_recursive(mail, &mut body, &mut attachments);
    (body, attachments)
}

fn extract_recursive(
    mail: &mailparse::ParsedMail,
    body: &mut String,
    attachments: &mut Vec<EmailAttachment>,
) {
    let mime = mail.ctype.mimetype.to_lowercase();

    if mime.starts_with("multipart/") {
        for sub in &mail.subparts {
            extract_recursive(sub, body, attachments);
        }
        return;
    }

    let cd = mail.get_content_disposition();
    let is_attachment = matches!(cd.disposition, mailparse::DispositionType::Attachment);

    if is_attachment {
        let filename = cd
            .params
            .get("filename")
            .or_else(|| mail.ctype.params.get("name"))
            .cloned()
            .unwrap_or_else(|| "attachment".to_string());
        if let Ok(data) = mail.get_body_raw() {
            attachments.push(EmailAttachment {
                filename,
                content_type: mime,
                data,
            });
        }
    } else if mime == "text/plain" && body.is_empty() {
        body.push_str(&mail.get_body().unwrap_or_default());
    }
    // skip text/html if we already have plain text
}

/// Save attachments to a directory, returning their paths.
pub fn save_attachments(atts: &[EmailAttachment], dir: &Path) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut paths = Vec::new();
    for att in atts {
        let safe = att
            .filename
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let safe = if safe.is_empty() {
            "attachment".to_string()
        } else {
            safe
        };
        let path = dir.join(&safe);
        std::fs::write(&path, &att.data)?;
        paths.push(path);
    }
    Ok(paths)
}

/// Send an SMTP reply. No-op if smtp_host is empty.
pub async fn send_smtp_reply(
    smtp_host: &str,
    smtp_port: u16,
    smtp_from: &str,
    smtp_user: &str,
    smtp_pass: &str,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<()> {
    if smtp_host.is_empty() || smtp_from.is_empty() {
        return Ok(());
    }
    use lettre::{
        transport::smtp::authentication::Credentials, AsyncSmtpTransport, AsyncTransport, Message,
        Tokio1Executor,
    };

    let email = Message::builder()
        .from(smtp_from.parse().context("smtp_from parse")?)
        .to(to.parse().context("smtp to parse")?)
        .subject(subject)
        .body(body.to_string())
        .context("lettre body")?;

    let creds = Credentials::new(smtp_user.to_string(), smtp_pass.to_string());
    let mailer = AsyncSmtpTransport::<Tokio1Executor>::relay(smtp_host)
        .context("smtp relay")?
        .port(smtp_port)
        .credentials(creds)
        .build();

    mailer.send(email).await.map(|_| ()).context("smtp send")
}

// ── IMAP polling ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    pub mailbox: String,
}

/// Poll the IMAP inbox for unseen messages, mark them read, return parsed emails.
/// Runs in a blocking thread internally.
pub async fn poll_imap(config: ImapConfig) -> Result<Vec<ParsedEmail>> {
    tokio::task::spawn_blocking(move || poll_imap_sync(&config))
        .await
        .context("spawn_blocking")?
}

fn poll_imap_sync(config: &ImapConfig) -> Result<Vec<ParsedEmail>> {
    let client = imap::ClientBuilder::new(&config.host, config.port)
        .connect()
        .context("imap connect")?;

    let mut session = client
        .login(&config.user, &config.pass)
        .map_err(|(e, _)| anyhow::anyhow!("IMAP login failed: {e}"))?;

    session.select(&config.mailbox).context("imap select")?;

    let uids: std::collections::HashSet<u32> =
        session.uid_search("UNSEEN").context("uid_search")?;
    let mut emails = Vec::new();

    for uid in &uids {
        let uid_str = uid.to_string();
        match session.uid_fetch(&uid_str, "(RFC822)") {
            Ok(msgs) => {
                for msg in msgs.iter() {
                    if let Some(body) = msg.body() {
                        match parse_raw(body) {
                            Ok(parsed) => emails.push(parsed),
                            Err(e) => warn!("imap parse uid={uid}: {e}"),
                        }
                    }
                }
                let _ = session.uid_store(&uid_str, "+FLAGS (\\Seen)");
            },
            Err(e) => warn!("imap uid_fetch uid={uid}: {e}"),
        }
    }

    let _ = session.logout();
    info!("imap poll: {} unseen messages", emails.len());
    Ok(emails)
}

/// Polling loop — runs forever, calling the callback for each batch of emails.
pub async fn run_imap_poller<F, Fut>(config: ImapConfig, interval_secs: u64, on_emails: F)
where
    F: Fn(Vec<ParsedEmail>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    info!(host = %config.host, "starting IMAP poller (interval {}s)", interval_secs);
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(interval_secs)).await;
        match poll_imap(config.clone()).await {
            Ok(emails) if !emails.is_empty() => on_emails(emails).await,
            Ok(_) => {},
            Err(e) => warn!("imap poll error: {e}"),
        }
    }
}
