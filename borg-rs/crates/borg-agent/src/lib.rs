pub mod claude;
pub use claude::extract_phase_result;
pub mod codex;
pub mod event;
pub mod gemini;
pub mod instruction;
pub mod ollama;

pub use gemini::GeminiBackend;
pub use ollama::OllamaBackend;
