use std::process::Stdio;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use borg_core::{
    agent::AgentBackend,
    types::{PhaseConfig, PhaseContext, PhaseOutput, Task},
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{debug, info, warn};

/// Runs Codex (openai/codex) as the agent backend.
///
/// Codex is invoked via `codex exec` in non-interactive mode.
/// The codex app-server JSON-RPC protocol is planned but not yet wired up.
pub struct CodexBackend {
    pub api_key: String,
    pub model: String,
    pub reasoning_effort: String,
    pub codex_bin: String,
    pub timeout_s: u64,
    pub git_author_name: String,
    pub git_author_email: String,
    pub git_committer_name: String,
    pub git_committer_email: String,
}

impl CodexBackend {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            reasoning_effort: "medium".into(),
            codex_bin: "codex".into(),
            timeout_s: 0,
            git_author_name: "Borg".into(),
            git_author_email: "borg@localhost".into(),
            git_committer_name: "Borg".into(),
            git_committer_email: "borg@localhost".into(),
        }
    }

    pub fn with_reasoning_effort(mut self, effort: impl Into<String>) -> Self {
        self.reasoning_effort = effort.into();
        self
    }

    pub fn with_bin(mut self, bin: impl Into<String>) -> Self {
        self.codex_bin = bin.into();
        self
    }

    pub fn with_timeout(mut self, timeout_s: u64) -> Self {
        self.timeout_s = timeout_s;
        self
    }

    pub fn with_git_identity(
        mut self,
        author_name: &str,
        author_email: &str,
        committer_name: &str,
        committer_email: &str,
    ) -> Self {
        if !author_name.is_empty() {
            self.git_author_name = author_name.into();
        }
        if !author_email.is_empty() {
            self.git_author_email = author_email.into();
        }
        if !committer_name.is_empty() {
            self.git_committer_name = committer_name.into();
        }
        if !committer_email.is_empty() {
            self.git_committer_email = committer_email.into();
        }
        self
    }

    pub async fn is_available(&self) -> bool {
        tokio::process::Command::new(&self.codex_bin)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn is_warning_stderr(line: &str) -> bool {
        let l = line.trim().to_ascii_lowercase();
        l.starts_with("error:")
            || l.starts_with("error ")
            || l.starts_with("fatal:")
            || l.contains("unexpected status")
            || l.contains("unauthorized")
            || l.contains("failed to")
            || l.contains("panic!")
            || l.contains("panicked at")
            || (l.contains("thread '") && l.contains(" panicked"))
    }
}

#[async_trait]
impl AgentBackend for CodexBackend {
    async fn run_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        ctx: PhaseContext,
    ) -> Result<PhaseOutput> {
        if !self.is_available().await {
            bail!("codex binary not found: {}", self.codex_bin);
        }

        let instruction = crate::instruction::build_instruction(task, phase, &ctx, None);

        info!(
            task_id = task.id,
            phase = %phase.name,
            model = %self.model,
            "spawning codex subprocess"
        );

        let mut cmd = tokio::process::Command::new(&self.codex_bin);
        cmd.arg("exec")
            .arg("--model")
            .arg(&self.model)
            .arg("-c")
            .arg(format!(
                "model_reasoning_effort=\"{}\"",
                self.reasoning_effort
            ))
            .arg("--full-auto")
            .arg(&instruction)
            .current_dir(&ctx.work_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.env("GIT_AUTHOR_NAME", &self.git_author_name);
        cmd.env("GIT_AUTHOR_EMAIL", &self.git_author_email);
        cmd.env("GIT_COMMITTER_NAME", &self.git_committer_name);
        cmd.env("GIT_COMMITTER_EMAIL", &self.git_committer_email);
        if !self.api_key.is_empty() {
            cmd.env("OPENAI_API_KEY", &self.api_key);
        }
        let mut child = cmd
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("failed to spawn codex binary: {}", self.codex_bin))?;

        let stdout = child.stdout.take().context("failed to take stdout")?;
        let stderr = child.stderr.take().context("failed to take stderr")?;
        let stream_tx = ctx.stream_tx.clone();

        let mut output_lines = Vec::new();
        let mut had_fatal_stderr = false;
        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();
        let mut stdout_done = false;
        let mut stderr_done = false;
        let mut timed_out = false;
        let timeout_enabled = self.timeout_s > 0;
        let timeout_secs = self.timeout_s.max(1);
        let timeout = tokio::time::sleep(std::time::Duration::from_secs(timeout_secs));
        tokio::pin!(timeout);

        while !(stdout_done && stderr_done) {
            tokio::select! {
                _ = &mut timeout, if timeout_enabled => {
                    timed_out = true;
                    warn!(
                        task_id = task.id,
                        phase = %phase.name,
                        timeout_s = timeout_secs,
                        "codex phase timed out, terminating subprocess"
                    );
                    let _ = child.start_kill();
                    break;
                }
                line = stdout_reader.next_line(), if !stdout_done => {
                    match line.context("error reading stdout")? {
                        Some(l) => {
                            if let Some(tx) = &stream_tx {
                                let _ = tx.send(l.clone());
                            }
                            if output_lines.len() < 50_000 {
                                output_lines.push(l);
                            }
                        }
                        None => {
                            stdout_done = true;
                        }
                    }
                }
                line = stderr_reader.next_line(), if !stderr_done => {
                    match line {
                        Ok(Some(l)) => {
                            if !l.is_empty() {
                                if Self::is_warning_stderr(&l) {
                                    had_fatal_stderr = true;
                                    warn!(task_id = task.id, phase = %phase.name, "codex stderr: {}", l);
                                } else {
                                    debug!(task_id = task.id, phase = %phase.name, "codex stderr: {}", l);
                                }
                            }
                        }
                        Ok(None) => {
                            stderr_done = true;
                        }
                        Err(e) => {
                            warn!(task_id = task.id, phase = %phase.name, "codex stderr read error: {e}");
                            stderr_done = true;
                        }
                    }
                }
            }
        }

        let exit_status = child
            .wait()
            .await
            .context("failed to wait for codex process")?;
        let output = output_lines.join("\n");

        info!(
            task_id = task.id,
            phase = %phase.name,
            success = exit_status.success() && !had_fatal_stderr,
            output_len = output.len(),
            "codex subprocess finished"
        );

        Ok(PhaseOutput {
            output,
            new_session_id: None,
            raw_stream: String::new(),
            success: !timed_out && exit_status.success() && !had_fatal_stderr,
            signal_json: None,
            ran_in_docker: false,
            container_test_results: Vec::new(),
        })
    }
}
