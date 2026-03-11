use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        Json,
    },
};
use borg_core::{
    config::{refresh_oauth_token, Config},
    db::Db,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::AsyncBufReadExt;
use tokio::sync::{broadcast, Mutex as TokioMutex};
use tokio_stream::{wrappers::UnboundedReceiverStream, StreamExt};

use super::{internal, require_project_access};
use crate::{storage::FileStorage, AppState};

#[derive(Deserialize)]
pub(crate) struct ChatMessagesQuery {
    pub thread: String,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct ChatPostBody {
    pub text: String,
    pub sender: Option<String>,
    pub thread: Option<String>,
}

pub(crate) fn sanitize_chat_key(key: &str) -> String {
    key.chars()
        .take(128)
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn project_chat_key(project_id: i64) -> String {
    format!("project:{project_id}")
}

pub(crate) fn parse_project_chat_key(chat_key: &str) -> Option<i64> {
    chat_key.strip_prefix("project:")?.parse::<i64>().ok()
}

pub(crate) fn workspace_chat_prefix(workspace_id: i64) -> String {
    format!("web:workspace:{workspace_id}:")
}

pub(crate) fn scoped_workspace_chat_thread(workspace_id: i64, requested: &str) -> String {
    let requested = requested.trim();
    let requested = if requested.is_empty() {
        "dashboard"
    } else {
        requested
    };
    format!(
        "{}{}",
        workspace_chat_prefix(workspace_id),
        sanitize_chat_key(requested)
    )
}

pub(crate) fn visible_workspace_chat_thread(workspace_id: i64, chat_jid: &str) -> Option<String> {
    chat_jid
        .strip_prefix(&workspace_chat_prefix(workspace_id))
        .map(|s| s.to_string())
}

pub(crate) fn visible_chat_thread_for_workspace(
    db: &Db,
    workspace_id: i64,
    chat_jid: &str,
) -> Option<String> {
    if let Some(thread) = visible_workspace_chat_thread(workspace_id, chat_jid) {
        return Some(thread);
    }
    let project_id = parse_project_chat_key(chat_jid)?;
    db.get_project_in_workspace(workspace_id, project_id)
        .ok()
        .flatten()
        .map(|_| chat_jid.to_string())
}

pub(crate) async fn run_chat_agent(
    chat_key: &str,
    run_id: &str,
    sender_name: &str,
    messages: &[String],
    sessions: &Arc<TokioMutex<HashMap<String, String>>>,
    config: &Config,
    db: &Arc<Db>,
    search: Option<Arc<crate::search::SearchClient>>,
    storage: &Arc<FileStorage>,
    chat_event_tx: &broadcast::Sender<String>,
    ai_request_count: &Arc<AtomicU64>,
) -> anyhow::Result<String> {
    let session_dir = format!(
        "{}/sessions/chat-{}",
        config.data_dir,
        sanitize_chat_key(chat_key)
    );
    std::fs::create_dir_all(&session_dir)?;

    let ts_secs = Utc::now().timestamp();
    for (i, msg) in messages.iter().enumerate() {
        let msg_id = format!("{}-{}-{}", chat_key, ts_secs, i);
        let _ = db.insert_chat_message(
            &msg_id,
            chat_key,
            Some(sender_name),
            Some(sender_name),
            msg,
            false,
            false,
        );
        let event = json!({
            "role": "user",
            "sender": sender_name,
            "text": msg,
            "ts": ts_secs,
            "thread": chat_key,
            "run_id": run_id,
        })
        .to_string();
        let _ = chat_event_tx.send(event);
    }

    let retrieval_query = messages.join("\n");
    let project_for_chat =
        parse_project_chat_key(chat_key).and_then(|pid| db.get_project(pid).ok().flatten());
    let prompt = if messages.len() == 1 {
        format!("{} says: {}", sender_name, messages[0])
    } else {
        let joined: Vec<String> = messages.iter().map(|m| format!("- {m}")).collect();
        format!("{} says:\n{}", sender_name, joined.join("\n"))
    };
    let prompt = if let Some(project) = project_for_chat.as_ref() {
        let ctx = super::projects::build_project_context(
            project,
            &retrieval_query,
            &session_dir,
            db,
            search.as_deref(),
            storage,
        )
        .await;
        if ctx.is_empty() {
            prompt
        } else {
            format!("{ctx}\n\nUser request:\n{prompt}")
        }
    } else {
        prompt
    };

    let mut system_prompt = config.chat_system_prompt();

    let project_mode = project_for_chat.as_ref().map(|p| p.mode.clone());
    let is_legal = matches!(project_mode.as_deref(), Some("lawborg" | "legal"));

    if is_legal {
        system_prompt.push_str(borg_domains::legal::legal_chat_system_suffix());
    }

    let knowledge_files = project_for_chat
        .as_ref()
        .and_then(|project| {
            db.list_all_knowledge_in_workspace(
                project.workspace_id,
                Some(&retrieval_query),
                Some(project.jurisdiction.as_str()),
                80,
            )
            .ok()
        })
        .unwrap_or_default();
    if !knowledge_files.is_empty() {
        let knowledge_dir = format!(
            "{}/knowledge/workspaces/{}",
            config.data_dir,
            project_for_chat
                .as_ref()
                .map(|p| p.workspace_id)
                .unwrap_or_default()
        );
        let selected = borg_agent::instruction::select_relevant_knowledge_files(
            &knowledge_files,
            &retrieval_query,
            project_mode.as_deref(),
            project_for_chat.as_ref().map(|p| p.jurisdiction.as_str()),
            project_for_chat.as_ref().map(|p| p.id),
            24,
        );
        let kb = borg_agent::instruction::build_knowledge_section(&selected, &knowledge_dir);
        if !kb.is_empty() {
            system_prompt.push('\n');
            system_prompt.push_str(&kb);
        }
    }

    let mut args = vec![
        "--model".to_string(),
        config.model.clone(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--dangerously-skip-permissions".to_string(),
        "--max-turns".to_string(),
        "64".to_string(),
        "--append-system-prompt".to_string(),
        system_prompt,
    ];

    if let Ok(Some(disallowed)) = db.get_config("chat_disallowed_tools") {
        let disallowed = disallowed.trim();
        if !disallowed.is_empty() {
            args.push("--disallowedTools".to_string());
            args.push(disallowed.to_string());
        }
    }

    let api_url = format!("http://127.0.0.1:{}", config.web_port);
    let api_token =
        std::fs::read_to_string(format!("{}/.api-token", config.data_dir)).unwrap_or_default();

    let mut mcp_servers = serde_json::Map::new();

    let borg_mcp_path = if let Ok(p) = std::env::var("BORG_MCP_SERVER") {
        std::path::PathBuf::from(p)
    } else {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../sidecar/borg-mcp/server.js")
    };
    if let Ok(mcp_server) = borg_mcp_path.canonicalize() {
        let mut env_vars = serde_json::Map::new();
        env_vars.insert("API_BASE_URL".into(), json!(api_url));
        if !api_token.is_empty() {
            env_vars.insert("API_TOKEN".into(), json!(api_token));
        }
        env_vars.insert("CHAT_THREAD".into(), json!(chat_key));
        if let Some(ref p) = project_for_chat {
            env_vars.insert("PROJECT_ID".into(), json!(p.id.to_string()));
            env_vars.insert("PROJECT_MODE".into(), json!(&p.mode));
        }
        mcp_servers.insert(
            "borg".into(),
            json!({
                "command": "bun",
                "args": ["run", mcp_server],
                "env": env_vars,
            }),
        );
    } else {
        tracing::warn!(chat_key, path = %borg_mcp_path.display(), "borg-mcp not found");
    }

    if is_legal {
        let legal_mcp_path = if let Ok(p) = std::env::var("LAWBORG_MCP_SERVER") {
            std::path::PathBuf::from(p)
        } else {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../sidecar/lawborg-mcp/server.js")
        };
        if let Ok(mcp_server) = legal_mcp_path.canonicalize() {
            tracing::info!(chat_key, path = %mcp_server.display(), "wiring lawborg-mcp for chat");
            let mut env_vars = serde_json::Map::new();
            let providers = [
                "lexisnexis",
                "westlaw",
                "clio",
                "imanage",
                "netdocuments",
                "congress",
                "openstates",
                "canlii",
                "regulations_gov",
            ];
            for provider in providers {
                if let Ok(Some(key)) = db.get_api_key("global", provider) {
                    let env_name = match provider {
                        "lexisnexis" => "LEXISNEXIS_API_KEY",
                        "westlaw" => "WESTLAW_API_KEY",
                        "clio" => "CLIO_API_KEY",
                        "imanage" => "IMANAGE_API_KEY",
                        "netdocuments" => "NETDOCUMENTS_API_KEY",
                        "congress" => "CONGRESS_API_KEY",
                        "openstates" => "OPENSTATES_API_KEY",
                        "canlii" => "CANLII_API_KEY",
                        "regulations_gov" => "REGULATIONS_GOV_API_KEY",
                        _ => continue,
                    };
                    env_vars.insert(env_name.into(), serde_json::Value::String(key));
                }
            }
            mcp_servers.insert(
                "legal".into(),
                json!({
                    "command": "bun",
                    "args": ["run", mcp_server],
                    "env": env_vars,
                }),
            );
        } else {
            tracing::warn!(chat_key, path = %legal_mcp_path.display(), "lawborg-mcp not found");
        }
    }

    if !mcp_servers.is_empty() {
        let config_json = json!({ "mcpServers": mcp_servers });
        let mcp_json_path = format!("{session_dir}/.mcp.json");
        if let Err(e) = std::fs::write(&mcp_json_path, config_json.to_string()) {
            tracing::warn!(chat_key, "failed to write .mcp.json: {e}");
        }
        args.push("--mcp-config".to_string());
        args.push(mcp_json_path);
    }

    let session_id = sessions.lock().await.get(chat_key).cloned().or_else(|| {
        db.get_session(&format!("chat-{}", sanitize_chat_key(chat_key)))
            .ok()
            .flatten()
    });
    if let Some(ref sid) = session_id {
        args.push("--resume".to_string());
        args.push(sid.clone());
    }

    args.push("--print".to_string());
    args.push(prompt);

    let token = refresh_oauth_token(&config.credentials_path, &config.oauth_token);

    if !api_token.is_empty() {
        let project_id_hint = project_for_chat
            .as_ref()
            .map(|p| {
                format!(
                    "Current project_id: {}\nCurrent project mode: {}\n",
                    p.id, p.mode
                )
            })
            .unwrap_or_default();
        let agent_claude_md = format!(
            "# Borg Chat Agent\n\n\
             {project_id_hint}\
             ## Tools\n\n\
             You have MCP tools for document search and pipeline task management:\n\
             - `search_documents` — hybrid semantic search across project documents. ALWAYS use this when the user asks about document content, especially with large document sets. Search iteratively with different queries rather than trying to read files one by one.\n\
             - `list_documents` — browse available documents by name\n\
             - `read_document` — read full text of a specific document by ID\n\
             - `create_task` — create a pipeline task for long-running async work (code changes, document generation, multi-step research). Ask clarifying questions first.\n\
             - `get_task_status` — check progress on a pipeline task\n\
             - `list_project_tasks` — see all tasks for the current project\n\n\
             ## When to search vs create a task\n\n\
             If BorgSearch returns `no_project_corpus`, ask the user to select or attach the relevant matter/project before continuing.\n\
             If a task needs exhaustive review of the attached matter corpus, set `requires_exhaustive_corpus_review=true` when creating it.\n\
             \n\
             - User asks a question about their documents → search_documents (may need multiple searches)\n\
             - User wants a document drafted, code changed, or complex multi-step work → create_task\n\
             - User asks about task status or project progress → get_task_status / list_project_tasks\n\
             - Quick factual question → answer directly\n",
        );
        let claude_md_path = format!("{session_dir}/CLAUDE.md");
        let _ = std::fs::write(&claude_md_path, &agent_claude_md);
    }

    let timeout = std::time::Duration::from_secs(config.agent_timeout_s.max(300) as u64);
    ai_request_count.fetch_add(1, Ordering::Relaxed);
    let mut child = tokio::process::Command::new("claude")
        .args(&args)
        .current_dir(&session_dir)
        .env("HOME", &session_dir)
        .env("CLAUDE_CODE_OAUTH_TOKEN", &token)
        .env("API_BASE_URL", &api_url)
        .env("API_TOKEN", &api_token)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().expect("stdout piped");
    let mut reader = tokio::io::BufReader::new(stdout).lines();
    let mut raw_lines: Vec<String> = Vec::new();
    let stream_result = tokio::time::timeout(timeout, async {
        while let Some(line) = reader.next_line().await? {
            raw_lines.push(line.clone());
            let stream_event = json!({
                "type": "chat_stream",
                "thread": chat_key,
                "run_id": run_id,
                "data": line,
            })
            .to_string();
            let _ = chat_event_tx.send(stream_event);
        }
        Ok::<(), anyhow::Error>(())
    })
    .await;

    let status = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait())
        .await
        .ok()
        .and_then(|r| r.ok());

    if let Err(_) = stream_result {
        let _ = child.kill().await;
        anyhow::bail!("chat agent timed out after {}s", timeout.as_secs());
    }

    if let Some(st) = status {
        if !st.success() {
            tracing::warn!("chat agent failed ({}) exit={:?}", chat_key, st.code());
        }
    }

    let raw = raw_lines.join("\n");
    let (text, new_session_id) = borg_agent::event::parse_stream(&raw);

    if let Some(sid) = new_session_id {
        sessions
            .lock()
            .await
            .insert(chat_key.to_string(), sid.clone());
        let folder = format!("chat-{}", sanitize_chat_key(chat_key));
        let _ = db.set_session(&folder, &sid);
    }

    if !text.is_empty() {
        let reply_ts = Utc::now().timestamp();
        let reply_id = format!("{}-bot-{}", chat_key, reply_ts);
        let stream_data = if raw.is_empty() {
            None
        } else {
            Some(raw.as_str())
        };
        let _ = db.insert_chat_message_with_stream(
            &reply_id,
            chat_key,
            Some("borg"),
            Some("borg"),
            &text,
            true,
            true,
            stream_data,
        );
        let event = json!({
            "role": "assistant",
            "sender": "borg",
            "text": &text,
            "ts": reply_ts,
            "thread": chat_key,
            "run_id": run_id,
        })
        .to_string();
        let _ = chat_event_tx.send(event);
    }

    Ok(text)
}

pub(crate) async fn sse_chat_events(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut live_rx = state.chat_event_tx.subscribe();
    let db = Arc::clone(&state.db);
    tokio::spawn(async move {
        loop {
            match live_rx.recv().await {
                Ok(line) => {
                    let Some(filtered) =
                        serde_json::from_str::<Value>(&line)
                            .ok()
                            .and_then(|mut payload| {
                                let thread = payload.get("thread")?.as_str()?;
                                let visible = visible_chat_thread_for_workspace(
                                    db.as_ref(),
                                    workspace.id,
                                    thread,
                                )?;
                                if let Some(obj) = payload.as_object_mut() {
                                    obj.insert("thread".into(), Value::String(visible));
                                }
                                serde_json::to_string(&payload).ok()
                            })
                    else {
                        continue;
                    };
                    if tx.send(filtered).is_err() {
                        return;
                    }
                },
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("chat SSE client lagged by {n} events, continuing");
                    continue;
                },
                Err(e) => {
                    tracing::debug!("chat SSE broadcast closed: {e}");
                    break;
                },
            }
        }
    });
    let stream = UnboundedReceiverStream::new(rx)
        .map(|data| Ok::<_, std::convert::Infallible>(Event::default().data(data)));
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    )
}

pub(crate) async fn get_chat_threads(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
) -> Result<Json<Value>, StatusCode> {
    let threads = state.db.get_chat_threads().map_err(internal)?;
    let v: Vec<Value> = threads
        .into_iter()
        .filter_map(|(jid, count, last_ts)| {
            visible_chat_thread_for_workspace(state.db.as_ref(), workspace.id, &jid).map(
                |visible_id| json!({ "id": visible_id, "message_count": count, "last_ts": last_ts }),
            )
        })
        .collect();
    Ok(Json(json!(v)))
}

pub(crate) async fn get_chat_messages(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Query(q): Query<ChatMessagesQuery>,
) -> Result<Json<Value>, StatusCode> {
    let actual_thread = if parse_project_chat_key(&q.thread).is_some() {
        let project_id = parse_project_chat_key(&q.thread).ok_or(StatusCode::BAD_REQUEST)?;
        require_project_access(state.as_ref(), &workspace, project_id)?;
        q.thread.clone()
    } else {
        scoped_workspace_chat_thread(workspace.id, &q.thread)
    };
    let msgs = match state
        .db
        .get_chat_messages(&actual_thread, q.limit.unwrap_or(100))
    {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("get_chat_messages({}): {e}", actual_thread);
            return Ok(Json(json!([])));
        },
    };
    let v: Vec<Value> = msgs
        .iter()
        .map(|m| {
            let mut obj = json!({
                "role": if m.is_from_me { "assistant" } else { "user" },
                "sender": m.sender_name,
                "text": m.content,
                "ts": m.timestamp,
                "thread": visible_chat_thread_for_workspace(state.db.as_ref(), workspace.id, &m.chat_jid)
                    .unwrap_or_else(|| q.thread.clone()),
            });
            if let Some(ref rs) = m.raw_stream {
                obj["raw_stream"] = json!(rs);
            }
            obj
        })
        .collect();
    Ok(Json(json!(v)))
}

