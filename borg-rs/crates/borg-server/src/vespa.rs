use anyhow::{Context, Result};
use borg_core::config::Config;
use serde_json::{json, Value};

use crate::search::{ChunkSearchHit, SearchHit};

fn escape_yql(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-' || *c == '_' || *c == '.')
        .collect()
}

fn expand_legal_query(query: &str) -> String {
    static SYNONYMS: &[&[&str]] = &[
        &["statute", "law", "legislation", "act", "enactment"],
        &["plaintiff", "claimant", "petitioner", "complainant"],
        &["defendant", "respondent", "accused"],
        &["contract", "agreement", "covenant"],
        &["breach", "violation", "infringement"],
        &["liability", "obligation", "responsibility"],
        &["indemnification", "indemnity", "hold harmless"],
        &["termination", "cancellation", "rescission"],
        &["jurisdiction", "venue", "forum"],
        &["damages", "compensation", "remedy", "relief"],
        &["negligence", "carelessness", "fault"],
        &["arbitration", "mediation", "dispute resolution", "adr"],
        &["confidential", "proprietary", "trade secret", "nda"],
        &[
            "intellectual property",
            "ip",
            "patent",
            "trademark",
            "copyright",
        ],
        &["amendment", "modification", "addendum", "supplement"],
        &["warranty", "guarantee", "representation"],
        &["injunction", "restraining order", "tro"],
        &["subpoena", "summons", "citation"],
        &["deposition", "testimony", "interrogatory"],
        &["privilege", "attorney-client", "work product"],
        &["fiduciary", "trustee", "duty of care"],
        &["force majeure", "act of god", "unforeseen circumstances"],
        &["assignment", "transfer", "novation"],
        &["severability", "separability"],
        &["governing law", "choice of law", "applicable law"],
    ];

    let lower = query.to_lowercase();
    let mut expanded_terms: Vec<String> = vec![query.to_string()];
    for group in SYNONYMS {
        for &term in *group {
            if lower.contains(term) {
                for &synonym in *group {
                    if synonym != term && !lower.contains(synonym) {
                        expanded_terms.push(synonym.to_string());
                    }
                }
                break;
            }
        }
    }
    if expanded_terms.len() == 1 {
        return query.to_string();
    }
    expanded_terms.join(" ")
}

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
    pub exclude_terms: Vec<String>,
}

#[derive(Clone)]
pub struct VespaClient {
    http: reqwest::Client,
    base_url: String,
    namespace: String,
    document_type: String,
}

fn build_document_search_yql(project_id: Option<i64>) -> String {
    let mut yql = "select * from sources * where userQuery()".to_string();
    if let Some(pid) = project_id {
        yql.push_str(&format!(" and project_id = {pid}"));
    }
    yql.push(';');
    yql
}

fn build_chunk_search_yql(
    use_query_embedding: bool,
    project_id: Option<i64>,
    filters: &ChunkFilters,
) -> String {
    let mut yql = "select * from project_chunk where ".to_string();
    if use_query_embedding {
        yql.push_str("(({targetHits:100}nearestNeighbor(embedding, q_embedding)) or userQuery())");
    } else {
        yql.push_str("userQuery()");
    }
    if let Some(pid) = project_id {
        yql.push_str(&format!(" and project_id = {pid}"));
    }
    if let Some(ref dt) = filters.doc_type {
        yql.push_str(&format!(" and doc_type contains '{}'", escape_yql(dt)));
    }
    if let Some(ref j) = filters.jurisdiction {
        yql.push_str(&format!(" and jurisdiction contains '{}'", escape_yql(j)));
    }
    if filters.privileged_only {
        yql.push_str(" and privileged = true");
    }
    for term in &filters.exclude_terms {
        let escaped = escape_yql(term);
        yql.push_str(&format!(" and !(content contains '{escaped}')"));
    }
    yql.push(';');
    yql
}

fn retain_search_hits_for_project(hits: &mut Vec<SearchHit>, project_id: Option<i64>) -> usize {
    let Some(pid) = project_id else {
        return 0;
    };
    let before = hits.len();
    hits.retain(|hit| hit.project_id == pid);
    before - hits.len()
}

