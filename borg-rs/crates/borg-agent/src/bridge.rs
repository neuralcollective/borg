use std::{
    collections::HashMap,
    path::Path,
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use borg_core::{
    agent::AgentBackend,
    traits::{
        BackendCapabilities, ChatContext, ChatRequest, ChatResponse, ProviderConfig, ToolCallRecord,
    },
    types::{PhaseConfig, PhaseContext, PhaseOutput, Task},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
};
use tracing::{debug, info, warn};

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_request_id() -> String {
    format!("req-{}", REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed))
}

// ── Protocol types ───────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct QueryRequest {
    r#type: String,
    id: String,
    prompt: String,
    options: QueryOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    allowed_tools: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    disallowed_tools: Vec<String>,
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    mcp_servers: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_turns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_budget_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resume: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    env: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
enum BridgeEvent {
    TextDelta { id: String, content: String },
    ToolUse { id: String, tool: String, input: serde_json::Value, timestamp: f64 },
    ToolResult { id: String, tool: String, output: String, duration_ms: u64, success: bool },
    Result { id: String, text: String, session_id: String, usage: UsageInfo, cost_usd: f64 },
    Error { id: String, message: String, code: String },
    Stop { id: String, reason: String, usage: UsageInfo },
}

#[derive(Debug, Deserialize, Default, Clone)]
struct UsageInfo {
    input_tokens: u64,
    output_tokens: u64,
}

// ── Bridge Process ───────────────────────────────────────────────────────

/// Path to the agent-bridge entry point, resolved relative to this crate.
fn resolve_bridge_path() -> String {
    if let Ok(p) = std::env::var("BORG_AGENT_BRIDGE") {
        return p;
    }
    // Resolve relative to the sidecar directory
    let manifest = env!("CARGO_MANIFEST_DIR");
    let candidate = Path::new(manifest).join("../../sidecar/agent-bridge/src/index.ts");
    if candidate.exists() {
        return candidate.to_string_lossy().to_string();
    }
    // Fallback: assume it's adjacent in the project root
    let alt = Path::new(manifest).join("../../../sidecar/agent-bridge/src/index.ts");
    alt.to_string_lossy().to_string()
}