pub(crate) async fn get_project_chat_messages(
    State(state): State<Arc<AppState>>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Query(q): Query<super::projects::ProjectFilesQuery>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    let thread = project_chat_key(id);
    let msgs = state
        .db
        .get_chat_messages(&thread, q.limit.unwrap_or(200))
        .map_err(internal)?;
    let v: Vec<Value> = msgs
        .iter()
        .map(|m| {
            let mut obj = json!({
                "role": if m.is_from_me { "assistant" } else { "user" },
                "sender": m.sender_name,
                "text": m.content,
                "ts": m.timestamp,
                "thread": m.chat_jid,
            });
            if let Some(ref rs) = m.raw_stream {
                obj["raw_stream"] = json!(rs);
            }
            obj
        })
        .collect();
    Ok(Json(json!(v)))
}

pub(crate) async fn post_project_chat(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Path(id): Path<i64>,
    Json(body): Json<ChatPostBody>,
) -> Result<Json<Value>, StatusCode> {
    let _project = require_project_access(state.as_ref(), &workspace, id)?;
    let thread = project_chat_key(id);
    let sender = body
        .sender
        .clone()
        .unwrap_or_else(|| "web-user".to_string());
    tracing::info!(
        target: "instrumentation.chat",
        message = "project chat submitted",
        user_id = user.id,
        username = user.username.as_str(),
        project_id = id,
        thread = thread.as_str(),
        sender = sender.as_str(),
        text_len = body.text.chars().count() as u64,
    );

    let state2 = Arc::clone(&state);
    let thread2 = thread.clone();
    let sender2 = sender.clone();
    let text2 = body.text.clone();
    tokio::spawn(async move {
        let run_id = crate::messaging_progress::new_chat_run_id();
        match run_chat_agent(
            &thread2,
            &run_id,
            &sender2,
            &[text2],
            &state2.web_sessions,
            &state2.config,
            &state2.db,
            state2.search.clone(),
            &state2.file_storage,
            &state2.chat_event_tx,
            &state2.ai_request_count,
        )
        .await
        {
            Ok(_) => {},
            Err(e) => tracing::warn!("project chat agent error: {e}"),
        }
    });

    Ok(Json(json!({ "ok": true })))
}

