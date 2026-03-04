use std::sync::Arc;

use anyhow::{Context, Result};
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_sqs::Client;
use borg_core::config::Config;
use borg_core::db::Db;
use serde_json::json;

use crate::storage::FileStorage;
use crate::opensearch::OpenSearchClient;

#[derive(Clone)]
pub enum IngestionQueue {
    Disabled,
    Sqs {
        queue_url: String,
        client: Client,
    },
}

impl IngestionQueue {
    pub async fn from_config(config: &Config) -> Result<Self> {
        if config
            .ingestion_queue_backend
            .trim()
            .eq_ignore_ascii_case("sqs")
        {
            if config.sqs_queue_url.trim().is_empty() {
                return Ok(Self::Disabled);
            }

            let mut loader =
                aws_config::defaults(BehaviorVersion::latest()).region(Region::new(
                    config.sqs_region.clone(),
                ));
            if !config.s3_access_key.is_empty() && !config.s3_secret_key.is_empty() {
                loader = loader.credentials_provider(Credentials::new(
                    config.s3_access_key.clone(),
                    config.s3_secret_key.clone(),
                    None,
                    None,
                    "borg-server-config",
                ));
            }
            let shared = loader.load().await;
            let client = Client::new(&shared);
            return Ok(Self::Sqs {
                queue_url: config.sqs_queue_url.clone(),
                client,
            });
        }

        Ok(Self::Disabled)
    }

    pub async fn enqueue_project_file(
        &self,
        project_id: i64,
        file_id: i64,
        file_name: &str,
        stored_path: &str,
        mime_type: &str,
        size_bytes: i64,
    ) -> Result<()> {
        let payload = json!({
            "kind": "project_file_ingest",
            "project_id": project_id,
            "file_id": file_id,
            "file_name": file_name,
            "stored_path": stored_path,
            "mime_type": mime_type,
            "size_bytes": size_bytes,
            "ts": chrono::Utc::now().to_rfc3339(),
        })
        .to_string();

        match self {
            Self::Disabled => Ok(()),
            Self::Sqs { queue_url, client } => {
                client
                    .send_message()
                    .queue_url(queue_url)
                    .message_body(payload)
                    .send()
                    .await
                    .context("sqs send_message project_file_ingest")?;
                Ok(())
            }
        }
    }

    pub async fn run_worker(
        self: Arc<Self>,
        db: Arc<Db>,
        storage: Arc<FileStorage>,
        search: Option<Arc<OpenSearchClient>>,
    ) {
        let (queue_url, client) = match self.as_ref() {
            Self::Disabled => return,
            Self::Sqs { queue_url, client } => (queue_url.clone(), client.clone()),
        };

        loop {
            let resp = client
                .receive_message()
                .queue_url(&queue_url)
                .max_number_of_messages(5)
                .wait_time_seconds(20)
                .visibility_timeout(120)
                .send()
                .await;
            let Ok(resp) = resp else {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            };

            let Some(messages) = resp.messages else {
                continue;
            };
            for message in messages {
                let body = message.body.as_deref().unwrap_or("");
                let receipt = message.receipt_handle.as_deref().unwrap_or("");
                if receipt.is_empty() {
                    continue;
                }

                let processed = process_message(body, &db, &storage, search.as_deref()).await;
                if processed {
                    let _ = client
                        .delete_message()
                        .queue_url(&queue_url)
                        .receipt_handle(receipt)
                        .send()
                        .await;
                }
            }
        }
    }
}

#[derive(serde::Deserialize)]
struct ProjectFileIngestMsg {
    kind: String,
    project_id: i64,
    file_id: i64,
    file_name: String,
    mime_type: String,
}

async fn process_message(
    body: &str,
    db: &Db,
    storage: &FileStorage,
    search: Option<&OpenSearchClient>,
) -> bool {
    let parsed = serde_json::from_str::<ProjectFileIngestMsg>(body);
    let Ok(msg) = parsed else {
        tracing::warn!("ingestion worker received non-json message");
        return true;
    };
    if msg.kind != "project_file_ingest" {
        return true;
    }

    let row = match db.get_project_file(msg.project_id, msg.file_id) {
        Ok(Some(r)) => r,
        Ok(None) => return true,
        Err(e) => {
            tracing::warn!("ingestion worker db lookup failed: {e}");
            return false;
        }
    };
    if !row.extracted_text.is_empty() {
        return true;
    }

    let bytes = match storage.read_all(&row.stored_path).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("ingestion worker read failed: {e}");
            return false;
        }
    };
    let text = match extract_text_from_bytes(&msg.file_name, &msg.mime_type, &bytes).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("ingestion worker extract failed: {e}");
            return false;
        }
    };
    if text.is_empty() {
        return true;
    }

    if let Err(e) = db.update_project_file_text(msg.file_id, &text) {
        tracing::warn!("ingestion worker update text failed: {e}");
        return false;
    }
    if let Err(e) = db.fts_index_document(msg.project_id, 0, &msg.file_name, &msg.file_name, &text) {
        tracing::warn!("ingestion worker fts index failed: {e}");
        return false;
    }
    if let Some(os) = search {
        let doc_id = format!("project-{}-file-{}", msg.project_id, msg.file_id);
        if let Err(e) = os
            .index_document(
                &doc_id,
                msg.project_id,
                0,
                &msg.file_name,
                &msg.file_name,
                &text,
            )
            .await
        {
            tracing::warn!("ingestion worker opensearch index failed: {e}");
        }
    }

    tracing::info!(
        project_id = msg.project_id,
        file_id = msg.file_id,
        chars = text.len(),
        "ingestion worker indexed file"
    );
    true
}

async fn extract_text_from_bytes(file_name: &str, mime: &str, bytes: &[u8]) -> Result<String> {
    let file_name = file_name.to_string();
    let mime = mime.to_string();
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
        let ext = file_name.rsplit('.').next().unwrap_or("").to_lowercase();
        let is_pdf = mime.contains("pdf") || ext == "pdf";
        let is_docx = mime.contains("wordprocessingml") || mime.contains("msword")
            || ext == "docx" || ext == "doc";
        let is_text = mime.starts_with("text/") || ext == "txt" || ext == "md"
            || ext == "csv" || ext == "json" || ext == "xml";

        if is_pdf {
            let tmp = tempfile::NamedTempFile::new()?;
            std::fs::write(tmp.path(), &bytes)?;
            let out = std::process::Command::new("pdftotext")
                .args(["-layout", tmp.path().to_str().unwrap_or(""), "-"])
                .output()?;
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        } else if is_docx {
            let suffix = if ext.is_empty() { "docx" } else { &ext };
            let tmp = tempfile::Builder::new().suffix(&format!(".{suffix}")).tempfile()?;
            std::fs::write(tmp.path(), &bytes)?;
            let out = std::process::Command::new("pandoc")
                .args([tmp.path().to_str().unwrap_or(""), "-t", "plain", "--wrap=none"])
                .output()?;
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        } else if is_text {
            Ok(String::from_utf8_lossy(&bytes).to_string())
        } else {
            Ok(String::new())
        }
    })
    .await
    .context("spawn_blocking extract_text")?
}
