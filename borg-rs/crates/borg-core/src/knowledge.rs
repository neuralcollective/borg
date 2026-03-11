use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use anyhow::{Context, Result};
use tracing::{debug, warn};

use crate::db::Db;

// ── Brave Search client ──────────────────────────────────────────────────

pub struct BraveSearchClient {
    http: reqwest::Client,
    api_key: String,
}

impl BraveSearchClient {
    pub fn new(api_key: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
        }
    }

    pub async fn search(&self, query: &str) -> Result<String> {
        let resp = self
            .http
            .get("https://api.search.brave.com/res/v1/web/search")
            .query(&[("q", query)])
            .header("X-Subscription-Token", &self.api_key)
            .header("Accept", "application/json")
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("brave search error {}: {}", status, text);
        }

        let json: serde_json::Value = resp.json().await?;
        let mut results = Vec::new();
        if let Some(web) = json
            .get("web")
            .and_then(|v| v.get("results"))
            .and_then(|v| v.as_array())
        {
            for res in web {
                let title = res.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let description = res
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let url = res.get("url").and_then(|v| v.as_str()).unwrap_or("");
                results.push(format!("### {title}\nURL: {url}\n{description}"));
            }
        }

        Ok(results.join("\n\n"))
    }
}

// ── Embedding client ─────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EmbeddingBackend {
    Ollama,
    OpenAI, // OpenAI-compatible API (Voyage AI, OpenAI, etc.)
}

pub const MODEL_GENERAL: &str = "voyage-4-large";
pub const MODEL_LAW: &str = "voyage-law-2";
pub const MODEL_FINANCE: &str = "voyage-finance-2";
pub const MODEL_CODE: &str = "voyage-code-3";
pub const VOYAGE_MODELS: &[&str] = &[MODEL_GENERAL, MODEL_LAW, MODEL_FINANCE, MODEL_CODE];

/// Returns the best Voyage embedding model for a given pipeline mode.
pub fn model_for_mode(mode: &str) -> &'static str {
    match mode {
        "lawborg" | "legal" => MODEL_LAW,
        "finance" | "finborg" => MODEL_FINANCE,
        "sweborg" | "code" => MODEL_CODE,
        _ => MODEL_GENERAL,
    }
}

pub struct EmbeddingClient {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: Option<String>,
    backend: EmbeddingBackend,
    dim: usize,
}

impl EmbeddingClient {
    pub fn new(
        base_url: &str,
        model: &str,
        api_key: Option<&str>,
        backend: EmbeddingBackend,
        dim: usize,
    ) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_else(|e| {
                    tracing::error!("failed to build HTTP client: {e}");
                    reqwest::Client::new()
                }),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            api_key: api_key.map(|s| s.to_string()),
            backend,
            dim,
        }
    }

    pub fn from_env() -> Self {
        let api_key = std::env::var("VOYAGE_API_KEY")
            .or_else(|_| std::env::var("EMBEDDING_API_KEY"))
            .ok();
        let backend = if api_key.is_some() {
            EmbeddingBackend::OpenAI
        } else {
            EmbeddingBackend::Ollama
        };

        let (default_url, default_model, default_dim) = match backend {
            EmbeddingBackend::OpenAI => ("https://api.voyageai.com", MODEL_GENERAL, 1024),
            EmbeddingBackend::Ollama => ("http://localhost:11434", "nomic-embed-text", 768),
        };

        let base_url = std::env::var("EMBEDDING_BASE_URL")
            .or_else(|_| std::env::var("OLLAMA_BASE_URL"))
            .unwrap_or_else(|_| default_url.to_string());
        let model = std::env::var("EMBEDDING_MODEL").unwrap_or_else(|_| default_model.to_string());
        let dim = std::env::var("EMBEDDING_DIM")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default_dim);

        tracing::info!(
            "embedding backend: {backend:?}, model: {model}, dim: {dim}, url: {base_url}"
        );
        Self::new(&base_url, &model, api_key.as_deref(), backend, dim)
    }

    pub fn model_name(&self) -> &str {
        &self.model
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn zero_embedding(&self) -> Vec<f32> {
        vec![0.0; self.dim]
    }

    pub async fn embed(&self, texts: &[&str], input_type: &str) -> Result<Vec<Vec<f32>>> {
        match self.backend {
            EmbeddingBackend::Ollama => self.embed_ollama(texts).await,
            EmbeddingBackend::OpenAI => self.embed_openai(texts, input_type).await,
        }
    }

    async fn embed_ollama(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
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

        let parsed: OllamaEmbedResponse =
            resp.json().await.context("parse ollama embed response")?;
        Ok(parsed.embeddings)
    }

    async fn embed_openai(&self, texts: &[&str], input_type: &str) -> Result<Vec<Vec<f32>>> {
        let mut body = serde_json::json!({
            "model": self.model,
            "input": texts,
        });
        if !input_type.is_empty() {
            body["input_type"] = serde_json::Value::String(input_type.to_string());
        }
        let mut req = self
            .http
            .post(format!("{}/v1/embeddings", self.base_url))
            .json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await.context("embedding API request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("embedding API returned {status}: {body}");
        }

        let parsed: OpenAIEmbedResponse = resp.json().await.context("parse embedding response")?;
        let mut result: Vec<(usize, Vec<f32>)> = parsed
            .data
            .into_iter()
            .map(|d| (d.index, d.embedding))
            .collect();
        result.sort_by_key(|(i, _)| *i);
        Ok(result.into_iter().map(|(_, e)| e).collect())
    }

    /// Embed a single text for indexing (document)
    pub async fn embed_document(&self, text: &str) -> Result<Vec<f32>> {
        let mut results = self.embed(&[text], "document").await?;
        results
            .pop()
            .ok_or_else(|| anyhow::anyhow!("empty embedding response"))
    }

    /// Embed a single text for search (query)
    pub async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let mut results = self.embed(&[text], "query").await?;
        results
            .pop()
            .ok_or_else(|| anyhow::anyhow!("empty embedding response"))
    }

    pub async fn is_available(&self) -> bool {
        match self.backend {
            EmbeddingBackend::Ollama => self
                .http
                .get(format!("{}/api/tags", self.base_url))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false),
            EmbeddingBackend::OpenAI => self.api_key.is_some(),
        }
    }
}

