use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ── Agent Backend ────────────────────────────────────────────────────────

/// Capabilities reported by a backend so callers can adapt behavior.
#[derive(Debug, Clone, Default)]
pub struct BackendCapabilities {
    pub supports_mcp: bool,
    pub supports_sessions: bool,
    pub supports_tools: bool,
    pub supports_streaming: bool,
    pub supports_sandbox: bool,
    pub supported_models: Vec<String>,
}

/// Request to run a chat agent turn.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub message: String,
    pub conversation_history: Vec<ChatMessage>,
    pub system_prompt: String,
    pub model: String,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub mcp_servers_json: serde_json::Value,
    pub max_turns: u32,
    pub max_budget_usd: Option<f64>,
}

/// A single message in a chat conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// Runtime context for chat execution.
#[derive(Debug, Clone)]
pub struct ChatContext {
    pub session_dir: String,
    pub session_id: Option<String>,
    pub oauth_token: String,
    pub provider_env: HashMap<String, String>,
    pub stream_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    pub borg_api_url: String,
    pub borg_api_token: String,
    pub project_id: i64,
    pub workspace_id: i64,
    pub mode: String,
    pub chat_thread: Option<String>,
    pub api_keys: HashMap<String, String>,
    pub knowledge_dir: String,
}

/// Response from a chat agent turn.
#[derive(Debug, Clone, Default)]
pub struct ChatResponse {
    pub text: String,
    pub session_id: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub tool_calls: Vec<ToolCallRecord>,
    pub raw_stream: String,
}

/// A recorded tool call for observability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub input_summary: String,
    pub output_summary: String,
    pub duration_ms: u64,
    pub success: bool,
    pub error: Option<String>,
}

// ── Search Provider ──────────────────────────────────────────────────────

/// Search result from a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub doc_id: String,
    pub score: f64,
    pub title: String,
    pub snippet: String,
    pub metadata: HashMap<String, String>,
}

/// Filters for search queries.
#[derive(Debug, Clone, Default)]
pub struct SearchFilters {
    pub project_id: Option<i64>,
    pub doc_type: Option<String>,
    pub jurisdiction: Option<String>,
    pub exclude_terms: Vec<String>,
    pub privileged_only: bool,
}

#[async_trait]
pub trait SearchProvider: Send + Sync {
    async fn query(
        &self,
        q: &str,
        filters: &SearchFilters,
        limit: usize,
        embedding_model: Option<&str>,
    ) -> Result<Vec<SearchResult>>;

    async fn index_document(
        &self,
        doc_id: &str,
        content: &str,
        embeddings: Option<&[f32]>,
        metadata: &HashMap<String, String>,
    ) -> Result<()>;

    async fn delete_document(&self, doc_id: &str) -> Result<()>;

    async fn health(&self) -> Result<bool>;
}

// ── Storage Provider ─────────────────────────────────────────────────────

#[async_trait]
pub trait StorageProvider: Send + Sync {
    async fn upload(&self, key: &str, data: bytes::Bytes, content_type: &str) -> Result<String>;
    async fn download(&self, key: &str) -> Result<bytes::Bytes>;
    async fn list(&self, prefix: &str) -> Result<Vec<String>>;
    async fn delete(&self, key: &str) -> Result<()>;
    async fn signed_url(&self, key: &str, expiry_secs: u64) -> Result<String>;
}

// ── Embedding Provider ───────────────────────────────────────────────────

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn model_name(&self) -> &str;
    fn dimensions(&self) -> usize;
}

// ── Message Channel ──────────────────────────────────────────────────────

/// Type of messaging channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Telegram,
    Discord,
    WhatsApp,
    Slack,
    Web,
    Email,
}

#[async_trait]
pub trait MessageChannel: Send + Sync {
    fn channel_type(&self) -> ChannelType;
    async fn send_message(&self, target: &str, content: &str) -> Result<String>;
    async fn edit_message(&self, message_id: &str, content: &str) -> Result<()>;
    async fn delete_message(&self, message_id: &str) -> Result<()>;
}

// ── Secret Store ─────────────────────────────────────────────────────────

#[async_trait]
pub trait SecretStore: Send + Sync {
    fn encrypt(&self, plaintext: &str) -> Result<String>;
    fn decrypt(&self, ciphertext: &str) -> Result<String>;
    async fn store(&self, key: &str, secret: &str) -> Result<()>;
    async fn retrieve(&self, key: &str) -> Result<Option<String>>;
}

// ── Ingestion Backend ────────────────────────────────────────────────────

/// A job to ingest a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionJob {
    pub id: String,
    pub project_id: i64,
    pub file_key: String,
    pub file_name: String,
    pub mime_type: String,
    pub metadata: HashMap<String, String>,
}

#[async_trait]
pub trait IngestionBackend: Send + Sync {
    async fn enqueue(&self, job: &IngestionJob) -> Result<()>;
    async fn dequeue(&self) -> Result<Option<IngestionJob>>;
    async fn ack(&self, job_id: &str) -> Result<()>;
}

// ── Backup Backend ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupEntry {
    pub id: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub size_bytes: u64,
    pub key: String,
}

