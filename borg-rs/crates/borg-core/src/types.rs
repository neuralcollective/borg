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
    /// Runs test/compile commands independently; loops back to previous phase on failure.
    Validate,
    /// Runs a git rebase operation with optional agent fix.
    Rebase,
    /// Runs a lint command; spawns an agent to fix errors if any.
    LintFix,
    /// Halts the pipeline until a human approves, rejects, or requests revision.
    HumanReview,
    /// Runs deterministic regulatory QA checks over prior phase output.
    ComplianceCheck,
    /// Deletes task vectors and raw session files to comply with data privacy policies.
    Purge,
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
    /// Commits to a branch but no PR — branch preserved for versioned document history.
    GitBranch,
    /// No VCS integration.
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
    /// Git branch name for this task.
    pub branch: String,
    /// Current pipeline phase / status (e.g. "backlog", "implement", "validate", "done", "failed", "blocked").
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
    pub updated_at: DateTime<Utc>,
    /// Claude Code session ID for resumption.
    pub session_id: String,
    /// Pipeline mode name (e.g. "sweborg", "lawborg", "webborg").
    pub mode: String,
    /// Agent backend override (e.g. "claude", "codex"). Empty = use global default.
    pub backend: String,
    /// Tenant workspace boundary (0 = unspecified/system fallback).
    #[serde(default)]
    pub workspace_id: i64,
    /// Link to a project/matter (0 = none).
    #[serde(default)]
    pub project_id: i64,
    /// Legal task type (research_memo, contract_analysis, etc.). Empty = unspecified.
    #[serde(default)]
    pub task_type: String,
    /// When true, the task must review the full attached corpus before making corpus-wide conclusions.
    #[serde(default)]
    pub requires_exhaustive_corpus_review: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_status: Option<String>,
    #[serde(default)]
    pub revision_count: i64,
    /// Originating chat thread key (e.g. "project:5"). Empty = not from chat.
    #[serde(default)]
    pub chat_thread: String,
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
    /// "queued" | "merging" | "merged" | "excluded" | "pending_review"
    pub status: String,
    pub queued_at: DateTime<Utc>,
    pub pr_number: i64,
}

// ── Pipeline State Snapshot ───────────────────────────────────────────────

/// One entry in the recent phase-output history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseHistoryEntry {
    pub phase: String,
    pub success: bool,
    /// Truncated agent output (first 2 000 chars).
    pub output: String,
    pub timestamp: DateTime<Utc>,
}

/// Point-in-time pipeline state written to `.borg/pipeline-state.json`
/// before each container agent launch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStateSnapshot {
    pub task_id: i64,
    pub task_title: String,
    pub phase: String,
    pub worktree_path: String,
    /// GitHub PR URL, or null if no PR has been opened yet.
    pub pr_url: Option<String>,
    /// Queue entries for this task that are in `pending_review` status
    /// (awaiting manual merge approval).
    pub pending_approvals: Vec<String>,
    /// Up to 5 most-recent phase outputs, oldest first.
    pub phase_history: Vec<PhaseHistoryEntry>,
    pub generated_at: DateTime<Utc>,
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
    /// GitHub repo slug (owner/repo). Used for `gh --repo` without a local checkout.
    pub repo_slug: String,
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
    pub disallowed_tools: String,
    pub use_docker: bool,

    // Prompt composition
    pub include_task_context: bool,
    pub include_file_listing: bool,

    // Timing
    #[serde(default)]
    pub wait_s: Option<i64>,

    // Post-agent actions
    pub runs_tests: bool,
    pub commits: bool,
    pub commit_message: String,
    /// File that must exist after phase completes.
    pub check_artifact: Option<String>,
    pub allow_no_changes: bool,
    /// Run a compile-only check after agent + commit (before tests).
    pub compile_check: bool,

    // Transitions
    pub next: String,
    /// Start with a fresh session (no resume).
    pub fresh_session: bool,
    /// Phase to resume from when a human requests revisions from this phase.
    #[serde(default)]
    pub revision_target: String,

    /// Instruction passed to the fix agent when rebase fails (rebase phases only).
    pub fix_instruction: String,

    /// Phase to loop back to on validation failure (Validate phases only).
    pub retry_phase: String,

    /// Compliance pack ID for ComplianceCheck phases (e.g. "uk_sra", "us_prof_resp").
    #[serde(default)]
    pub compliance_profile: String,
    /// Enforcement mode for ComplianceCheck: "warn" (default) or "block".
    #[serde(default)]
    pub compliance_enforcement: String,
}

