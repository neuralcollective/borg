use anyhow::{Context, Result};
use borg_core::config::Config;
use serde_json::{json, Value};

#[derive(Clone)]
pub struct OpenSearchClient {
    http: reqwest::Client,
    base_url: String,
    index: String,
    username: String,
    password: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchHit {
    pub project_id: i64,
    pub task_id: i64,
    pub file_path: String,
    pub title_snippet: String,
    pub content_snippet: String,
    pub score: f64,
}

impl OpenSearchClient {
    pub fn from_config(config: &Config) -> Option<Self> {
        if !config.search_backend.eq_ignore_ascii_case("opensearch") {
            return None;
        }
        if config.opensearch_url.trim().is_empty() {
            return None;
        }
        Some(Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .ok()?,
            base_url: config.opensearch_url.trim_end_matches('/').to_string(),
            index: config.opensearch_index.clone(),
            username: config.opensearch_username.clone(),
            password: config.opensearch_password.clone(),
        })
    }

    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.username.is_empty() {
            req
        } else {
            req.basic_auth(&self.username, Some(&self.password))
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
        let url = format!("{}/{}/_doc/{}", self.base_url, self.index, doc_id);
        let body = json!({
            "project_id": project_id,
            "task_id": task_id,
            "file_path": file_path,
            "title": title,
            "content": content,
            "indexed_at": chrono::Utc::now().to_rfc3339(),
        });
        let resp = self
            .authed(self.http.put(url))
            .json(&body)
            .send()
            .await
            .context("opensearch index request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("opensearch index failed ({status}): {text}");
        }
        Ok(())
    }

    pub async fn search(
        &self,
        query: &str,
        project_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<SearchHit>> {
        let mut must = vec![json!({
            "simple_query_string": {
                "query": query,
                "fields": ["title^2", "content", "file_path"],
                "default_operator": "and"
            }
        })];
        if let Some(pid) = project_id {
            must.push(json!({ "term": { "project_id": pid } }));
        }
        let body = json!({
            "size": limit.max(1),
            "query": {
                "bool": {
                    "must": must
                }
            },
            "highlight": {
                "fields": {
                    "title": {},
                    "content": {}
                }
            }
        });
        let url = format!("{}/{}/_search", self.base_url, self.index);
        let resp = self
            .authed(self.http.post(url))
            .json(&body)
            .send()
            .await
            .context("opensearch search request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("opensearch search failed ({status}): {text}");
        }
        let json: Value = resp.json().await.context("parse opensearch search response")?;
        let hits = json["hits"]["hits"].as_array().cloned().unwrap_or_default();
        let mut out = Vec::with_capacity(hits.len());
        for h in hits {
            let src = &h["_source"];
            let hl = &h["highlight"];
            let title_snippet = hl["title"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| src["title"].as_str().unwrap_or(""))
                .to_string();
            let content_snippet = hl["content"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| src["content"].as_str().unwrap_or(""))
                .to_string();
            out.push(SearchHit {
                project_id: src["project_id"].as_i64().unwrap_or(0),
                task_id: src["task_id"].as_i64().unwrap_or(0),
                file_path: src["file_path"].as_str().unwrap_or("").to_string(),
                title_snippet,
                content_snippet,
                score: h["_score"].as_f64().unwrap_or(0.0),
            });
        }
        Ok(out)
    }
}
