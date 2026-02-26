use std::process::Stdio;
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use borg_core::{
    agent::AgentBackend,
    types::{PhaseConfig, PhaseContext, PhaseOutput, Task},
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{info, warn};

/// Runs Codex (openai/codex) as the agent backend.
///
/// Codex is invoked via the `codex` CLI with `--full-auto` mode.
/// The codex app-server JSON-RPC protocol is planned but not yet wired up.
pub struct CodexBackend {
    pub api_key: String,
    pub model: String,
    pub codex_bin: String,
}

impl CodexBackend {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            codex_bin: "codex".into(),
        }
    }

    pub fn with_bin(mut self, bin: impl Into<String>) -> Self {
        self.codex_bin = bin.into();
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

    fn build_instruction(&self, task: &Task, phase: &PhaseConfig, ctx: &PhaseContext) -> String {
        let mut instruction = String::new();

        if phase.include_task_context {
            instruction.push_str(&format!("Task: {}\n\n{}\n\n---\n\n", task.title, task.description));
        }

        instruction.push_str(&phase.instruction);

        if !task.last_error.is_empty() && !phase.error_instruction.is_empty() {
            let error_section = phase.error_instruction.replace("{ERROR}", &task.last_error);
            instruction.push('\n');
            instruction.push_str(&error_section);
        }

        if !ctx.pending_messages.is_empty() {
            instruction.push_str("\n\n---\nThe following messages were sent by the user or director while this task was queued:\n");
            for (role, content) in &ctx.pending_messages {
                instruction.push_str(&format!("\n[{}]: {}", role, content));
            }
        }

        instruction
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

        let instruction = self.build_instruction(task, phase, &ctx);

        info!(
            task_id = task.id,
            phase = %phase.name,
            model = %self.model,
            "spawning codex subprocess"
        );

        let mut child = tokio::process::Command::new(&self.codex_bin)
            .arg("--model")
            .arg(&self.model)
            .arg("--approval-mode")
            .arg("full-auto")
            .arg(&instruction)
            .current_dir(&ctx.worktree_path)
            .env("OPENAI_API_KEY", &self.api_key)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn codex binary: {}", self.codex_bin))?;

        let stdout = child.stdout.take().context("failed to take stdout")?;
        let stderr = child.stderr.take().context("failed to take stderr")?;

        let mut output_lines = Vec::new();
        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        loop {
            tokio::select! {
                line = stdout_reader.next_line() => {
                    match line.context("error reading stdout")? {
                        Some(l) => output_lines.push(l),
                        None => break,
                    }
                }
                line = stderr_reader.next_line() => {
                    if let Ok(Some(l)) = line {
                        if !l.is_empty() {
                            warn!(task_id = task.id, phase = %phase.name, "codex stderr: {}", l);
                        }
                    }
                }
            }
        }

        while let Ok(Some(l)) = stderr_reader.next_line().await {
            if !l.is_empty() {
                warn!(task_id = task.id, phase = %phase.name, "codex stderr: {}", l);
            }
        }

        let exit_status = child.wait().await.context("failed to wait for codex process")?;
        let output = output_lines.join("\n");

        info!(
            task_id = task.id,
            phase = %phase.name,
            success = exit_status.success(),
            output_len = output.len(),
            "codex subprocess finished"
        );

        Ok(PhaseOutput {
            output,
            new_session_id: None,
            raw_stream: String::new(),
            success: exit_status.success(),
        })
    }

    async fn inject_message(&self, session_id: &str, message: &str) -> Result<()> {
        // Codex app-server JSON-RPC injection — not yet implemented
        warn!(
            session_id = %session_id,
            msg_len = message.len(),
            "inject_message not yet implemented for CodexBackend"
        );
        Ok(())
    }

    async fn interrupt(&self, session_id: &str) -> Result<()> {
        warn!(session_id = %session_id, "interrupt not yet implemented for CodexBackend");
        Ok(())
    }
}
