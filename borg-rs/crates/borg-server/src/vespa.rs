use anyhow::{Context, Result};
use borg_core::config::Config;
use serde_json::{json, Value};

use crate::search::{ChunkSearchHit, SearchHit};

#[derive(Default, Clone)]
pub struct ChunkMetadata {
    pub doc_type: String,
    pub jurisdiction: String,
    pub privileged: bool,
    pub mime_type: String,
}

#[derive(Default)]
pub struct ChunkFilters {
    pub doc_type: Option<String>,
    pub jurisdiction: Option<String>,
    pub privileged_only: bool,
}

#[derive(Clone)]
pub struct VespaClient {
    http: reqwest::Client,
    base_url: String,
    namespace: String,
    document_type: String,
}

impl VespaClient {
    pub fn from_config(config: &Config) -> Option<Self> {
        if !config.search_backend.eq_ignore_ascii_case("vespa") {
            return None;
        }
        if config.vespa_url.trim().is_empty() {
            return None;
        }
        Some(Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .ok()?,
            base_url: config.vespa_url.trim_end_matches('/').to_string(),
            namespace: config.vespa_namespace.trim().to_string(),
            document_type: config.vespa_document_type.trim().to_string(),
        })
    }

    pub fn target(&self) -> String {
        format!(
            "{}/{}/{}",
            self.base_url, self.namespace, self.document_type
        )
    }

    fn percent_encode(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for b in value.bytes() {
            match b {
                b'A'..=b'Z'
                | b'a'..=b'z'
                | b'0'..=b'9'
                | b'-'
                | b'_'
                | b'.'
                | b'~' => out.push(b as char),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }

    fn document_url(&self, doc_id: &str) -> String {
        format!(
            "{}/document/v1/{}/{}/docid/{}",
            self.base_url,
            self.namespace,
            self.document_type,
            Self::percent_encode(doc_id)
        )
    }

    pub async fn healthcheck(&self) -> Result<()> {
        let url = format!("{}/state/v1/health", self.base_url);
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .context("vespa health request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("vespa health failed ({status}): {text}");
        }
        Ok(())
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
        let url = self.document_url(doc_id);
        let body = json!({
            "fields": {
                "project_id": project_id,
                "task_id": task_id,
                "file_path": file_path,
                "title": title,
                "content": content,
                "indexed_at": chrono::Utc::now().to_rfc3339(),
            }
        });
        let resp = self
            .http
            .put(url)
            .json(&body)
            .send()
            .await
            .context("vespa document feed failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("vespa document feed failed ({status}): {text}");
        }
        Ok(())
    }

    pub async fn search(
        &self,
        query: &str,
        project_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<SearchHit>> {
        let mut yql = "select * from sources * where userQuery()".to_string();
        if let Some(pid) = project_id {
            yql.push_str(&format!(" and project_id = {pid}"));
        }
        yql.push(';');
        let resp = self
            .http
            .post(format!("{}/search/", self.base_url))
            .json(&json!({
                "yql": yql,
                "query": query,
                "hits": limit.max(1),
                "ranking.profile": "default",
                "presentation.summary": "default"
            }))
            .send()
            .await
            .context("vespa search request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("vespa search failed ({status}): {text}");
        }
        let json: Value = resp.json().await.context("parse vespa search response")?;
        let hits = json["root"]["children"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::with_capacity(hits.len());
        for hit in hits {
            let fields = &hit["fields"];
            let title = fields["title"].as_str().unwrap_or("");
            let content = fields["content"].as_str().unwrap_or("");
            let file_path = fields["file_path"].as_str().unwrap_or("").to_string();
            out.push(SearchHit {
                project_id: fields["project_id"].as_i64().unwrap_or(0),
                task_id: fields["task_id"].as_i64().unwrap_or(0),
                file_path,
                title_snippet: title.to_string(),
                content_snippet: excerpt_for_query(content, query),
                score: hit["relevance"].as_f64().unwrap_or(0.0),
            });
        }
        Ok(out)
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
        let now = chrono::Utc::now().to_rfc3339();
        for (chunk_index, (chunk_text, embedding)) in chunks.iter().enumerate() {
            let doc_id = format!("p{project_id}-f{file_id}-c{chunk_index}");
            let url = format!(
                "{}/document/v1/{}/project_chunk/docid/{}",
                self.base_url,
                self.namespace,
                Self::percent_encode(&doc_id),
            );
            let body = json!({
                "fields": {
                    "project_id": project_id,
                    "file_id": file_id,
                    "chunk_index": chunk_index,
                    "file_path": file_path,
                    "title": title,
                    "content": chunk_text,
                    "doc_type": metadata.doc_type,
                    "jurisdiction": metadata.jurisdiction,
                    "privileged": metadata.privileged,
                    "mime_type": metadata.mime_type,
                    "indexed_at": &now,
                    "embedding": { "values": embedding },
                }
            });
            let resp = self
                .http
                .put(&url)
                .json(&body)
                .send()
                .await
                .with_context(|| format!("vespa chunk feed failed for {doc_id}"))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("vespa chunk feed failed for {doc_id} ({status}): {text}");
            }
        }
        Ok(())
    }

    pub async fn delete_file_chunks(&self, project_id: i64, file_id: i64) -> Result<()> {
        let selection = format!(
            "project_chunk.project_id=={project_id} and project_chunk.file_id=={file_id}"
        );
        let url = format!(
            "{}/document/v1/{}/project_chunk/docid/",
            self.base_url, self.namespace,
        );
        let resp = self
            .http
            .delete(&url)
            .query(&[("selection", &selection), ("cluster", &"borg".to_string())])
            .send()
            .await
            .context("vespa delete_file_chunks request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("vespa delete_file_chunks failed ({status}): {text}");
        }
        Ok(())
    }

    pub async fn search_chunks(
        &self,
        query: &str,
        query_embedding: Option<&[f32]>,
        project_id: Option<i64>,
        filters: &ChunkFilters,
        limit: i64,
    ) -> Result<Vec<ChunkSearchHit>> {
        let mut yql = "select * from project_chunk where ".to_string();
        if query_embedding.is_some() {
            yql.push_str("({targetHits:100}nearestNeighbor(embedding, q_embedding)) or userQuery()");
        } else {
            yql.push_str("userQuery()");
        }
        if let Some(pid) = project_id {
            yql.push_str(&format!(" and project_id = {pid}"));
        }
        if let Some(ref dt) = filters.doc_type {
            yql.push_str(&format!(" and doc_type contains '{dt}'"));
        }
        if let Some(ref j) = filters.jurisdiction {
            yql.push_str(&format!(" and jurisdiction contains '{j}'"));
        }
        if filters.privileged_only {
            yql.push_str(" and privileged = true");
        }
        yql.push(';');

        let ranking = if query_embedding.is_some() {
            "hybrid"
        } else {
            "default"
        };

        let mut request_body = json!({
            "yql": yql,
            "query": query,
            "hits": limit.max(1),
            "ranking.profile": ranking,
            "presentation.summary": "default",
        });

        if let Some(emb) = query_embedding {
            let emb_list: Vec<Value> = emb.iter().map(|&v| json!(v)).collect();
            request_body["input.query(q_embedding)"] = Value::Array(emb_list);
        }

        let resp = self
            .http
            .post(format!("{}/search/", self.base_url))
            .json(&request_body)
            .send()
            .await
            .context("vespa chunk search request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("vespa chunk search failed ({status}): {text}");
        }
        let json: Value = resp.json().await.context("parse vespa chunk search response")?;
        let hits = json["root"]["children"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::with_capacity(hits.len());
        for hit in hits {
            let fields = &hit["fields"];
            out.push(ChunkSearchHit {
                project_id: fields["project_id"].as_i64().unwrap_or(0),
                file_id: fields["file_id"].as_i64().unwrap_or(0),
                chunk_index: fields["chunk_index"].as_i64().unwrap_or(0) as i32,
                file_path: fields["file_path"].as_str().unwrap_or("").to_string(),
                title: fields["title"].as_str().unwrap_or("").to_string(),
                content: fields["content"].as_str().unwrap_or("").to_string(),
                doc_type: fields["doc_type"].as_str().unwrap_or("").to_string(),
                score: hit["relevance"].as_f64().unwrap_or(0.0),
            });
        }
        Ok(out)
    }
}

fn excerpt_for_query(content: &str, query: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lowered = trimmed.to_lowercase();
    let term = query
        .split_whitespace()
        .find(|part| part.len() >= 4)
        .map(|part| part.to_lowercase());
    if let Some(term) = term {
        if let Some(idx) = lowered.find(&term) {
            let char_idx = trimmed.char_indices()
                .position(|(i, _)| i >= idx)
                .unwrap_or(0);
            let chars: Vec<char> = trimmed.chars().collect();
            let start = char_idx.saturating_sub(160);
            let end = (char_idx + term.chars().count() + 240).min(chars.len());
            return chars[start..end].iter().collect::<String>().replace('\n', " ");
        }
    }
    trimmed.chars().take(360).collect::<String>().replace('\n', " ")
}
