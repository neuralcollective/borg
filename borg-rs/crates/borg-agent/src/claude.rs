use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use borg_core::{
    agent::AgentBackend,
    sandbox::{Sandbox, SandboxMode},
    types::{ContainerTestResult, PhaseConfig, PhaseContext, PhaseOutput, Task},
};
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
};
use tracing::{debug, info, warn};

const BORG_SIGNAL_MARKER: &str = "BORG_SIGNAL:";
const BORG_MCP_READ_ONLY_TOOLS: &[&str] = &[
    "mcp__borg__list_services",
    "mcp__borg__search_documents",
    "mcp__borg__list_documents",
    "mcp__borg__read_document",
    "mcp__borg__get_document_categories",
    "mcp__borg__check_coverage",
];

/// Utility to extract phase results from Claude output.
pub fn extract_phase_result(output: &str) -> Option<String> {
    const START: &str = "---PHASE_RESULT_START---";
    const END: &str = "---PHASE_RESULT_END---";

    let start_idx = output.rfind(START)?;
    let end_idx = output[start_idx..].rfind(END)?;
    let content = &output[start_idx + START.len()..start_idx + end_idx];
    let trimmed = content.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub use borg_core::pipeline::derive_compile_check;

fn merge_allowed_tools(base: &str, extras: &[&str]) -> String {
    // Keep empty allowlists empty so unrestricted phases preserve existing semantics.
    if base.trim().is_empty() {
        return String::new();
    }

    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for tool in base.split(',').chain(extras.iter().copied()) {
        let tool = tool.trim();
        if !tool.is_empty() && seen.insert(tool.to_string()) {
            merged.push(tool.to_string());
        }
    }
    merged.join(",")
}

fn isolated_proxy_base_url(base_url: &str) -> String {
    if let Some(port) = base_url.strip_prefix("http://127.0.0.1:") {
        return format!("http://172.31.0.1:{port}");
    }
    if let Some(port) = base_url.strip_prefix("http://localhost:") {
        return format!("http://172.31.0.1:{port}");
    }
    if base_url.is_empty() {
        "http://172.31.0.1:3132".to_string()
    } else {
        base_url.to_string()
    }
}

fn container_host_ip(isolated: bool) -> &'static str {
    if isolated {
        "172.31.0.1"
    } else {
        "172.30.0.1"
    }
}

fn container_reachable_url(base_url: &str, host_ip: &str) -> String {
    if let Some(port) = base_url.strip_prefix("http://127.0.0.1:") {
        return format!("http://{host_ip}:{port}");
    }
    if let Some(port) = base_url.strip_prefix("http://localhost:") {
        return format!("http://{host_ip}:{port}");
    }
    base_url.to_string()
}

fn latest_jsonl_file(root: &Path) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;

    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file()
                || path.extension().and_then(|ext| ext.to_str()) != Some("jsonl")
            {
                continue;
            }

            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let Ok(modified_at) = metadata.modified() else {
                continue;
            };

            let replace = newest
                .as_ref()
                .map(|(current, _)| modified_at >= *current)
                .unwrap_or(true);
            if replace {
                newest = Some((modified_at, path));
            }
        }
    }

    newest.map(|(_, path)| path)
}

fn load_latest_session_transcript(session_dir: &str) -> Option<String> {
    let transcript_root = Path::new(session_dir).join(".claude/projects");
    let transcript_path = latest_jsonl_file(&transcript_root)?;
    std::fs::read_to_string(&transcript_path).ok()
}

/// Runs Claude Code as a subprocess, with configurable sandbox isolation.
pub struct ClaudeBackend {
    /// Path to the `claude` CLI binary.
    pub claude_bin: String,
    /// Which sandbox backend to use for `phase.use_docker` phases.
    /// Use `Sandbox::detect()` at startup to pick the best available option.
    pub sandbox_mode: SandboxMode,
    /// Docker image name (only used when `sandbox_mode == Docker`).
    pub docker_image: String,
    /// Kill subprocess and return failure after this many seconds (0 = no limit).
    pub timeout_s: u64,
    /// Path to Claude credentials file for fresh token reads.
    pub credentials_path: String,
    /// Memory limit for Docker containers in MiB (0 = no limit).
    pub container_memory_mb: u64,
    /// CPU quota for Docker containers (0.0 = no limit).
    pub container_cpus: f64,
    pub git_author_name: String,
    pub git_author_email: String,
    pub base_url: String,
    pub reasoning_effort: String,
}