fn retain_chunk_hits_for_project(hits: &mut Vec<ChunkSearchHit>, project_id: Option<i64>) -> usize {
    let Some(pid) = project_id else {
        return 0;
    };
    let before = hits.len();
    hits.retain(|hit| hit.project_id == pid);
    before - hits.len()
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
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char)
                },
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

    pub async fn document_count(&self, doc_type: &str) -> Result<i64> {
        let yql = format!("select * from {doc_type} where true");
        let resp = self
            .http
            .post(format!("{}/search/", self.base_url))
            .json(&json!({
                "yql": yql,
                "hits": 0,
            }))
            .send()
            .await
            .context("vespa document_count request failed")?;
        if !resp.status().is_success() {
            return Ok(-1);
        }
        let json: Value = resp.json().await?;
        Ok(json["root"]["fields"]["totalCount"].as_i64().unwrap_or(0))
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
            .post(url)
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
        let yql = build_document_search_yql(project_id);
        let resp = self
            .http
            .post(format!("{}/search/", self.base_url))
            .json(&json!({
                "yql": yql,
                "query": expand_legal_query(query),
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
        let removed = retain_search_hits_for_project(&mut out, project_id);
        if removed > 0 {
            tracing::warn!(
                requested_project_id = project_id.unwrap_or_default(),
                removed = removed,
                "vespa document search returned cross-project hits; filtering them server-side"
            );
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
        let now = chrono::Utc::now().timestamp_millis();
        const BATCH_SIZE: usize = 10;

        let all_ops: Vec<_> = chunks
            .iter()
            .enumerate()
            .map(|(chunk_index, (chunk_text, embedding))| {
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
                        "indexed_at": now,
                        "embedding": { "values": embedding },
                    }
                });
                (doc_id, url, body)
            })
            .collect();

        for batch in all_ops.chunks(BATCH_SIZE) {
            let mut set = tokio::task::JoinSet::new();
            for (doc_id, url, body) in batch {
                let client = self.http.clone();
                let url = url.clone();
                let body = body.clone();
                let doc_id = doc_id.clone();
                set.spawn(async move {
                    let resp = client.post(&url).json(&body).send().await;
                    match resp {
                        Ok(r) if r.status().is_success() => Ok(()),
                        Ok(r) => {
                            let status = r.status();
                            let text = r.text().await.unwrap_or_default();
                            Err(anyhow::anyhow!(
                                "chunk feed failed for {doc_id} ({status}): {text}"
                            ))
                        },
                        Err(e) => Err(anyhow::anyhow!("chunk feed failed for {doc_id}: {e}")),
                    }
                });
            }
            while let Some(result) = set.join_next().await {
                if let Ok(Err(e)) = result {
                    tracing::warn!("{e}");
                }
            }
        }
        Ok(())
    }

    pub async fn delete_file_chunks(&self, project_id: i64, file_id: i64) -> Result<()> {
        let selection =
            format!("project_chunk.project_id=={project_id} and project_chunk.file_id=={file_id}");
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

    pub async fn delete_project_chunks(&self, project_id: i64) -> Result<()> {
        let selection = format!("project_chunk.project_id=={project_id}");
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
            .context("vespa delete_project_chunks request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("vespa delete_project_chunks failed ({status}): {text}");
        }
        Ok(())
    }

    pub async fn facet_counts(&self, project_id: i64, field: &str) -> Result<Vec<(String, i64)>> {
        if !field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            anyhow::bail!("invalid facet field name");
        }
        let yql = format!(
            "select * from project_chunk where project_id = {project_id} | all(group({field}) each(output(count())))"
        );
        let resp = self
            .http
            .post(format!("{}/search/", self.base_url))
            .json(&serde_json::json!({
                "yql": yql,
                "hits": 0,
            }))
            .send()
            .await
            .context("vespa facet request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("vespa facet failed ({status}): {text}");
        }
        let json: serde_json::Value = resp.json().await?;
        let mut results = Vec::new();
        if let Some(root_children) = json["root"]["children"].as_array() {
            for child in root_children {
                if let Some(group_list) = child["children"].as_array() {
                    for group in group_list {
                        let value = group["value"]
                            .as_str()
                            .map(|s| s.to_string())
                            .or_else(|| group["value"].as_i64().map(|n| n.to_string()))
                            .unwrap_or_default();
                        let count = group["fields"]["count()"].as_i64().unwrap_or(0);
                        if !value.is_empty() && count > 0 {
                            results.push((value, count));
                        }
                    }
                }
            }
        }
        Ok(results)
    }

    pub async fn search_chunks(
        &self,
        query: &str,
        query_embedding: Option<&[f32]>,
        project_id: Option<i64>,
        filters: &ChunkFilters,
        limit: i64,
    ) -> Result<Vec<ChunkSearchHit>> {
        let yql = build_chunk_search_yql(query_embedding.is_some(), project_id, filters);

        let ranking = if query_embedding.is_some() {
            "hybrid"
        } else {
            "default"
        };

        let mut request_body = json!({
            "yql": yql,
            "query": expand_legal_query(query),
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
        let json: Value = resp
            .json()
            .await
            .context("parse vespa chunk search response")?;
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
        let removed = retain_chunk_hits_for_project(&mut out, project_id);
        if removed > 0 {
            tracing::warn!(
                requested_project_id = project_id.unwrap_or_default(),
                removed = removed,
                "vespa chunk search returned cross-project hits; filtering them server-side"
            );
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
            let char_idx = trimmed
                .char_indices()
                .position(|(i, _)| i >= idx)
                .unwrap_or(0);
            let chars: Vec<char> = trimmed.chars().collect();
            let start = char_idx.saturating_sub(160);
            let end = (char_idx + term.chars().count() + 240).min(chars.len());
            return chars[start..end]
                .iter()
                .collect::<String>()
                .replace('\n', " ");
        }
    }
    trimmed
        .chars()
        .take(360)
        .collect::<String>()
        .replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_search_yql_applies_project_filter() {
        assert_eq!(
            build_document_search_yql(Some(7)),
            "select * from sources * where userQuery() and project_id = 7;"
        );
    }

    #[test]
    fn chunk_search_yql_groups_hybrid_clause_before_project_filter() {
        assert_eq!(
            build_chunk_search_yql(true, Some(7), &ChunkFilters::default()),
            "select * from project_chunk where (({targetHits:100}nearestNeighbor(embedding, q_embedding)) or userQuery()) and project_id = 7;"
        );
    }

    #[test]
    fn chunk_search_yql_includes_all_filters() {
        let yql = build_chunk_search_yql(
            true,
            Some(9),
            &ChunkFilters {
                doc_type: Some("contract".to_string()),
                jurisdiction: Some("England & Wales".to_string()),
                privileged_only: true,
                exclude_terms: vec!["board's".to_string()],
            },
        );

        assert!(yql.contains("and project_id = 9"));
        assert!(yql.contains("and doc_type contains 'contract'"));
        assert!(yql.contains("and jurisdiction contains 'England  Wales'"));
        assert!(yql.contains("and privileged = true"));
        assert!(yql.contains("and !(content contains 'boards')"));
    }

    #[test]
    fn retain_chunk_hits_for_project_drops_cross_project_hits() {
        let mut hits = vec![
            ChunkSearchHit {
                project_id: 1,
                file_id: 10,
                chunk_index: 0,
                file_path: "DOC-001.md".to_string(),
                title: "DOC-001.md".to_string(),
                content: "one".to_string(),
                doc_type: "contract".to_string(),
                score: 1.0,
            },
            ChunkSearchHit {
                project_id: 2,
                file_id: 11,
                chunk_index: 0,
                file_path: "DOC-002.md".to_string(),
                title: "DOC-002.md".to_string(),
                content: "two".to_string(),
                doc_type: "memo".to_string(),
                score: 0.5,
            },
        ];

        let removed = retain_chunk_hits_for_project(&mut hits, Some(1));

        assert_eq!(removed, 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].project_id, 1);
    }
}
