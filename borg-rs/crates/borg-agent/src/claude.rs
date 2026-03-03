use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use borg_core::{
    agent::AgentBackend,
    pipeline::derive_compile_check,
    sandbox::{Sandbox, SandboxMode},
    types::{ContainerTestResult, PhaseConfig, PhaseContext, PhaseOutput, Task},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
};
use tracing::{info, warn};

const BORG_SIGNAL_MARKER: &str = "---BORG_SIGNAL---";
const BORG_EVENT_MARKER: &str = "---BORG_EVENT---";
const BORG_TEST_RESULT_MARKER: &str = "---BORG_TEST_RESULT---";

pub const PHASE_RESULT_START: &str = "---PHASE_RESULT_START---";
pub const PHASE_RESULT_END: &str = "---PHASE_RESULT_END---";

/// Extract the last complete marker block from decoded text.
/// Returns a trimmed slice of the content between the last pair of markers, or None.
pub fn extract_phase_result(text: &str) -> Option<&str> {
    let mut last_content: Option<&str> = None;
    let mut search = text;
    while let Some(start_pos) = search.find(PHASE_RESULT_START) {
        let after_start = &search[start_pos + PHASE_RESULT_START.len()..];
        if let Some(end_pos) = after_start.find(PHASE_RESULT_END) {
            let content = after_start[..end_pos].trim();
            if !content.is_empty() {
                last_content = Some(content);
            } else {
                last_content = None;
            }
            search = &after_start[end_pos + PHASE_RESULT_END.len()..];
        } else {
            break;
        }
    }
    last_content
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
        }
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
        if !name.is_empty() { self.git_author_name = name.into(); }
        if !email.is_empty() { self.git_author_email = email.into(); }
        self
    }

    /// Refresh OAuth token (triggers CLI refresh if near-expiry, then re-reads from disk).
    fn fresh_oauth_token(&self, fallback: &str) -> String {
        borg_core::config::refresh_oauth_token(&self.credentials_path, fallback)
    }

    /// Build JSON payload for the container entrypoint (Docker mode).
    fn build_docker_input(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        ctx: &PhaseContext,
        instruction: &str,
        system_prompt: &str,
        session_id: &str,
        compile_check_cmd: &str,
        lint_cmd: &str,
        test_cmd: &str,
        gh_token: &str,
    ) -> Vec<u8> {
        let commit_message = if !ctx.user_coauthor.is_empty() {
            format!("{}\n\nCo-Authored-By: {}", phase.commit_message, ctx.user_coauthor)
        } else {
            phase.commit_message.clone()
        };

        let repo_name = std::path::Path::new(&task.repo_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let branch = format!("task-{}", task.id);
        let gh_token = gh_token.to_string();

        // Docker containers need a GitHub URL, not a local path
        let repo_url = if !ctx.repo_config.repo_slug.is_empty() && !gh_token.is_empty() {
            format!("https://x-access-token:{gh_token}@github.com/{}.git", ctx.repo_config.repo_slug)
        } else if !ctx.repo_config.repo_slug.is_empty() {
            format!("https://github.com/{}.git", ctx.repo_config.repo_slug)
        } else {
            task.repo_path.clone()
        };

        let author_name = &self.git_author_name;
        let author_email = &self.git_author_email;

        let mut payload = serde_json::json!({
            "prompt": instruction,
            "model": ctx.model,
            "systemPrompt": system_prompt,
            "allowedTools": phase.allowed_tools,
            "maxTurns": 200,
            "repoUrl": repo_url,
            "mirrorPath": format!("/mirrors/{repo_name}.git"),
            "branch": branch,
            "base": "origin/main",
            "commitMessage": commit_message,
            "gitAuthorName": author_name,
            "gitAuthorEmail": author_email,
            "pushAfterCommit": !gh_token.is_empty(),
        });
        if !session_id.is_empty() {
            payload["resumeSessionId"] = serde_json::Value::String(session_id.to_string());
        }
        if !compile_check_cmd.is_empty() {
            payload["compileCheckCmd"] = serde_json::Value::String(compile_check_cmd.to_string());
        }
        if !lint_cmd.is_empty() {
            payload["lintCmd"] = serde_json::Value::String(lint_cmd.to_string());
        }
        if !test_cmd.is_empty() {
            payload["testCmd"] = serde_json::Value::String(test_cmd.to_string());
        }

        serde_json::to_vec(&payload).unwrap_or_default()
    }

    /// Parse a `---BORG_TEST_RESULT---{json}` line emitted by the container entrypoint.
    fn parse_test_result(line: &str) -> Option<ContainerTestResult> {
        let json_str = line.strip_prefix(BORG_TEST_RESULT_MARKER)?;
        let v: serde_json::Value = serde_json::from_str(json_str).ok()?;
        Some(ContainerTestResult {
            phase: v["phase"].as_str().unwrap_or("").to_string(),
            passed: v["passed"].as_bool().unwrap_or(false),
            exit_code: v["exitCode"].as_i64().unwrap_or(1) as i32,
            output: v["output"].as_str().unwrap_or("").to_string(),
        })
    }

    fn resolve_gh_token() -> String {
        std::env::var("GH_TOKEN")
            .or_else(|_| std::env::var("GITHUB_TOKEN"))
            .unwrap_or_else(|_| {
                std::process::Command::new("gh")
                    .args(["auth", "token"])
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default()
            })
    }

    fn host_mirror_path(task: &Task, data_dir: &str) -> String {
        let repo_name = std::path::Path::new(&task.repo_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let raw = format!("{data_dir}/mirrors/{repo_name}.git");
        std::fs::canonicalize(&raw)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(raw)
    }

    fn container_mirror_path(task: &Task) -> String {
        let repo_name = std::path::Path::new(&task.repo_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        format!("/mirrors/{repo_name}.git")
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
            git.ls_files(&ctx.work_dir).ok()
        } else {
            None
        };
        let instruction =
            crate::instruction::build_instruction(task, phase, &ctx, file_listing.as_deref());

        let mut claude_args = vec![
            "--model".to_string(),
            ctx.model.clone(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            "--dangerously-skip-permissions".to_string(),
            "--max-turns".to_string(),
            "200".to_string(),
        ];

        let disallowed = ctx.disallowed_tools.trim();
        if !disallowed.is_empty() {
            claude_args.push("--disallowedTools".to_string());
            claude_args.push(disallowed.to_string());
        }

        // Build combined system prompt from phase + config-derived suffix
        let mut system_prompt = phase.system_prompt.clone();
        if !ctx.system_prompt_suffix.is_empty() {
            if !system_prompt.is_empty() {
                system_prompt.push('\n');
            }
            system_prompt.push_str(&ctx.system_prompt_suffix);
        }
        if !system_prompt.is_empty() {
            claude_args.push("--append-system-prompt".to_string());
            claude_args.push(system_prompt.clone());
        }

        // Build MCP config based on mode — each mode gets its relevant MCP servers.
        let mcp_config_path = {
            let sidecar_base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../sidecar");
            let mut mcp_servers = serde_json::Map::new();
            let mode = ctx.task.mode.as_str();

            // lawborg + healthborg share the legal research MCP (CourtListener, EDGAR, etc.)
            if matches!(mode, "lawborg" | "healthborg") {
                let legal_mcp_path = if let Ok(p) = std::env::var("LAWBORG_MCP_SERVER") {
                    std::path::PathBuf::from(p)
                } else {
                    sidecar_base.join("lawborg-mcp/server.js")
                };
                match legal_mcp_path.canonicalize() {
                    Ok(p) => {
                        let mut env_vars = serde_json::Map::new();
                        for (provider, key) in &ctx.api_keys {
                            let env_name = match provider.as_str() {
                                "lexisnexis" => "LEXISNEXIS_API_KEY",
                                "lexmachina" => "LEXMACHINA_API_KEY",
                                "intelligize" => "INTELLIGIZE_API_KEY",
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
                            env_vars.insert(env_name.into(), serde_json::Value::String(key.clone()));
                        }
                        mcp_servers.insert("legal".into(), serde_json::json!({
                            "command": "bun",
                            "args": ["run", p.to_string_lossy()],
                            "env": env_vars,
                        }));
                    }
                    Err(e) => {
                        tracing::warn!("lawborg MCP server not found at {}: {e}", legal_mcp_path.display());
                    }
                }
            }

            // buildborg gets the Shovels permits/contractors MCP
            if mode == "buildborg" {
                let shovels_path = sidecar_base.join("shovels-mcp/server.js");
                match shovels_path.canonicalize() {
                    Ok(p) => {
                        let mut env_vars = serde_json::Map::new();
                        if let Some(key) = ctx.api_keys.get("shovels") {
                            env_vars.insert("SHOVELS_API_KEY".into(), serde_json::Value::String(key.clone()));
                        }
                        mcp_servers.insert("shovels".into(), serde_json::json!({
                            "command": "bun",
                            "args": ["run", p.to_string_lossy()],
                            "env": env_vars,
                        }));
                    }
                    Err(e) => {
                        tracing::warn!("shovels MCP server not found at {}: {e}", shovels_path.display());
                    }
                }
            }

            // Plaid banking MCP — available when plaid keys are configured
            if let (Some(client_id), Some(secret)) = (ctx.api_keys.get("plaid_client_id"), ctx.api_keys.get("plaid_secret")) {
                let plaid_path = sidecar_base.join("plaid-mcp/server.js");
                if let Ok(p) = plaid_path.canonicalize() {
                    let mut env_vars = serde_json::Map::new();
                    env_vars.insert("PLAID_CLIENT_ID".into(), serde_json::Value::String(client_id.clone()));
                    env_vars.insert("PLAID_SECRET".into(), serde_json::Value::String(secret.clone()));
                    if let Some(env) = ctx.api_keys.get("plaid_env") {
                        env_vars.insert("PLAID_ENV".into(), serde_json::Value::String(env.clone()));
                    }
                    mcp_servers.insert("plaid".into(), serde_json::json!({
                        "command": "bun",
                        "args": ["run", p.to_string_lossy()],
                        "env": env_vars,
                    }));
                }
            }

            // kreuzberg OCR — available to document-heavy modes when installed
            if matches!(mode, "lawborg" | "healthborg" | "buildborg") {
                let has_kreuzberg = tokio::task::spawn_blocking(|| {
                    std::process::Command::new("kreuzberg")
                        .arg("--version")
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                })
                .await
                .unwrap_or(false);
                if has_kreuzberg {
                    mcp_servers.insert("ocr".into(), serde_json::json!({
                        "command": "kreuzberg",
                        "args": ["mcp"],
                    }));
                }
            }

            if mcp_servers.is_empty() {
                None
            } else {
                let mcp_dir = format!("{}/mcp", ctx.session_dir);
                std::fs::create_dir_all(&mcp_dir).ok();
                let config_json = serde_json::json!({ "mcpServers": mcp_servers });
                let config_path = format!("{}/mcp-config.json", mcp_dir);
                std::fs::write(&config_path, config_json.to_string())
                    .with_context(|| format!("failed to write MCP config to {config_path}"))?;
                Some(config_path)
            }
        };

        if let Some(ref path) = mcp_config_path {
            claude_args.push("--mcp-config".to_string());
            claude_args.push(path.clone());
        }

        let session_id = ctx.task.session_id.clone();
        if !session_id.is_empty() && !phase.fresh_session {
            claude_args.push("--resume".to_string());
            claude_args.push(session_id.clone());
        }

        claude_args.push("--print".to_string());
        claude_args.push(instruction.clone());

        let effective_mode = &self.sandbox_mode;

        let creds_path = self.credentials_path.clone();
        let oauth_fallback = ctx.oauth_token.clone();
        let oauth_token = tokio::task::spawn_blocking(move || {
            borg_core::config::refresh_oauth_token(&creds_path, &oauth_fallback)
        })
        .await
        .unwrap_or_else(|_| ctx.oauth_token.clone());

        info!(
            task_id = task.id,
            phase = %phase.name,
            session_id = %session_id,
            sandbox = ?effective_mode,
            "spawning claude subprocess"
        );

        let is_docker = effective_mode == &SandboxMode::Docker;
        let mut full_cmd: Vec<String> = vec![self.claude_bin.clone()];
        full_cmd.extend(claude_args);

        if is_docker {
            let repo_name = std::path::Path::new(&task.repo_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let evt = serde_json::json!({
                "type": "container_event",
                "event": "container_starting",
                "image": self.docker_image,
                "repo": repo_name,
                "branch": format!("task-{}", task.id),
            })
            .to_string();
            if let Some(tx) = &ctx.stream_tx {
                let _ = tx.send(evt);
            }
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

        let real_home = std::env::var("HOME").unwrap_or_default();
        let rustup_home = std::env::var("RUSTUP_HOME")
            .unwrap_or_else(|_| format!("{real_home}/.rustup"));
        let cargo_home = std::env::var("CARGO_HOME")
            .unwrap_or_else(|_| format!("{real_home}/.cargo"));

        // Resolve GH token upfront (may invoke `gh auth token` subprocess)
        let gh_token = if matches!(effective_mode, SandboxMode::Docker) {
            tokio::task::spawn_blocking(Self::resolve_gh_token)
                .await
                .unwrap_or_default()
        } else {
            String::new()
        };

        let mut child = match effective_mode {
            SandboxMode::Bwrap => {
                // .git must be writable so the agent can commit.
                let git_dir = std::path::Path::new(&task.repo_path).join(".git");
                let git_dir_str = git_dir.to_string_lossy().to_string();
                let writable: Vec<&str> = vec![ctx.work_dir.as_str(), ctx.session_dir.as_str(), &git_dir_str];
                Sandbox::bwrap_command(&writable, &ctx.work_dir, &full_cmd)
                    .kill_on_drop(true)
                    .env("HOME", &ctx.session_dir)
                    .env("RUSTUP_HOME", &rustup_home)
                    .env("CARGO_HOME", &cargo_home)
                    .env("CLAUDE_CODE_OAUTH_TOKEN", &oauth_token)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .context("failed to spawn bwrap")?
            },
            SandboxMode::Docker => {
                // Session dir (rw) + optional bare mirror (ro) + optional setup script (ro).
                // The container clones the repo itself; no repo bind needed.
                let host_mirror = Self::host_mirror_path(task, &ctx.data_dir);
                let container_mirror = Self::container_mirror_path(task);
                let mut binds: Vec<(String, String, bool)> = vec![
                    (ctx.session_dir.clone(), ctx.session_dir.clone(), false),
                ];
                if std::path::Path::new(&host_mirror).exists() {
                    binds.push((host_mirror, container_mirror, true));
                }
                if !ctx.setup_script.is_empty() {
                    binds.push((ctx.setup_script.clone(), "/workspace/setup.sh".to_string(), true));
                }
                if !ctx.knowledge_dir.is_empty() && std::path::Path::new(&ctx.knowledge_dir).exists() {
                    binds.push((ctx.knowledge_dir.clone(), "/knowledge".to_string(), true));
                }

                let repo_name = std::path::Path::new(&task.repo_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                // Per-branch cache volumes — tasks on different branches get isolated
                // target dirs, while retries on the same branch reuse the same cache.
                // Global caches (cargo registry, bun) are shared across all branches.
                let branch = format!("task-{}", task.id);
                let target_vol = Sandbox::branch_volume_name(&repo_name, &branch, "target");
                let node_vol = Sandbox::branch_volume_name(&repo_name, &branch, "node-modules");
                // Warm branch caches from main on first use (async, fire-and-forget)
                {
                    let img = self.docker_image.clone();
                    let rn = repo_name.clone();
                    let br = branch.clone();
                    tokio::spawn(async move {
                        Sandbox::warm_branch_cache(&rn, &br, "target", &img).await;
                        Sandbox::warm_branch_cache(&rn, &br, "node-modules", &img).await;
                    });
                }

                let volumes_owned: Vec<(String, String)> = vec![
                    (target_vol, "/workspace/repo/target".to_string()),
                    (node_vol, "/workspace/repo/node_modules".to_string()),
                    (format!("borg-cache-{repo_name}-bun-cache"), "/home/bun/.bun/install/cache".to_string()),
                    (format!("borg-cache-{repo_name}-cargo-registry"), "/home/bun/.cargo/registry".to_string()),
                    ("borg-cache-rustup".to_string(), "/home/bun/.rustup".to_string()),
                ];

                let mut env_kv: Vec<(String, String)> = vec![
                    ("HOME".to_string(), ctx.session_dir.clone()),
                    ("RUSTUP_HOME".to_string(), "/home/bun/.rustup".to_string()),
                    ("CARGO_HOME".to_string(), "/home/bun/.cargo".to_string()),
                    ("CLAUDE_CODE_OAUTH_TOKEN".to_string(), oauth_token.clone()),
                ];
                if !gh_token.is_empty() {
                    env_kv.push(("GH_TOKEN".to_string(), gh_token.clone()));
                }

                let binds_ref: Vec<(&str, &str, bool)> = binds
                    .iter()
                    .map(|(h, c, ro)| (h.as_str(), c.as_str(), *ro))
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
                    // Inject --cidfile before the image name by re-building args.
                    // docker_command already set args; we insert --cidfile early.
                    let existing_args: Vec<_> = docker_cmd
                        .as_std()
                        .get_args()
                        .map(|a| a.to_os_string())
                        .collect();
                    let mut new_cmd = Command::new("docker");
                    // Insert --cidfile after "run" (first arg)
                    let (head, tail) = existing_args.split_at(1.min(existing_args.len()));
                    new_cmd.args(head);
                    new_cmd.arg("--cidfile");
                    new_cmd.arg(cid_path);
                    new_cmd.args(tail);
                    docker_cmd = new_cmd;
                }
                docker_cmd
                    .kill_on_drop(true)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .context("failed to spawn docker")?
            },
            SandboxMode::Direct => {
                let path = std::env::var("PATH")
                    .unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_string());
                let augmented_path = format!(
                    "{path}:{real_home}/.local/bin:/usr/local/bin"
                );
                Command::new(&self.claude_bin)
                    .args(&full_cmd[1..])
                    .kill_on_drop(true)
                    .current_dir(&ctx.work_dir)
                    .env("HOME", &ctx.session_dir)
                    .env("RUSTUP_HOME", &rustup_home)
                    .env("CARGO_HOME", &cargo_home)
                    .env("PATH", &augmented_path)
                    .env("CLAUDE_CODE_OAUTH_TOKEN", &oauth_token)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .with_context(|| format!("failed to spawn claude: {}", self.claude_bin))?
            },
        };

        // For Docker mode, send JSON input on stdin then close it.
        if is_docker {
            if let Some(mut stdin) = child.stdin.take() {
                let repo_test_cmd = ctx.repo_config.test_cmd.clone();
                let compile_check_cmd = if phase.compile_check {
                    derive_compile_check(&repo_test_cmd).unwrap_or_default()
                } else {
                    String::new()
                };
                let runs_test_cmd = if phase.runs_tests { repo_test_cmd.as_str() } else { "" };
                let payload = self.build_docker_input(
                    task,
                    phase,
                    &ctx,
                    &instruction,
                    &system_prompt,
                    &session_id,
                    &compile_check_cmd,
                    "",
                    runs_test_cmd,
                    &gh_token,
                );
                let _ = stdin.write_all(&payload).await;
                // stdin dropped here → EOF to container
            }
        }

        // Read container ID from cidfile (Docker writes it shortly after container start).
        if let Some(ref cid_path) = cidfile_path {
            let cid_path_clone = cid_path.clone();
            let stream_tx_cid = ctx.stream_tx.clone();
            let tid = task.id;
            tokio::spawn(async move {
                for _ in 0..30u8 {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    if let Ok(s) = tokio::fs::read_to_string(&cid_path_clone).await {
                        let cid = s.trim().to_string();
                        if !cid.is_empty() {
                            info!(task_id = tid, container_id = %cid, "docker container started");
                            let evt = serde_json::json!({
                                "type": "container_event",
                                "event": "container_id",
                                "id": cid,
                                "task_id": tid,
                            })
                            .to_string();
                            if let Some(tx) = stream_tx_cid {
                                let _ = tx.send(evt);
                            }
                            break;
                        }
                    }
                }
            });
        }

        let stdout = child.stdout.take().context("failed to take stdout")?;
        let stderr = child.stderr.take().context("failed to take stderr")?;

        let task_id = task.id;
        let phase_name = phase.name.clone();
        let timeout_s = self.timeout_s;
        let stream_tx = ctx.stream_tx.clone();
        let is_docker_io = is_docker;

        let io_future = async move {
            let mut raw_stream = String::new();
            let mut signal_json: Option<String> = None;
            let mut container_test_results: Vec<ContainerTestResult> = Vec::new();
            let mut stderr_tail: Vec<String> = Vec::new();
            let mut stdout_reader = BufReader::new(stdout).lines();
            let mut stderr_reader = BufReader::new(stderr).lines();

            loop {
                tokio::select! {
                    line = stdout_reader.next_line() => {
                        match line.context("error reading stdout")? {
                            Some(l) => {
                                if let Some(sig) = l.strip_prefix(BORG_SIGNAL_MARKER) {
                                    signal_json = Some(sig.to_string());
                                } else if let Some(r) = Self::parse_test_result(&l) {
                                    container_test_results.push(r);
                                } else {
                                    if let Some(tx) = &stream_tx {
                                        let _ = tx.send(l.clone());
                                    }
                                    raw_stream.push_str(&l);
                                    raw_stream.push('\n');
                                }
                            }
                            None => break,
                        }
                    }
                    line = stderr_reader.next_line() => {
                        if let Ok(Some(l)) = line {
                            if !l.is_empty() {
                                if is_docker_io {
                                    if let Some(evt) = l.strip_prefix(BORG_EVENT_MARKER) {
                                        if let Some(tx) = &stream_tx {
                                            let _ = tx.send(evt.to_string());
                                        }
                                    } else {
                                        warn!(task_id, phase = %phase_name, "container stderr: {}", l);
                                        if stderr_tail.len() >= 30 {
                                            stderr_tail.remove(0);
                                        }
                                        stderr_tail.push(l);
                                    }
                                } else {
                                    warn!(task_id, phase = %phase_name, "claude stderr: {}", l);
                                }
                            }
                        }
                    }
                }
            }

            while let Ok(Some(l)) = stderr_reader.next_line().await {
                if !l.is_empty() {
                    if is_docker_io {
                        if let Some(evt) = l.strip_prefix(BORG_EVENT_MARKER) {
                            if let Some(tx) = &stream_tx {
                                let _ = tx.send(evt.to_string());
                            }
                        } else {
                            warn!(task_id, phase = %phase_name, "container stderr: {}", l);
                            if stderr_tail.len() >= 30 {
                                stderr_tail.remove(0);
                            }
                            stderr_tail.push(l);
                        }
                    } else {
                        warn!(task_id, phase = %phase_name, "claude stderr: {}", l);
                    }
                }
            }

            let exit_status = child.wait().await.context("failed to wait for claude")?;
            let success = exit_status.success();

            if !success && is_docker_io && !stderr_tail.is_empty() {
                let evt = serde_json::json!({
                    "type": "container_event",
                    "event": "container_error",
                    "exit_code": exit_status.code().unwrap_or(-1),
                    "stderr_tail": stderr_tail.join("\n"),
                })
                .to_string();
                if let Some(tx) = &stream_tx {
                    let _ = tx.send(evt);
                }
            }

            anyhow::Ok((raw_stream, signal_json, container_test_results, success))
        };

        let (raw_stream, signal_json, container_test_results, success) = if timeout_s > 0 {
            match tokio::time::timeout(std::time::Duration::from_secs(timeout_s), io_future).await {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => return Err(e),
                Err(_elapsed) => {
                    warn!(task_id = task.id, phase = %phase.name, timeout_s, "claude subprocess timed out");
                    // Kill the Docker container — kill_on_drop only kills the CLI wrapper
                    if let Some(ref cid_path) = cidfile_path {
                        if let Ok(cid) = std::fs::read_to_string(cid_path) {
                            let cid = cid.trim().to_string();
                            if !cid.is_empty() {
                                let _ = std::process::Command::new("docker")
                                    .args(["stop", "--time=5", &cid])
                                    .output();
                            }
                        }
                        let _ = std::fs::remove_file(cid_path);
                    }
                    return Ok(PhaseOutput::failed(String::new()));
                },
            }
        } else {
            io_future.await?
        };

        if let Some(ref cid_path) = cidfile_path {
            let _ = std::fs::remove_file(cid_path);
        }

        let (output, new_session_id) = crate::event::parse_stream(&raw_stream);

        info!(
            task_id = task.id,
            phase = %phase.name,
            success,
            new_session_id = ?new_session_id,
            output_len = output.len(),
            "claude subprocess finished"
        );

        Ok(PhaseOutput {
            output,
            new_session_id,
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
    use super::derive_compile_check;

    #[test]
    fn bare_cargo_test() {
        assert_eq!(derive_compile_check("cargo test"), Some("cargo test --no-run".into()));
    }

    #[test]
    fn cargo_test_with_flags() {
        assert_eq!(
            derive_compile_check("cargo test --workspace"),
            Some("cargo test --workspace --no-run".into())
        );
    }

    #[test]
    fn whitespace_is_trimmed() {
        assert_eq!(
            derive_compile_check("  cargo test --workspace  "),
            Some("cargo test --workspace --no-run".into())
        );
    }

    #[test]
    fn non_cargo_command_returns_none() {
        assert_eq!(derive_compile_check("npm test"), None);
    }

    #[test]
    fn empty_string_returns_none() {
        assert_eq!(derive_compile_check(""), None);
    }
}
