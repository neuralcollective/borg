use anyhow::{Context, Result};
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_sqs::Client;
use borg_core::config::Config;
use serde_json::json;

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
}
