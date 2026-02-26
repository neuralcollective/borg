use anyhow::Result;
use async_trait::async_trait;

use crate::types::{PhaseConfig, PhaseContext, PhaseOutput, Task};

#[async_trait]
pub trait AgentBackend: Send + Sync {
    async fn run_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        ctx: PhaseContext,
    ) -> Result<PhaseOutput>;

    async fn inject_message(&self, session_id: &str, message: &str) -> Result<()>;

    async fn interrupt(&self, session_id: &str) -> Result<()>;
}
