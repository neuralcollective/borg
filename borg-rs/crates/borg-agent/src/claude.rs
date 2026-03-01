use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use borg_core::{
    agent::AgentBackend,
    sandbox::{Sandbox, SandboxMode},
    types::{PhaseConfig, PhaseContext, PhaseOutput, Task},
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tracing::{info, warn};

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

        let allowed_tools = if phase.allowed_tools.is_empty() {
            "Read,Glob,Grep,Write,Edit,Bash".to_string()
        } else {
            phase.allowed_tools.clone()
        };

        let mut claude_args = vec![
            "--model".to_string(),
            ctx.model.clone(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            "--allowedTools".to_string(),
            allowed_tools,
            "--max-turns".to_string(),
            "200".to_string(),
        ];

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
            claude_args.push(system_prompt);
        }

        // If task has a LexisNexis API key, generate MCP config and pass it
        let mcp_config_path = if ctx.api_keys.contains_key("lexisnexis") {
            let mcp_dir = format!("{}/mcp", ctx.session_dir);
            std::fs::create_dir_all(&mcp_dir).ok();
            let lexis_mcp_server =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../sidecar/lexis-mcp/server.js")
                    .canonicalize()
                    .unwrap_or_default();
            let config_json = serde_json::json!({
                "mcpServers": {
                    "lexisnexis": {
                        "command": "bun",
                        "args": ["run", lexis_mcp_server],
                        "env": {
                            "LEXISNEXIS_API_KEY": ctx.api_keys.get("lexisnexis").unwrap_or(&String::new()),
                        }
                    }
                }
            });
            let config_path = format!("{}/mcp-config.json", mcp_dir);
            std::fs::write(&config_path, config_json.to_string()).ok();
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
        claude_args.push(instruction);

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
                let mut binds = vec![
                    (ctx.worktree_path.as_str(), ctx.worktree_path.as_str()),
                    (ctx.session_dir.as_str(), ctx.session_dir.as_str()),
                ];
                if !ctx.setup_script.is_empty() {
                    binds.push((ctx.setup_script.as_str(), "/workspace/setup.sh"));
                }
                Sandbox::docker_command(&self.docker_image, &binds, &ctx.worktree_path, &full_cmd)
                    .kill_on_drop(true)
                    .env("HOME", &ctx.session_dir)
                    .env("CLAUDE_CODE_OAUTH_TOKEN", &oauth_token)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .context("failed to spawn docker")?
            },
            SandboxMode::Direct => {
                let path = std::env::var("PATH").unwrap_or_default();
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

        let stdout = child.stdout.take().context("failed to take stdout")?;
        let stderr = child.stderr.take().context("failed to take stderr")?;

        let task_id = task.id;
        let phase_name = phase.name.clone();
        let timeout_s = self.timeout_s;
        let stream_tx = ctx.stream_tx.clone();

        let io_future = async move {
            let mut raw_stream = String::new();
            let mut stdout_reader = BufReader::new(stdout).lines();
            let mut stderr_reader = BufReader::new(stderr).lines();

            loop {
                tokio::select! {
                    line = stdout_reader.next_line() => {
                        match line.context("error reading stdout")? {
                            Some(l) => {
                                if let Some(tx) = &stream_tx {
                                    let _ = tx.send(l.clone());
                                }
                                raw_stream.push_str(&l);
                                raw_stream.push('\n');
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
            anyhow::Ok((raw_stream, exit_status.success()))
        };

        let (raw_stream, success) = if timeout_s > 0 {
            match tokio::time::timeout(std::time::Duration::from_secs(timeout_s), io_future).await {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => return Err(e),
                Err(_elapsed) => {
                    warn!(task_id = task.id, phase = %phase.name, timeout_s, "claude subprocess timed out");
                    return Ok(PhaseOutput {
                        output: String::new(),
                        new_session_id: None,
                        raw_stream: String::new(),
                        success: false,
                    });
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
