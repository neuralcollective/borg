use anyhow::Result;
use async_trait::async_trait;

use crate::traits::{BackendCapabilities, ChatContext, ChatRequest, ChatResponse};
use crate::types::{PhaseConfig, PhaseContext, PhaseOutput, Task};

#[async_trait]
pub trait AgentBackend: Send + Sync {
    /// Execute a pipeline phase (existing interface, unchanged).
    async fn run_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        ctx: PhaseContext,
    ) -> Result<PhaseOutput>;

    /// Execute a chat agent turn. Default returns "not supported" for backends
    /// that only handle pipeline phases.
    async fn run_chat(
        &self,
        _request: &ChatRequest,
        _ctx: &ChatContext,
    ) -> Result<ChatResponse> {
        anyhow::bail!("{} does not support chat", self.name())
    }

    /// Report what this backend supports.
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::default()
    }

    /// Backend identifier (e.g. "claude", "agent-sdk", "codex", "ollama").
    fn name(&self) -> &str;
}