/// Configuration for a seed scan mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedConfig {
    pub name: String,
    pub label: String,
    pub prompt: String,
    pub output_type: SeedOutputType,
    /// Tools available to the seed agent. Empty = default set (Read,Glob,Grep,Bash).
    #[serde(default)]
    pub allowed_tools: String,
    /// When true, output (tasks/proposals) is attributed to the primary (is_self) repo
    /// rather than the watched repo being seeded. Used for cross-pollinate seeds.
    #[serde(default)]
    pub target_primary_repo: bool,
}

/// A complete pipeline mode definition (e.g. "sweborg", "lawborg", "webborg").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineMode {
    pub name: String,
    pub label: String,
    #[serde(default)]
    pub category: String,
    pub phases: Vec<PhaseConfig>,
    pub seed_modes: Vec<SeedConfig>,
    pub initial_status: String,
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
        matches!(status, "done" | "merged" | "failed" | "purged")
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
            disallowed_tools: String::new(),
            use_docker: false,
            include_task_context: false,
            include_file_listing: false,
            wait_s: None,
            runs_tests: false,
            commits: false,
            commit_message: String::new(),
            check_artifact: None,
            allow_no_changes: false,
            compile_check: false,
            next: "done".into(),
            fresh_session: false,
            revision_target: String::new(),
            fix_instruction: String::new(),
            retry_phase: String::new(),
            compliance_profile: String::new(),
            compliance_enforcement: "warn".into(),
        }
    }
}

// ── Pipeline Events ──────────────────────────────────────────────────────

/// Broadcast event emitted after each significant pipeline state change.
#[derive(Debug, Clone)]
pub enum PipelineEvent {
    Phase {
        task_id: Option<i64>,
        message: String,
    },
    Output {
        task_id: Option<i64>,
        message: String,
    },
    Notify {
        chat_id: String,
        message: String,
    },
    PhaseResult {
        task_id: i64,
        phase: String,
        content: String,
        chat_id: String,
    },
}

impl PipelineEvent {
    pub fn kind(&self) -> &str {
        match self {
            Self::Phase { .. } => "task_phase",
            Self::Output { .. } => "task_output",
            Self::Notify { .. } => "notify",
            Self::PhaseResult { .. } => "phase_result",
        }
    }
    pub fn task_id(&self) -> Option<i64> {
        match self {
            Self::Phase { task_id, .. } | Self::Output { task_id, .. } => *task_id,
            Self::Notify { .. } => None,
            Self::PhaseResult { task_id, .. } => Some(*task_id),
        }
    }
    pub fn message(&self) -> &str {
        match self {
            Self::Phase { message, .. }
            | Self::Output { message, .. }
            | Self::Notify { message, .. } => message,
            Self::PhaseResult { content, .. } => content,
        }
    }
}

// ── Agent Signal ─────────────────────────────────────────────────────

/// Signal written by an agent to `.borg/signal.json` before exiting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSignal {
    /// "done" (default), "blocked", or "abandon".
    #[serde(default = "AgentSignal::default_status")]
    pub status: String,
    /// Free-text reason (required for blocked/abandon).
    #[serde(default)]
    pub reason: String,
    /// Optional question for the human (when blocked).
    #[serde(default)]
    pub question: String,
}

impl Default for AgentSignal {
    fn default() -> Self {
        Self {
            status: "done".into(),
            reason: String::new(),
            question: String::new(),
        }
    }
}

impl AgentSignal {
    fn default_status() -> String {
        "done".into()
    }

    pub fn done() -> Self {
        Self::default()
    }

    pub fn is_blocked(&self) -> bool {
        self.status == "blocked"
    }

    pub fn is_abandon(&self) -> bool {
        self.status == "abandon"
    }
}