/// Holds multiple EmbeddingClient instances keyed by model name.
/// Allows agents to pick the best model for their search domain.
pub struct EmbeddingRegistry {
    clients: std::collections::HashMap<String, EmbeddingClient>,
    default_model: String,
}

impl EmbeddingRegistry {
    /// Build from env. Creates the default client plus all Voyage domain models
    /// if a Voyage API key is available.
    pub fn from_env() -> Self {
        let default = EmbeddingClient::from_env();
        let default_model = default.model.clone();
        let mut clients = std::collections::HashMap::new();

        let voyage_key = std::env::var("VOYAGE_API_KEY")
            .or_else(|_| std::env::var("EMBEDDING_API_KEY"))
            .ok();

        if let Some(ref key) = voyage_key {
            let base = std::env::var("EMBEDDING_BASE_URL")
                .unwrap_or_else(|_| "https://api.voyageai.com".to_string());
            for &model in VOYAGE_MODELS {
                if model == default.model {
                    continue;
                }
                clients.insert(
                    model.to_string(),
                    EmbeddingClient::new(&base, model, Some(key), EmbeddingBackend::OpenAI, 1024),
                );
            }
        }

        clients.insert(default_model.clone(), default);
        tracing::info!(
            "embedding registry: {} models loaded (default: {})",
            clients.len(),
            default_model
        );

        Self {
            clients,
            default_model,
        }
    }

    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    /// Get the default client (backward-compatible).
    pub fn default_client(&self) -> &EmbeddingClient {
        self.clients
            .get(&self.default_model)
            .expect("default embedding client missing")
    }

    /// Get a client by model name, falling back to the default.
    pub fn client(&self, model: &str) -> &EmbeddingClient {
        self.clients
            .get(model)
            .unwrap_or_else(|| self.default_client())
    }

    /// Get the best client for a pipeline mode.
    pub fn client_for_mode(&self, mode: &str) -> &EmbeddingClient {
        let model = model_for_mode(mode);
        self.client(model)
    }

    pub fn available_models(&self) -> Vec<&str> {
        self.clients.keys().map(|s| s.as_str()).collect()
    }
}

#[derive(serde::Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[derive(serde::Deserialize)]
struct OpenAIEmbedResponse {
    data: Vec<OpenAIEmbedData>,
}

#[derive(serde::Deserialize)]
struct OpenAIEmbedData {
    embedding: Vec<f32>,
    index: usize,
}

// ── Chunking ─────────────────────────────────────────────────────────────

const CHUNK_SIZE: usize = 512;
const CHUNK_OVERLAP: usize = 64;
const MIN_SECTION_WORDS: usize = 40;