/// Runs a single query through the agent bridge (spawn per-query for simplicity).
async fn run_bridge_query(
    request: &QueryRequest,
    provider: &ProviderConfig,
    timeout_s: u64,
    stream_tx: Option<&tokio::sync::mpsc::UnboundedSender<String>>,
) -> Result<BridgeResult> {
    let bridge_path = resolve_bridge_path();

    let mut cmd = Command::new("bun");
    cmd.arg("run")
        .arg(&bridge_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    // Pass provider env vars to the bridge process
    for (k, v) in provider.to_env_vars() {
        cmd.env(&k, &v);
    }

    // Pass through oauth token if set
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        cmd.env("CLAUDE_CODE_OAUTH_TOKEN", &token);
    }

    let mut child = cmd.spawn().context("failed to spawn agent-bridge")?;

    // Write request to stdin
    let request_json = serde_json::to_string(request)?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(request_json.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        drop(stdin);
    }

    // Read events from stdout
    let stdout = child.stdout.take().context("no stdout")?;
    let stderr = child.stderr.take().context("no stderr")?;

    let stream_tx_clone = stream_tx.cloned();
    let io_future = async move {
        let mut result = BridgeResult::default();
        let mut text_parts: Vec<String> = Vec::new();
        let mut tool_calls: Vec<ToolCallRecord> = Vec::new();
        let mut pending_tool: Option<(String, String)> = None;

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();
        let mut stdout_done = false;
        let mut stderr_done = false;

        while !stdout_done || !stderr_done {
            tokio::select! {
                line = stdout_reader.next_line(), if !stdout_done => {
                    match line {
                        Ok(Some(l)) => {
                            if l.trim().is_empty() { continue; }

                            // Forward raw line to stream
                            if let Some(tx) = &stream_tx_clone {
                                let _ = tx.send(l.clone());
                            }

                            match serde_json::from_str::<BridgeEvent>(&l) {
                                Ok(BridgeEvent::TextDelta { content, .. }) => {
                                    text_parts.push(content);
                                }
                                Ok(BridgeEvent::ToolUse { tool, input, .. }) => {
                                    let input_str = serde_json::to_string(&input).unwrap_or_default();
                                    let summary = if input_str.len() > 200 {
                                        format!("{}...", &input_str[..200])
                                    } else {
                                        input_str
                                    };
                                    pending_tool = Some((tool, summary));
                                }
                                Ok(BridgeEvent::ToolResult { tool, output, duration_ms, success, .. }) => {
                                    let input_summary = pending_tool
                                        .take()
                                        .map(|(_, s)| s)
                                        .unwrap_or_default();
                                    let out_summary = if output.len() > 200 {
                                        format!("{}...", &output[..200])
                                    } else {
                                        output
                                    };
                                    tool_calls.push(ToolCallRecord {
                                        tool_name: tool,
                                        input_summary,
                                        output_summary: out_summary,
                                        duration_ms,
                                        success,
                                        error: None,
                                    });
                                }
                                Ok(BridgeEvent::Result { text, session_id, usage, cost_usd, .. }) => {
                                    result.text = text;
                                    result.session_id = Some(session_id);
                                    result.input_tokens = usage.input_tokens;
                                    result.output_tokens = usage.output_tokens;
                                    result.cost_usd = cost_usd;
                                }
                                Ok(BridgeEvent::Error { message, .. }) => {
                                    result.error = Some(message);
                                }
                                Ok(BridgeEvent::Stop { usage, .. }) => {
                                    result.input_tokens = usage.input_tokens;
                                    result.output_tokens = usage.output_tokens;
                                }
                                Err(_) => {
                                    debug!("non-JSON bridge line: {l}");
                                }
                            }
                        }
                        Ok(None) => stdout_done = true,
                        Err(e) => {
                            warn!("bridge stdout error: {e}");
                            stdout_done = true;
                        }
                    }
                }
                line = stderr_reader.next_line(), if !stderr_done => {
                    match line {
                        Ok(Some(l)) => {
                            if !l.is_empty() {
                                debug!("bridge stderr: {l}");
                                result.stderr.push(l);
                            }
                        }
                        Ok(None) => stderr_done = true,
                        Err(_) => stderr_done = true,
                    }
                }
            }
        }

        if result.text.is_empty() && !text_parts.is_empty() {
            result.text = text_parts.join("");
        }
        result.tool_calls = tool_calls;
        result.raw_stream = text_parts.join("");

        let exit = child.wait().await.ok();
        result.success = exit.map(|s| s.success()).unwrap_or(false) && result.error.is_none();

        result
    };

    let result = if timeout_s > 0 {
        match tokio::time::timeout(std::time::Duration::from_secs(timeout_s), io_future).await {
            Ok(r) => r,
            Err(_) => {
                warn!("agent-bridge timed out after {timeout_s}s");
                BridgeResult {
                    error: Some(format!("timed out after {timeout_s}s")),
                    ..Default::default()
                }
            }
        }
    } else {
        io_future.await
    };

    Ok(result)
}

#[derive(Debug, Default)]
struct BridgeResult {
    text: String,
    session_id: Option<String>,
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: f64,
    tool_calls: Vec<ToolCallRecord>,
    raw_stream: String,
    error: Option<String>,
    success: bool,
    stderr: Vec<String>,
}

// ── AgentSdkBackend ──────────────────────────────────────────────────────

pub struct AgentSdkBackend {
    pub provider: ProviderConfig,
    pub timeout_s: u64,
    pub base_url: String,
}

impl AgentSdkBackend {
    pub fn new(provider: ProviderConfig) -> Self {
        Self {
            provider,
            timeout_s: 0,
            base_url: String::new(),
        }
    }

