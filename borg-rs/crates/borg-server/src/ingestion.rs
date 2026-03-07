use std::sync::Arc;

use anyhow::{Context, Result};
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_sqs::Client;
use borg_core::{config::Config, db::Db};
use serde_json::json;

use crate::{search::SearchClient, storage::FileStorage};

#[derive(Clone)]
pub enum IngestionQueue {
    Disabled,
    Sqs { queue_url: String, client: Client },
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

            let mut loader = aws_config::defaults(BehaviorVersion::latest())
                .region(Region::new(config.sqs_region.clone()));
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
            },
        }
    }

    pub async fn run_worker(
        self: Arc<Self>,
        db: Arc<Db>,
        storage: Arc<FileStorage>,
        search: Option<Arc<SearchClient>>,
        embed_client: Arc<borg_core::knowledge::EmbeddingClient>,
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

                let processed = process_message(body, &db, &storage, search.as_deref(), Some(&*embed_client)).await;
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
    search: Option<&SearchClient>,
    embed_client: Option<&borg_core::knowledge::EmbeddingClient>,
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
        },
    };
    if !row.extracted_text.is_empty() {
        return true;
    }

    let bytes = match storage.read_all(&row.stored_path).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("ingestion worker read failed: {e}");
            return false;
        },
    };
    let text = match extract_text_from_bytes(&msg.file_name, &msg.mime_type, &bytes).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("ingestion worker extract failed: {e}");
            return false;
        },
    };
    if text.is_empty() {
        return true;
    }

    if let Err(e) = db.update_project_file_text(msg.file_id, &text) {
        tracing::warn!("ingestion worker update text failed: {e}");
        return false;
    }
    if let Err(e) = db.fts_index_document(msg.project_id, 0, &msg.file_name, &msg.file_name, &text)
    {
        tracing::warn!("ingestion worker fts index failed: {e}");
        return false;
    }
    // Chunk, embed, and index to Vespa
    if let Some(os) = search {
        let _ = os.delete_file_chunks(msg.project_id, msg.file_id).await;
        let chunks_text = borg_core::knowledge::chunk_text(&text);
        if !chunks_text.is_empty() {
            let metadata = crate::vespa::ChunkMetadata {
                doc_type: detect_doc_type(&msg.file_name, &msg.mime_type, &text),
                jurisdiction: String::new(),
                privileged: row.privileged,
                mime_type: msg.mime_type.clone(),
            };

            let dim = embed_client.map(|ec| ec.dim()).unwrap_or(1024);
            let mut chunks_with_embeddings: Vec<(String, Vec<f32>)> = Vec::new();
            if let Some(ec) = embed_client {
                for chunk in &chunks_text {
                    match ec.embed_document(chunk).await {
                        Ok(emb) => chunks_with_embeddings.push((chunk.clone(), emb)),
                        Err(e) => {
                            tracing::warn!("embedding failed for chunk: {e}");
                            chunks_with_embeddings.push((chunk.clone(), vec![0.0; dim]));
                        }
                    }
                }
            } else {
                for chunk in &chunks_text {
                    chunks_with_embeddings.push((chunk.clone(), vec![0.0; dim]));
                }
            }

            if let Err(e) = os.index_chunks(
                msg.project_id,
                msg.file_id,
                &msg.file_name,
                &msg.file_name,
                &chunks_with_embeddings,
                &metadata,
            ).await {
                tracing::warn!("ingestion worker chunk index failed: {e}");
            }
        }

        // Also keep legacy whole-doc index for backward compat
        let doc_id = format!("project-{}-file-{}", msg.project_id, msg.file_id);
        if let Err(e) = os
            .index_document(&doc_id, msg.project_id, 0, &msg.file_name, &msg.file_name, &text)
            .await
        {
            tracing::warn!("ingestion worker search index failed: {e}");
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

pub(crate) fn detect_doc_type(file_name: &str, mime: &str, text: &str) -> String {
    let name_lower = file_name.to_lowercase();
    let ext = name_lower.rsplit('.').next().unwrap_or("");

    // By extension/mime
    if mime.contains("pdf") || ext == "pdf" {
        let text_lower = text.to_lowercase();
        let first_2k = if text_lower.len() > 2000 { &text_lower[..2000] } else { &text_lower };
        if first_2k.contains("agreement") || first_2k.contains("contract") || (first_2k.contains("between") && first_2k.contains("parties")) {
            return "contract".to_string();
        }
        if first_2k.contains("court") || first_2k.contains("plaintiff") || first_2k.contains("defendant") || first_2k.contains("v.") {
            return "filing".to_string();
        }
        if first_2k.contains("statute") || (first_2k.contains("section") && first_2k.contains("chapter")) {
            return "statute".to_string();
        }
        return "document".to_string();
    }
    if ext == "docx" || ext == "doc" || mime.contains("wordprocessingml") {
        return "document".to_string();
    }
    if ext == "md" || ext == "txt" {
        let text_lower = text.to_lowercase();
        let first_2k = if text_lower.len() > 2000 { &text_lower[..2000] } else { &text_lower };
        if first_2k.contains("agreement") || first_2k.contains("contract") || (first_2k.contains("between") && first_2k.contains("parties")) {
            return "contract".to_string();
        }
        if first_2k.contains("court") || first_2k.contains("plaintiff") || first_2k.contains("defendant") || first_2k.contains(" v. ") {
            return "filing".to_string();
        }
        if first_2k.contains("statute") || (first_2k.contains("section") && first_2k.contains("chapter")) {
            return "statute".to_string();
        }
        return "memo".to_string();
    }
    if ext == "csv" || ext == "json" || ext == "xml" {
        return "data".to_string();
    }
    "document".to_string()
}

pub(crate) fn detect_jurisdiction(text: &str) -> String {
    static JURISDICTIONS: &[&str] = &[
        "Delaware", "New York", "California", "Texas", "Illinois",
        "Massachusetts", "Florida", "Pennsylvania", "Virginia",
        "District of Columbia", "Nevada", "Georgia", "Federal",
    ];
    let first_4k = if text.len() > 4000 { &text[..text.floor_char_boundary(4000)] } else { text };
    for &j in JURISDICTIONS {
        if first_4k.contains(j) {
            return j.to_string();
        }
    }
    String::new()
}

pub(crate) async fn extract_text_from_bytes(file_name: &str, mime: &str, bytes: &[u8]) -> Result<String> {
    let file_name = file_name.to_string();
    let mime = mime.to_string();
    let bytes = bytes.to_vec();
    tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
        let ext = file_name.rsplit('.').next().unwrap_or("").to_lowercase();
        let is_pdf = mime.contains("pdf") || ext == "pdf";
        let is_docx = mime.contains("wordprocessingml")
            || mime.contains("msword")
            || ext == "docx"
            || ext == "doc";
        let is_text = mime.starts_with("text/")
            || ext == "txt"
            || ext == "md"
            || ext == "csv"
            || ext == "json"
            || ext == "xml";

        if is_pdf {
            let tmp = tempfile::NamedTempFile::new()?;
            std::fs::write(tmp.path(), &bytes)?;
            let out = std::process::Command::new("pdftotext")
                .args(["-layout", tmp.path().to_str().unwrap_or(""), "-"])
                .output()?;
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        } else if is_docx {
            let suffix = if ext.is_empty() { "docx" } else { &ext };
            let tmp = tempfile::Builder::new()
                .suffix(&format!(".{suffix}"))
                .tempfile()?;
            std::fs::write(tmp.path(), &bytes)?;
            let out = std::process::Command::new("pandoc")
                .args([
                    tmp.path().to_str().unwrap_or(""),
                    "-t",
                    "plain",
                    "--wrap=none",
                ])
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
