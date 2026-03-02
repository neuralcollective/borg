pub mod claude;
pub use claude::{extract_phase_result, provider_env_var};
pub mod codex;
pub mod event;
pub mod instruction;
pub mod ollama;

pub use ollama::OllamaBackend;