    pub fn with_timeout(mut self, timeout_s: u64) -> Self {
        self.timeout_s = timeout_s;
        self
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    fn build_env(&self, extra: &HashMap<String, String>) -> HashMap<String, String> {
        let mut env = self.provider.to_env_vars();
        env.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
        if !self.base_url.is_empty() {
            env.insert("ANTHROPIC_BASE_URL".into(), self.base_url.clone());
        }
        env
    }
}

#[async_trait]
impl AgentBackend for AgentSdkBackend {
    async fn run_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        ctx: PhaseContext,
    ) -> Result<PhaseOutput> {
        let file_listing = if phase.include_file_listing {
            let git = borg_core::git::Git::new(&ctx.work_dir);
            git.ls_files_manifest(&ctx.work_dir, 200, 16_000).ok()
        } else {
            None
        };
        let instruction =
            crate::instruction::build_instruction(task, phase, &ctx, file_listing.as_deref());

        let allowed: Vec<String> = if phase.allowed_tools.is_empty() {
            Vec::new()
        } else {
            phase.allowed_tools.split(',').map(|s| s.trim().to_string()).collect()
        };

        let mut disallowed: Vec<String> = if phase.disallowed_tools.is_empty() {
            Vec::new()
        } else {
            phase.disallowed_tools.split(',').map(|s| s.trim().to_string()).collect()
        };
        if !ctx.disallowed_tools.is_empty() {
            for t in ctx.disallowed_tools.split(',') {
                let t = t.trim().to_string();
                if !t.is_empty() && !disallowed.contains(&t) {
                    disallowed.push(t);
                }
            }
        }

        // Build MCP servers config
        let mcp_servers = if !ctx.borg_api_token.is_empty() && !ctx.borg_api_url.is_empty() {
            let api_keys_vec: Vec<(String, String)> = ctx.api_keys.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let servers = crate::mcp::build_mcp_servers_json(
                &ctx.borg_api_url,
                &ctx.borg_api_token,
                &task.mode,
                task.project_id,
                task.workspace_id,
                None,
                &api_keys_vec,
            );
            json!(servers)
        } else {
            serde_json::Value::Null
        };

        let resume = if !phase.fresh_session && !task.session_id.is_empty() {
            Some(task.session_id.clone())
        } else {
            None
        };

        let model = if ctx.model.is_empty() {
            None
        } else {
            Some(ctx.model.clone())
        };

        let request_id = next_request_id();
        let request = QueryRequest {
            r#type: "query".into(),
            id: request_id.clone(),
            prompt: instruction,
            options: QueryOptions {
                cwd: Some(ctx.work_dir.clone()),
                system_prompt: if phase.system_prompt.is_empty() {
                    None
                } else {
                    Some(phase.system_prompt.clone())
                },
                allowed_tools: allowed,
                disallowed_tools: disallowed,
                mcp_servers,
                model,
                max_turns: Some(200),
                max_budget_usd: None,
                permission_mode: Some("bypassPermissions".into()),
                resume,
                env: self.build_env(&ctx.api_keys),
            },
        };

        info!(task_id = task.id, phase = %phase.name, "spawning agent-sdk bridge");

        if let Some(tx) = &ctx.stream_tx {
            let evt = json!({"type": "status", "status": "Spawning agent (Agent SDK)..."}).to_string();
            let _ = tx.send(evt);
        }

        let result = run_bridge_query(
            &request,
            &self.provider,
            self.timeout_s,
            ctx.stream_tx.as_ref(),
        )
        .await?;

        if let Some(err) = &result.error {
            warn!(task_id = task.id, phase = %phase.name, "bridge error: {err}");
        }

        Ok(PhaseOutput {
            output: result.text,
            new_session_id: result.session_id,
            raw_stream: result.raw_stream,
            success: result.success,
            signal_json: None,
            ran_in_docker: false,
            container_test_results: Vec::new(),
        })
    }

    async fn run_chat(
        &self,
        request: &ChatRequest,
        ctx: &ChatContext,
    ) -> Result<ChatResponse> {
        // Build MCP servers
        let mcp_servers = if !ctx.borg_api_token.is_empty() && !ctx.borg_api_url.is_empty() {
            let api_keys_vec: Vec<(String, String)> = ctx.api_keys.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let servers = crate::mcp::build_mcp_servers_json(
                &ctx.borg_api_url,
                &ctx.borg_api_token,
                &ctx.mode,
                ctx.project_id,
                ctx.workspace_id,
                ctx.chat_thread.as_deref(),
                &api_keys_vec,
            );
            json!(servers)
        } else {
            serde_json::Value::Null
        };

        let model = if request.model.is_empty() {
            None
        } else {
            Some(request.model.clone())
        };

        let mut env = self.build_env(&ctx.api_keys);
        if !ctx.oauth_token.is_empty() {
            env.insert("CLAUDE_CODE_OAUTH_TOKEN".into(), ctx.oauth_token.clone());
        }
        env.extend(ctx.provider_env.iter().map(|(k, v)| (k.clone(), v.clone())));

        let request_id = next_request_id();
        let query = QueryRequest {
            r#type: "query".into(),
            id: request_id,
            prompt: request.message.clone(),
            options: QueryOptions {
                cwd: Some(ctx.session_dir.clone()),
                system_prompt: if request.system_prompt.is_empty() {
                    None
                } else {
                    Some(request.system_prompt.clone())
                },
                allowed_tools: request.allowed_tools.clone(),
                disallowed_tools: request.disallowed_tools.clone(),
                mcp_servers,
                model,
                max_turns: Some(request.max_turns),
                max_budget_usd: request.max_budget_usd,
                permission_mode: None,
                resume: ctx.session_id.clone(),
                env,
            },
        };

        let result = run_bridge_query(
            &query,
            &self.provider,
            self.timeout_s,
            ctx.stream_tx.as_ref(),
        )
        .await?;

        if let Some(err) = &result.error {
            anyhow::bail!("agent-sdk error: {err}");
        }

        Ok(ChatResponse {
            text: result.text,
            session_id: result.session_id,
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            cost_usd: result.cost_usd,
            tool_calls: result.tool_calls,
            raw_stream: result.raw_stream,
        })
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            supports_mcp: true,
            supports_sessions: true,
            supports_tools: true,
            supports_streaming: true,
            supports_sandbox: false,
            supported_models: vec![
                "claude-opus-4-6".into(),
                "claude-sonnet-4-6".into(),
                "claude-haiku-4-5".into(),
            ],
        }
    }

    fn name(&self) -> &str {
        "agent-sdk"
    }
}

