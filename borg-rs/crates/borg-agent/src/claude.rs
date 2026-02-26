use anyhow::{Context, Result};
use async_trait::async_trait;
use borg_core::{
    agent::AgentBackend,
    sandbox::{Sandbox, SandboxMode},
    types::{PhaseConfig, PhaseContext, PhaseOutput, Task},
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use std::process::Stdio;
use tracing::{info, warn};

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
}

impl ClaudeBackend {
    pub fn new(claude_bin: impl Into<String>, sandbox_mode: SandboxMode, docker_image: impl Into<String>) -> Self {
        Self {
            claude_bin: claude_bin.into(),
            sandbox_mode,
            docker_image: docker_image.into(),
            timeout_s: 0,
        }
    }

    pub fn with_timeout(mut self, timeout_s: u64) -> Self {
        self.timeout_s = timeout_s;
        self
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
        let instruction = crate::instruction::build_instruction(task, phase, &ctx, file_listing.as_deref());

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
                    .env("ANTHROPIC_API_KEY", &ctx.oauth_token)
                    .env("CLAUDE_CODE_OAUTH_TOKEN", &ctx.oauth_token)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .context("failed to spawn bwrap")?
            }
            SandboxMode::Docker => {
                let binds = [
                    (ctx.worktree_path.as_str(), ctx.worktree_path.as_str()),
                    (ctx.session_dir.as_str(), ctx.session_dir.as_str()),
                ];
                Sandbox::docker_command(&self.docker_image, &binds, &ctx.worktree_path, &full_cmd)
                    .kill_on_drop(true)
                    .env("HOME", &ctx.session_dir)
                    .env("ANTHROPIC_API_KEY", &ctx.oauth_token)
                    .env("CLAUDE_CODE_OAUTH_TOKEN", &ctx.oauth_token)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .context("failed to spawn docker")?
            }
            SandboxMode::Direct => {
                Command::new(&self.claude_bin)
                    .args(&full_cmd[1..])
                    .kill_on_drop(true)
                    .current_dir(&ctx.worktree_path)
                    .env("HOME", &ctx.session_dir)
                    .env("ANTHROPIC_API_KEY", &ctx.oauth_token)
                    .env("CLAUDE_CODE_OAUTH_TOKEN", &ctx.oauth_token)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .with_context(|| format!("failed to spawn claude: {}", self.claude_bin))?
            }
        };

        let stdout = child.stdout.take().context("failed to take stdout")?;
        let stderr = child.stderr.take().context("failed to take stderr")?;

        let task_id = task.id;
        let phase_name = phase.name.clone();
        let timeout_s = self.timeout_s;

        let io_future = async move {
            let mut raw_stream = String::new();
            let mut stdout_reader = BufReader::new(stdout).lines();
            let mut stderr_reader = BufReader::new(stderr).lines();

            loop {
                tokio::select! {
                    line = stdout_reader.next_line() => {
                        match line.context("error reading stdout")? {
                            Some(l) => {
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
                }
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
