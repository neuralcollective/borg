use anyhow::{Context, Result};
use borg_core::config::Config;
use serde_json::{json, Value};

use crate::search::SearchHit;

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
            let start = idx.saturating_sub(160);
            let end = (idx + term.len() + 240).min(trimmed.len());
            return trimmed[start..end].replace('\n', " ");
        }
    }
    trimmed.chars().take(360).collect::<String>().replace('\n', " ")
}