// ── Persistence helpers ──────────────────────────────────────────────────
// These are called by the pipeline / chat route layers that own the DB handle,
// after run_phase / run_chat returns a BridgeResult / ChatResponse.

/// Persist tool call records to the database.
pub async fn persist_tool_calls(
    db: &borg_core::db::Db,
    tool_calls: &[ToolCallRecord],
    task_id: Option<i64>,
    chat_key: Option<&str>,
    run_id: &str,
) -> Result<()> {
    for tc in tool_calls {
        let id = db.insert_tool_call(
            run_id,
            &tc.tool_name,
            task_id,
            chat_key,
            Some(&tc.input_summary),
        )?;
        db.complete_tool_call(
            id,
            Some(&tc.output_summary),
            tc.duration_ms as i64,
            tc.success,
            tc.error.as_deref(),
        )?;
    }
    Ok(())
}

/// Persist usage/cost data from an agent run.
/// Calls update_message_usage for chat messages and/or accumulate_task_usage for pipeline tasks.
pub async fn persist_usage(
    db: &borg_core::db::Db,
    task_id: Option<i64>,
    message_id: Option<&str>,
    chat_jid: Option<&str>,
    input_tokens: i64,
    output_tokens: i64,
    cost_usd: f64,
    model: &str,
) -> Result<()> {
    if let (Some(msg_id), Some(jid)) = (message_id, chat_jid) {
        db.update_message_usage(msg_id, jid, input_tokens, output_tokens, cost_usd, model)?;
    }
    if let Some(tid) = task_id {
        db.accumulate_task_usage(tid, input_tokens, output_tokens, cost_usd)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_request_id_increments() {
        let a = next_request_id();
        let b = next_request_id();
        assert_ne!(a, b);
        assert!(a.starts_with("req-"));
    }

    #[test]
    fn provider_env_bedrock() {
        let backend = AgentSdkBackend::new(ProviderConfig::Bedrock {
            region: "eu-west-1".into(),
            profile: Some("prod".into()),
        });
        let env = backend.build_env(&HashMap::new());
        assert_eq!(env.get("CLAUDE_CODE_USE_BEDROCK").unwrap(), "1");
        assert_eq!(env.get("AWS_REGION").unwrap(), "eu-west-1");
        assert_eq!(env.get("AWS_PROFILE").unwrap(), "prod");
    }

    #[test]
    fn provider_env_subscription_is_empty() {
        let backend = AgentSdkBackend::new(ProviderConfig::Subscription);
        let env = backend.build_env(&HashMap::new());
        assert!(env.is_empty());
    }

    #[test]
    fn build_env_merges_extra() {
        let backend = AgentSdkBackend::new(ProviderConfig::Direct {
            api_key: "sk-test".into(),
        })
        .with_base_url("https://proxy.example.com");
        let mut extra = HashMap::new();
        extra.insert("CUSTOM_VAR".into(), "value".into());
        let env = backend.build_env(&extra);
        assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-test");
        assert_eq!(
            env.get("ANTHROPIC_BASE_URL").unwrap(),
            "https://proxy.example.com"
        );
        assert_eq!(env.get("CUSTOM_VAR").unwrap(), "value");
    }
}
