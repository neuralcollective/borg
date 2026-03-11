use std::{collections::HashSet, sync::Arc};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::internal;
use crate::AppState;

#[derive(Deserialize)]
pub(crate) struct ReindexQuery {
    project_id: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct FacetsQuery {
    project_id: i64,
}

#[derive(Deserialize)]
pub(crate) struct AgentSearchQuery {
    q: String,
    #[serde(default)]
    project_id: Option<i64>,
    #[serde(default = "default_agent_search_limit")]
    limit: i64,
    #[serde(default)]
    doc_type: Option<String>,
    #[serde(default)]
    jurisdiction: Option<String>,
    #[serde(default)]
    privileged_only: bool,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    exclude: Option<String>,
}
fn default_agent_search_limit() -> i64 {
    20
}

#[derive(Deserialize)]
pub(crate) struct AgentFileQuery {
    project_id: i64,
}

#[derive(Deserialize)]
pub(crate) struct AgentFilesQuery {
    project_id: i64,
    #[serde(default)]
    q: Option<String>,
    #[serde(default = "default_agent_files_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}
fn default_agent_files_limit() -> i64 {
    50
}

#[derive(Deserialize)]
pub(crate) struct CoverageQuery {
    q: String,
    project_id: i64,
    #[serde(default = "default_coverage_limit")]
    limit: i64,
    #[serde(default)]
    doc_type: Option<String>,
    #[serde(default)]
    model: Option<String>,
}
fn default_coverage_limit() -> i64 {
    100
}

pub(crate) async fn borgsearch_reindex(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ReindexQuery>,
) -> Result<Json<Value>, StatusCode> {
    let search = state
        .search
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .clone();
    let db = state.db.clone();

    let project_ids: Vec<i64> = if let Some(pid) = query.project_id {
        vec![pid]
    } else {
        db.list_projects()
            .map_err(internal)?
            .into_iter()
            .map(|p| p.id)
            .collect()
    };

    let total_projects = project_ids.len();
    let embed_reg = Arc::clone(&state.embed_registry);
    tokio::spawn(async move {
        let mut total_files = 0usize;
        let mut total_chunks = 0usize;
        for pid in &project_ids {
            let project_mode = db
                .get_project(*pid)
                .ok()
                .flatten()
                .map(|p| p.mode)
                .unwrap_or_default();
            let embed = embed_reg.client_for_mode(&project_mode);
            let files = match db.list_project_files(*pid) {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!("reindex: failed to list files for project {pid}: {e}");
                    continue;
                },
            };
            for file in &files {
                if file.extracted_text.is_empty() {
                    continue;
                }
                let _ = search.delete_file_chunks(*pid, file.id).await;
                let chunks_text = borg_core::knowledge::chunk_text(&file.extracted_text);
                if chunks_text.is_empty() {
                    continue;
                }
                let metadata = crate::vespa::ChunkMetadata {
                    doc_type: crate::ingestion::detect_doc_type(
                        &file.file_name,
                        &file.mime_type,
                        &file.extracted_text,
                    ),
                    jurisdiction: String::new(),
                    privileged: file.privileged,
                    mime_type: file.mime_type.clone(),
                };
                let mut chunks_with_embeddings: Vec<(String, Vec<f32>)> = Vec::new();
                for chunk in &chunks_text {
                    match embed.embed_document(chunk).await {
                        Ok(emb) => chunks_with_embeddings.push((chunk.clone(), emb)),
                        Err(_) => {
                            chunks_with_embeddings.push((chunk.clone(), embed.zero_embedding()))
                        },
                    }
                }
                total_chunks += chunks_with_embeddings.len();
                if let Err(e) = search
                    .index_chunks(
                        *pid,
                        file.id,
                        &file.file_name,
                        &file.file_name,
                        &chunks_with_embeddings,
                        &metadata,
                    )
                    .await
                {
                    tracing::warn!("reindex: chunk indexing failed for file {}: {e}", file.id);
                }
                total_files += 1;
            }
        }
        tracing::info!(
            "reindex complete: {total_projects} projects, {total_files} files, {total_chunks} chunks"
        );
    });

