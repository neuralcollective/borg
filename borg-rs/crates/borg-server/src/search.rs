use anyhow::Result;
use borg_core::config::Config;

use crate::vespa::VespaClient;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchHit {
    pub project_id: i64,
    pub task_id: i64,
    pub file_path: String,
    pub title_snippet: String,
    pub content_snippet: String,
    pub score: f64,
}

#[derive(Clone)]
pub enum SearchClient {
    Vespa(VespaClient),
}

impl SearchClient {
    pub fn from_config(config: &Config) -> Option<Self> {
        if config.search_backend.eq_ignore_ascii_case("vespa") {
            VespaClient::from_config(config).map(Self::Vespa)
        } else {
            None
        }
    }

    pub fn backend_name(&self) -> &'static str {
        match self {
            Self::Vespa(_) => "vespa",
        }
    }

    pub fn target(&self) -> String {
        match self {
            Self::Vespa(client) => client.target(),
        }
    }

    pub async fn index_document(
        &self,
        doc_id: &str,
        project_id: i64,
        task_id: i64,
        file_path: &str,
        title: &str,
        content: &str,
    ) -> Result<()> {
        match self {
            Self::Vespa(client) => {
                client
                    .index_document(doc_id, project_id, task_id, file_path, title, content)
                    .await
            },
        }
    }

    pub async fn search(
        &self,
        query: &str,
        project_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<SearchHit>> {
        match self {
            Self::Vespa(client) => client.search(query, project_id, limit).await,
        }
    }

    pub async fn healthcheck(&self) -> Result<()> {
        match self {
            Self::Vespa(client) => client.healthcheck().await,
        }
    }
}
