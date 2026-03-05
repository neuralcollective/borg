pub mod claude;
pub use claude::extract_phase_result;
pub mod codex;
pub mod gemini;
pub mod event;
pub mod instruction;
pub mod ollama;

pub use ollama::OllamaBackend;
pub use gemini::GeminiBackend;