impl ClaudeBackend {
    pub fn new(
        claude_bin: impl Into<String>,
        sandbox_mode: SandboxMode,
        docker_image: impl Into<String>,
    ) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        Self {
            claude_bin: claude_bin.into(),
            sandbox_mode,
            docker_image: docker_image.into(),
            timeout_s: 0,
            credentials_path: format!("{home}/.claude/.credentials.json"),
            container_memory_mb: 0,
            container_cpus: 0.0,
            git_author_name: "Borg".into(),
            git_author_email: "borg@localhost".into(),
            base_url: String::new(),
            reasoning_effort: String::new(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    pub fn with_timeout(mut self, timeout_s: u64) -> Self {
        self.timeout_s = timeout_s;
        self
    }

    pub fn with_resource_limits(mut self, memory_mb: u64, cpus: f64) -> Self {
        self.container_memory_mb = memory_mb;
        self.container_cpus = cpus;
        self
    }

    pub fn with_git_author(mut self, name: &str, email: &str) -> Self {
        self.git_author_name = name.to_string();
        self.git_author_email = email.to_string();
        self
    }

    pub fn with_reasoning_effort(mut self, effort: impl Into<String>) -> Self {
        self.reasoning_effort = effort.into();
        self
    }

    fn normalized_reasoning_effort(&self) -> Option<String> {
        let effort = self.reasoning_effort.trim().to_ascii_lowercase();
        match effort.as_str() {
            "" => None,
            "low" | "medium" | "high" | "max" => Some(effort),
            _ => {
                warn!(effort = %self.reasoning_effort, "ignoring unsupported Claude reasoning effort");
                None
            },
        }
    }

    fn resolve_gh_token() -> String {
        std::process::Command::new("gh")
            .args(["auth", "token"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }
}

#[async_trait]
impl AgentBackend for ClaudeBackend {
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
        let effective_mode = if phase.use_docker {
            self.sandbox_mode.clone()
        } else {
            SandboxMode::Direct
        };
        let is_docker = matches!(effective_mode, SandboxMode::Docker);
        let host_ip = container_host_ip(ctx.isolated);
        let reachable_borg_api_url = if is_docker {
            container_reachable_url(&ctx.borg_api_url, host_ip)
        } else {
            ctx.borg_api_url.clone()
        };

        let mut effective_allowed_tools = phase.allowed_tools.clone();

        let mut claude_args = vec![
            "--print".to_string(),
            "--dangerously-skip-permissions".to_string(),
        ];
        if let Some(effort) = self.normalized_reasoning_effort() {
            claude_args.push("--effort".to_string());
            claude_args.push(effort);
        }

        // Disallowed tools (combine phase-specific and global context)
        let mut disallowed = phase.disallowed_tools.clone();
        if !ctx.disallowed_tools.is_empty() {
            if !disallowed.is_empty() {
                disallowed.push(',');
            }
            disallowed.push_str(&ctx.disallowed_tools);
        }
        if !disallowed.is_empty() {
            claude_args.push(format!("--disallowed-tools={disallowed}"));
        }

        if !phase.fresh_session && !task.session_id.is_empty() {
            claude_args.push("--resume".into());
            claude_args.push(task.session_id.clone());
        }

        // Wire MCP servers for pipeline tasks
        if !ctx.borg_api_token.is_empty() && !ctx.borg_api_url.is_empty() {
            let api_keys_vec: Vec<(String, String)> = ctx
                .api_keys
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let mcp_servers = crate::mcp::build_mcp_servers_json(
                &reachable_borg_api_url,
                &ctx.borg_api_token,
                &task.mode,
                task.project_id,
                task.workspace_id,
                None,
                &api_keys_vec,
            );
            if !mcp_servers.is_empty() {
                let borg_loaded = mcp_servers.contains_key("borg");
                if borg_loaded {
                    effective_allowed_tools =
                        merge_allowed_tools(&phase.allowed_tools, BORG_MCP_READ_ONLY_TOOLS);
                }
                let config_json = json!({ "mcpServers": mcp_servers });
                let mcp_json_path = format!("{}/.mcp.json", ctx.session_dir);
                if let Err(e) = std::fs::write(&mcp_json_path, config_json.to_string()) {
                    warn!(task_id = task.id, "failed to write .mcp.json: {e}");
                } else {
                    claude_args.push("--mcp-config".into());
                    claude_args.push(mcp_json_path);
                }
            }
        }

        if !effective_allowed_tools.is_empty() {
            claude_args.push(format!("--allowed-tools={effective_allowed_tools}"));
        }

        claude_args.push(instruction.clone());

        let full_cmd: Vec<String> = std::iter::once(self.claude_bin.clone())
            .chain(claude_args)
            .collect();

        let oauth_token = ctx.oauth_token.clone();
        let has_linked_auth = Path::new(&ctx.session_dir)
            .join(".claude/.credentials.json")
            .exists();

        info!(
            task_id = task.id,
            phase = %phase.name,
            mode = ?effective_mode,
            "spawning claude"
        );

        let stream_tx = ctx.stream_tx.clone();
        if let Some(tx) = &stream_tx {
            let evt = json!({
                "type": "status",
                "status": format!("Spawning agent ({:?})...", effective_mode),
            })
            .to_string();
            let _ = tx.send(evt);
        }

        let cidfile_path = if is_docker {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let p = format!("/tmp/borg-cid-{}-{}.txt", task.id, ms);
            let _ = std::fs::remove_file(&p);
            Some(p)
        } else {
            None
        };

        // Write CLAUDE.md with search API instructions for the agent
        if !ctx.borg_api_token.is_empty() && !reachable_borg_api_url.is_empty() {
            let project_id_hint = if task.project_id > 0 {
                format!("Current project_id: {}\n", task.project_id)
            } else {
                "Current project_id: none\n".to_string()
            };
            let agent_claude_md = format!(
                "# BorgSearch — Project Document Search\n\n\
                 {project_id_hint}\
                 You have access to BorgSearch, a project document search API. Use curl or WebFetch to query it.\n\
                 This is NOT the same as Claude's built-in Grep/Glob/Read tools — BorgSearch searches uploaded project documents (contracts, filings, memos, etc.) via Vespa.\n\n\
                 BorgSearch MCP tools are always available in this session. If no matter/project corpus is attached, corpus tools return `no_project_corpus`; ask for the relevant project context instead of guessing.\n\n\
                 ## Endpoints\n\n\
                 All requests need: `Authorization: Bearer $API_TOKEN`\n\n\
                 - `GET $API_BASE_URL/api/borgsearch/query?q=<query>&project_id=<id>&limit=20&doc_type=<type>&jurisdiction=<jur>&privileged_only=true&model=<model>&exclude=<terms>` \
                 — hybrid keyword+semantic search with optional filters. doc_type: contract, filing, statute, memo, document, data. jurisdiction: e.g. US-CA, UK, EU. exclude: comma-separated terms to NOT match (e.g. exclude=indemnification,hold%20harmless).\n\
                 - `GET $API_BASE_URL/api/borgsearch/files?project_id=<id>&q=<filter>&limit=50&offset=0` — list project files\n\
                 - `GET $API_BASE_URL/api/borgsearch/file/<file_id>?project_id=<id>` — read full file content\n\
                 - `GET $API_BASE_URL/api/borgsearch/coverage?q=<query>&project_id=<id>&limit=100` — COMPLETENESS CHECK: returns matched AND unmatched documents for a query\n\n\
                 ## Embedding Models\n\n\
                 The `model` parameter on the query endpoint selects which Voyage AI embedding model to use for semantic search.\n\
                 Choose the model that best matches the domain of your query:\n\n\
                 - `voyage-4-large` — general-purpose (default). Best for broad queries across mixed content.\n\
                 - `voyage-law-2` — legal domain. Use for contracts, case law, statutes, regulatory filings.\n\
                 - `voyage-finance-2` — financial domain. Use for financial statements, SEC filings, market analysis.\n\
                 - `voyage-code-3` — source code. Use for code search, technical documentation, API references.\n\n\
                 If omitted, the default model is used. Pick the domain model when your query is clearly domain-specific — it significantly improves relevance.\n\n\
                 Responses are plain text. Use BorgSearch to find relevant documents before answering questions about project content.\n\n\
                 ## MCP Tools (preferred)\n\n\
                 If the `borg` MCP server loaded successfully, prefer using MCP tools instead of raw HTTP:\n\
                 - `search_documents` — hybrid search (same as /api/borgsearch/query)\n\
                 - `list_documents` — list project files (same as /api/borgsearch/files)\n\
                 - `read_document` — read full document (same as /api/borgsearch/file/<id>)\n\
                 - `check_coverage` — COMPLETENESS CHECK: shows which documents matched AND which did NOT match a query\n\
                 - `get_document_categories` — get all doc_type and jurisdiction facets with counts for a project\n\
                 - `create_task` / `get_task_status` / `list_project_tasks` — pipeline task management\n\
                 - `list_services` — discover available tools and integrations\n\n\
                 ## Completeness Methodology\n\n\
                 For exhaustive document reviews (clause extraction, compliance checks, etc.):\n\
                 1. ALWAYS start with `list_documents` to get the full file inventory and total count\n\
                 2. Run at least 2 distinct `search_documents` / BorgSearch query passes before concluding a clause or issue is absent\n\
                 3. Use `check_coverage` to identify documents that did NOT match — these may have absent clauses or different terminology\n\
                 4. Read at least 1 full document (`read_document` or a staged `project_files/` read), ideally from unmatched results, to verify snippets vs. actual text\n\
                 5. Report coverage stats: \"Reviewed X of Y total documents. Z matched, W had no relevant clause.\"\n\
                 The pipeline validates this protocol for exhaustive legal review tasks and will retry the run if you skip it.\n\
                 NEVER claim exhaustive coverage without cross-referencing against the full file list.\n"
            );
            let claude_md_path = format!("{}/CLAUDE.md", ctx.session_dir);
            let _ = std::fs::write(&claude_md_path, &agent_claude_md);
        }

        let real_home = std::env::var("HOME").unwrap_or_default();
        let rustup_home =
            std::env::var("RUSTUP_HOME").unwrap_or_else(|_| format!("{real_home}/.rustup"));
        let cargo_home =
            std::env::var("CARGO_HOME").unwrap_or_else(|_| format!("{real_home}/.cargo"));

        let gh_token = if !ctx.github_token.is_empty() {
            ctx.github_token.clone()
        } else if is_docker {
            tokio::task::spawn_blocking(Self::resolve_gh_token)
                .await
                .unwrap_or_default()
        } else {
            String::new()
        };

        let effective_base_url = if is_docker {
            container_reachable_url(&isolated_proxy_base_url(&self.base_url), host_ip)
        } else {
            String::new()
        };

        let mut child: tokio::process::Child = match effective_mode {
            SandboxMode::Bwrap => {
                let git_dir = Path::new(&task.repo_path).join(".git");
                let git_dir_str = git_dir.to_string_lossy().to_string();
                let writable: Vec<&str> = vec![
                    ctx.work_dir.as_str(),
                    ctx.session_dir.as_str(),
                    &git_dir_str,
                ];
                // Hide user home dirs to prevent credential/key exposure,
                // then restore only the tool paths the agent actually needs.
                let hide: Vec<&str> = vec![&real_home, "/root"];
                let ro_restore: Vec<&str> = vec![&rustup_home, &cargo_home];
                let mut cmd =
                    Sandbox::bwrap_command(&writable, &hide, &ro_restore, &ctx.work_dir, &full_cmd);
                cmd.kill_on_drop(true)
                    .env("HOME", &ctx.session_dir)
                    .env("RUSTUP_HOME", &rustup_home)
                    .env("CARGO_HOME", &cargo_home)
                    .env("API_BASE_URL", &reachable_borg_api_url)
                    .env("API_TOKEN", &ctx.borg_api_token);
                if !gh_token.is_empty() {
                    cmd.env("GH_TOKEN", &gh_token);
                }
                if !has_linked_auth && !oauth_token.is_empty() {
                    cmd.env("CLAUDE_CODE_OAUTH_TOKEN", &oauth_token);
                }

                if !effective_base_url.is_empty() {
                    cmd.env("ANTHROPIC_BASE_URL", &effective_base_url);
                }

                cmd.stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .context("failed to spawn bwrap")?
            },
            SandboxMode::Docker => {
                let workspace_host = if !task.repo_path.is_empty()
                    && Path::new(&task.repo_path).join(".git").exists()
                {
                    task.repo_path.clone()
                } else {
                    ctx.work_dir.clone()
                };
                let binds = vec![
                    (workspace_host, "/workspace".to_string(), false),
                    (ctx.session_dir.clone(), "/home/bun".to_string(), false),
                ];
                let volumes_owned = vec![
                    ("rustup-cache".to_string(), "/home/bun/.rustup".to_string()),
                    ("cargo-cache".to_string(), "/home/bun/.cargo".to_string()),
                ];
                let mut env_kv = vec![
                    ("HOME".to_string(), "/home/bun".to_string()),
                    ("RUSTUP_HOME".to_string(), "/home/bun/.rustup".to_string()),
                    ("CARGO_HOME".to_string(), "/home/bun/.cargo".to_string()),
                ];
                if !has_linked_auth && !oauth_token.is_empty() {
                    env_kv.push(("CLAUDE_CODE_OAUTH_TOKEN".to_string(), oauth_token.clone()));
                }
                if !gh_token.is_empty() {
                    env_kv.push(("GH_TOKEN".to_string(), gh_token));
                }
                if !effective_base_url.is_empty() {
                    env_kv.push(("ANTHROPIC_BASE_URL".to_string(), effective_base_url));
                }
                if !reachable_borg_api_url.is_empty() {
                    env_kv.push(("API_BASE_URL".to_string(), reachable_borg_api_url.clone()));
                }
                if !ctx.borg_api_token.is_empty() {
                    env_kv.push(("API_TOKEN".to_string(), ctx.borg_api_token.clone()));
                }

                env_kv.push(("BORG_HOST_IP".to_string(), host_ip.to_string()));

                let binds_ref: Vec<(&str, &str, bool)> = binds
                    .iter()
                    .map(|(h, c, r)| (h.as_str(), c.as_str(), *r))
                    .collect();
                let volumes_ref: Vec<(&str, &str)> = volumes_owned
                    .iter()
                    .map(|(n, c)| (n.as_str(), c.as_str()))
                    .collect();
                let env_ref: Vec<(&str, &str)> = env_kv
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();

                let mut docker_cmd = Sandbox::docker_command(
                    &self.docker_image,
                    &binds_ref,
                    &volumes_ref,
                    "",
                    &[],
                    &env_ref,
                    self.container_memory_mb,
                    self.container_cpus,
                    ctx.agent_network.as_deref(),
                );
                if let Some(ref cid_path) = cidfile_path {
                    let existing_args: Vec<_> = docker_cmd
                        .as_std()
                        .get_args()
                        .map(|a| a.to_os_string())
                        .collect();
                    let mut new_cmd = Command::new("docker");
                    new_cmd.arg("run").arg("--cidfile").arg(cid_path);
                    for arg in existing_args.into_iter().skip(1) {
                        new_cmd.arg(arg);
                    }
                    docker_cmd = new_cmd;
                }

                docker_cmd
                    .kill_on_drop(true)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .stdin(Stdio::piped())
                    .spawn()
                    .context("failed to spawn docker")?
            },
            SandboxMode::Direct => {
                let augmented_path = format!(
                    "{}/bin:{}:{}",
                    cargo_home,
                    std::env::var("PATH").unwrap_or_default(),
                    "/usr/local/bin:/usr/bin:/bin"
                );
                let mut cmd = Command::new(&self.claude_bin);
                cmd.args(&full_cmd[1..])
                    .kill_on_drop(true)
                    .current_dir(&ctx.work_dir)
                    .env("HOME", &ctx.session_dir)
                    .env("RUSTUP_HOME", &rustup_home)
                    .env("CARGO_HOME", &cargo_home)
                    .env("PATH", &augmented_path)
                    .env("API_BASE_URL", &reachable_borg_api_url)
                    .env("API_TOKEN", &ctx.borg_api_token);
                if !gh_token.is_empty() {
                    cmd.env("GH_TOKEN", &gh_token);
                }
                if !has_linked_auth && !oauth_token.is_empty() {
                    cmd.env("CLAUDE_CODE_OAUTH_TOKEN", &oauth_token);
                }

                if !effective_base_url.is_empty() {
                    cmd.env("ANTHROPIC_BASE_URL", &effective_base_url);
                }

                cmd.stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .with_context(|| format!("failed to spawn claude: {}", self.claude_bin))?
            },
        };

        if is_docker {
            if let Some(mut stdin) = child.stdin.take() {
                let repo_test_cmd = ctx.repo_config.test_cmd.clone();
                let compile_check_cmd = if phase.compile_check {
                    derive_compile_check(&repo_test_cmd).unwrap_or_default()
                } else {
                    String::new()
                };
                let input = serde_json::json!({
                    "prompt": instruction,
                    "model": if ctx.model.is_empty() { "claude-sonnet-4-6".to_string() } else { ctx.model.clone() },
                    "sessionId": task.session_id.clone(),
                    "systemPrompt": phase.system_prompt.clone(),
                    "allowedTools": effective_allowed_tools.clone(),
                    "maxTurns": 200,
                    "projectId": task.project_id,
                    "testCmd": repo_test_cmd,
                    "compileCheckCmd": compile_check_cmd,
                    "lintCmd": ctx.repo_config.lint_cmd,
                });
                let payload = serde_json::to_vec(&input).unwrap_or_default();
                let _ = stdin.write_all(&payload).await;
                drop(stdin);
            }
        }

        let stdout = child.stdout.take().context("failed to take stdout")?;
        let stderr = child.stderr.take().context("failed to take stderr")?;

        let timeout_s = self.timeout_s;
        let io_future = async move {
            let mut signal_json: Option<String> = None;
            let mut output_lines: Vec<String> = Vec::new();
            let mut container_test_results: Vec<ContainerTestResult> = Vec::new();

            let mut stdout_reader = BufReader::new(stdout).lines();
            let mut stderr_reader = BufReader::new(stderr).lines();

            let mut stdout_done = false;
            let mut stderr_done = false;

            while !stdout_done || !stderr_done {
                tokio::select! {
                    line = stdout_reader.next_line(), if !stdout_done => {
                        match line {
                            Ok(Some(l)) => {
                                if let Some(sig) = l.strip_prefix(BORG_SIGNAL_MARKER) {
                                    signal_json = Some(sig.to_string());
                                }
                                if let Some(tx) = &stream_tx {
                                    let _ = tx.send(l.clone());
                                }
                                if output_lines.len() < 50_000 && l.len() < 100_000 {
                                    output_lines.push(l);
                                }
                            }
                            Ok(None) => stdout_done = true,
                            Err(e) => {
                                warn!("stdout read error: {e}");
                                stdout_done = true;
                            }
                        }
                    }
                    line = stderr_reader.next_line(), if !stderr_done => {
                        match line {
                            Ok(Some(l)) => {
                                if !l.is_empty() {
                                    let test_line = l.strip_prefix("---BORG_TEST_RESULT---").unwrap_or(&l);
                                    if let Ok(res) = serde_json::from_str::<ContainerTestResult>(test_line) {
                                        container_test_results.push(res);
                                    } else {
                                        if let Some(tx) = &stream_tx {
                                            let evt = serde_json::json!({
                                                "type": "stderr",
                                                "content": l,
                                            }).to_string();
                                            let _ = tx.send(evt);
                                        }
                                        debug!("claude stderr: {l}");
                                    }
                                }
                            }
                            Ok(None) => stderr_done = true,
                            Err(e) => {
                                warn!("stderr read error: {e}");
                                stderr_done = true;
                            }
                        }
                    }
                }
            }

            let exit_status = child.wait().await.ok();
            let success = exit_status.map(|s| s.success()).unwrap_or(false);
            (
                output_lines.join("\n"),
                signal_json,
                container_test_results,
                success,
            )
        };

        let (stdout_stream, signal_json, container_test_results, success) = if timeout_s > 0 {
            match tokio::time::timeout(std::time::Duration::from_secs(timeout_s), io_future).await {
                Ok(res) => res,
                Err(_) => {
                    warn!("claude timed out after {}s", timeout_s);
                    (String::new(), None, Vec::new(), false)
                },
            }
        } else {
            io_future.await
        };

        let raw_stream = load_latest_session_transcript(&ctx.session_dir).unwrap_or_else(|| {
            warn!(
                task_id = task.id,
                phase = %phase.name,
                session_dir = %ctx.session_dir,
                "claude transcript not found; falling back to printed stdout for raw_stream"
            );
            stdout_stream.clone()
        });

        if let Some(cid_path) = cidfile_path {
            if let Ok(cid) = std::fs::read_to_string(&cid_path) {
                let cid = cid.trim();
                if !cid.is_empty() {
                    info!("cleaning up container {cid}");
                    let _ = std::process::Command::new("docker")
                        .args(["rm", "-f", cid])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            }
            let _ = std::fs::remove_file(cid_path);
        }

        Ok(PhaseOutput {
            output: stdout_stream,
            new_session_id: None,
            raw_stream,
            success,
            signal_json,
            ran_in_docker: is_docker,
            container_test_results,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use borg_core::sandbox::SandboxMode;

    use super::{
        container_reachable_url, isolated_proxy_base_url, latest_jsonl_file,
        load_latest_session_transcript, merge_allowed_tools, ClaudeBackend,
        BORG_MCP_READ_ONLY_TOOLS,
    };

    fn temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("borg-claude-tests-{label}-{nonce}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn merge_allowed_tools_appends_borg_tools_without_duplicates() {
        let merged = merge_allowed_tools("Read,Glob,Grep,Write,Edit", BORG_MCP_READ_ONLY_TOOLS);
        assert!(merged.contains("Read"));
        assert!(merged.contains("mcp__borg__search_documents"));
        assert_eq!(merged.matches("mcp__borg__search_documents").count(), 1);
    }

    #[test]
    fn merge_allowed_tools_preserves_empty_semantics() {
        assert!(merge_allowed_tools("", BORG_MCP_READ_ONLY_TOOLS).is_empty());
        assert!(merge_allowed_tools("   ", BORG_MCP_READ_ONLY_TOOLS).is_empty());
    }

    #[test]
    fn isolated_proxy_base_url_rewrites_local_proxy_host() {
        assert_eq!(
            isolated_proxy_base_url("http://127.0.0.1:3232"),
            "http://172.31.0.1:3232"
        );
        assert_eq!(
            isolated_proxy_base_url("http://localhost:4232"),
            "http://172.31.0.1:4232"
        );
    }

    #[test]
    fn container_reachable_url_rewrites_local_api_host() {
        assert_eq!(
            container_reachable_url("http://127.0.0.1:3231", "172.31.0.1"),
            "http://172.31.0.1:3231"
        );
        assert_eq!(
            container_reachable_url("http://localhost:4231", "172.30.0.1"),
            "http://172.30.0.1:4231"
        );
    }

    #[test]
    fn latest_jsonl_file_prefers_newest_transcript() {
        let root = temp_dir("latest-jsonl");
        let older_dir = root.join("older");
        let newer_dir = root.join("nested/newer");
        fs::create_dir_all(&older_dir).unwrap();
        fs::create_dir_all(&newer_dir).unwrap();

        let older = older_dir.join("older.jsonl");
        let newer = newer_dir.join("newer.jsonl");
        fs::write(&older, "older").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        fs::write(&newer, "newer").unwrap();

        let found = latest_jsonl_file(&root).unwrap();

        assert_eq!(found, newer);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn load_latest_session_transcript_reads_newest_jsonl_contents() {
        let session_dir = temp_dir("load-transcript");
        let transcript_root = session_dir.join(".claude/projects/demo");
        fs::create_dir_all(&transcript_root).unwrap();
        fs::write(
            transcript_root.join("run.jsonl"),
            "{\"type\":\"assistant\"}\n",
        )
        .unwrap();

        let transcript = load_latest_session_transcript(session_dir.to_str().unwrap()).unwrap();

        assert_eq!(transcript, "{\"type\":\"assistant\"}\n");
        fs::remove_dir_all(session_dir).unwrap();
    }

    #[test]
    fn normalized_reasoning_effort_accepts_supported_values() {
        let backend = ClaudeBackend::new("claude", SandboxMode::Direct, "ignored")
            .with_reasoning_effort("HIGH");
        assert_eq!(
            backend.normalized_reasoning_effort().as_deref(),
            Some("high")
        );
    }

    #[test]
    fn normalized_reasoning_effort_rejects_unknown_values() {
        let backend = ClaudeBackend::new("claude", SandboxMode::Direct, "ignored")
            .with_reasoning_effort("xhigh");
        assert!(backend.normalized_reasoning_effort().is_none());
    }
}
