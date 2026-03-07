use anyhow::Result;
use borg_core::config::Config;

use crate::vespa::{ChunkFilters, ChunkMetadata, VespaClient};

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchHit {
    pub project_id: i64,
    pub task_id: i64,
    pub file_path: String,
    pub title_snippet: String,
    pub content_snippet: String,
    pub score: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ChunkSearchHit {
    pub project_id: i64,
    pub file_id: i64,
    pub chunk_index: i32,
    pub file_path: String,
    pub title: String,
    pub content: String,
    pub doc_type: String,
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

    pub async fn index_chunks(
        &self,
        project_id: i64,
        file_id: i64,
        file_path: &str,
        title: &str,
        chunks: &[(String, Vec<f32>)],
        metadata: &ChunkMetadata,
    ) -> Result<()> {
        match self {
            Self::Vespa(client) => {
                client
                    .index_chunks(project_id, file_id, file_path, title, chunks, metadata)
                    .await
            }
        }
    }

    pub async fn delete_file_chunks(&self, project_id: i64, file_id: i64) -> Result<()> {
        match self {
            Self::Vespa(client) => client.delete_file_chunks(project_id, file_id).await,
        }
    }

    pub async fn search_chunks(
        &self,
        query: &str,
        query_embedding: Option<&[f32]>,
        project_id: Option<i64>,
        filters: &ChunkFilters,
        limit: i64,
    ) -> Result<Vec<ChunkSearchHit>> {
        match self {
            Self::Vespa(client) => {
                client
                    .search_chunks(query, query_embedding, project_id, filters, limit)
                    .await
            }
        }
    }
}