pub fn chunk_text(text: &str) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return vec![];
    }
    if words.len() <= CHUNK_SIZE {
        return vec![words.join(" ")];
    }

    let sections = split_sections(text);
    if sections.len() <= 1 {
        return chunk_words(&words);
    }

    let mut chunks = Vec::new();
    let mut buffer = String::new();
    let mut buf_words = 0usize;

    for section in &sections {
        let sec_words: Vec<&str> = section.split_whitespace().collect();
        if sec_words.is_empty() {
            continue;
        }

        // If adding this section would exceed chunk size, flush buffer first
        if buf_words > 0 && buf_words + sec_words.len() > CHUNK_SIZE {
            let bw: Vec<&str> = buffer.split_whitespace().collect();
            chunks.extend(chunk_words(&bw));
            buffer.clear();
            buf_words = 0;
        }

        // If a single section is too large, chunk it independently
        if sec_words.len() > CHUNK_SIZE {
            if buf_words > 0 {
                let bw: Vec<&str> = buffer.split_whitespace().collect();
                chunks.extend(chunk_words(&bw));
                buffer.clear();
                buf_words = 0;
            }
            chunks.extend(chunk_words(&sec_words));
            continue;
        }

        if !buffer.is_empty() {
            buffer.push(' ');
        }
        buffer.push_str(section);
        buf_words += sec_words.len();
    }

    if buf_words > 0 {
        let bw: Vec<&str> = buffer.split_whitespace().collect();
        chunks.extend(chunk_words(&bw));
    }

    if chunks.is_empty() {
        chunk_words(&words)
    } else {
        chunks
    }
}

/// Split text into sections at structural boundaries: headings, numbered clauses,
/// double newlines. Merges tiny fragments into the next section.
fn split_sections(text: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            // Double newline / paragraph break — potential split point
            if !current.trim().is_empty() {
                let words: Vec<&str> = current.split_whitespace().collect();
                if words.len() >= MIN_SECTION_WORDS {
                    sections.push(current.split_whitespace().collect::<Vec<_>>().join(" "));
                    current = String::new();
                    continue;
                }
            }
            continue;
        }

        if is_section_heading(trimmed) && !current.trim().is_empty() {
            let words: Vec<&str> = current.split_whitespace().collect();
            if words.len() >= MIN_SECTION_WORDS {
                sections.push(words.join(" "));
                current = String::new();
            }
        }

        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(trimmed);
    }

    if !current.trim().is_empty() {
        sections.push(current.split_whitespace().collect::<Vec<_>>().join(" "));
    }

    sections
}

/// Detect lines that look like section headings or numbered clauses.
fn is_section_heading(line: &str) -> bool {
    let bytes = line.as_bytes();
    if bytes.is_empty() {
        return false;
    }

    // Numbered clauses: "1.", "1.2", "1.2.3", "(a)", "(i)", "Section 5", "Article III"
    if bytes[0].is_ascii_digit() {
        if let Some(dot_pos) = line.find('.') {
            if dot_pos < 8 {
                return true;
            }
        }
    }

    // Parenthetical numbering: (a), (i), (1), (A)
    if bytes[0] == b'(' && line.len() >= 3 {
        let close = line.find(')');
        if let Some(pos) = close {
            if pos <= 5 {
                return true;
            }
        }
    }

    let upper = line.to_uppercase();
    if upper.starts_with("SECTION ")
        || upper.starts_with("ARTICLE ")
        || upper.starts_with("CLAUSE ")
        || upper.starts_with("SCHEDULE ")
        || upper.starts_with("EXHIBIT ")
        || upper.starts_with("APPENDIX ")
        || upper.starts_with("PART ")
        || upper.starts_with("RECITAL")
        || upper.starts_with("DEFINITION")
        || upper.starts_with("WHEREAS")
    {
        return true;
    }

    // ALL CAPS headings (at least 3 words, all uppercase letters)
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() >= 2 && words.len() <= 12 {
        let all_upper = words.iter().all(|w| {
            w.chars()
                .filter(|c| c.is_alphabetic())
                .all(|c| c.is_uppercase())
                && w.chars().any(|c| c.is_alphabetic())
        });
        if all_upper {
            return true;
        }
    }

    false
}

/// Fixed-size word chunking with overlap (the original algorithm).
fn chunk_words(words: &[&str]) -> Vec<String> {
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
            match embed_client.embed_document(chunk).await {
                Ok(embedding) => {
                    if let Err(e) =
                        db.upsert_embedding(project_id, Some(task_id), chunk, file, &embedding)
                    {
                        warn!("failed to store embedding for task #{task_id}: {e}");
                    } else {
                        indexed += 1;
                    }
                },
                Err(e) => {
                    warn!("embedding failed for task #{task_id} chunk: {e}");
                    return;
                },
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
        (0..n)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ")
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
    match embed_client.embed_query(query).await {
        Ok(query_emb) => db
            .search_embeddings(&query_emb, limit, project_id)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.score > 0.5)
            .collect(),
        Err(e) => {
            warn!("failed to embed query for prior research: {e}");
            vec![]
        },
    }
}