    Ok(Json(json!({
        "status": "started",
        "projects": total_projects,
    })))
}

pub(crate) async fn borgsearch_facets(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FacetsQuery>,
) -> Result<Json<Value>, StatusCode> {
    let search = state
        .search
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let doc_types = search
        .facet_counts(query.project_id, "doc_type")
        .await
        .unwrap_or_default();
    let jurisdictions = search
        .facet_counts(query.project_id, "jurisdiction")
        .await
        .unwrap_or_default();
    Ok(Json(json!({
        "doc_types": doc_types.into_iter().map(|(v, c)| json!({"value": v, "count": c})).collect::<Vec<_>>(),
        "jurisdictions": jurisdictions.into_iter().map(|(v, c)| json!({"value": v, "count": c})).collect::<Vec<_>>(),
    })))
}

pub(crate) async fn agent_search(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    Query(query): Query<AgentSearchQuery>,
) -> Result<String, StatusCode> {
    if query.q.trim().is_empty() {
        return Ok("No query provided.".to_string());
    }

    let limit = query.limit.clamp(1, 100);
    let exclude_terms: Vec<String> = query
        .exclude
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let filters = crate::vespa::ChunkFilters {
        doc_type: query.doc_type.clone(),
        jurisdiction: query.jurisdiction.clone(),
        privileged_only: query.privileged_only,
        exclude_terms,
    };

    if let Some(search) = &state.search {
        let embed_client = match query.model.as_deref() {
            Some(m) => state.embed_registry.client(m),
            None => state.embed_registry.default_client(),
        };
        let query_emb = embed_client.embed_query(&query.q).await.ok();
        let emb_ref = query_emb.as_deref();

        match search
            .search_chunks(&query.q, emb_ref, query.project_id, &filters, limit)
            .await
        {
            Ok(hits) if !hits.is_empty() => {
                let mut seen_files: HashSet<i64> = HashSet::new();
                let hits: Vec<_> = hits
                    .into_iter()
                    .filter(|h| seen_files.insert(h.file_id))
                    .collect();

                let mut out = format!("Search results for: {}\n", query.q);
                if let Some(dt) = &query.doc_type {
                    out.push_str(&format!("Filter: doc_type={}\n", dt));
                }
                if let Some(j) = &query.jurisdiction {
                    out.push_str(&format!("Filter: jurisdiction={}\n", j));
                }
                out.push('\n');
                for (i, hit) in hits.iter().enumerate() {
                    out.push_str(&format!(
                        "--- Result {} (score: {:.3}, type: {}) ---\nFile: {} [id={}, chunk={}]\n{}\n\n",
                        i + 1,
                        hit.score,
                        if hit.doc_type.is_empty() { "unknown" } else { &hit.doc_type },
                        hit.file_path,
                        hit.file_id,
                        hit.chunk_index,
                        hit.content,
                    ));
                }
                tracing::info!(
                    target: "instrumentation.search",
                    message = "agent search completed",
                    user_id = user.id,
                    username = user.username.as_str(),
                    project_id = query.project_id,
                    limit = limit,
                    query_len = query.q.chars().count() as u64,
                    result_count = hits.len() as u64,
                    doc_type = query.doc_type.as_deref().unwrap_or(""),
                    jurisdiction = query.jurisdiction.as_deref().unwrap_or(""),
                    privileged_only = query.privileged_only,
                    source = "chunk_hybrid",
                );
                return Ok(out);
            },
            Ok(_) => {},
            Err(e) => {
                tracing::warn!("chunk search failed, falling back: {e}");
            },
        }
    }

    let mut results: Vec<(String, String, f64, String)> = Vec::new();

    if let Some(search) = &state.search {
        if let Ok(hits) = search.search(&query.q, query.project_id, limit).await {
            for r in hits {
                let snippet = if !r.content_snippet.is_empty() {
                    r.content_snippet.clone()
                } else {
                    r.title_snippet.clone()
                };
                results.push((
                    r.file_path,
                    snippet,
                    r.score,
                    search.backend_name().to_string(),
                ));
            }
        }
    }

    if state.db.embedding_count() > 0 {
        let fallback_ec = match query.model.as_deref() {
            Some(m) => state.embed_registry.client(m),
            None => state.embed_registry.default_client(),
        };
        if let Ok(query_emb) = fallback_ec.embed_query(&query.q).await {
            if let Ok(sem) =
                state
                    .db
                    .search_embeddings(&query_emb, limit as usize, query.project_id)
            {
                for r in sem.iter().filter(|r| r.score > 0.5) {
                    let already = results.iter().any(|(p, _, _, _)| *p == r.file_path);
                    if !already {
                        let snippet = if r.chunk_text.len() > 300 {
                            format!(
                                "{}...",
                                &r.chunk_text[..r.chunk_text.floor_char_boundary(300)]
                            )
                        } else {
                            r.chunk_text.clone()
                        };
                        results.push((
                            r.file_path.clone(),
                            snippet,
                            r.score.into(),
                            "semantic".to_string(),
                        ));
                    }
                }
            }
        }
    }

    if results.is_empty() {
        tracing::info!(
            target: "instrumentation.search",
            message = "agent search completed",
            user_id = user.id,
            username = user.username.as_str(),
            project_id = query.project_id,
            limit = limit,
            query_len = query.q.chars().count() as u64,
            result_count = 0u64,
            doc_type = query.doc_type.as_deref().unwrap_or(""),
            jurisdiction = query.jurisdiction.as_deref().unwrap_or(""),
            privileged_only = query.privileged_only,
            source = "fallback",
        );
        return Ok(format!("No results found for: {}", query.q));
    }

    results.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit as usize);

