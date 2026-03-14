use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use borg_core::{
    agent::AgentBackend,
    traits::{
        AgentError, BackendCapabilities, ChatContext, ChatRequest, ChatResponse, RetryPolicy,
    },
    types::{PhaseConfig, PhaseContext, PhaseOutput, Task},
};
use tracing::{info, warn};

/// Wraps any `AgentBackend` with retry/backoff logic.
pub struct ReliableBackend {
    inner: Arc<dyn AgentBackend>,
    policy: RetryPolicy,
}

impl ReliableBackend {
    pub fn new(inner: Arc<dyn AgentBackend>, policy: RetryPolicy) -> Self {
        Self { inner, policy }
    }

    pub fn wrap(inner: Arc<dyn AgentBackend>) -> Self {
        Self::new(inner, RetryPolicy::default())
    }

    fn compute_backoff(&self, attempt: u32) -> std::time::Duration {
        let base = self.policy.initial_backoff.as_secs_f64()
            * self.policy.backoff_multiplier.powi(attempt as i32);
        let capped = base.min(self.policy.max_backoff.as_secs_f64());
        let with_jitter = if self.policy.jitter {
            let jitter = rand::random::<f64>() * capped * 0.5;
            capped + jitter
        } else {
            capped
        };
        std::time::Duration::from_secs_f64(with_jitter)
    }

    fn classify_error(err: &anyhow::Error) -> AgentError {
        let msg = err.to_string().to_lowercase();

        if msg.contains("rate limit") || msg.contains("too many requests") || msg.contains("429") {
            AgentError::RateLimit { retry_after: None }
        } else if msg.contains("context") && (msg.contains("window") || msg.contains("too long")) {
            AgentError::ContextOverflow
        } else if msg.contains("unauthorized")
            || msg.contains("authentication")
            || msg.contains("invalid api key")
            || msg.contains("403")
        {
            AgentError::AuthError
        } else if msg.contains("insufficient") || msg.contains("quota") || msg.contains("balance")
        {
            AgentError::InsufficientBalance
        } else if msg.contains("timeout") || msg.contains("timed out") {
            AgentError::Timeout
        } else if msg.contains("500")
            || msg.contains("502")
            || msg.contains("503")
            || msg.contains("overloaded")
            || msg.contains("capacity")
        {
            AgentError::ServerError { status: 500 }
        } else {
            AgentError::Unknown {
                message: err.to_string(),
            }
        }
    }
}

#[async_trait]
impl AgentBackend for ReliableBackend {
    async fn run_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        ctx: PhaseContext,
    ) -> Result<PhaseOutput> {
        let mut last_err = None;
        for attempt in 0..=self.policy.max_retries {
            match self.inner.run_phase(task, phase, ctx.clone()).await {
                Ok(output) => return Ok(output),
                Err(err) => {
                    let classified = Self::classify_error(&err);
                    if !classified.is_retryable() || attempt == self.policy.max_retries {
                        warn!(
                            backend = self.inner.name(),
                            task_id = task.id,
                            phase = %phase.name,
                            attempt,
                            error = %classified,
                            "non-retryable error or max retries reached"
                        );
                        return Err(err);
                    }

                    let backoff = if let AgentError::RateLimit {
                        retry_after: Some(d),
                    } = &classified
                    {
                        *d
                    } else {
                        self.compute_backoff(attempt)
                    };

                    info!(
                        backend = self.inner.name(),
                        task_id = task.id,
                        phase = %phase.name,
                        attempt,
                        error = %classified,
                        backoff_ms = backoff.as_millis() as u64,
                        "retrying after error"
                    );

                    tokio::time::sleep(backoff).await;
                    last_err = Some(err);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("max retries exceeded")))
    }

    async fn run_chat(
        &self,
        request: &ChatRequest,
        ctx: &ChatContext,
    ) -> Result<ChatResponse> {
        let mut last_err = None;
        for attempt in 0..=self.policy.max_retries {
            match self.inner.run_chat(request, ctx).await {
                Ok(response) => return Ok(response),
                Err(err) => {
                    let classified = Self::classify_error(&err);
                    if !classified.is_retryable() || attempt == self.policy.max_retries {
                        warn!(
                            backend = self.inner.name(),
                            attempt,
                            error = %classified,
                            "chat: non-retryable error or max retries reached"
                        );
                        return Err(err);
                    }

                    let backoff = if let AgentError::RateLimit {
                        retry_after: Some(d),
                    } = &classified
                    {
                        *d
                    } else {
                        self.compute_backoff(attempt)
                    };

                    info!(
                        backend = self.inner.name(),
                        attempt,
                        error = %classified,
                        backoff_ms = backoff.as_millis() as u64,
                        "chat: retrying after error"
                    );

                    tokio::time::sleep(backoff).await;
                    last_err = Some(err);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("max retries exceeded")))
    }

    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_rate_limit() {
        let err = anyhow::anyhow!("Error 429: rate limit exceeded");
        let classified = ReliableBackend::classify_error(&err);
        assert!(classified.is_retryable());
        assert!(matches!(classified, AgentError::RateLimit { .. }));
    }

    #[test]
    fn classify_auth_error() {
        let err = anyhow::anyhow!("Error: unauthorized - invalid API key");
        let classified = ReliableBackend::classify_error(&err);
        assert!(!classified.is_retryable());
        assert!(matches!(classified, AgentError::AuthError));
    }

    #[test]
    fn classify_context_overflow() {
        let err = anyhow::anyhow!("context window is too long for this model");
        let classified = ReliableBackend::classify_error(&err);
        assert!(!classified.is_retryable());
        assert!(matches!(classified, AgentError::ContextOverflow));
    }

    #[test]
    fn classify_server_error() {
        let err = anyhow::anyhow!("HTTP 503: service overloaded");
        let classified = ReliableBackend::classify_error(&err);
        assert!(classified.is_retryable());
    }

    #[test]
    fn classify_timeout() {
        let err = anyhow::anyhow!("request timed out after 300s");
        let classified = ReliableBackend::classify_error(&err);
        assert!(classified.is_retryable());
        assert!(matches!(classified, AgentError::Timeout));
    }

    #[test]
    fn classify_insufficient_balance() {
        let err = anyhow::anyhow!("insufficient balance on account");
        let classified = ReliableBackend::classify_error(&err);
        assert!(!classified.is_retryable());
    }

    #[test]
    fn backoff_increases_exponentially() {
        let policy = RetryPolicy {
            jitter: false,
            ..Default::default()
        };
        let reliable = ReliableBackend::new(
            Arc::new(DummyBackend),
            policy,
        );
        let b0 = reliable.compute_backoff(0);
        let b1 = reliable.compute_backoff(1);
        let b2 = reliable.compute_backoff(2);
        assert!(b1 > b0);
        assert!(b2 > b1);
        // Max should be capped at 60s
        let b10 = reliable.compute_backoff(10);
        assert!(b10 <= std::time::Duration::from_secs(61));
    }

    struct DummyBackend;

    #[async_trait]
    impl AgentBackend for DummyBackend {
        async fn run_phase(
            &self,
            _task: &Task,
            _phase: &PhaseConfig,
            _ctx: PhaseContext,
        ) -> Result<PhaseOutput> {
            Ok(PhaseOutput::failed("dummy"))
        }
        fn name(&self) -> &str {
            "dummy"
        }
    }
}