#[async_trait]
pub trait BackupBackend: Send + Sync {
    async fn backup(&self, db_path: &str) -> Result<String>;
    async fn restore(&self, backup_id: &str, target_path: &str) -> Result<()>;
    async fn list(&self) -> Result<Vec<BackupEntry>>;
}

// ── Document Parser ──────────────────────────────────────────────────────

/// A parsed document with extracted text and structure.
#[derive(Debug, Clone, Default)]
pub struct ParsedDocument {
    pub text: String,
    pub metadata: HashMap<String, String>,
    pub sections: Vec<DocumentSection>,
    pub page_count: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct DocumentSection {
    pub heading: String,
    pub content: String,
    pub level: u8,
}

pub trait DocumentParser: Send + Sync {
    fn parse(&self, data: &[u8], filename: &str, mime_type: &str) -> Result<ParsedDocument>;
    fn supported_types(&self) -> Vec<String>;
}

// ── Provider Config ──────────────────────────────────────────────────────

/// Which LLM hosting backend to route through.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderConfig {
    Subscription,
    Direct { api_key: String },
    Bedrock { region: String, profile: Option<String> },
    Vertex { project_id: String, region: String },
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self::Subscription
    }
}

impl ProviderConfig {
    pub fn to_env_vars(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();
        match self {
            Self::Subscription => {}
            Self::Direct { api_key } => {
                env.insert("ANTHROPIC_API_KEY".into(), api_key.clone());
            }
            Self::Bedrock { region, profile } => {
                env.insert("CLAUDE_CODE_USE_BEDROCK".into(), "1".into());
                env.insert("AWS_REGION".into(), region.clone());
                if let Some(p) = profile {
                    env.insert("AWS_PROFILE".into(), p.clone());
                }
            }
            Self::Vertex { project_id, region } => {
                env.insert("CLAUDE_CODE_USE_VERTEX".into(), "1".into());
                env.insert("ANTHROPIC_VERTEX_PROJECT_ID".into(), project_id.clone());
                env.insert("CLOUD_ML_REGION".into(), region.clone());
            }
        }
        env
    }

    pub fn from_env() -> Self {
        let provider = std::env::var("BORG_PROVIDER").unwrap_or_default();
        match provider.as_str() {
            "bedrock" => Self::Bedrock {
                region: std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".into()),
                profile: std::env::var("AWS_PROFILE").ok(),
            },
            "vertex" => Self::Vertex {
                project_id: std::env::var("ANTHROPIC_VERTEX_PROJECT_ID").unwrap_or_default(),
                region: std::env::var("CLOUD_ML_REGION")
                    .unwrap_or_else(|_| "us-east1".into()),
            },
            "direct" => Self::Direct {
                api_key: std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
            },
            _ => Self::Subscription,
        }
    }
}

// ── Reliable Provider ────────────────────────────────────────────────────

/// Classified agent error for retry decisions.
#[derive(Debug, Clone)]
pub enum AgentError {
    RateLimit { retry_after: Option<std::time::Duration> },
    ServerError { status: u16 },
    Timeout,
    AuthError,
    ContextOverflow,
    InsufficientBalance,
    InvalidRequest { message: String },
    ProcessCrash { exit_code: Option<i32> },
    Unknown { message: String },
}

impl AgentError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimit { .. }
                | Self::ServerError { .. }
                | Self::Timeout
                | Self::ProcessCrash { .. }
        )
    }

    pub fn from_error_code(code: &str, message: &str) -> Self {
        match code {
            "rate_limit" => Self::RateLimit { retry_after: None },
            "auth" => Self::AuthError,
            "context_overflow" => Self::ContextOverflow,
            _ => {
                let lower = message.to_lowercase();
                if lower.contains("rate limit") || lower.contains("too many requests") {
                    Self::RateLimit { retry_after: None }
                } else if lower.contains("insufficient") || lower.contains("quota") {
                    Self::InsufficientBalance
                } else if lower.contains("context") || lower.contains("token limit") {
                    Self::ContextOverflow
                } else if lower.contains("unauthorized") || lower.contains("authentication") {
                    Self::AuthError
                } else {
                    Self::Unknown {
                        message: message.to_string(),
                    }
                }
            }
        }
    }
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RateLimit { retry_after } => {
                write!(f, "rate limited")?;
                if let Some(d) = retry_after {
                    write!(f, " (retry after {d:?})")?;
                }
                Ok(())
            }
            Self::ServerError { status } => write!(f, "server error ({status})"),
            Self::Timeout => write!(f, "timeout"),
            Self::AuthError => write!(f, "authentication error"),
            Self::ContextOverflow => write!(f, "context window overflow"),
            Self::InsufficientBalance => write!(f, "insufficient balance/quota"),
            Self::InvalidRequest { message } => write!(f, "invalid request: {message}"),
            Self::ProcessCrash { exit_code } => write!(f, "process crashed (exit={exit_code:?})"),
            Self::Unknown { message } => write!(f, "unknown error: {message}"),
        }
    }
}

/// Retry policy configuration.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub initial_backoff: std::time::Duration,
    pub max_backoff: std::time::Duration,
    pub backoff_multiplier: f64,
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: std::time::Duration::from_secs(1),
            max_backoff: std::time::Duration::from_secs(60),
            backoff_multiplier: 2.0,
            jitter: true,
        }
    }
}
