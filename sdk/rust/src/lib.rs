pub mod client;
pub mod types;
pub mod upload;

pub use client::{
    BorgClient, BorgClientConfig, RevisionResponse, TaskBackendResponse, TaskMessagesResponse,
    TaskOutputsResponse, DEFAULT_BASE_URL, DEFAULT_POLL_INTERVAL_MS, DEFAULT_TIMEOUT_MS,
    UPLOAD_CHUNK_SIZE,
};
pub use types::*;
pub use upload::{guess_mime_type, is_expected_text_file, upload_directory, wait_for_ingestion};

#[derive(Debug, thiserror::Error)]
pub enum BorgError {
    #[error("auth: {0}")]
    Auth(String),

    #[error("{method} {path} -> {status}: {body}")]
    Api {
        method: String,
        path: String,
        status: u16,
        body: String,
    },

    #[error("request: {0}")]
    Request(String),

    #[error("timeout: {0}")]
    Timeout(String),
}
