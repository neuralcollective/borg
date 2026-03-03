use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use anyhow::{Context, Result};
use tracing::{debug, warn};

use crate::db::Db;

// ── Embedding client ─────────────────────────────────────────────────────

pub struct EmbeddingClient {
    http: reqwest::Client,
    base_url: String,
    model: String,
}

impl EmbeddingClient {
    pub fn new(base_url: &str, model: &str) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
        }
    }

    pub fn from_env() -> Self {
        let base_url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());
        let model = std::env::var("EMBEDDING_MODEL")
            .unwrap_or_else(|_| "nomic-embed-text".to_string());
        Self::new(&base_url, &model)
    }

    pub async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });
        let resp = self
            .http
            .post(format!("{}/api/embed", self.base_url))
            .json(&body)
            .send()
            .await
            .context("ollama embed request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("ollama embed returned {status}: {body}");
        }

        let parsed: EmbedResponse = resp.json().await.context("parse embed response")?;
        Ok(parsed.embeddings)
    }

    pub async fn embed_single(&self, text: &str) -> Result<Vec<f32>> {
        let mut results = self.embed(&[text]).await?;
        results
            .pop()
            .ok_or_else(|| anyhow::anyhow!("empty embedding response"))
    }

    pub async fn is_available(&self) -> bool {
        self.http
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

#[derive(serde::Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

// ── Chunking ─────────────────────────────────────────────────────────────

const CHUNK_SIZE: usize = 512;
const CHUNK_OVERLAP: usize = 64;

pub fn chunk_text(text: &str) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return vec![];
    }
    if words.len() <= CHUNK_SIZE {
        return vec![words.join(" ")];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < words.len() {
        let end = (start + CHUNK_SIZE).min(words.len());
        chunks.push(words[start..end].join(" "));
        if end >= words.len() {
            break;
        }
        start = end - CHUNK_OVERLAP;
    }
    chunks
}

pub fn hash_chunk(text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

// ── Vector math ──────────────────────────────────────────────────────────

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < 1e-10 {
        0.0
    } else {
        dot / denom
    }
}

pub fn embedding_to_bytes(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|f| f.to_le_bytes()).collect()
}

pub fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

// ── Types ────────────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
pub struct EmbeddingSearchResult {
    pub chunk_text: String,
    pub file_path: String,
    pub project_id: Option<i64>,
    pub task_id: Option<i64>,
    pub score: f32,
}

// ── Pipeline integration ─────────────────────────────────────────────────

pub async fn index_task_embeddings(
    db: &Db,
    embed_client: &EmbeddingClient,
    task_id: i64,
    project_id: Option<i64>,
    repo_path: &str,
) {
    if repo_path.is_empty() {
        return;
    }
    let branch = format!("task-{}", task_id);
    let path = std::path::Path::new(repo_path);
    if !path.join(".git").exists() {
        return;
    }

    if !embed_client.is_available().await {
        debug!("ollama not available, skipping embedding for task #{task_id}");
        return;
    }

    let output = std::process::Command::new("git")
        .args(["-C", repo_path, "ls-tree", "-r", "--name-only", &branch])
        .stderr(std::process::Stdio::null())
        .output();
    let file_list = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return,
    };

    let _ = db.remove_task_embeddings(task_id);

    let mut indexed = 0usize;
    for file in file_list.lines() {
        if !file.ends_with(".md") {
            continue;
        }
        let content = std::process::Command::new("git")
            .args(["-C", repo_path, "show", &format!("{branch}:{file}")])
            .stderr(std::process::Stdio::null())
            .output();
        let text = match content {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => continue,
        };
        if text.trim().is_empty() {
            continue;
        }

        let chunks = chunk_text(&text);
        for chunk in &chunks {
            if chunk.split_whitespace().count() < 10 {
                continue;
            }
            match embed_client.embed_single(chunk).await {
                Ok(embedding) => {
                    if let Err(e) = db.upsert_embedding(project_id, Some(task_id), chunk, file, &embedding) {
                        warn!("failed to store embedding for task #{task_id}: {e}");
                    } else {
                        indexed += 1;
                    }
                }
                Err(e) => {
                    warn!("embedding failed for task #{task_id} chunk: {e}");
                    return;
                }
            }
        }
    }
    if indexed > 0 {
        debug!("indexed {indexed} embeddings for task #{task_id}");
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_words(n: usize) -> String {
        (0..n).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn empty_string_returns_empty_vec() {
        assert_eq!(chunk_text(""), Vec::<String>::new());
    }

    #[test]
    fn fewer_than_512_words_is_single_chunk() {
        let text = make_words(100);
        let chunks = chunk_text(&text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn exactly_512_words_is_single_chunk() {
        let words: Vec<String> = (0..512).map(|i| format!("w{i}")).collect();
        let text = words.join(" ");
        let chunks = chunk_text(&text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn exactly_513_words_produces_two_chunks_with_correct_boundaries() {
        let words: Vec<String> = (0..513).map(|i| format!("w{i}")).collect();
        let text = words.join(" ");
        let chunks = chunk_text(&text);
        assert_eq!(chunks.len(), 2);
        // first chunk: words 0..512
        assert_eq!(chunks[0], words[..512].join(" "));
        // second chunk: starts at word 448 (the 449th word), ends at 512
        assert_eq!(chunks[1], words[448..].join(" "));
        assert!(chunks[1].starts_with("w448 "));
    }

    #[test]
    fn no_whitespace_is_single_chunk() {
        let text = "abcdefghijklmnopqrstuvwxyz".repeat(50);
        let chunks = chunk_text(&text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }
}

pub async fn get_prior_research(
    db: &Db,
    embed_client: &EmbeddingClient,
    query: &str,
    project_id: Option<i64>,
    limit: usize,
) -> Vec<EmbeddingSearchResult> {
    if db.embedding_count() == 0 {
        return vec![];
    }
    if !embed_client.is_available().await {
        return vec![];
    }
    match embed_client.embed_single(query).await {
        Ok(query_emb) => db
            .search_embeddings(&query_emb, limit, project_id)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.score > 0.5)
            .collect(),
        Err(e) => {
            warn!("failed to embed query for prior research: {e}");
            vec![]
        }
    }
}
