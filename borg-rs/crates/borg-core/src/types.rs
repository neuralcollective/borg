use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── Transport / Messaging ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Telegram,
    WhatsApp,
    Discord,
    Web,
}

/// Identifies the originating chat for reply routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sender {
    pub transport: Transport,
    /// Chat JID / channel ID / user ID depending on transport.
    pub chat_id: String,
    /// Optional message ID to reply to.
    pub reply_to: Option<String>,
}

// ── Pipeline Mode Enums ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseType {
    /// No-op setup phase; transitions immediately to next.
    Setup,
    /// Runs an AI agent (direct or in Docker).
    Agent,
    /// Runs a git rebase operation with optional agent fix.
    Rebase,
    /// Runs a lint command; spawns an agent to fix errors if any.
    LintFix,
}

impl Default for PhaseType {
    fn default() -> Self {
        Self::Agent
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationType {
    /// Creates GitHub PRs and manages merge queue.
    GitPr,
    /// No VCS integration (e.g. legal/document pipelines).
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedOutputType {
    Task,
    Proposal,
}

// ── Pipeline Task ────────────────────────────────────────────────────────

/// A pipeline task as stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub repo_path: String,
    /// Git branch name for this task's worktree.
    pub branch: String,
    /// Current pipeline phase / status (e.g. "backlog", "spec", "impl", "done").
    pub status: String,
    pub attempt: i64,
    pub max_attempts: i64,
    /// Output from the last failed phase, passed as context to the next attempt.
    pub last_error: String,
    /// Who created the task (chat JID, "pipeline", "seed", etc.).
    pub created_by: String,
    /// Chat to notify on completion (may be empty).
    pub notify_chat: String,
    pub created_at: DateTime<Utc>,
    /// Claude Code session ID for resumption.
    pub session_id: String,
    /// Pipeline mode name (e.g. "sweborg", "lawborg", "webborg").
    pub mode: String,
    /// Agent backend override (e.g. "claude", "codex"). Empty = use global default.
    pub backend: String,
}

/// A user-facing proposal that can be promoted to a Task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub id: i64,
    pub repo_path: String,
    pub title: String,
    pub description: String,
    pub rationale: String,
    /// "proposed" | "approved" | "dismissed"
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub triage_score: i64,
    pub triage_impact: i64,
    pub triage_feasibility: i64,
    pub triage_risk: i64,
    pub triage_effort: i64,
    pub triage_reasoning: String,
}

/// A pending merge-queue entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueEntry {
    pub id: i64,
    pub task_id: i64,
    pub branch: String,
    pub repo_path: String,
    /// "pending" | "merging" | "merged" | "failed"
    pub status: String,
    pub queued_at: DateTime<Utc>,
    pub pr_number: i64,
}

// ── Config Types ─────────────────────────────────────────────────────────

/// Per-repository pipeline configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    pub path: String,
    pub test_cmd: String,
    pub prompt_file: String,
    /// Pipeline mode name (default: "sweborg").
    pub mode: String,
    /// Is this the primary self-hosted repo (triggers self-update on merge)?
    pub is_self: bool,
    /// Auto-merge PRs when tests pass (false = manual merge mode).
    pub auto_merge: bool,
    /// Optional lint command for the lint_fix phase. Falls back to `.borg/lint.sh`.
    pub lint_cmd: String,
    /// Agent backend override for this repo. Empty = use global default.
    pub backend: String,
}

// ── Phase Config ─────────────────────────────────────────────────────────

/// Configuration for a single pipeline phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseConfig {
    pub name: String,
    pub label: String,
    #[serde(default)]
    pub phase_type: PhaseType,

    // Agent config
    pub system_prompt: String,
    pub instruction: String,
    /// Appended when task.last_error is set; supports `{ERROR}` placeholder.
    pub error_instruction: String,
    pub allowed_tools: String,
    pub use_docker: bool,

    // Prompt composition
    pub include_task_context: bool,
    pub include_file_listing: bool,

    // Post-agent actions
    pub runs_tests: bool,
    pub commits: bool,
    pub commit_message: String,
    /// File that must exist after phase completes.
    pub check_artifact: Option<String>,
    pub allow_no_changes: bool,

    // Transitions
    pub next: String,
    /// On test failure, check if error is in test files → route to qa_fix.
    pub has_qa_fix_routing: bool,
    /// Start with a fresh session (no resume).
    pub fresh_session: bool,

    // Rebase-specific
    pub fix_instruction: String,
    pub fix_error_instruction: String,

    /// Lower = processed first.
    pub priority: u8,
}

/// Configuration for a seed scan mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedConfig {
    pub name: String,
    pub label: String,
    pub prompt: String,
    pub output_type: SeedOutputType,
}

/// A complete pipeline mode definition (e.g. "sweborg", "lawborg", "webborg").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineMode {
    pub name: String,
    pub label: String,
    pub phases: Vec<PhaseConfig>,
    pub seed_modes: Vec<SeedConfig>,
    pub initial_status: String,
    pub uses_git_worktrees: bool,
    pub uses_docker: bool,
    pub uses_test_cmd: bool,
    pub integration: IntegrationType,
    pub default_max_attempts: u8,
}

impl PipelineMode {
    pub fn get_phase(&self, name: &str) -> Option<&PhaseConfig> {
        self.phases.iter().find(|p| p.name == name)
    }

    pub fn get_phase_index(&self, name: &str) -> Option<usize> {
        self.phases.iter().position(|p| p.name == name)
    }

    pub fn is_terminal(&self, status: &str) -> bool {
        matches!(status, "done" | "merged" | "failed")
    }
}

impl Default for PhaseConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            label: String::new(),
            phase_type: PhaseType::Agent,
            system_prompt: String::new(),
            instruction: String::new(),
            error_instruction: String::new(),
            allowed_tools: "Read,Glob,Grep,Write".into(),
            use_docker: false,
            include_task_context: false,
            include_file_listing: false,
            runs_tests: false,
            commits: false,
            commit_message: String::new(),
            check_artifact: None,
            allow_no_changes: false,
            next: "done".into(),
            has_qa_fix_routing: false,
            fresh_session: false,
            fix_instruction: String::new(),
            fix_error_instruction: String::new(),
            priority: 100,
        }
    }
}

// ── Phase Execution ──────────────────────────────────────────────────────

/// Runtime context passed to a phase executor.
#[derive(Debug, Clone)]
pub struct PhaseContext {
    pub task: Task,
    pub repo_config: RepoConfig,
    pub session_dir: String,
    pub worktree_path: String,
    pub oauth_token: String,
    pub model: String,
    /// Pending messages (role, content) to inject into this phase's instruction.
    pub pending_messages: Vec<(String, String)>,
    /// Extra system prompt appended to every agent run (co-author instructions etc.).
    pub system_prompt_suffix: String,
    /// If non-empty, append as Co-Authored-By trailer on git commits.
    pub user_coauthor: String,
}

/// Output produced by a phase executor.
#[derive(Debug, Clone)]
pub struct PhaseOutput {
    pub output: String,
    pub new_session_id: Option<String>,
    pub raw_stream: String,
    pub success: bool,
}

impl PhaseOutput {
    pub fn failed(output: impl Into<String>) -> Self {
        Self { output: output.into(), new_session_id: None, raw_stream: String::new(), success: false }
    }
}
