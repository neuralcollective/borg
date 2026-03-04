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
}