pub(crate) async fn post_chat(
    State(state): State<Arc<AppState>>,
    axum::Extension(user): axum::Extension<crate::auth::AuthUser>,
    axum::Extension(workspace): axum::Extension<crate::auth::WorkspaceContext>,
    Json(body): Json<ChatPostBody>,
) -> Result<Json<Value>, StatusCode> {
    if body.text.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let requested_thread = body
        .thread
        .clone()
        .unwrap_or_else(|| "dashboard".to_string());
    if parse_project_chat_key(&requested_thread).is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let thread = scoped_workspace_chat_thread(workspace.id, &requested_thread);

    let rate = state.config.chat_rate_limit.max(1) as u64;
    let cooldown = std::time::Duration::from_secs(60 / rate);
    {
        let mut map = state.chat_rate.lock().unwrap_or_else(|e| e.into_inner());
        let now = std::time::Instant::now();
        if map.len() > 1000 {
            map.retain(|_, last| now.duration_since(*last) < cooldown * 10);
        }
        if let Some(last) = map.get(&thread) {
            if now.duration_since(*last) < cooldown {
                return Err(StatusCode::TOO_MANY_REQUESTS);
            }
        }
        map.insert(thread.clone(), now);
    }
    let sender = body
        .sender
        .clone()
        .unwrap_or_else(|| "web-user".to_string());
    tracing::info!(
        target: "instrumentation.chat",
        message = "chat submitted",
        user_id = user.id,
        username = user.username.as_str(),
        thread = thread.as_str(),
        sender = sender.as_str(),
        text_len = body.text.chars().count() as u64,
    );

    let state2 = Arc::clone(&state);
    let thread2 = thread.clone();
    let sender2 = sender.clone();
    let text2 = body.text.clone();
    tokio::spawn(async move {
        let run_id = crate::messaging_progress::new_chat_run_id();
        match run_chat_agent(
            &thread2,
            &run_id,
            &sender2,
            &[text2],
            &state2.web_sessions,
            &state2.config,
            &state2.db,
            state2.search.clone(),
            &state2.file_storage,
            &state2.chat_event_tx,
            &state2.ai_request_count,
        )
        .await
        {
            Ok(_) => {},
            Err(e) => tracing::warn!("web chat agent error: {e}"),
        }
    });

    Ok(Json(json!({ "ok": true })))
}
