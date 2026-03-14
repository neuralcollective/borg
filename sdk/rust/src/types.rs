use serde::{Deserialize, Serialize};

// --- Projects ---

#[derive(Debug, Clone, Serialize)]
pub struct CreateProjectBody {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matter_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privilege_level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UpdateProjectBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matter_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opposing_counsel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privilege_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_template_id: Option<Option<i64>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectTaskCounts {
    pub total: i64,
    pub pending: i64,
    pub running: i64,
    pub done: i64,
    pub failed: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub mode: String,
    pub repo_path: String,
    pub client_name: String,
    pub case_number: String,
    pub jurisdiction: String,
    pub matter_type: String,
    pub opposing_counsel: String,
    pub deadline: Option<String>,
    pub privilege_level: String,
    pub status: String,
    pub default_template_id: Option<i64>,
    pub session_privileged: bool,
    pub created_at: String,
    pub task_counts: Option<ProjectTaskCounts>,
}

// --- Tasks ---

#[derive(Debug, Clone, Serialize)]
pub struct CreateTaskBody {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_exhaustive_corpus_review: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify_chat: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_thread: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PatchTaskBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskOutput {
    pub id: i64,
    pub task_id: i64,
    pub phase: String,
    pub output: String,
    pub exit_code: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskMessage {
    pub id: i64,
    pub task_id: i64,
    pub role: String,
    pub content: String,
    pub created_at: String,
    pub delivered_phase: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub repo_path: String,
    pub branch: String,
    pub status: String,
    pub attempt: i64,
    pub max_attempts: i64,
    pub last_error: String,
    pub mode: String,
    pub project_id: i64,
    pub task_type: String,
    pub requires_exhaustive_corpus_review: bool,
    pub review_status: Option<String>,
    pub revision_count: i64,
    pub outputs: Option<Vec<TaskOutput>>,
    pub structured_data: Option<serde_json::Value>,
}

// --- Files & Uploads ---

#[derive(Debug, Clone, Serialize)]
pub struct CreateUploadSessionBody {
    pub file_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub file_size: i64,
    pub chunk_size: i64,
    pub total_chunks: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_zip: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privileged: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UploadSession {
    pub session_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectFile {
    pub id: i64,
    pub project_id: i64,
    pub file_name: String,
    pub source_path: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub privileged: bool,
    pub has_text: bool,
    pub text_chars: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectFilesSummary {
    pub total_files: i64,
    pub text_files: i64,
    pub privileged_files: Option<i64>,
    pub total_bytes: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectFilesResponse {
    pub total: Option<i64>,
    pub summary: Option<ProjectFilesSummary>,
    pub items: Option<Vec<ProjectFile>>,
}

// --- Documents ---

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectDocument {
    pub task_id: i64,
    pub branch: String,
    pub path: String,
    pub repo_slug: String,
    pub task_title: String,
    pub task_status: String,
}

// --- Chat ---

#[derive(Debug, Clone, Serialize)]
pub struct ChatPostBody {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub sender: Option<String>,
    pub text: String,
    pub ts: Option<serde_json::Value>,
    pub thread: Option<String>,
    pub raw_stream: Option<String>,
}

// --- Search ---

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResult {
    pub id: Option<i64>,
    pub file_name: Option<String>,
    pub snippet: Option<String>,
    pub score: Option<f64>,
    pub project_id: Option<i64>,
}

// --- System ---

#[derive(Debug, Clone, Deserialize)]
pub struct SystemStatus {
    pub running: i64,
    pub queued: i64,
}

// --- Upload helper ---

#[derive(Debug, Clone)]
pub struct UploadedFile {
    pub relative_path: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub expected_text: bool,
}
