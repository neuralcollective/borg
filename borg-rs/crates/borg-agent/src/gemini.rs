use std::process::Stdio;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use borg_core::{
    agent::AgentBackend,
    types::{PhaseConfig, PhaseContext, PhaseOutput, Task},
};
use tracing::info;

use crate::drain::{drain_child, DrainConfig};

/// Runs Gemini CLI (@google/gemini-cli) as the agent backend.
///
/// Gemini is invoked non-interactively via `gemini --approval-mode=yolo <prompt>`.
pub struct GeminiBackend {
    pub api_key: String,
    pub gemini_bin: String,
    pub timeout_s: u64,
}

impl GeminiBackend {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            gemini_bin: "gemini".into(),
            timeout_s: 0,
        }
    }

    pub fn with_bin(mut self, bin: impl Into<String>) -> Self {
        self.gemini_bin = bin.into();
        self
    }

    pub fn with_timeout(mut self, timeout_s: u64) -> Self {
        self.timeout_s = timeout_s;
        self
    }

    pub async fn is_available(&self) -> bool {
        tokio::process::Command::new(&self.gemini_bin)
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
            || l.contains("panic!")
    }
}

#[async_trait]
impl AgentBackend for GeminiBackend {
    async fn run_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        ctx: PhaseContext,
    ) -> Result<PhaseOutput> {
        if !self.is_available().await {
            bail!("gemini binary not found: {}", self.gemini_bin);
        }

        let instruction = crate::instruction::build_instruction(task, phase, &ctx, None);

        info!(
            task_id = task.id,
            phase = %phase.name,
            "spawning gemini subprocess"
        );

        let mut cmd = tokio::process::Command::new(&self.gemini_bin);
        cmd.arg("--approval-mode=yolo")
            .arg(&instruction)
            .current_dir(&ctx.work_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if !self.api_key.is_empty() {
            cmd.env("GEMINI_API_KEY", &self.api_key);
        }

        let mut child = cmd
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("failed to spawn gemini binary: {}", self.gemini_bin))?;

        let drain = drain_child(
            &mut child,
            DrainConfig {
                backend: "gemini",
                task_id: task.id,
                phase_name: &phase.name,
                timeout_s: self.timeout_s,
                stream_tx: ctx.stream_tx.clone(),
                is_warning_stderr: Self::is_warning_stderr,
            },
        )
        .await?;

        let exit_status = child
            .wait()
            .await
            .context("failed to wait for gemini process")?;

        info!(
            task_id = task.id,
            phase = %phase.name,
            success = exit_status.success() && !drain.had_fatal_stderr,
            output_len = drain.output.len(),
            "gemini subprocess finished"
        );

        Ok(PhaseOutput {
            output: drain.output,
            new_session_id: None,
            raw_stream: String::new(),
            success: !drain.timed_out && exit_status.success() && !drain.had_fatal_stderr,
            signal_json: None,
            ran_in_docker: false,
            container_test_results: Vec::new(),
        })
    }

    fn name(&self) -> &str {
        "gemini"
    }

    fn capabilities(&self) -> borg_core::BackendCapabilities {
        borg_core::BackendCapabilities {
            supports_mcp: false,
            supports_sessions: false,
            supports_tools: true,
            supports_streaming: true,
            supports_sandbox: false,
            supported_models: vec!["gemini-2.5-pro".into(), "gemini-2.5-flash".into()],
        }
    }
}