    let mut out = format!("Search results for: {}\n\n", query.q);
    for (i, (path, snippet, score, source)) in results.iter().enumerate() {
        out.push_str(&format!(
            "--- Result {} (score: {:.3}, source: {}) ---\nFile: {}\n{}\n\n",
            i + 1,
            score,
            source,
            path,
            snippet
        ));
    }
    tracing::info!(
        target: "instrumentation.search",
        message = "agent search completed",
        user_id = user.id,
        username = user.username.as_str(),
        project_id = query.project_id,
        limit = limit,
        query_len = query.q.chars().count() as u64,
        result_count = results.len() as u64,
        doc_type = query.doc_type.as_deref().unwrap_or(""),
        jurisdiction = query.jurisdiction.as_deref().unwrap_or(""),
        privileged_only = query.privileged_only,
        source = "fallback",
    );
    Ok(out)
}

pub(crate) async fn agent_get_file(
    State(state): State<Arc<AppState>>,
    Path(file_id): Path<i64>,
    Query(query): Query<AgentFileQuery>,
) -> Result<String, StatusCode> {
    let file = state
        .db
        .get_project_file(query.project_id, file_id)
        .map_err(internal)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let text = if !file.extracted_text.trim().is_empty() {
        file.extracted_text.clone()
    } else if !super::projects::is_binary_mime(&file.mime_type) {
        match state.file_storage.read_all(&file.stored_path).await {
            Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
            Err(_) => return Err(StatusCode::NOT_FOUND),
        }
    } else {
        return Ok(format!(
            "File: {}\nType: {}\nSize: {} bytes\n\n(Binary file — no text content available)",
            file.file_name, file.mime_type, file.size_bytes
        ));
    };

    let mut out = format!(
        "File: {}\nPath: {}\nType: {}\nSize: {} bytes\n\n",
        file.file_name, file.source_path, file.mime_type, file.size_bytes
    );
    out.push_str(&text);
    Ok(out)
}

