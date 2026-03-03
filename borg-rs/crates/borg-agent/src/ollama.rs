use anyhow::Result;
use async_trait::async_trait;
use borg_core::{
    agent::AgentBackend,
    types::{PhaseConfig, PhaseContext, PhaseOutput, Task},
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Calls a locally-hosted Ollama model via its native chat API.
///
/// Intended for privacy-sensitive pipelines (legal, HR, medical) where
/// task content must not leave the local machine. No tool-calling support;
/// phases that require tool use will receive plain-text output only.
pub struct OllamaBackend {
    pub base_url: String,
    pub model: String,
    pub timeout_secs: u64,
    http: reqwest::Client,
}

fn build_client(timeout_secs: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .expect("failed to build HTTP client")
}

impl OllamaBackend {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        let timeout_secs = 300u64;
        Self {
            base_url: base_url.into(),
            model: model.into(),
            http: build_client(timeout_secs),
            timeout_secs,
        }
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self.http = build_client(secs);
        self
    }
}

#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    content: String,
}

#[async_trait]
impl AgentBackend for OllamaBackend {
    async fn run_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        ctx: PhaseContext,
    ) -> Result<PhaseOutput> {
        let user_content = crate::instruction::build_instruction(task, phase, &ctx, None);

        let mut messages = Vec::new();
        if !phase.system_prompt.is_empty() {
            messages.push(OllamaMessage {
                role: "system".into(),
                content: phase.system_prompt.clone(),
            });
        }
        messages.push(OllamaMessage {
            role: "user".into(),
            content: user_content,
        });

        let request_body = OllamaChatRequest {
            model: self.model.clone(),
            messages,
            stream: false,
        };

        info!(
            task_id = task.id,
            phase = %phase.name,
            model = %self.model,
            base_url = %self.base_url,
            "calling ollama chat API"
        );

        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));

        let response = match self.http.post(&url).json(&request_body).send().await {
            Ok(r) => r,
            Err(e) if e.is_timeout() => {
                warn!(
                    task_id = task.id,
                    phase = %phase.name,
                    timeout_secs = self.timeout_secs,
                    "ollama request timed out"
                );
                return Ok(PhaseOutput::failed(format!(
                    "Ollama request timed out after {}s",
                    self.timeout_secs
                )));
            },
            Err(e) => {
                warn!(task_id = task.id, phase = %phase.name, "ollama request failed: {}", e);
                return Ok(PhaseOutput::failed(format!("Ollama request failed: {}", e)));
            },
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            warn!(
                task_id = task.id,
                phase = %phase.name,
                status = %status,
                "ollama returned non-200: {}",
                body
            );
            return Ok(PhaseOutput::failed(format!("Ollama error {}: {}", status, body)));
        }

        let parsed: OllamaChatResponse = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                warn!(task_id = task.id, phase = %phase.name, "failed to parse ollama response: {}", e);
                return Ok(PhaseOutput::failed(format!(
                    "Failed to parse Ollama response: {}",
                    e
                )));
            },
        };

        let output = parsed.message.content;

        info!(
            task_id = task.id,
            phase = %phase.name,
            output_len = output.len(),
            "ollama response received"
        );

        Ok(PhaseOutput {
            raw_stream: output.clone(),
            output,
            new_session_id: None,
            success: true,
            signal_json: None,
            ran_in_docker: false,
            container_test_results: Vec::new(),
        })
    }
}
