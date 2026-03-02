use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use borg_core::{
    agent::AgentBackend,
    sandbox::{Sandbox, SandboxMode},
    types::{PhaseConfig, PhaseContext, PhaseOutput, Task},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
};
use tracing::{info, warn};

const BORG_SIGNAL_MARKER: &str = "---BORG_SIGNAL---";

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
        }
    }

    pub fn with_timeout(mut self, timeout_s: u64) -> Self {
        self.timeout_s = timeout_s;
        self
    }

    /// Refresh OAuth token (triggers CLI refresh if near-expiry, then re-reads from disk).
    fn fresh_oauth_token(&self, fallback: &str) -> String {
        borg_core::config::refresh_oauth_token(&self.credentials_path, fallback)
    }

    /// Build JSON payload for the container entrypoint (Docker mode).
    fn build_docker_input(
        task: &Task,
        phase: &PhaseConfig,
        ctx: &PhaseContext,
        instruction: &str,
        system_prompt: &str,
        session_id: &str,
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
        let gh_token = std::env::var("GH_TOKEN").unwrap_or_default();

        let home = std::env::var("HOME").unwrap_or_default();
        let gitconfig = std::fs::read_to_string(format!("{home}/.gitconfig")).unwrap_or_default();
        let author_name = gitconfig.lines()
            .find(|l| l.trim_start().starts_with("name ="))
            .and_then(|l| l.splitn(2, '=').nth(1))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "Borg".to_string());
        let author_email = gitconfig.lines()
            .find(|l| l.trim_start().starts_with("email ="))
            .and_then(|l| l.splitn(2, '=').nth(1))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "borg@localhost".to_string());

        let mut payload = serde_json::json!({
            "prompt": instruction,
            "model": ctx.model,
            "systemPrompt": system_prompt,
            "allowedTools": phase.allowed_tools,
            "maxTurns": 200,
            "repoUrl": task.repo_path,
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

        serde_json::to_vec(&payload).unwrap_or_default()
    }

    fn host_mirror_path(task: &Task) -> String {
        let repo_name = std::path::Path::new(&task.repo_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        format!("/home/shulgin/borg-data/mirrors/{repo_name}.git")
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
            let git = borg_core::git::Git::new(&ctx.worktree_path);
            git.ls_files(&ctx.worktree_path).ok()
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

        // For legal mode tasks, always include the unified legal MCP server.
        // Free tools (CourtListener, EDGAR, etc.) work without keys.
        // BYOK tools (LexisNexis, Westlaw, etc.) activate when keys are present.
        let mcp_config_path = if ctx.task.mode == "lawborg" {
            let mcp_dir = format!("{}/mcp", ctx.session_dir);
            std::fs::create_dir_all(&mcp_dir).ok();
            let legal_mcp_path = if let Ok(p) = std::env::var("LAWBORG_MCP_SERVER") {
                std::path::PathBuf::from(p)
            } else {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../sidecar/lawborg-mcp/server.js")
            };
            let legal_mcp_server = match legal_mcp_path.canonicalize() {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("lawborg MCP server not found at {}: {e}", legal_mcp_path.display());
                    return Err(anyhow::anyhow!("lawborg MCP server not found: {e}"));
                }
            };
            let mut env_vars = serde_json::Map::new();
            for (provider, key) in &ctx.api_keys {
                let env_name = match provider.as_str() {
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
                env_vars.insert(env_name.into(), serde_json::Value::String(key.clone()));
            }
            let config_json = serde_json::json!({
                "mcpServers": {
                    "legal": {
                        "command": "bun",
                        "args": ["run", legal_mcp_server],
                        "env": env_vars,
                    }
                }
            });
            let config_path = format!("{}/mcp-config.json", mcp_dir);
            std::fs::write(&config_path, config_json.to_string())
                .with_context(|| format!("failed to write MCP config to {config_path}"))?;
            Some(config_path)
        } else {
            None
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

        // Determine effective mode: only apply sandbox when the phase requests it
        let effective_mode = if phase.use_docker {
            &self.sandbox_mode
        } else {
            &SandboxMode::Direct
        };

        let oauth_token = self.fresh_oauth_token(&ctx.oauth_token);

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

        let mut child = match effective_mode {
            SandboxMode::Bwrap => {
                let writable = [ctx.worktree_path.as_str(), ctx.session_dir.as_str()];
                Sandbox::bwrap_command(&writable, &ctx.worktree_path, &full_cmd)
                    .kill_on_drop(true)
                    .env("HOME", &ctx.session_dir)
                    .env("CLAUDE_CODE_OAUTH_TOKEN", &oauth_token)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .context("failed to spawn bwrap")?
            },
            SandboxMode::Docker => {
                // Session dir (rw) + optional bare mirror (ro) + optional setup script (ro).
                // The container clones the repo itself; no worktree bind needed.
                let host_mirror = Self::host_mirror_path(task);
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

                let gh_token = std::env::var("GH_TOKEN").unwrap_or_default();
                let mut env_kv: Vec<(String, String)> = vec![
                    ("HOME".to_string(), ctx.session_dir.clone()),
                    ("CLAUDE_CODE_OAUTH_TOKEN".to_string(), oauth_token.clone()),
                ];
                if !gh_token.is_empty() {
                    env_kv.push(("GH_TOKEN".to_string(), gh_token));
                }

                let binds_ref: Vec<(&str, &str, bool)> = binds
                    .iter()
                    .map(|(h, c, ro)| (h.as_str(), c.as_str(), *ro))
                    .collect();
                let env_ref: Vec<(&str, &str)> = env_kv
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();

                Sandbox::docker_command(
                    &self.docker_image,
                    &binds_ref,
                    "",
                    &[],
                    &env_ref,
                )
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
                let home = std::env::var("HOME").unwrap_or_default();
                let augmented_path = format!(
                    "{path}:{home}/.local/bin:/usr/local/bin"
                );
                Command::new(&self.claude_bin)
                    .args(&full_cmd[1..])
                    .kill_on_drop(true)
                    .current_dir(&ctx.worktree_path)
                    .env("HOME", &ctx.session_dir)
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
                let payload = Self::build_docker_input(
                    task,
                    phase,
                    &ctx,
                    &instruction,
                    &system_prompt,
                    &session_id,
                );
                let _ = stdin.write_all(&payload).await;
                // stdin dropped here → EOF to container
            }
        }

        let stdout = child.stdout.take().context("failed to take stdout")?;
        let stderr = child.stderr.take().context("failed to take stderr")?;

        let task_id = task.id;
        let phase_name = phase.name.clone();
        let timeout_s = self.timeout_s;
        let stream_tx = ctx.stream_tx.clone();

        let io_future = async move {
            let mut raw_stream = String::new();
            let mut signal_json: Option<String> = None;
            let mut stdout_reader = BufReader::new(stdout).lines();
            let mut stderr_reader = BufReader::new(stderr).lines();

            loop {
                tokio::select! {
                    line = stdout_reader.next_line() => {
                        match line.context("error reading stdout")? {
                            Some(l) => {
                                if let Some(sig) = l.strip_prefix(BORG_SIGNAL_MARKER) {
                                    signal_json = Some(sig.to_string());
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
                                warn!(task_id, phase = %phase_name, "claude stderr: {}", l);
                            }
                        }
                    }
                }
            }

            while let Ok(Some(l)) = stderr_reader.next_line().await {
                if !l.is_empty() {
                    warn!(task_id, phase = %phase_name, "claude stderr: {}", l);
                }
            }

            let exit_status = child.wait().await.context("failed to wait for claude")?;
            anyhow::Ok((raw_stream, signal_json, exit_status.success()))
        };

        let (raw_stream, signal_json, success) = if timeout_s > 0 {
            match tokio::time::timeout(std::time::Duration::from_secs(timeout_s), io_future).await {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => return Err(e),
                Err(_elapsed) => {
                    warn!(task_id = task.id, phase = %phase.name, timeout_s, "claude subprocess timed out");
                    return Ok(PhaseOutput::failed(String::new()));
                },
            }
        } else {
            io_future.await?
        };

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
        })
    }

    async fn inject_message(&self, session_id: &str, message: &str) -> Result<()> {
        warn!(
            session_id = %session_id,
            msg_len = message.len(),
            "inject_message not yet implemented (requires TypeScript sidecar extension)"
        );
        Ok(())
    }

    async fn interrupt(&self, session_id: &str) -> Result<()> {
        warn!(session_id = %session_id, "interrupt not yet implemented");
        Ok(())
    }
}