/// Agent-written verdict stored in `.borg/phase-verdict.json` before a phase may advance.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PhaseCompletionVerdict {
    /// Task this verdict belongs to.
    #[serde(default)]
    pub task_id: i64,
    /// Phase name this verdict approves or rejects.
    #[serde(default)]
    pub phase: String,
    /// Task attempt/run number this verdict was written for.
    #[serde(default)]
    pub attempt: i64,
    /// Per-run token generated by the pipeline; must match exactly.
    #[serde(default)]
    pub gate_token: String,
    /// True only when the current phase's work has been checked against the original request
    /// and is ready to move to the next pipeline phase.
    #[serde(default)]
    pub ready_to_advance: bool,
    /// Short explanation of what was checked and why the phase is or is not ready.
    #[serde(default)]
    pub rationale: String,
    /// Any explicit requirements or follow-up items that remain incomplete.
    #[serde(default)]
    pub missing_requirements: Vec<String>,
}

// ── Phase Execution ──────────────────────────────────────────────────────

/// Runtime context passed to a phase executor.
#[derive(Debug, Clone)]
pub struct PhaseContext {
    pub task: Task,
    pub repo_config: RepoConfig,
    pub data_dir: String,
    pub session_dir: String,
    pub work_dir: String,
    pub oauth_token: String,
    pub model: String,
    /// Pending messages (role, content) to inject into this phase's instruction.
    pub pending_messages: Vec<(String, String)>,
    /// Current run's task attempt number for the phase gate contract.
    pub phase_attempt: i64,
    /// Per-run phase gate token that the verdict must echo back.
    pub phase_gate_token: String,
    /// Extra system prompt appended to every agent run (co-author instructions etc.).
    pub system_prompt_suffix: String,
    /// If non-empty, append as Co-Authored-By trailer on git commits.
    pub user_coauthor: String,
    /// If set, each raw stdout line from the agent is sent here for live streaming.
    pub stream_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    /// Absolute path to a setup script to source at container start (mounted as /workspace/setup.sh).
    pub setup_script: String,
    /// Per-task API keys (provider → key_value), looked up from DB at dispatch time.
    pub api_keys: std::collections::HashMap<String, String>,
    /// Comma-separated tools to disallow (from settings). Empty = all allowed.
    pub disallowed_tools: String,
    /// Knowledge base files to inject into the agent prompt.
    pub knowledge_files: Vec<crate::db::KnowledgeFile>,
    /// Absolute path to the knowledge directory on the host (store/knowledge/).
    pub knowledge_dir: String,
    /// Docker bridge network name for agent containers. None = use --network host.
    pub agent_network: Option<String>,
    /// Prior research chunks from the knowledge graph (injected for lawborg tasks).
    pub prior_research: Vec<String>,
    /// How many revision rounds this task has been through. 0 = first draft.
    pub revision_count: i64,
    /// Whether non-core experimental domains are enabled for runtime behavior.
    pub experimental_domains: bool,
    /// Whether to run in a network-isolated environment (air-gapped).
    pub isolated: bool,
    /// Borg API URL for agent search endpoints (e.g. http://127.0.0.1:3131).
    pub borg_api_url: String,
    /// Borg API bearer token for agent search endpoints.
    pub borg_api_token: String,
    /// Originating chat messages (sender, content) from the chat thread that created this task.
    pub chat_context: Vec<(String, String)>,
    /// Resolved GitHub token for this task (per-user override or global fallback).
    pub github_token: String,
}

/// A single in-container test/lint/compile result emitted by the entrypoint.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContainerTestResult {
    pub phase: String,
    pub passed: bool,
    pub exit_code: i32,
    pub output: String,
}

/// Output produced by a phase executor.
#[derive(Debug, Clone)]
pub struct PhaseOutput {
    pub output: String,
    pub new_session_id: Option<String>,
    pub raw_stream: String,
    pub success: bool,
    /// AgentSignal JSON extracted from the container's `---BORG_SIGNAL---` stdout line.
    pub signal_json: Option<String>,
    /// True when the agent ran inside Docker (container handled git clone/commit/push).
    pub ran_in_docker: bool,
    /// In-container test results emitted by the entrypoint (compile check / lint / test).
    pub container_test_results: Vec<ContainerTestResult>,
}

impl PhaseOutput {
    pub fn failed(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            new_session_id: None,
            raw_stream: String::new(),
            success: false,
            signal_json: None,
            ran_in_docker: false,
            container_test_results: Vec::new(),
        }
    }
}