pub(crate) async fn agent_list_files(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AgentFilesQuery>,
) -> Result<String, StatusCode> {
    let (files, total) = state
        .db
        .list_project_file_page(
            query.project_id,
            query.q.as_deref(),
            query.limit.clamp(1, 200),
            query.offset.max(0),
            None,
            Some(true),
            None,
        )
        .map_err(internal)?;

    if files.is_empty() {
        return Ok(format!(
            "No files found for project {} (total: {}).",
            query.project_id, total
        ));
    }

    let mut out = format!(
        "Project files (showing {}-{} of {}):\n\n",
        query.offset + 1,
        query.offset + files.len() as i64,
        total
    );
    for f in &files {
        out.push_str(&format!(
            "  [id={}] {} ({}, {} bytes)\n",
            f.id, f.source_path, f.mime_type, f.size_bytes
        ));
    }
    out.push_str(&format!(
        "\nUse /api/borgsearch/file/<id>?project_id={} to read a file's content.",
        query.project_id
    ));
    Ok(out)
}

pub(crate) async fn agent_coverage(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CoverageQuery>,
) -> Result<String, StatusCode> {
    let (all_files, total) = state
        .db
        .list_project_file_page(query.project_id, None, 10000, 0, None, Some(true), None)
        .map_err(internal)?;

    if all_files.is_empty() {
        return Ok(format!("No files found for project {}.", query.project_id));
    }

    let limit = query.limit.clamp(1, 500);
    let filters = crate::vespa::ChunkFilters {
        doc_type: query.doc_type.clone(),
        ..Default::default()
    };

    let mut matched_file_ids: HashSet<i64> = HashSet::new();

    if let Some(search) = &state.search {
        let embed_client = match query.model.as_deref() {
            Some(m) => state.embed_registry.client(m),
            None => state.embed_registry.default_client(),
        };
        let query_emb = embed_client.embed_query(&query.q).await.ok();
        let emb_ref = query_emb.as_deref();

        if let Ok(hits) = search
            .search_chunks(&query.q, emb_ref, Some(query.project_id), &filters, limit)
            .await
        {
            for h in &hits {
                matched_file_ids.insert(h.file_id);
            }
        }
    }

    if state.db.embedding_count() > 0 {
        let ec = match query.model.as_deref() {
            Some(m) => state.embed_registry.client(m),
            None => state.embed_registry.default_client(),
        };
        if let Ok(emb) = ec.embed_query(&query.q).await {
            if let Ok(sem) = state
                .db
                .search_embeddings(&emb, 500, Some(query.project_id))
            {
                for r in sem.iter().filter(|r| r.score > 0.4) {
                    if let Some(f) = all_files.iter().find(|f| f.source_path == r.file_path) {
                        matched_file_ids.insert(f.id);
                    }
                }
            }
        }
    }

    let mut matched = Vec::new();
    let mut unmatched = Vec::new();
    for f in &all_files {
        if matched_file_ids.contains(&f.id) {
            matched.push(f);
        } else {
            unmatched.push(f);
        }
    }

    let pct = if total > 0 {
        (matched.len() as f64 / total as f64 * 100.0).round() as i64
    } else {
        0
    };

    let mut out =
        format!(
        "## Coverage Report: \"{}\"\n\nTotal documents: {}\nMatched: {} ({}%)\nNot matched: {}\n\n",
        query.q, total, matched.len(), pct, unmatched.len()
    );

    if !unmatched.is_empty() {
        out.push_str("### Documents NOT matching query:\n\n");
        for f in &unmatched {
            out.push_str(&format!(
                "  [id={}] {} ({}, {} bytes)\n",
                f.id, f.source_path, f.mime_type, f.size_bytes
            ));
        }
        out.push('\n');
    }

    if !matched.is_empty() {
        out.push_str("### Documents matching query:\n\n");
        for f in &matched {
            out.push_str(&format!(
                "  [id={}] {} ({}, {} bytes)\n",
                f.id, f.source_path, f.mime_type, f.size_bytes
            ));
        }
    }

    Ok(out)
}
