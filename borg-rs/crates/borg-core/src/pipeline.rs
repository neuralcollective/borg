use std::{
    collections::{HashMap, HashSet},
    ffi::CString,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::Deserialize;
use tokio::sync::{broadcast, Mutex};
use tracing::{error, info, warn};

pub use crate::types::PipelineEvent;
use crate::{
    agent::AgentBackend,
    config::Config,
    db::Db,
    git::Git,
    linked_credentials::{
        capture_bundle, claude_oauth_token_from_home, restore_bundle, should_revalidate,
        validate_home, PROVIDER_CLAUDE, PROVIDER_OPENAI,
    },
    modes::get_mode,
    sandbox::{Sandbox, SandboxMode},
    stream::TaskStreamManager,
    types::{
        ContainerTestResult, IntegrationType, PhaseCompletionVerdict, PhaseConfig, PhaseContext,
        PhaseHistoryEntry, PhaseOutput, PhaseType, PipelineMode, PipelineStateSnapshot, Proposal,
        RepoConfig, SeedOutputType, Task,
    },
};

/// Derive a compile-only check command from a test command, if possible.
/// For `cargo test` commands, returns the same command with `--no-run` appended.
pub fn derive_compile_check(test_cmd: &str) -> Option<String> {
    let trimmed = test_cmd.trim();
    if !trimmed.contains("cargo test") {
        return None;
    }
    if trimmed.contains("--no-run") {
        return Some(trimmed.to_string());
    }
    Some(format!("{trimmed} --no-run"))
}

pub struct Pipeline {
    pub db: Arc<Db>,
    pub backends: HashMap<String, Arc<dyn AgentBackend>>,
    pub config: Arc<Config>,
    pub ai_request_count: Arc<AtomicU64>,
    pub sandbox: Sandbox,
    pub sandbox_mode: SandboxMode,
    pub event_tx: broadcast::Sender<PipelineEvent>,
    pub stream_manager: Arc<TaskStreamManager>,
    pub force_restart: Arc<std::sync::atomic::AtomicBool>,
    /// Per-(repo_path, seed_name) last-run timestamp for independent per-seed cooldowns.
    seed_cooldowns: Mutex<HashMap<(String, String), i64>>,
    pub chat_event_tx: Option<broadcast::Sender<String>>,
    pub(crate) last_self_update_secs: std::sync::atomic::AtomicI64,
    last_cache_prune_secs: std::sync::atomic::AtomicI64,
    last_session_prune_secs: std::sync::atomic::AtomicI64,
    pub(crate) startup_heads: HashMap<String, String>,
    in_flight: Mutex<HashSet<i64>>,
    in_flight_repos: Mutex<HashSet<String>>,
    /// Per-task last agent dispatch timestamp (epoch seconds) for rate limiting.
    last_agent_dispatch: Mutex<HashMap<i64, i64>>,
    /// Per-task deferred retry unlock timestamp (epoch seconds).
    retry_not_before: Mutex<HashMap<i64, i64>>,
    /// Prevents overlapping seed runs (seeding is spawned in background).
    seeding_active: std::sync::atomic::AtomicBool,
    /// Tracks repeated phase-failure signatures per task to detect stuck loops.
    failure_signatures: std::sync::Mutex<HashMap<(i64, String), (String, u32)>>,
    /// Whether the borg-agent-net Docker bridge network was successfully created at startup.
    pub agent_network_available: bool,
    pub embed_registry: crate::knowledge::EmbeddingRegistry,
}

#[derive(Debug, Deserialize)]
struct GithubIssueLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GithubIssue {
    number: i64,
    title: String,
    #[serde(default)]
    body: String,
    url: String,
    #[serde(default)]
    labels: Vec<GithubIssueLabel>,
}

impl Pipeline {
    fn task_ready_for_dispatch(&self, task: &Task) -> bool {
        let Some(mode) = self.resolve_mode(&task.mode) else {
            let err = format!("unknown pipeline mode: {}", task.mode);
            error!("task #{}: {err}", task.id);
            let _ = self.db.update_task_status(task.id, "failed", Some(&err));
            return false;
        };
        let Some(phase) = mode.get_phase(&task.status) else {
            return false;
        };
        if phase.phase_type == PhaseType::HumanReview {
            return false;
        }
        if let Some(wait_s) = phase.wait_s {
            let ready_at = task.updated_at + chrono::Duration::seconds(wait_s.max(0));
            if Utc::now() < ready_at {
                return false;
            }
        }
        true
    }

    fn task_session_dir(task_id: i64) -> String {
        let rel = format!("store/sessions/task-{task_id}");
        std::fs::canonicalize(&rel)
            .unwrap_or_else(|_| std::path::PathBuf::from(&rel))
            .to_string_lossy()
            .to_string()
    }

    fn custom_modes_from_db(&self) -> Vec<PipelineMode> {
        let raw = match self.db.get_config("custom_modes") {
            Ok(Some(v)) => v,
            _ => return Vec::new(),
        };
        serde_json::from_str::<Vec<PipelineMode>>(&raw).unwrap_or_default()
    }

    fn resolve_mode(&self, name: &str) -> Option<PipelineMode> {
        get_mode(name).or_else(|| {
            self.custom_modes_from_db()
                .into_iter()
                .find(|m| m.name == name)
        })
    }

    pub fn new(
        db: Arc<Db>,
        backends: HashMap<String, Arc<dyn AgentBackend>>,
        config: Arc<Config>,
        sandbox_mode: SandboxMode,
        force_restart: Arc<std::sync::atomic::AtomicBool>,
        agent_network_available: bool,
        ai_request_count: Arc<AtomicU64>,
    ) -> (Self, broadcast::Receiver<PipelineEvent>) {
        let (tx, rx) = broadcast::channel(256);
        // Capture git HEAD for each watched repo at startup (used for self-update detection)
        let mut startup_heads = HashMap::new();
        for repo in &config.watched_repos {
            if repo.is_self {
                if let Ok(head) = crate::git::Git::new(&repo.path).rev_parse_head() {
                    startup_heads.insert(repo.path.clone(), head);
                }
            }
        }
        let seed_cooldowns = db.get_seed_cooldowns().unwrap_or_default();
        let p = Self {
            db,
            backends,
            config,
            ai_request_count,
            sandbox: Sandbox,
            sandbox_mode,
            event_tx: tx,
            stream_manager: TaskStreamManager::new(),
            chat_event_tx: None,
            force_restart,
            seed_cooldowns: Mutex::new(seed_cooldowns),
            last_self_update_secs: std::sync::atomic::AtomicI64::new(0),
            last_cache_prune_secs: std::sync::atomic::AtomicI64::new(0),
            last_session_prune_secs: std::sync::atomic::AtomicI64::new(0),
            startup_heads,
            in_flight: Mutex::new(HashSet::new()),
            in_flight_repos: Mutex::new(HashSet::new()),
            last_agent_dispatch: Mutex::new(HashMap::new()),
            retry_not_before: Mutex::new(HashMap::new()),
            seeding_active: std::sync::atomic::AtomicBool::new(false),
            failure_signatures: std::sync::Mutex::new(HashMap::new()),
            agent_network_available,
            embed_registry: crate::knowledge::EmbeddingRegistry::from_env(),
        };
        (p, rx)
    }

    // ── Backend resolution ────────────────────────────────────────────────

    /// Select the agent backend for a task: task override → repo override → global.
    /// Returns None if the resolved backend name isn't registered (missing API key, etc).
    fn resolve_backend(&self, task: &Task) -> Option<Arc<dyn AgentBackend>> {
        let name = self.selected_backend_name(task);
        if let Some(b) = self.backends.get(&name) {
            return Some(Arc::clone(b));
        }
        warn!(
            task_id = task.id,
            backend = %name,
            "backend not registered (missing API key?), skipping task"
        );
        None
    }

    async fn run_backend_phase(
        &self,
        backend: &Arc<dyn AgentBackend>,
        task: &Task,
        phase: &PhaseConfig,
        ctx: PhaseContext,
    ) -> Result<PhaseOutput> {
        self.ai_request_count.fetch_add(1, Ordering::Relaxed);
        backend.run_phase(task, phase, ctx).await
    }

    // ── Small helpers ─────────────────────────────────────────────────────

    pub fn active_agent_count(&self) -> usize {
        self.in_flight.try_lock().map(|g| g.len()).unwrap_or(0)
    }

    /// Resolve repo config for a task, filling in defaults if not found.
    fn repo_config(&self, task: &Task) -> RepoConfig {
        self.config
            .watched_repos
            .iter()
            .find(|r| r.path == task.repo_path)
            .cloned()
            .unwrap_or_else(|| RepoConfig {
                path: task.repo_path.clone(),
                test_cmd: String::new(),
                prompt_file: String::new(),
                mode: task.mode.clone(),
                is_self: false,
                auto_merge: true,
                lint_cmd: String::new(),
                backend: String::new(),
                repo_slug: String::new(),
            })
    }

    /// Resolve the backend name that will be used for this task.
    fn selected_backend_name(&self, task: &Task) -> String {
        if !task.backend.is_empty() {
            return task.backend.clone();
        }
        if let Some(repo) = self
            .config
            .watched_repos
            .iter()
            .find(|r| r.path == task.repo_path)
        {
            if !repo.backend.is_empty() {
                return repo.backend.clone();
            }
        }
        self.config.backend.clone()
    }

    fn repo_lint_cmd(&self, repo_path: &str, _worktree_path: &str) -> Option<String> {
        let repo = self
            .config
            .watched_repos
            .iter()
            .find(|r| r.path == repo_path)?;
        let lint_cmd = repo.lint_cmd.trim();
        if lint_cmd.is_empty() {
            None
        } else {
            Some(lint_cmd.to_string())
        }
    }

    fn task_wall_timeout_s(&self) -> u64 {
        // Whole-task timeout should be materially above per-command timeouts.
        (self.config.agent_timeout_s.max(300) as u64)
            .saturating_mul(3)
            .max(900)
    }

    fn retry_backoff_secs(&self, task_id: i64, attempt: i64, error: &str) -> Option<i64> {
        let class = classify_retry_error(error);
        let exp = ((attempt - 1).max(0) as u32).min(6);
        let secs = match class {
            RetryClass::Resource => (30_i64 * (1_i64 << exp)).min(600),
            RetryClass::Transient => (15_i64 * (1_i64 << exp)).min(300),
            _ => return None,
        };
        let now = Utc::now().timestamp();
        let unlock_at = now + secs;
        if let Ok(mut m) = self.retry_not_before.try_lock() {
            m.insert(task_id, unlock_at);
        }
        Some(secs)
    }

    fn should_defer_retry(&self, task_id: i64) -> Option<i64> {
        let now = Utc::now().timestamp();
        let map = match self.retry_not_before.try_lock() {
            Ok(m) => m,
            Err(_) => return Some(5),
        };
        let unlock_at = *map.get(&task_id)?;
        if unlock_at > now {
            Some(unlock_at - now)
        } else {
            None
        }
    }

    fn pipeline_tmp_dir(&self) -> PathBuf {
        PathBuf::from(format!("{}/tmp", self.config.data_dir))
    }

    fn ensure_tmp_capacity(&self, task_id: i64, phase: &str) -> Result<()> {
        const MIN_TMP_FREE_BYTES: u64 = 512 * 1024 * 1024;
        const MIN_TMP_FREE_INODES: u64 = 5_000;
        const MAX_TMP_INODE_USED_PCT: f64 = 85.0;

        let is_healthy = |h: &TmpHealth| {
            h.inode_used_pct < MAX_TMP_INODE_USED_PCT
                && h.free_bytes >= MIN_TMP_FREE_BYTES
                && h.free_inodes >= MIN_TMP_FREE_INODES
        };

        let before = tmp_health("/tmp");
        if before.as_ref().is_some_and(is_healthy) {
            return Ok(());
        }

        let msg = if let Some(h) = before {
            format!(
                "Self-heal: low /tmp capacity before {phase} (task #{task_id}): inode_used={:.1}% free_inodes={} free_bytes={}MB",
                h.inode_used_pct,
                h.free_inodes,
                h.free_bytes / (1024 * 1024)
            )
        } else {
            format!("Self-heal: could not read /tmp statvfs before {phase} (task #{task_id})")
        };
        warn!("{msg}");
        self.notify(&self.config.pipeline_admin_chat, &msg);

        let removed_tmp = cleanup_tmp_prefixes("/tmp", &["borg-rebase-task-", "borg-", "task-"]);
        let pipeline_tmp = self.pipeline_tmp_dir();
        std::fs::create_dir_all(&pipeline_tmp).ok();
        let removed_pipeline_tmp = cleanup_tmp_prefixes(
            &pipeline_tmp.to_string_lossy(),
            &["borg-rebase-task-", "borg-", "task-"],
        );

        let after = tmp_health("/tmp");
        if after.as_ref().is_some_and(is_healthy) {
            if let Some(h) = after {
                let healed = format!(
                    "Self-heal success: cleaned tmp artifacts ({removed_tmp} in /tmp, {removed_pipeline_tmp} in {}) now inode_used={:.1}% free_inodes={} free_bytes={}MB",
                    pipeline_tmp.display(),
                    h.inode_used_pct,
                    h.free_inodes,
                    h.free_bytes / (1024 * 1024)
                );
                info!("{healed}");
                self.notify(&self.config.pipeline_admin_chat, &healed);
            }
            return Ok(());
        }

        if let Some(h) = after {
            anyhow::bail!(
                "tmp still unhealthy after self-heal before {phase}: inode_used={:.1}% free_inodes={} free_bytes={}MB",
                h.inode_used_pct,
                h.free_inodes,
                h.free_bytes / (1024 * 1024)
            );
        }
        anyhow::bail!("tmp still unhealthy after self-heal before {phase}");
    }

    fn maybe_self_heal_tmp(&self) {
        const HEAL_INTERVAL_S: i64 = 120;
        let now = Utc::now().timestamp();
        let last = self.db.get_ts("last_tmp_self_heal_ts");
        if now - last < HEAL_INTERVAL_S {
            return;
        }
        self.db.set_ts("last_tmp_self_heal_ts", now);
        let _ = self.ensure_tmp_capacity(0, "tick_guardrail");
    }

    /// Resolve the GitHub token for a task: per-user setting → global config → `gh auth token`.
    fn resolve_gh_token(&self, created_by: &str) -> String {
        if !created_by.is_empty() {
            if let Ok(Some((uid, _, _, _, _))) = self.db.get_user_by_username(created_by) {
                if let Ok(Some(tok)) = self.db.get_user_setting(uid, "github_token") {
                    if !tok.is_empty() {
                        return tok;
                    }
                }
            }
        }
        if !self.config.github_token.is_empty() {
            return self.config.github_token.clone();
        }
        std::process::Command::new("gh")
            .args(["auth", "token"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }

    /// Build a PhaseContext for a task phase.
    fn make_context(
        &self,
        task: &Task,
        work_dir: String,
        session_dir: String,
        pending_messages: Vec<(String, String)>,
    ) -> PhaseContext {
        let (claude_coauthor, user_coauthor) = self.git_coauthor_settings();
        let system_prompt_suffix =
            Self::build_system_prompt_suffix(claude_coauthor, &user_coauthor);
        let setup_script = if self.config.container_setup.is_empty() {
            String::new()
        } else {
            std::fs::canonicalize(&self.config.container_setup)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| self.config.container_setup.clone())
        };
        let mut api_keys = std::collections::HashMap::new();
        let workspace_owner =
            (task.workspace_id > 0).then(|| format!("workspace:{}", task.workspace_id));
        let user_owner = (!task.created_by.is_empty()).then(|| task.created_by.clone());
        for provider in [
            "lexisnexis",
            "lexmachina",
            "intelligize",
            "westlaw",
            "clio",
            "imanage",
            "netdocuments",
            "congress",
            "openstates",
            "canlii",
            "regulations_gov",
            "shovels",
            "plaid_client_id",
            "plaid_secret",
            "plaid_env",
        ] {
            let resolved = workspace_owner
                .as_deref()
                .and_then(|owner| self.db.get_api_key_exact(owner, provider).ok().flatten())
                .or_else(|| {
                    user_owner
                        .as_deref()
                        .and_then(|owner| self.db.get_api_key_exact(owner, provider).ok().flatten())
                })
                .or_else(|| self.db.get_api_key_exact("global", provider).ok().flatten());
            if let Some(key) = resolved {
                api_keys.insert(provider.to_string(), key);
            }
        }
        let mut disallowed_tools = self
            .db
            .get_config("pipeline_disallowed_tools")
            .ok()
            .flatten()
            .unwrap_or_default();
        let knowledge_query = format!("{} {} {}", task.title, task.description, task.task_type);
        let knowledge_files = self
            .db
            .list_knowledge_file_page(Some(&knowledge_query), None, None, 80, 0)
            .map(|(files, _)| files)
            .unwrap_or_default();
        let knowledge_dir = format!("{}/knowledge", self.config.data_dir);
        let knowledge_repo_paths = self
            .db
            .list_all_knowledge_repos()
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.status == "ready" && !r.local_path.is_empty())
            .map(|r| r.local_path)
            .collect::<Vec<_>>();
        let isolated = task.mode == "lawborg" || task.mode == "legal";
        if isolated
            && task.project_id > 0
            && self
                .db
                .is_session_privileged(task.project_id)
                .unwrap_or(false)
        {
            if !disallowed_tools.is_empty() {
                disallowed_tools.push(',');
            }
            disallowed_tools.push_str("web_search,WebFetch");
        }
        let agent_network = if isolated {
            Some(Sandbox::ISOLATED_NETWORK.to_string())
        } else if self.agent_network_available {
            Some(Sandbox::AGENT_NETWORK.to_string())
        } else {
            None
        };

        let chat_context = if !task.chat_thread.is_empty() && task.attempt == 0 {
            self.db
                .get_chat_messages(&task.chat_thread, 20)
                .unwrap_or_default()
                .into_iter()
                .map(|m| {
                    let sender = m.sender_name.unwrap_or_else(|| {
                        if m.is_from_me {
                            "assistant".into()
                        } else {
                            "user".into()
                        }
                    });
                    (sender, m.content)
                })
                .collect()
        } else {
            Vec::new()
        };

        PhaseContext {
            task: task.clone(),
            repo_config: self.repo_config(task),
            data_dir: self.config.data_dir.clone(),
            session_dir,
            work_dir,
            oauth_token: self.config.oauth_token.clone(),
            model: self.config.model.clone(),
            pending_messages,
            phase_attempt: task.attempt,
            phase_gate_token: format!(
                "gate:{}:{}",
                task.id,
                Utc::now()
                    .timestamp_nanos_opt()
                    .unwrap_or_else(|| Utc::now().timestamp_micros() * 1_000)
            ),
            system_prompt_suffix,
            user_coauthor,
            stream_tx: None,
            setup_script,
            api_keys,
            disallowed_tools,
            knowledge_files,
            knowledge_dir,
            knowledge_repo_paths,
            agent_network,
            prior_research: Vec::new(),
            revision_count: task.revision_count,
            experimental_domains: self.config.experimental_domains,
            isolated,
            borg_api_url: format!("http://127.0.0.1:{}", self.config.web_port),
            borg_api_token: std::fs::read_to_string(format!("{}/.api-token", self.config.data_dir))
                .unwrap_or_default()
                .trim()
                .to_string(),
            chat_context,
            github_token: self.resolve_gh_token(&task.created_by),
        }
    }

    fn clear_session_provider_credentials(session_dir: &str, provider: &str) {
        let path = match provider {
            PROVIDER_CLAUDE => Path::new(session_dir).join(".claude"),
            PROVIDER_OPENAI => Path::new(session_dir).join(".codex"),
            _ => return,
        };
        let _ = std::fs::remove_dir_all(path);
    }

    async fn prepare_linked_agent_credentials(
        &self,
        task: &Task,
        backend_name: &str,
        ctx: &mut PhaseContext,
    ) -> Result<()> {
        let provider = match backend_name {
            "claude" => PROVIDER_CLAUDE,
            "codex" => PROVIDER_OPENAI,
            _ => return Ok(()),
        };
        let Some((user_id, _, _, _, _)) = self.db.get_user_by_username(&task.created_by)? else {
            return Ok(());
        };
        let Some(secret) = self.db.get_user_linked_credential(user_id, provider)? else {
            return Ok(());
        };
        if secret.entry.status != "connected" {
            return Ok(());
        }
        restore_bundle(&secret.bundle, Path::new(&ctx.session_dir))
            .context("restore linked credential bundle into task session")?;

        if should_revalidate(&secret.entry.last_validated_at, &secret.entry.expires_at) {
            let validation = validate_home(provider, Path::new(&ctx.session_dir)).await?;
            let now = Utc::now().to_rfc3339();
            if validation.ok {
                let refreshed_bundle = capture_bundle(provider, Path::new(&ctx.session_dir))
                    .context("capture refreshed linked credential bundle")?;
                self.db.update_user_linked_credential_state(
                    user_id,
                    provider,
                    &validation.auth_kind,
                    if validation.account_email.is_empty() {
                        &secret.entry.account_email
                    } else {
                        &validation.account_email
                    },
                    if validation.account_label.is_empty() {
                        &secret.entry.account_label
                    } else {
                        &validation.account_label
                    },
                    "connected",
                    &validation.expires_at,
                    &now,
                    "",
                    Some(&refreshed_bundle),
                )?;
            } else {
                self.db.update_user_linked_credential_state(
                    user_id,
                    provider,
                    if validation.auth_kind.is_empty() {
                        &secret.entry.auth_kind
                    } else {
                        &validation.auth_kind
                    },
                    if validation.account_email.is_empty() {
                        &secret.entry.account_email
                    } else {
                        &validation.account_email
                    },
                    if validation.account_label.is_empty() {
                        &secret.entry.account_label
                    } else {
                        &validation.account_label
                    },
                    "expired",
                    &validation.expires_at,
                    &now,
                    &validation.last_error,
                    None,
                )?;
                Self::clear_session_provider_credentials(&ctx.session_dir, provider);
                return Ok(());
            }
        }

        if provider == PROVIDER_CLAUDE {
            if let Some(token) = claude_oauth_token_from_home(Path::new(&ctx.session_dir)) {
                ctx.oauth_token = token;
            }
        }
        self.db
            .touch_user_linked_credential_used(user_id, provider)?;
        Ok(())
    }

    /// Increment attempt and set the retry status, or fail if attempts exhausted.
    /// After 3 failed attempts, clears the session ID to force a fresh start and
    /// builds a summary of previous attempts so the new session has context.
    fn fail_or_retry(&self, task: &Task, retry_status: &str, error: &str) -> Result<()> {
        let repeat_count = self.note_failure_signature(task.id, retry_status, error);
        if repeat_count >= 3 {
            let reason = format!(
                "stuck loop detected in phase '{retry_status}' (same failure signature repeated {repeat_count}x): {error}"
            );
            self.db
                .update_task_status(task.id, "blocked", Some(&reason))?;
            let project_id = if task.project_id > 0 {
                Some(task.project_id)
            } else {
                None
            };
            let _ = self.db.log_event_full(
                Some(task.id),
                None,
                project_id,
                "pipeline",
                "task.stuck_loop_detected",
                &serde_json::json!({
                    "phase": retry_status,
                    "repeat_count": repeat_count,
                    "error": error,
                }),
            );
            return Ok(());
        }

        self.db.increment_attempt(task.id)?;
        let current = self.db.get_task(task.id)?.unwrap_or_else(|| {
            // Fallback: use stale snapshot but with incremented attempt so check is correct
            let mut t = task.clone();
            t.attempt += 1;
            t
        });
        if current.attempt >= current.max_attempts {
            self.db.update_task_status(task.id, "failed", Some(error))?;
            let project_id = if task.project_id > 0 {
                Some(task.project_id)
            } else {
                None
            };
            let _ = self.db.log_event_full(
                Some(task.id),
                None,
                project_id,
                "pipeline",
                "task.failed_max_attempts",
                &serde_json::json!({
                    "phase": retry_status,
                    "attempt": current.attempt,
                    "max_attempts": current.max_attempts,
                    "error": error,
                }),
            );
        } else {
            if let Some(backoff_s) = self.retry_backoff_secs(task.id, current.attempt, error) {
                info!(
                    "task #{} retry backoff scheduled: {}s (attempt {} phase {})",
                    task.id, backoff_s, current.attempt, retry_status
                );
            }
            // After 3 attempts, force a fresh session with a summary of what was tried
            let error_ctx = if current.attempt >= 3 {
                self.db.update_task_session(task.id, "").ok();
                info!(
                    "task #{} attempt {} — clearing session for fresh start",
                    task.id, current.attempt
                );
                let project_id = if task.project_id > 0 {
                    Some(task.project_id)
                } else {
                    None
                };
                let _ = self.db.log_event_full(
                    Some(task.id),
                    None,
                    project_id,
                    "pipeline",
                    "task.session_reset_for_retry",
                    &serde_json::json!({
                        "phase": retry_status,
                        "attempt": current.attempt,
                    }),
                );
                self.build_retry_summary(task.id, error)
            } else {
                error.to_string()
            };
            self.db
                .update_task_status(task.id, retry_status, Some(&error_ctx))?;
            let project_id = if task.project_id > 0 {
                Some(task.project_id)
            } else {
                None
            };
            let _ = self.db.log_event_full(
                Some(task.id),
                None,
                project_id,
                "pipeline",
                "task.retry_scheduled",
                &serde_json::json!({
                    "phase": retry_status,
                    "attempt": current.attempt,
                    "max_attempts": current.max_attempts,
                    "error": error,
                }),
            );
        }
        Ok(())
    }

    fn normalize_error_signature(error: &str) -> String {
        let mut out = String::with_capacity(256);
        let mut prev_space = false;
        for ch in error.chars().flat_map(|c| c.to_lowercase()) {
            let mapped = if ch.is_ascii_digit() {
                '#'
            } else if ch.is_ascii_alphanumeric() {
                ch
            } else {
                ' '
            };
            if mapped == ' ' {
                if !prev_space {
                    out.push(' ');
                    prev_space = true;
                }
            } else {
                out.push(mapped);
                prev_space = false;
            }
            if out.len() >= 220 {
                break;
            }
        }
        out.trim().to_string()
    }

    fn note_failure_signature(&self, task_id: i64, phase: &str, error: &str) -> u32 {
        let sig = Self::normalize_error_signature(error);
        let mut map = self
            .failure_signatures
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let key = (task_id, phase.to_string());
        match map.get_mut(&key) {
            Some((prev_sig, count)) if *prev_sig == sig => {
                *count += 1;
                *count
            },
            Some((prev_sig, count)) => {
                *prev_sig = sig;
                *count = 1;
                1
            },
            None => {
                map.insert(key, (sig, 1));
                1
            },
        }
    }

    fn clear_failure_signatures(&self, task_id: i64) {
        let mut map = self
            .failure_signatures
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        map.retain(|(id, _), _| *id != task_id);
        if let Ok(mut retry_map) = self.retry_not_before.try_lock() {
            retry_map.remove(&task_id);
        }
    }

    /// Build a summary of previous failed attempts for fresh-session retries.
    fn build_retry_summary(&self, task_id: i64, current_error: &str) -> String {
        let outputs = self.db.get_task_outputs(task_id).unwrap_or_default();
        let mut summary =
            String::from("FRESH RETRY — previous approaches failed. Summary of attempts:\n");
        for (i, output) in outputs.iter().rev().take(3).enumerate() {
            let truncated: String = output.output.chars().take(500).collect();
            summary.push_str(&format!(
                "\nAttempt {} ({}): {}\n",
                i + 1,
                output.phase,
                truncated
            ));
        }
        summary.push_str(&format!(
            "\nLatest error:\n{}\n\nTry a fundamentally different approach.",
            current_error.chars().take(2000).collect::<String>()
        ));
        summary
    }

    /// Git author pair from config, or None if not configured.
    fn git_author(&self) -> Option<(&str, &str)> {
        if self.config.git_author_name.is_empty() {
            None
        } else {
            Some((
                self.config.git_author_name.as_str(),
                self.config.git_author_email.as_str(),
            ))
        }
    }

    // ── Main loop ─────────────────────────────────────────────────────────

    /// Main tick: dispatch ready tasks and run all periodic background work.
    pub async fn tick(self: Arc<Self>) -> Result<()> {
        // Reset integration_queue entries stuck in "merging" (crash mid-merge)
        if let Ok(n) = self.db.reset_stale_merging_queue() {
            if n > 0 {
                info!("Reset {n} stale merging integration_queue entries to queued");
            }
        }

        // Re-enqueue any "done" tasks that lost their queue entry (e.g. after restart)
        if let Ok(orphans) = self.db.list_done_tasks_without_queue() {
            for task in orphans {
                if let Some(mode) = self.resolve_mode(&task.mode) {
                    if mode.integration == IntegrationType::GitPr {
                        let branch = format!("task-{}", task.id);
                        if let Err(e) =
                            self.db
                                .enqueue_or_requeue(task.id, &branch, &task.repo_path, 0)
                        {
                            warn!("re-enqueue orphaned done task #{}: {e}", task.id);
                        } else {
                            info!(
                                "re-enqueued orphaned done task #{}: {}",
                                task.id, task.title
                            );
                        }
                    }
                }
            }
        }

        // Dispatch ready tasks
        let tasks = self.db.list_active_tasks().context("list_active_tasks")?;
        let max_agents = self.config.pipeline_max_agents as usize;
        let mut dispatched = 0usize;

        for task in tasks {
            if !self.task_ready_for_dispatch(&task) {
                continue;
            }
            let mut id_guard = self.in_flight.lock().await;
            if id_guard.len() >= max_agents {
                break;
            }
            if id_guard.contains(&task.id) {
                continue;
            }
            let mut repo_guard = self.in_flight_repos.lock().await;
            if repo_guard.contains(&task.repo_path) {
                continue;
            }
            id_guard.insert(task.id);
            repo_guard.insert(task.repo_path.clone());
            drop(repo_guard);
            drop(id_guard);

            dispatched += 1;
            let pipeline = Arc::clone(&self);
            let inner_pipeline = Arc::clone(&self);
            let task_id = task.id;
            let task_repo = task.repo_path.clone();
            let task_for_recovery = task.clone();
            tokio::spawn(async move {
                // Drop guard ensures in_flight slot is released even if this future is cancelled.
                struct InFlightGuard {
                    pipeline: Arc<Pipeline>,
                    task_id: i64,
                    task_repo: String,
                }
                impl Drop for InFlightGuard {
                    fn drop(&mut self) {
                        let pipeline = Arc::clone(&self.pipeline);
                        let task_id = self.task_id;
                        let task_repo = self.task_repo.clone();
                        tokio::spawn(async move {
                            pipeline.in_flight.lock().await.remove(&task_id);
                            pipeline.in_flight_repos.lock().await.remove(&task_repo);
                        });
                    }
                }
                let _guard = InFlightGuard {
                    pipeline: Arc::clone(&pipeline),
                    task_id,
                    task_repo,
                };

                let timeout_s = pipeline.task_wall_timeout_s();
                let mut handle =
                    tokio::spawn(
                        async move { Arc::clone(&inner_pipeline).process_task(task).await },
                    );
                match tokio::time::timeout(std::time::Duration::from_secs(timeout_s), &mut handle)
                    .await
                {
                    Ok(Ok(Ok(()))) => {},
                    Ok(Ok(Err(e))) => error!("process_task #{task_id} error: {e}"),
                    Ok(Err(join_err)) => {
                        let msg = if join_err.is_panic() {
                            let panic = join_err.into_panic();
                            match panic.downcast_ref::<String>() {
                                Some(s) => s.clone(),
                                None => match panic.downcast_ref::<&str>() {
                                    Some(s) => s.to_string(),
                                    None => "unknown panic".to_string(),
                                },
                            }
                        } else {
                            "task cancelled".to_string()
                        };
                        error!("process_task #{task_id} panicked: {msg}");
                        if let Err(e) = pipeline.fail_or_retry(
                            &task_for_recovery,
                            &task_for_recovery.status,
                            &format!("panic: {msg}"),
                        ) {
                            error!("process_task #{task_id} panic recovery DB update failed: {e}");
                        }
                    },
                    Err(_) => {
                        handle.abort();
                        let msg = format!("task wall timeout after {timeout_s}s");
                        error!("process_task #{task_id} timed out: {msg}");
                        if let Err(e) = pipeline.fail_or_retry(
                            &task_for_recovery,
                            &task_for_recovery.status,
                            &msg,
                        ) {
                            error!(
                                "process_task #{task_id} timeout recovery DB update failed: {e}"
                            );
                        }
                    },
                }
            });
        }

        if dispatched == 0 {
            // Hold the lock across the CAS so the emptiness check and the
            // seeding_active flip are jointly atomic with task dispatch.
            let guard = self.in_flight.lock().await;
            if guard.is_empty()
                && self
                    .seeding_active
                    .compare_exchange(
                        false,
                        true,
                        std::sync::atomic::Ordering::AcqRel,
                        std::sync::atomic::Ordering::Relaxed,
                    )
                    .is_ok()
            {
                drop(guard);
                let pipeline = Arc::clone(&self);
                tokio::spawn(async move {
                    if let Err(e) = pipeline.seed_if_idle().await {
                        warn!("seed_if_idle error: {e}");
                    }
                    pipeline
                        .seeding_active
                        .store(false, std::sync::atomic::Ordering::Release);
                });
            }
        }

        // Periodic background work (each is internally throttled)
        self.clone()
            .check_integration()
            .await
            .unwrap_or_else(|e| warn!("check_integration: {e}"));
        self.maybe_auto_promote_proposals();
        self.maybe_auto_triage().await;
        self.check_health()
            .await
            .unwrap_or_else(|e| warn!("check_health: {e}"));
        self.check_remote_updates().await;
        self.maybe_apply_self_update();
        self.refresh_mirrors().await;
        self.maybe_self_heal_tmp();
        self.maybe_alert_guardrails();
        self.maybe_prune_cache_volumes().await;
        self.maybe_prune_session_dirs().await;

        // Check if main loop should exit for self-update restart
        if self
            .force_restart
            .load(std::sync::atomic::Ordering::Acquire)
        {
            info!("force_restart flag set — returning error to trigger graceful shutdown");
            anyhow::bail!("force_restart");
        }

        Ok(())
    }

    // ── Task dispatch ─────────────────────────────────────────────────────

    /// Process a single task through its current phase.
    async fn process_task(self: Arc<Self>, task: Task) -> Result<()> {
        if let Some(wait_s) = self.should_defer_retry(task.id) {
            info!(
                "task #{} [{}] deferred by retry backoff ({}s remaining)",
                task.id, task.status, wait_s
            );
            return Ok(());
        }

        // Freshly requeued tasks should not inherit in-memory loop signatures
        // from previous failed runs.
        if task.attempt == 0 || task.status == "backlog" {
            self.clear_failure_signatures(task.id);
        }

        if let Some(latest) = self.db.get_task(task.id)? {
            if latest.status != task.status {
                info!(
                    "task #{} status changed from '{}' to '{}' before dispatch; skipping stale snapshot",
                    task.id, task.status, latest.status
                );
                let project_id = if latest.project_id > 0 {
                    Some(latest.project_id)
                } else {
                    None
                };
                let _ = self.db.log_event_full(
                    Some(task.id),
                    None,
                    project_id,
                    "pipeline",
                    "task.dispatch_stale_snapshot_skipped",
                    &serde_json::json!({
                        "snapshot_status": task.status,
                        "latest_status": latest.status,
                    }),
                );
                return Ok(());
            }
        }

        let mode = match self.resolve_mode(&task.mode) {
            Some(m) => m,
            None => {
                let err = format!("unknown pipeline mode: {}", task.mode);
                error!("task #{}: {err}", task.id);
                let _ = self.db.update_task_status(task.id, "failed", Some(&err));
                return Ok(());
            },
        };

        let phase = match mode.get_phase(&task.status) {
            Some(p) => p.clone(),
            None => {
                error!(
                    "task #{} has unknown phase '{}' for mode '{}'",
                    task.id, task.status, mode.name
                );
                return Ok(());
            },
        };

        if let Some(wait_s) = phase.wait_s {
            let ready_at = task.updated_at + chrono::Duration::seconds(wait_s.max(0));
            if Utc::now() < ready_at {
                return Ok(());
            }
        }

        // Rate-limit only agent phases (spawns a Claude subprocess).
        // Setup, Validate, LintFix, and Rebase are local ops — no cooldown needed.
        if phase.phase_type == PhaseType::Agent {
            let cooldown = self.config.pipeline_agent_cooldown_s;
            if cooldown > 0 {
                let now = Utc::now().timestamp();
                let mut map = self.last_agent_dispatch.lock().await;
                if let Some(&last) = map.get(&task.id) {
                    let elapsed = now - last;
                    if elapsed < cooldown {
                        info!(
                            "task #{} [{}] rate-limited ({elapsed}s/{cooldown}s), skipping",
                            task.id, task.status
                        );
                        return Ok(());
                    }
                }
                map.insert(task.id, now);
                // Prune stale entries to prevent unbounded growth
                if map.len() > 100 {
                    let cutoff = now - cooldown * 2;
                    map.retain(|_, &mut ts| ts > cutoff);
                }
            }
        }

        info!(
            "pipeline dispatching task #{} [{}] in {}: {}",
            task.id, task.status, task.repo_path, task.title
        );

        if phase.phase_type == PhaseType::Agent {
            let _ = self.db.mark_task_started(task.id);
        }

        match phase.phase_type {
            PhaseType::Setup => self.setup_branch(&task, &mode).await?,
            PhaseType::Agent => self.run_agent_phase(&task, &phase, &mode).await?,
            PhaseType::Validate => self.run_validate_phase(&task, &phase, &mode).await?,
            PhaseType::Rebase => self.run_rebase_phase(&task, &phase, &mode).await?,
            PhaseType::LintFix => self.run_lint_fix_phase(&task, &phase, &mode).await?,
            PhaseType::ComplianceCheck => {
                self.run_compliance_check_phase(&task, &phase, &mode)
                    .await?
            },
            PhaseType::HumanReview => {
                // Task sits in this status until a human acts via the API.
                // Do not dispatch to any backend — just return.
                return Ok(());
            },
            PhaseType::Purge => self.run_purge_phase(&task, &phase, &mode).await?,
        }

        // Async embedding indexing for completed tasks
        if phase.next == "done" && !task.repo_path.is_empty() {
            let db = Arc::clone(&self.db);
            let embed = self.embed_registry.client_for_mode(&task.mode);
            let pid = if task.project_id > 0 {
                Some(task.project_id)
            } else {
                None
            };
            crate::knowledge::index_task_embeddings(&db, embed, task.id, pid, &task.repo_path)
                .await;
        }

        self.clear_failure_signatures(task.id);

        Ok(())
    }

    /// Read git co-author settings from DB (runtime-editable), falling back to Config.
    fn git_coauthor_settings(&self) -> (bool, String) {
        let claude_coauthor = self
            .db
            .get_config("git_claude_coauthor")
            .ok()
            .flatten()
            .map(|v| v == "true")
            .unwrap_or(self.config.git_claude_coauthor);
        let user_coauthor = self
            .db
            .get_config("git_user_coauthor")
            .ok()
            .flatten()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| self.config.git_user_coauthor.clone());
        (claude_coauthor, user_coauthor)
    }

    /// Build system prompt suffix for co-author instructions.
    fn build_system_prompt_suffix(claude_coauthor: bool, user_coauthor: &str) -> String {
        let mut s = String::new();
        if !claude_coauthor {
            s.push_str("Do not add Co-Authored-By trailers to commit messages.");
        }
        if !user_coauthor.is_empty() {
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str("Git author is configured via environment variables — do not override with --author.");
        }
        s
    }

    /// Append user co-author trailer to a commit message if configured.
    fn with_user_coauthor(message: &str, user_coauthor: &str) -> String {
        if user_coauthor.is_empty() {
            return message.to_string();
        }
        format!("{message}\n\nCo-Authored-By: {user_coauthor}")
    }

    // ── Phase handlers ────────────────────────────────────────────────────

    /// Setup phase: record branch name, create per-task worktree, and advance.
    async fn setup_branch(&self, task: &Task, mode: &PipelineMode) -> Result<()> {
        let next = mode
            .phases
            .iter()
            .find(|p| p.phase_type != PhaseType::Setup)
            .map(|p| p.name.as_str())
            .unwrap_or("spec");

        let branch = format!("task-{}", task.id);
        self.db.update_task_branch(task.id, &branch)?;

        // Create per-task worktree for concurrent agent isolation
        if !task.repo_path.is_empty() {
            let git = crate::git::Git::new(&task.repo_path);
            let _ = git.fetch_origin();
            let worktree_dir = format!("{}/.worktrees/task-{}", task.repo_path, task.id);
            match git.create_worktree(&worktree_dir, &branch, "origin/main") {
                Ok(()) => {
                    self.db.update_task_repo_path(task.id, &worktree_dir)?;
                    info!("task #{} created worktree at {}", task.id, worktree_dir);
                }
                Err(e) => {
                    warn!("task #{} worktree creation failed: {e}", task.id);
                }
            }
        }

        self.db.update_task_status(task.id, next, None)?;

        self.emit(PipelineEvent::Phase {
            task_id: Some(task.id),
            message: format!("task #{} started branch {}", task.id, branch),
        });

        Ok(())
    }

    async fn run_compliance_check_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        _mode: &PipelineMode,
    ) -> Result<()> {
        let outputs = self.db.get_task_outputs(task.id).unwrap_or_default();
        let latest_text = outputs
            .iter()
            .rev()
            .find(|o| !o.output.trim().is_empty())
            .map(|o| o.output.as_str())
            .unwrap_or("");
        let profile = if phase.compliance_profile.trim().is_empty() {
            "uk_sra"
        } else {
            phase.compliance_profile.trim()
        };
        let enforcement = if phase.compliance_enforcement.trim().is_empty() {
            "warn"
        } else {
            phase.compliance_enforcement.trim()
        };

        let findings = run_compliance_pack(profile, latest_text);
        let mut report = String::new();
        report.push_str("# Compliance Check\n\n");
        report.push_str(&format!(
            "- Profile: `{profile}`\n- Enforcement: `{enforcement}`\n"
        ));
        if findings.is_empty() {
            report.push_str("\nResult: PASS. No compliance findings.\n");
        } else {
            report.push_str("\nResult: FINDINGS\n\n");
            for f in &findings {
                report.push_str(&format!(
                    "- [{}] {} ({})\n",
                    f.severity, f.issue, f.check_id
                ));
            }
            report.push_str("\nRecommended remediation: add a `Regulatory Considerations` section with source links and an as-of date.\n");
        }

        let compliance_json = serde_json::json!({
            "phase": phase.name,
            "profile": profile,
            "enforcement": enforcement,
            "checked_at": chrono::Utc::now().to_rfc3339(),
            "passed": findings.is_empty(),
            "findings": findings.iter().map(|f| serde_json::json!({
                "check_id": f.check_id,
                "severity": f.severity,
                "issue": f.issue,
                "source_url": f.source_url,
                "as_of": f.as_of,
            })).collect::<Vec<_>>(),
        });
        if let Ok(existing_raw) = self.db.get_task_structured_data(task.id) {
            let mut base = serde_json::from_str::<serde_json::Value>(&existing_raw)
                .unwrap_or_else(|_| serde_json::json!({}));
            if !base.is_object() {
                base = serde_json::json!({});
            }
            base["compliance_check"] = compliance_json;
            if let Ok(serialized) = serde_json::to_string(&base) {
                let _ = self.db.update_task_structured_data(task.id, &serialized);
            }
        }

        let blocked = compliance_should_block(enforcement, &findings);
        let success = !blocked;
        let exit_code = if success { 0 } else { 1 };
        if let Err(e) = self
            .db
            .insert_task_output(task.id, &phase.name, &report, "", exit_code)
        {
            warn!("task #{}: insert_task_output({}): {e}", task.id, phase.name);
        }

        if findings.is_empty() {
            self.db.update_task_status(task.id, &phase.next, None)?;
            return Ok(());
        }

        if blocked {
            self.db
                .update_task_status(task.id, "pending_review", Some(&report))?;
            self.emit(PipelineEvent::Phase {
                task_id: Some(task.id),
                message: format!(
                    "task #{} blocked by compliance check ({profile}) — moved to pending_review",
                    task.id
                ),
            });
            return Ok(());
        }

        self.db.update_task_status(task.id, &phase.next, None)?;
        Ok(())
    }

    /// Run an agent phase.
    async fn run_agent_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        mode: &PipelineMode,
    ) -> Result<()> {
        let session_dir_rel = format!("store/sessions/task-{}", task.id);
        tokio::fs::create_dir_all(&session_dir_rel).await.ok();
        let session_dir = Self::task_session_dir(task.id);

        // Use the task worktree as work_dir when available (created in setup_branch).
        // This ensures Docker containers bind-mount the actual repo, not the session dir.
        let work_dir = if !task.repo_path.is_empty()
            && std::path::Path::new(&task.repo_path).join(".git").exists()
        {
            task.repo_path.clone()
        } else {
            session_dir.clone()
        };

        let pending_messages = self
            .db
            .get_pending_task_messages(task.id)
            .unwrap_or_default()
            .into_iter()
            .map(|m| (m.role, m.content))
            .collect::<Vec<_>>();

        let backend_name = self.selected_backend_name(task);
        let mut ctx = self.make_context(task, work_dir.clone(), session_dir, pending_messages);
        self.prepare_linked_agent_credentials(task, &backend_name, &mut ctx)
            .await
            .unwrap_or_else(|err| {
                warn!(
                    task_id = task.id,
                    backend = backend_name.as_str(),
                    "failed to prepare linked credentials: {err}"
                );
            });
        let had_pending = !ctx.pending_messages.is_empty();
        let phase_gate_token = Self::build_phase_gate_token(task, phase);
        ctx.phase_attempt = task.attempt;
        ctx.phase_gate_token = phase_gate_token.clone();
        let test_cmd = ctx.repo_config.test_cmd.clone();

        // Inject prior research from knowledge graph for lawborg tasks
        if task.mode == "lawborg" || task.mode == "legal" {
            let pid = if task.project_id > 0 {
                Some(task.project_id)
            } else {
                None
            };
            let query = format!("{} {}", task.title, task.description);
            let results = crate::knowledge::get_prior_research(
                &self.db,
                self.embed_registry.client_for_mode(&task.mode),
                &query,
                pid,
                5,
            )
            .await;
            ctx.prior_research = results
                .into_iter()
                .map(|r| format!("[{}] {}", r.file_path, r.chunk_text))
                .collect();
        }

        // Wire live NDJSON stream for the dashboard LiveTerminal.
        let (stream_tx, mut stream_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        ctx.stream_tx = Some(stream_tx);
        self.stream_manager.start(task.id).await;
        let sm = Arc::clone(&self.stream_manager);
        let stream_task_id = task.id;
        let chat_tx = if task.project_id > 0 {
            self.chat_event_tx.clone().map(|tx| (tx, task.project_id))
        } else {
            None
        };
        tokio::spawn(async move {
            while let Some(line) = stream_rx.recv().await {
                if let Some((ref tx, pid)) = chat_tx {
                    let evt = serde_json::json!({
                        "type": "task_stream",
                        "thread": format!("project:{pid}"),
                        "task_id": stream_task_id,
                        "data": &line,
                    })
                    .to_string();
                    let _ = tx.send(evt);
                }
                sm.push_line(stream_task_id, line).await;
            }
            sm.end_task(stream_task_id).await;
        });

        info!("running {} phase for task #{}", phase.name, task.id);

        let backend = match self.resolve_backend(task) {
            Some(b) => b,
            None => {
                warn!("task #{}: no backend configured, failing task", task.id);
                self.fail_or_retry(task, &phase.name, "no agent backend configured")?;
                return Ok(());
            },
        };
        Self::clear_phase_control_files(&work_dir);
        if let Err(e) = self
            .write_pipeline_state_snapshot(task, &phase.name, &work_dir)
            .await
        {
            warn!("task #{}: write_pipeline_state_snapshot: {e}", task.id);
        }
        let result = self
            .run_backend_phase(&backend, task, phase, ctx)
            .await
            .unwrap_or_else(|e| {
                error!("backend.run_phase for task #{}: {e}", task.id);
                PhaseOutput::failed(String::new())
            });

        if let Some(ref sid) = result.new_session_id {
            if let Err(e) = self.db.update_task_session(task.id, sid) {
                warn!("task #{}: update_task_session: {e}", task.id);
            }
        }

        let exit_code: i64 = if result.success { 0 } else { 1 };
        if let Err(e) = self.db.insert_task_output(
            task.id,
            &phase.name,
            &result.output,
            &result.raw_stream,
            exit_code,
        ) {
            warn!("task #{}: insert_task_output: {e}", task.id);
        }

        self.emit(PipelineEvent::Output {
            task_id: Some(task.id),
            message: format!(
                "task #{} phase {} completed (success={})",
                task.id, phase.name, result.success
            ),
        });

        // Read agent signal from .borg/signal.json (if present), or from stdout.
        let signal = Self::read_agent_signal(&work_dir, result.signal_json.as_deref());
        if !signal.reason.is_empty() {
            info!(
                "task #{} signal: status={} reason={}",
                task.id, signal.status, signal.reason
            );
        }

        // Handle abandon signal: mark failed immediately, don't burn retry budget.
        if signal.is_abandon() {
            let reason = if signal.reason.is_empty() {
                "agent abandoned task".to_string()
            } else {
                format!("agent abandoned: {}", signal.reason)
            };
            self.db
                .update_task_status(task.id, "failed", Some(&reason))?;
            return Ok(());
        }

        // Handle blocked signal: pause task, don't retry.
        if signal.is_blocked() {
            let reason = if signal.reason.is_empty() {
                "agent blocked (no reason given)".to_string()
            } else {
                signal.reason.clone()
            };
            let block_detail = if signal.question.is_empty() {
                reason.clone()
            } else {
                format!("{}\n\nQuestion: {}", reason, signal.question)
            };
            self.db
                .update_task_status(task.id, "blocked", Some(&block_detail))?;
            self.emit(PipelineEvent::Phase {
                task_id: Some(task.id),
                message: format!("task #{} blocked: {}", task.id, reason),
            });
            return Ok(());
        }

        // Never advance on a failed agent run; retry the same logical phase path.
        if !result.success {
            let error_msg = if result.output.trim().is_empty() {
                format!("{} phase failed", phase.name)
            } else {
                result.output.clone()
            };
            let retry_status = if phase.name == "impl" || phase.name == "retry" {
                "retry"
            } else {
                phase.name.as_str()
            };
            self.fail_or_retry(task, retry_status, error_msg.trim())?;
            return Ok(());
        }

        if let Some(ref artifact) = phase.check_artifact {
            if !crate::ipc::check_artifact(&work_dir, artifact) {
                self.fail_or_retry(task, &phase.name, &format!("missing artifact: {artifact}"))?;
                return Ok(());
            }
        }

        // For Docker phases, commit agent changes from the host (the container
        // bind-mounts the worktree but cannot push).
        if result.ran_in_docker && !task.repo_path.is_empty() {
            let git = crate::git::Git::new(&task.repo_path);
            let (_, user_coauthor) = self.git_coauthor_settings();
            let msg = Self::with_user_coauthor("feat: borg agent changes", &user_coauthor);
            match git.commit_all(&work_dir, &msg, self.git_author()) {
                Ok(true) => info!("task #{} committed Docker agent changes", task.id),
                Ok(false) => info!("task #{} Docker phase: no changes to commit", task.id),
                Err(e) => warn!("task #{} post-Docker commit failed: {e}", task.id),
            }
        }

        if phase.compile_check && !test_cmd.is_empty() {
            if let Some(check_cmd) = derive_compile_check(&test_cmd) {
                let out = if result.ran_in_docker {
                    container_result_as_test_output(&result.container_test_results, "compileCheck")
                } else {
                    match self
                        .run_test_command_for_task(task, &work_dir, &check_cmd)
                        .await
                    {
                        Ok(o) => Some(o),
                        Err(e) => {
                            warn!("compile check error for task #{}: {e}", task.id);
                            None
                        },
                    }
                };
                if let Some(ref o) = out {
                    if o.exit_code != 0 {
                        let compile_err = format!("{}\n{}", o.stdout, o.stderr);
                        info!("task #{} compile check failed, running fix agent", task.id);
                        if !self
                            .run_compile_fix(task, &work_dir, &check_cmd, &compile_err)
                            .await?
                        {
                            let msg = format!(
                                "Compile fix failed after 2 attempts\n\n{}",
                                compile_err.chars().take(2000).collect::<String>()
                            );
                            self.fail_or_retry(task, &phase.name, &msg)?;
                            return Ok(());
                        }
                    }
                }
            }
        }

        if let Some(protocol_error) =
            self.enforce_legal_retrieval_protocol(task, phase, &result.raw_stream)
        {
            self.fail_or_retry(task, &phase.name, &protocol_error)?;
            return Ok(());
        }

        if phase.runs_tests && mode.uses_test_cmd && !test_cmd.is_empty() {
            let out = if result.ran_in_docker {
                container_result_as_test_output(&result.container_test_results, "test")
            } else {
                match self
                    .run_test_command_for_task(task, &work_dir, &test_cmd)
                    .await
                {
                    Ok(o) => Some(o),
                    Err(e) => {
                        warn!("test command error for task #{}: {}", task.id, e);
                        return Ok(());
                    },
                }
            };
            if let Some(o) = out {
                if o.exit_code != 0 {
                    let error_msg = format!("{}\n{}", o.stdout, o.stderr);
                    self.fail_or_retry(task, "retry", &error_msg)?;
                    return Ok(());
                }
            }
        }

        let verdict = match Self::read_phase_completion_verdict(&work_dir) {
            Some(verdict) => verdict,
            None => {
                self.fail_or_retry(
                    task,
                    &phase.name,
                    "missing or invalid .borg/phase-verdict.json; phase may not advance without an explicit completion verdict",
                )?;
                return Ok(());
            },
        };
        if let Err(error) =
            Self::validate_phase_completion_verdict(&verdict, task, phase, &phase_gate_token)
        {
            self.fail_or_retry(task, &phase.name, &error)?;
            return Ok(());
        }
        if !verdict.ready_to_advance {
            let mut msg = if verdict.rationale.trim().is_empty() {
                "agent reported that the phase is not ready to advance".to_string()
            } else {
                format!(
                    "agent self-check did not approve phase advancement: {}",
                    verdict.rationale.trim()
                )
            };
            if !verdict.missing_requirements.is_empty() {
                msg.push_str("\n\nMissing requirements:\n");
                for item in &verdict.missing_requirements {
                    msg.push_str("- ");
                    msg.push_str(item.trim());
                    msg.push('\n');
                }
            }
            self.fail_or_retry(task, &phase.name, msg.trim())?;
            return Ok(());
        }

        self.advance_phase(task, phase, mode)?;
        if had_pending {
            if let Err(e) = self.db.mark_messages_delivered(task.id, &phase.name) {
                warn!("task #{}: mark_messages_delivered: {e}", task.id);
            }
        }
        Ok(())
    }

    /// Read `.borg/signal.json` from the work dir. Returns default (done) if missing or malformed.
    fn read_agent_signal(
        work_dir: &str,
        phase_output_signal: Option<&str>,
    ) -> crate::types::AgentSignal {
        // Try direct path first, then Docker container path
        let paths = [
            format!("{work_dir}/.borg/signal.json"),
            format!("{work_dir}/repo/.borg/signal.json"),
        ];
        for path in &paths {
            if let Ok(contents) = std::fs::read_to_string(path) {
                std::fs::remove_file(path).ok();
                if let Ok(sig) = serde_json::from_str(&contents) {
                    return sig;
                }
            }
        }
        // Fall back to signal from agent stdout
        if let Some(json_str) = phase_output_signal {
            if let Ok(sig) = serde_json::from_str(json_str) {
                return sig;
            }
        }
        crate::types::AgentSignal::default()
    }

    fn build_phase_gate_token(task: &Task, phase: &PhaseConfig) -> String {
        format!(
            "{}:{}:{}:{}",
            task.id,
            phase.name,
            task.attempt,
            Utc::now()
                .timestamp_nanos_opt()
                .unwrap_or_else(|| Utc::now().timestamp_micros() * 1_000)
        )
    }

    fn clear_phase_control_files(work_dir: &str) {
        for path in Self::phase_control_paths(work_dir, "signal.json")
            .into_iter()
            .chain(Self::phase_control_paths(work_dir, "phase-verdict.json"))
        {
            if let Err(e) = std::fs::remove_file(&path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    warn!("failed to remove stale phase control file {}: {}", path, e);
                }
            }
        }
    }

    fn phase_control_paths(work_dir: &str, file_name: &str) -> [String; 2] {
        [
            format!("{work_dir}/.borg/{file_name}"),
            format!("{work_dir}/repo/.borg/{file_name}"),
        ]
    }

    /// Read `.borg/phase-verdict.json` from the work dir. Returns `None` if missing or malformed.
    fn read_phase_completion_verdict(work_dir: &str) -> Option<PhaseCompletionVerdict> {
        for path in Self::phase_control_paths(work_dir, "phase-verdict.json") {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                std::fs::remove_file(&path).ok();
                match serde_json::from_str::<PhaseCompletionVerdict>(&raw) {
                    Ok(verdict) => return Some(verdict),
                    Err(e) => {
                        warn!("invalid phase-verdict.json at {}: {}", path, e);
                        return None;
                    },
                }
            }
        }
        None
    }

    fn validate_phase_completion_verdict(
        verdict: &PhaseCompletionVerdict,
        task: &Task,
        phase: &PhaseConfig,
        gate_token: &str,
    ) -> std::result::Result<(), String> {
        let mut problems = Vec::new();

        if verdict.task_id != task.id {
            problems.push(format!(
                "task_id mismatch (expected {}, got {})",
                task.id, verdict.task_id
            ));
        }
        if verdict.phase.trim() != phase.name {
            problems.push(format!(
                "phase mismatch (expected {}, got {})",
                phase.name,
                verdict.phase.trim()
            ));
        }
        if verdict.attempt != task.attempt {
            problems.push(format!(
                "attempt mismatch (expected {}, got {})",
                task.attempt, verdict.attempt
            ));
        }
        if verdict.gate_token != gate_token {
            problems.push("gate token mismatch (verdict is stale or from another run)".to_string());
        }
        if verdict.rationale.trim().is_empty() {
            problems.push("rationale must not be empty".to_string());
        }
        if verdict
            .missing_requirements
            .iter()
            .any(|item| item.trim().is_empty())
        {
            problems.push("missing_requirements must not contain blank items".to_string());
        }
        if verdict.ready_to_advance && !verdict.missing_requirements.is_empty() {
            problems.push(
                "ready_to_advance cannot be true when missing_requirements is non-empty"
                    .to_string(),
            );
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "invalid or stale .borg/phase-verdict.json: {}",
                problems.join("; ")
            ))
        }
    }

    fn enforce_legal_retrieval_protocol(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        raw_stream: &str,
    ) -> Option<String> {
        if !self.config.enforce_retrieval_protocol {
            return None;
        }
        if raw_stream.trim().is_empty() {
            return None;
        }
        let stats = self.db.get_project_file_stats(task.project_id).ok()?;
        let Some(trigger_source) = legal_retrieval_protocol_trigger(task, phase, &stats) else {
            return None;
        };

        let mut report = inspect_legal_retrieval_trace(raw_stream);
        report.enforced = true;
        report.trigger_source = trigger_source.to_string();

        let mut missing = Vec::new();
        if report.inventory_calls == 0 {
            missing.push("call `list_documents` to inventory the full corpus".to_string());
        }
        if report.search_calls < 2 || report.distinct_search_queries.len() < 2 {
            missing.push(
                "run at least 2 `search_documents` / BorgSearch query passes with distinct queries"
                    .to_string(),
            );
        }
        if report.coverage_calls == 0 {
            missing.push(
                "call `check_coverage` (or `/api/borgsearch/coverage`) to find unmatched documents"
                    .to_string(),
            );
        }
        if report.full_document_reads == 0 {
            missing.push(
                "inspect at least 1 full document via `read_document`, BorgSearch file fetch, or a staged `project_files/` read".to_string(),
            );
        }

        report.missing_steps = missing.clone();
        report.passed = missing.is_empty();
        self.persist_retrieval_protocol_report(task.id, &report);

        if report.passed {
            return None;
        }

        let seen_queries = if report.search_queries.is_empty() {
            "none".to_string()
        } else {
            report.search_queries.join(" | ")
        };
        Some(format!(
            "Exhaustive legal retrieval protocol was not satisfied.\n\
             Required for this task: inventory the corpus, iterate search with distinct queries, run coverage, and inspect full documents. Trigger source: {}.\n\
             Observed: list_documents={}, get_document_categories={}, search_documents={}, check_coverage={}, full_document_reads={}; search queries={}\n\
             Missing: {}\n\
             Retry and complete the retrieval protocol before drafting conclusions about the corpus.",
            trigger_source,
            report.inventory_calls,
            report.category_calls,
            report.search_calls,
            report.coverage_calls,
            report.full_document_reads,
            seen_queries,
            missing.join("; "),
        ))
    }

    fn persist_retrieval_protocol_report(&self, task_id: i64, report: &LegalRetrievalTrace) {
        let payload = serde_json::json!({
            "checked_at": chrono::Utc::now().to_rfc3339(),
            "enforced": report.enforced,
            "passed": report.passed,
            "trigger_source": report.trigger_source,
            "tool_counts": {
                "list_documents": report.inventory_calls,
                "get_document_categories": report.category_calls,
                "search_documents": report.search_calls,
                "check_coverage": report.coverage_calls,
                "full_document_reads": report.full_document_reads,
            },
            "search_queries": report.search_queries,
            "distinct_search_queries": report.distinct_search_queries,
            "coverage_queries": report.coverage_queries,
            "mcp_servers": report.mcp_servers,
            "missing_steps": report.missing_steps,
        });

        let mut base = self
            .db
            .get_task_structured_data(task_id)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .filter(|v| v.is_object())
            .unwrap_or_else(|| serde_json::json!({}));
        base["retrieval_protocol"] = payload;
        if let Ok(serialized) = serde_json::to_string(&base) {
            let _ = self.db.update_task_structured_data(task_id, &serialized);
        }
    }

    /// Run a purge phase: delete vectors, messages, and raw files for a task.
    async fn run_purge_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        mode: &PipelineMode,
    ) -> Result<()> {
        info!("task #{} [{}] executing purge phase", task.id, task.status);

        // Delete DB vectors and messages
        self.db.purge_task_data(task.id)?;

        // Delete session directory
        let session_dir = format!("{}/sessions/task-{}", self.config.data_dir, task.id);
        if let Err(e) = std::fs::remove_dir_all(&session_dir) {
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    "task #{} failed to remove session dir {}: {}",
                    task.id, session_dir, e
                );
            }
        }

        // Delete worktree directory if it's outside the main repo
        if task.repo_path.contains(".worktrees") {
            if let Err(e) = std::fs::remove_dir_all(&task.repo_path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    warn!(
                        "task #{} failed to remove worktree {}: {}",
                        task.id, task.repo_path, e
                    );
                }
            }
        }

        // We do NOT delete the task record itself, or task_outputs, so the status and final draft survive
        self.advance_phase(task, phase, mode)?;
        Ok(())
    }

    /// Run a validate phase: execute test/compile commands independently, loop back on failure.
    async fn run_validate_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        mode: &PipelineMode,
    ) -> Result<()> {
        let work_dir = task.repo_path.clone();

        let test_cmd = self.repo_config(task).test_cmd;
        if test_cmd.is_empty() {
            self.advance_phase(task, phase, mode)?;
            info!("task #{} validate: no test command, skipping", task.id);
            return Ok(());
        }

        let use_docker = self.sandbox_mode == SandboxMode::Docker;

        // Compile check first (if derivable from test command)
        if let Some(check_cmd) = derive_compile_check(&test_cmd) {
            let out = if use_docker {
                self.run_test_in_container(task, &check_cmd).await?
            } else {
                self.run_test_command_for_task(task, &work_dir, &check_cmd)
                    .await?
            };
            if out.exit_code != 0 {
                let error_msg = format!("{}\n{}", out.stdout, out.stderr);
                info!("task #{} validate: compile check failed", task.id);
                if let Err(e) = self.db.insert_task_output(
                    task.id,
                    "validate",
                    error_msg.trim(),
                    "",
                    out.exit_code as i64,
                ) {
                    warn!("task #{}: insert_task_output(validate): {e}", task.id);
                }
                let retry_status = if phase.retry_phase.is_empty() {
                    &phase.name
                } else {
                    &phase.retry_phase
                };
                self.fail_or_retry(task, retry_status, error_msg.trim())?;
                return Ok(());
            }
        }

        // Run the full test suite
        let out = if use_docker {
            self.run_test_in_container(task, &test_cmd).await?
        } else {
            match self
                .run_test_command_for_task(task, &work_dir, &test_cmd)
                .await
            {
                Ok(o) => o,
                Err(e) => {
                    warn!("task #{} validate: test command error: {e}", task.id);
                    self.fail_or_retry(task, "validate", &format!("test command error: {e}"))?;
                    return Ok(());
                },
            }
        };
        let full_output = format!("{}\n{}", out.stdout, out.stderr);
        if let Err(e) = self.db.insert_task_output(
            task.id,
            "validate",
            full_output.trim(),
            "",
            out.exit_code as i64,
        ) {
            warn!("task #{}: insert_task_output(validate): {e}", task.id);
        }
        if out.exit_code == 0 {
            info!("task #{} validate: all tests pass", task.id);
            self.advance_phase(task, phase, mode)?;
        } else {
            info!("task #{} validate: tests failed", task.id);
            let retry_status = if phase.retry_phase.is_empty() {
                &phase.name
            } else {
                &phase.retry_phase
            };
            self.fail_or_retry(task, retry_status, full_output.trim())?;
        }

        Ok(())
    }

    /// Rebase: try GitHub update-branch API first; on conflict spawn a Docker agent.
    async fn run_rebase_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        mode: &PipelineMode,
    ) -> Result<()> {
        let repo = self.repo_config(task);
        if repo.repo_slug.is_empty() {
            warn!("task #{} rebase: no repo_slug, skipping", task.id);
            self.advance_phase(task, phase, mode)?;
            return Ok(());
        }

        let branch = format!("task-{}", task.id);
        let slug = &repo.repo_slug;

        // Find the PR number for this branch
        let pr_num_out = self
            .gh(&[
                "pr", "view", &branch, "--repo", slug, "--json", "number", "--jq", ".number",
            ])
            .await;
        let pr_num = pr_num_out
            .ok()
            .filter(|o| o.exit_code == 0)
            .and_then(|o| o.stdout.trim().parse::<u64>().ok());

        if let Some(num) = pr_num {
            let update_out = self
                .gh(&[
                    "api",
                    "-X",
                    "PUT",
                    &format!("repos/{slug}/pulls/{num}/update-branch"),
                ])
                .await;
            match update_out {
                Ok(o) if o.exit_code == 0 => {
                    info!("task #{} rebase: update-branch succeeded", task.id);
                    self.advance_phase(task, phase, mode)?;
                    return Ok(());
                },
                Ok(o) => {
                    let err = o.stderr.trim().chars().take(300).collect::<String>();
                    let err_lc = err.to_ascii_lowercase();
                    if err_lc.contains("expected head sha") || err_lc.contains("head ref") {
                        // GitHub branch-tip race; retry on next tick instead of spawning an agent.
                        info!(
                            "task #{} rebase: head SHA race, will retry update-branch on next tick",
                            task.id
                        );
                        return Ok(());
                    }
                    if err_lc.contains("could not resolve host")
                        || err_lc.contains("temporary failure in name resolution")
                        || err_lc.contains("network is unreachable")
                    {
                        warn!(
                            "task #{} rebase: GitHub DNS/network unavailable; skipping agent spawn",
                            task.id
                        );
                        self.fail_or_retry(task, "rebase", &err)?;
                        return Ok(());
                    }
                    warn!(
                        "task #{} rebase: update-branch failed, spawning agent: {err}",
                        task.id
                    );
                },
                Err(e) => {
                    let es = e.to_string();
                    let err_lc = es.to_ascii_lowercase();
                    if err_lc.contains("could not resolve host")
                        || err_lc.contains("temporary failure in name resolution")
                        || err_lc.contains("network is unreachable")
                    {
                        warn!(
                            "task #{} rebase: GitHub DNS/network unavailable; skipping agent spawn",
                            task.id
                        );
                        self.fail_or_retry(task, "rebase", &es)?;
                        return Ok(());
                    }
                    warn!(
                        "task #{} rebase: update-branch error, spawning agent: {e}",
                        task.id
                    );
                },
            }
        } else {
            info!("task #{} rebase: no PR found, advancing", task.id);
            self.advance_phase(task, phase, mode)?;
            return Ok(());
        }

        // Codex backend runs directly on host work_dir; rebase phases use session dirs.
        // Use deterministic local git rebase path to avoid "not a repo" / sandbox loops.
        if self.selected_backend_name(task) == "codex" {
            return self
                .run_rebase_non_interactive(task, phase, mode, slug, &branch)
                .await;
        }

        // GitHub API couldn't auto-merge — spawn an agent to resolve conflicts
        self.run_rebase_agent(task, phase, mode, &branch).await
    }

    async fn verify_rebased_branch(&self, _task: &Task, slug: &str, branch: &str) -> Result<()> {
        let compare = self
            .gh(&[
                "api",
                &format!("repos/{slug}/compare/main...{branch}"),
                "--jq",
                ".behind_by",
            ])
            .await?;
        let behind_by = compare.stdout.trim().parse::<u64>().unwrap_or(1);
        if behind_by > 0 {
            anyhow::bail!("branch {branch} is still behind main by {behind_by}");
        }

        let state_out = self
            .gh(&[
                "pr",
                "view",
                branch,
                "--repo",
                slug,
                "--json",
                "state,number",
                "--jq",
                ".state + \" \" + (.number|tostring)",
            ])
            .await;
        if let Ok(o) = state_out {
            if o.exit_code == 0 {
                let mut parts = o.stdout.split_whitespace();
                let state = parts.next().unwrap_or_default();
                let num = parts.next().unwrap_or_default();
                if state == "CLOSED" {
                    let reopen = self
                        .gh(&["pr", "reopen", num, "--repo", slug])
                        .await
                        .ok()
                        .filter(|x| x.exit_code == 0);
                    if reopen.is_none() {
                        anyhow::bail!("PR #{num} is closed and could not be reopened");
                    }
                }
            }
        }
        Ok(())
    }

    async fn run_rebase_non_interactive(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        mode: &PipelineMode,
        slug: &str,
        branch: &str,
    ) -> Result<()> {
        if let Err(e) = self.ensure_tmp_capacity(task.id, "rebase_non_interactive") {
            self.fail_or_retry(task, "rebase", &format!("tmp capacity check failed: {e}"))?;
            return Ok(());
        }

        let ts = Utc::now().timestamp_millis();
        let tmp_root = self.pipeline_tmp_dir();
        std::fs::create_dir_all(&tmp_root).ok();
        let temp_root = tmp_root.join(format!("borg-rebase-task-{}-{ts}", task.id));
        std::fs::create_dir_all(&temp_root)
            .with_context(|| format!("create temp rebase dir {}", temp_root.display()))?;
        struct TempDirGuard(PathBuf);
        impl Drop for TempDirGuard {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _temp_guard = TempDirGuard(temp_root.clone());

        let work_dir = temp_root.join("repo");
        let work_dir_s = work_dir.to_string_lossy().to_string();
        let tmp_env = self.pipeline_tmp_dir().to_string_lossy().to_string();

        let clone = tokio::process::Command::new("git")
            .args(["clone", "--no-tags", &task.repo_path, &work_dir_s])
            .env("TMPDIR", &tmp_env)
            .output()
            .await
            .context("git clone for non-interactive rebase")?;
        if !clone.status.success() {
            let err = String::from_utf8_lossy(&clone.stderr).to_string();
            self.fail_or_retry(task, "rebase", &format!("clone failed: {err}"))?;
            return Ok(());
        }

        let fetch = tokio::process::Command::new("git")
            .args([
                "fetch",
                "origin",
                "main:refs/remotes/origin/main",
                &format!("{branch}:refs/remotes/origin/{branch}"),
            ])
            .current_dir(&work_dir_s)
            .env("TMPDIR", &tmp_env)
            .output()
            .await
            .context("git fetch origin main")?;
        if !fetch.status.success() {
            let err = String::from_utf8_lossy(&fetch.stderr).to_string();
            self.fail_or_retry(task, "rebase", &format!("fetch failed: {err}"))?;
            return Ok(());
        }

        let checkout = tokio::process::Command::new("git")
            .args(["checkout", branch])
            .current_dir(&work_dir_s)
            .env("TMPDIR", &tmp_env)
            .output()
            .await
            .context("git checkout branch for rebase")?;
        if !checkout.status.success() {
            let err = String::from_utf8_lossy(&checkout.stderr).to_string();
            self.fail_or_retry(task, "rebase", &format!("checkout failed: {err}"))?;
            return Ok(());
        }

        let rebase = tokio::process::Command::new("git")
            .args(["rebase", "-X", "theirs", "origin/main"])
            .current_dir(&work_dir_s)
            .env("TMPDIR", &tmp_env)
            .output()
            .await
            .context("git rebase origin/main")?;
        if !rebase.status.success() {
            let err = String::from_utf8_lossy(&rebase.stderr).to_string();
            self.fail_or_retry(task, "rebase", &format!("rebase failed: {err}"))?;
            return Ok(());
        }

        let test_cmd = self.repo_config(task).test_cmd;
        if let Some(check_cmd) = derive_compile_check(&test_cmd) {
            let out = self
                .run_test_command_for_task(task, &work_dir_s, &check_cmd)
                .await?;
            if out.exit_code != 0 {
                let err = format!("{}\n{}", out.stdout, out.stderr);
                self.fail_or_retry(task, "rebase", &format!("compile check failed: {err}"))?;
                return Ok(());
            }
        }

        let gh_token = self.resolve_gh_token(&task.created_by);
        let origin_url = if !gh_token.is_empty() {
            format!("https://x-access-token:{gh_token}@github.com/{slug}.git")
        } else {
            format!("https://github.com/{slug}.git")
        };
        let set_url = tokio::process::Command::new("git")
            .args(["remote", "set-url", "origin", &origin_url])
            .current_dir(&work_dir_s)
            .env("TMPDIR", &tmp_env)
            .output()
            .await
            .context("git remote set-url origin")?;
        if !set_url.status.success() {
            let err = String::from_utf8_lossy(&set_url.stderr).to_string();
            self.fail_or_retry(task, "rebase", &format!("set-url failed: {err}"))?;
            return Ok(());
        }

        let push = tokio::process::Command::new("git")
            .args(["push", "--force-with-lease", "origin", branch])
            .current_dir(&work_dir_s)
            .env("TMPDIR", &tmp_env)
            .output()
            .await
            .context("git push --force-with-lease")?;
        if !push.status.success() {
            let err = String::from_utf8_lossy(&push.stderr).to_string();
            self.fail_or_retry(task, "rebase", &format!("push failed: {err}"))?;
            return Ok(());
        }

        if let Err(e) = self.verify_rebased_branch(task, slug, branch).await {
            self.fail_or_retry(
                task,
                "rebase",
                &format!("post-rebase verification failed: {e}"),
            )?;
            return Ok(());
        }

        self.advance_phase(task, phase, mode)?;
        Ok(())
    }

    /// Spawn a Docker agent to rebase the branch onto main and resolve conflicts.
    async fn run_rebase_agent(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        mode: &PipelineMode,
        branch: &str,
    ) -> Result<()> {
        let rebase_phase = PhaseConfig {
            name: "rebase_fix".into(),
            label: "Rebase Fix".into(),
            system_prompt: "You are a rebase agent. Your job is to rebase the current branch \
onto origin/main and resolve any merge conflicts. Preserve the intent of the branch's changes \
while incorporating upstream updates. After resolving conflicts, ensure the code compiles and \
tests pass if a test command is available. Push the result.".into(),
            instruction: format!(
                "Rebase branch `{branch}` onto `origin/main`. Steps:\n\
1. `git fetch origin`\n\
2. `git rebase origin/main`\n\
3. If conflicts arise, resolve them preserving the branch's intent\n\
4. `git rebase --continue` after resolving each conflict\n\
5. After rebase, run the project's compile check (e.g. `cargo check`) to verify the result compiles\n\
6. Fix any compile errors introduced by the rebase before pushing\n\
7. `git push --force-with-lease origin {branch}`\n\n\
If the rebase is too complex or the conflicts are unclear, abort with `git rebase --abort` \
and report what went wrong.",
            ),
            allowed_tools: "Read,Glob,Grep,Write,Edit,Bash".into(),
            use_docker: true,
            fresh_session: true,
            error_instruction: "\n\n---\n## Previous Attempt Failed\n{ERROR}\n\n\
                Analyze what went wrong and take a different approach. \
                Pay close attention to any compile errors — fix them before pushing.".into(),
            ..PhaseConfig::default()
        };

        let session_dir_rel = format!("store/sessions/task-{}", task.id);
        tokio::fs::create_dir_all(&session_dir_rel).await.ok();
        let session_dir = std::fs::canonicalize(&session_dir_rel)
            .unwrap_or_else(|_| std::path::PathBuf::from(&session_dir_rel))
            .to_string_lossy()
            .to_string();

        let ctx = self.make_context(task, session_dir.clone(), session_dir, Vec::new());

        let backend = match self.resolve_backend(task) {
            Some(b) => b,
            None => {
                warn!("task #{} rebase: no backend available", task.id);
                self.fail_or_retry(task, "rebase", "no agent backend")?;
                return Ok(());
            },
        };

        let result = self
            .run_backend_phase(&backend, task, &rebase_phase, ctx)
            .await
            .unwrap_or_else(|e| {
                error!("rebase agent for task #{}: {e}", task.id);
                PhaseOutput::failed(String::new())
            });

        if let Some(ref sid) = result.new_session_id {
            self.db.update_task_session(task.id, sid).ok();
        }

        self.db
            .insert_task_output(
                task.id,
                "rebase_fix",
                &result.output,
                &result.raw_stream,
                if result.success { 0 } else { 1 },
            )
            .ok();

        if result.success {
            // If the container ran a compile check, enforce it before advancing.
            // A bad conflict resolution often compiles fine locally but fails here.
            let compile_result = result
                .container_test_results
                .iter()
                .find(|r| r.phase == "compileCheck");
            if let Some(cr) = compile_result {
                if !cr.passed {
                    let errors = cr.output.chars().take(3000).collect::<String>();
                    warn!(
                        "task #{} rebase: compile check failed after rebase, retrying",
                        task.id
                    );
                    self.db
                        .insert_task_output(task.id, "rebase_compile_fail", &errors, "", 1)
                        .ok();
                    self.fail_or_retry(
                        task,
                        "rebase",
                        &format!("Compile failed after rebase:\n{errors}"),
                    )?;
                    return Ok(());
                }
            }
            let repo = self.repo_config(task);
            if let Err(e) = self
                .verify_rebased_branch(task, &repo.repo_slug, branch)
                .await
            {
                self.fail_or_retry(
                    task,
                    "rebase",
                    &format!("post-rebase verification failed: {e}"),
                )?;
                return Ok(());
            }
            info!("task #{} rebase: agent resolved conflicts", task.id);
            self.advance_phase(task, phase, mode)?;
        } else {
            warn!(
                "task #{} rebase: agent failed to resolve conflicts",
                task.id
            );
            self.fail_or_retry(task, "rebase", &result.output)?;
        }

        Ok(())
    }

    /// Lint is handled inside the Docker container by the entrypoint.
    async fn run_lint_fix_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        mode: &PipelineMode,
    ) -> Result<()> {
        // In Docker mode, lint is handled inside the container by the entrypoint.
        if self.sandbox_mode == SandboxMode::Docker {
            self.advance_phase(task, phase, mode)?;
            return Ok(());
        }

        let wt_path = task.repo_path.clone();

        let lint_cmd = match self.repo_lint_cmd(&task.repo_path, &wt_path) {
            Some(cmd) => cmd,
            None => {
                self.advance_phase(task, phase, mode)?;
                info!("task #{} lint_fix: no lint command, skipping", task.id);
                return Ok(());
            },
        };

        const LINT_FIX_SYSTEM: &str = "You are a lint-fix agent. Your only job is to make the \
codebase pass the project's linter with zero warnings or errors. Do not refactor, rename, or \
change logic — only fix what the linter reports. Read the lint output carefully and make the \
minimal changes needed. After editing, do not run the linter yourself — the pipeline will verify.";

        let mut lint_out = self
            .run_test_command_for_task(task, &wt_path, &lint_cmd)
            .await?;
        if lint_out.exit_code == 0 {
            self.advance_phase(task, phase, mode)?;
            info!("task #{} lint_fix: already clean", task.id);
            return Ok(());
        }

        let session_dir = Self::task_session_dir(task.id);

        for fix_attempt in 0..2u32 {
            let lint_output_text = format!("{}\n{}", lint_out.stdout, lint_out.stderr)
                .trim()
                .to_string();

            info!(
                "task #{} lint_fix: running fix agent (attempt {})",
                task.id,
                fix_attempt + 1
            );

            let fix_phase = PhaseConfig {
                name: format!("lint_fix_{fix_attempt}"),
                label: "Lint Fix".into(),
                system_prompt: LINT_FIX_SYSTEM.into(),
                instruction: format!(
                    "Fix all lint errors. Lint output:\n\n```\n{lint_output_text}\n```\n\n\
Make only the minimal changes the linter requires. Do not refactor or change logic.",
                ),
                allowed_tools: "Read,Glob,Grep,Write,Edit,Bash".into(),
                use_docker: true,
                allow_no_changes: true,
                fresh_session: true,
                ..PhaseConfig::default()
            };

            let ctx = self.make_context(task, wt_path.clone(), session_dir.clone(), Vec::new());

            let agent_result = match self.resolve_backend(task) {
                Some(b) => {
                    if let Err(e) = self
                        .write_pipeline_state_snapshot(task, &fix_phase.name, &wt_path)
                        .await
                    {
                        warn!("task #{}: write_pipeline_state_snapshot: {e}", task.id);
                    }
                    self.run_backend_phase(&b, task, &fix_phase, ctx)
                        .await
                        .unwrap_or_else(|e| {
                            error!("lint-fix agent for task #{}: {e}", task.id);
                            PhaseOutput::failed(String::new())
                        })
                },
                None => {
                    warn!(
                        "task #{}: no backend, skipping lint fix attempt {}",
                        task.id, fix_attempt
                    );
                    self.advance_phase(task, phase, mode)?;
                    return Ok(());
                },
            };

            if let Some(ref sid) = agent_result.new_session_id {
                self.db.update_task_session(task.id, sid).ok();
            }
            self.db
                .insert_task_output(
                    task.id,
                    &fix_phase.name,
                    &agent_result.output,
                    &agent_result.raw_stream,
                    if agent_result.success { 0 } else { 1 },
                )
                .ok();

            let git = Git::new(&task.repo_path);
            let (_, user_coauthor) = self.git_coauthor_settings();
            let lint_commit_msg = Self::with_user_coauthor("fix: lint errors", &user_coauthor);
            let _ = git.commit_all(&wt_path, &lint_commit_msg, self.git_author());

            lint_out = self
                .run_test_command_for_task(task, &wt_path, &lint_cmd)
                .await?;
            if lint_out.exit_code == 0 {
                self.advance_phase(task, phase, mode)?;
                info!(
                    "task #{} lint_fix: clean after {} fix attempt(s)",
                    task.id,
                    fix_attempt + 1
                );
                return Ok(());
            }
        }

        let error_msg = format!("{}\n{}", lint_out.stdout, lint_out.stderr);
        self.fail_or_retry(task, "lint_fix", error_msg.trim())?;
        Ok(())
    }

    /// Inline compile-fix agent: tries up to 2 times to fix compile errors.
    /// Returns true if the compile check passes after fixing.
    async fn run_compile_fix(
        &self,
        task: &Task,
        work_dir: &str,
        check_cmd: &str,
        initial_errors: &str,
    ) -> Result<bool> {
        let session_dir = Self::task_session_dir(task.id);

        let mut errors = initial_errors.to_string();

        for attempt in 0..2u32 {
            info!("task #{} compile_fix: attempt {}", task.id, attempt + 1);

            let fix_phase = PhaseConfig {
                name: format!("compile_fix_{attempt}"),
                label: "Compile Fix".into(),
                system_prompt:
                    "You are a compile-error fix agent. Fix compile errors with minimal changes."
                        .into(),
                instruction: format!(
                    "The code does not compile. Fix the compile errors below.\n\
                     Make only the minimal changes needed to fix the errors.\n\
                     Do not refactor, rename, or change logic.\n\n\
                     ```\n{}\n```",
                    errors.chars().take(4000).collect::<String>()
                ),
                allowed_tools: "Read,Glob,Grep,Write,Edit,Bash".into(),
                use_docker: true,
                allow_no_changes: true,
                fresh_session: true,
                ..PhaseConfig::default()
            };

            let ctx =
                self.make_context(task, work_dir.to_string(), session_dir.clone(), Vec::new());

            let result = match self.resolve_backend(task) {
                Some(b) => self
                    .run_backend_phase(&b, task, &fix_phase, ctx)
                    .await
                    .unwrap_or_else(|e| {
                        error!("compile-fix agent for task #{}: {e}", task.id);
                        PhaseOutput::failed(String::new())
                    }),
                None => return Ok(false),
            };

            if let Some(ref sid) = result.new_session_id {
                self.db.update_task_session(task.id, sid).ok();
            }
            self.db
                .insert_task_output(
                    task.id,
                    &fix_phase.name,
                    &result.output,
                    &result.raw_stream,
                    if result.success { 0 } else { 1 },
                )
                .ok();

            let git = Git::new(&task.repo_path);
            let (_, user_coauthor) = self.git_coauthor_settings();
            let msg = Self::with_user_coauthor("fix: compile errors", &user_coauthor);
            let _ = git.commit_all(work_dir, &msg, self.git_author());

            match self
                .run_test_command_for_task(task, work_dir, check_cmd)
                .await
            {
                Ok(ref out) if out.exit_code == 0 => {
                    info!(
                        "task #{} compile_fix: resolved after {} attempt(s)",
                        task.id,
                        attempt + 1
                    );
                    return Ok(true);
                },
                Ok(ref out) => {
                    errors = format!("{}\n{}", out.stdout, out.stderr);
                },
                Err(e) => {
                    warn!("task #{} compile_fix: check command error: {e}", task.id);
                    return Ok(false);
                },
            }
        }

        Ok(false)
    }

    // ── Phase transition ──────────────────────────────────────────────────

    /// Advance a task to the next phase, or enqueue for integration when done.
    fn advance_phase(&self, task: &Task, phase: &PhaseConfig, mode: &PipelineMode) -> Result<()> {
        let next = phase.next.as_str();
        self.promote_session_privilege_on_phase2_transition(task, mode, next);
        if next == "done" || next == "human_review" {
            self.read_structured_output(task);
        }
        if next == "done" {
            self.index_task_documents(task);

            self.db.update_task_status(task.id, "done", Some(""))?;
            let _ = self.db.mark_task_completed(task.id);
            let pid = if task.project_id > 0 {
                Some(task.project_id)
            } else {
                None
            };
            let _ = self.db.log_event_full(
                Some(task.id),
                None,
                pid,
                "pipeline",
                "task.completed",
                &serde_json::json!({ "title": task.title }),
            );

            let duration_str = self
                .db
                .get_task(task.id)
                .ok()
                .flatten()
                .and_then(|t| t.duration_secs)
                .map(|s| {
                    if s >= 3600 {
                        format!("{}h{}m", s / 3600, (s % 3600) / 60)
                    } else if s >= 60 {
                        format!("{}m{}s", s / 60, s % 60)
                    } else {
                        format!("{}s", s)
                    }
                })
                .unwrap_or_default();

            match mode.integration {
                IntegrationType::GitPr => {
                    let branch = format!("task-{}", task.id);
                    if let Err(e) = self
                        .db
                        .enqueue_or_requeue(task.id, &branch, &task.repo_path, 0)
                    {
                        warn!("enqueue for task #{}: {}", task.id, e);
                    } else {
                        info!("task #{} done, queued for integration", task.id);
                    }
                    if !task.notify_chat.is_empty() {
                        let msg = format!(
                            "Task #{} \"{}\" completed{}, queued for merge.",
                            task.id,
                            task.title,
                            if duration_str.is_empty() {
                                String::new()
                            } else {
                                format!(" ({})", duration_str)
                            },
                        );
                        self.notify(&task.notify_chat, &msg);
                    }
                },
                IntegrationType::GitBranch => {
                    info!("task #{} done, branch preserved", task.id);
                },
                IntegrationType::None => {
                    if !task.notify_chat.is_empty() {
                        let summary = self
                            .db
                            .get_task_structured_data(task.id)
                            .ok()
                            .filter(|s| !s.is_empty())
                            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                            .and_then(|v| {
                                v.get("summary").and_then(|s| s.as_str()).map(String::from)
                            });
                        let mut msg = format!(
                            "Task #{} \"{}\" completed{}.",
                            task.id,
                            task.title,
                            if duration_str.is_empty() {
                                String::new()
                            } else {
                                format!(" ({})", duration_str)
                            },
                        );
                        if let Some(ref sum) = summary {
                            msg.push_str(&format!("\n\n{}", sum));
                        }
                        self.notify(&task.notify_chat, &msg);
                    }
                },
            }
        } else {
            self.db.update_task_status(task.id, next, Some(""))?;
        }
        self.emit(PipelineEvent::Phase {
            task_id: Some(task.id),
            message: format!("task #{} advanced to '{}'", task.id, next),
        });
        Ok(())
    }

    fn promote_session_privilege_on_phase2_transition(
        &self,
        task: &Task,
        mode: &PipelineMode,
        next_status: &str,
    ) {
        if task.project_id <= 0 {
            return;
        }
        let is_legal_mode = matches!(task.mode.as_str(), "lawborg" | "legal")
            || matches!(mode.name.as_str(), "lawborg" | "legal");
        if !is_legal_mode {
            return;
        }
        if !Self::is_phase2_or_later(mode, next_status) {
            return;
        }
        if let Err(e) = self.db.set_session_privileged(task.project_id) {
            warn!(
                "task #{} failed to mark project {} as session_privileged on phase transition to '{}': {}",
                task.id, task.project_id, next_status, e
            );
        }
    }

    fn is_phase2_or_later(mode: &PipelineMode, status: &str) -> bool {
        let mut agent_phase_count = 0usize;
        for phase in &mode.phases {
            if phase.phase_type == PhaseType::Agent {
                agent_phase_count += 1;
            }
            if phase.name == status {
                return agent_phase_count >= 2;
            }
        }
        false
    }

    fn read_structured_output(&self, task: &Task) {
        if task.repo_path.is_empty() {
            return;
        }
        let branch = format!("task-{}", task.id);
        let path = std::path::Path::new(&task.repo_path);
        if !path.join(".git").exists() {
            return;
        }
        let out = std::process::Command::new("git")
            .args([
                "-C",
                &task.repo_path,
                "show",
                &format!("{branch}:structured.json"),
            ])
            .stderr(std::process::Stdio::null())
            .output();
        if let Ok(output) = out {
            if output.status.success() {
                let data = String::from_utf8_lossy(&output.stdout);
                let trimmed = data.trim();
                if !trimmed.is_empty() {
                    let merged = match self.db.get_task_structured_data(task.id) {
                        Ok(existing_raw) => {
                            let mut existing =
                                serde_json::from_str::<serde_json::Value>(&existing_raw)
                                    .unwrap_or_else(|_| serde_json::json!({}));
                            let fresh = serde_json::from_str::<serde_json::Value>(trimmed)
                                .unwrap_or_else(|_| serde_json::json!({}));
                            if existing.is_object() && fresh.is_object() {
                                if let (Some(existing_obj), Some(fresh_obj)) =
                                    (existing.as_object_mut(), fresh.as_object())
                                {
                                    for (k, v) in fresh_obj {
                                        existing_obj.insert(k.clone(), v.clone());
                                    }
                                    serde_json::to_string(&existing)
                                        .unwrap_or_else(|_| trimmed.to_string())
                                } else {
                                    trimmed.to_string()
                                }
                            } else {
                                trimmed.to_string()
                            }
                        },
                        Err(_) => trimmed.to_string(),
                    };
                    if let Err(e) = self.db.update_task_structured_data(task.id, &merged) {
                        tracing::warn!("task #{}: failed to save structured data: {e}", task.id);
                    } else {
                        tracing::info!(
                            "task #{}: saved structured output ({} bytes)",
                            task.id,
                            trimmed.len()
                        );
                    }
                }
            }
        }
    }

    fn index_task_documents(&self, task: &Task) {
        if task.repo_path.is_empty() || task.project_id == 0 {
            return;
        }
        let branch = format!("task-{}", task.id);
        let path = std::path::Path::new(&task.repo_path);
        if !path.join(".git").exists() {
            return;
        }
        // List .md files on the task branch
        let out = std::process::Command::new("git")
            .args([
                "-C",
                &task.repo_path,
                "ls-tree",
                "-r",
                "--name-only",
                &branch,
            ])
            .stderr(std::process::Stdio::null())
            .output();
        let files = match out {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => return,
        };
        // Clear old index for this task
        let _ = self.db.fts_remove_task(task.id);
        let mut count = 0;
        for file in files.lines() {
            if !file.ends_with(".md") {
                continue;
            }
            let show = std::process::Command::new("git")
                .args(["-C", &task.repo_path, "show", &format!("{branch}:{file}")])
                .stderr(std::process::Stdio::null())
                .output();
            if let Ok(o) = show {
                if o.status.success() {
                    let content = String::from_utf8_lossy(&o.stdout);
                    let title = content
                        .lines()
                        .next()
                        .unwrap_or(file)
                        .trim_start_matches('#')
                        .trim();
                    if let Err(e) =
                        self.db
                            .fts_index_document(task.project_id, task.id, file, title, &content)
                    {
                        tracing::warn!("task #{}: FTS index failed for {file}: {e}", task.id);
                    } else {
                        count += 1;
                    }
                }
            }
        }
        if count > 0 {
            tracing::info!("task #{}: indexed {count} documents for FTS", task.id);
        }
    }

    // ── Pipeline state snapshot ───────────────────────────────────────────

    /// Write `.borg/pipeline-state.json` into the working directory before agent launch.
    /// Logs a warning and returns Ok(()) on any error so phase execution is
    /// never aborted by snapshot failures.
    async fn write_pipeline_state_snapshot(
        &self,
        task: &Task,
        phase_name: &str,
        work_dir: &str,
    ) -> Result<()> {
        // Build phase_history from last 5 task outputs, truncating output to 2 000 chars.
        let phase_history: Vec<PhaseHistoryEntry> = self
            .db
            .get_task_outputs(task.id)
            .unwrap_or_default()
            .into_iter()
            .rev()
            .take(5)
            .rev()
            .map(|o| PhaseHistoryEntry {
                phase: o.phase,
                success: o.exit_code == 0,
                output: o.output.chars().take(2_000).collect(),
                timestamp: o.created_at,
            })
            .collect();

        // Look up queue entries for this task to populate pending_approvals and pr_url.
        let queue_entries = self
            .db
            .get_queue_entries_for_task(task.id)
            .unwrap_or_default();

        let pending_approvals: Vec<String> = queue_entries
            .iter()
            .filter(|e| e.status == "pending_review")
            .map(|e| e.branch.clone())
            .collect();

        // Derive PR URL by calling `gh pr view` if any queue entry exists.
        let pr_url: Option<String> = if let Some(entry) = queue_entries.first() {
            let out = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                tokio::process::Command::new("gh")
                    .args(["pr", "view", &entry.branch, "--json", "url", "--jq", ".url"])
                    .stderr(std::process::Stdio::null())
                    .output(),
            )
            .await
            .ok()
            .and_then(|r| r.ok());
            out.and_then(|o| {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            })
        } else {
            None
        };

        let snapshot = PipelineStateSnapshot {
            task_id: task.id,
            task_title: task.title.clone(),
            phase: phase_name.to_string(),
            worktree_path: work_dir.to_string(),
            pr_url,
            pending_approvals,
            phase_history,
            generated_at: Utc::now(),
        };

        let borg_dir = format!("{work_dir}/.borg");
        tokio::fs::create_dir_all(&borg_dir).await?;
        let json = serde_json::to_string_pretty(&snapshot)?;
        tokio::fs::write(format!("{borg_dir}/pipeline-state.json"), json).await?;

        Ok(())
    }

    // ── Lint helpers ──────────────────────────────────────────────────────

    // ── Test runner ───────────────────────────────────────────────────────

    pub(crate) async fn run_test_command_for_task(
        &self,
        task: &Task,
        dir: &str,
        cmd: &str,
    ) -> Result<TestOutput> {
        self.ensure_tmp_capacity(task.id, "run_test_command")?;
        self.run_test_command(dir, cmd).await
    }

    pub(crate) async fn run_test_command(&self, dir: &str, cmd: &str) -> Result<TestOutput> {
        self.ensure_tmp_capacity(0, "run_test_command")?;
        let tmp_dir = self.pipeline_tmp_dir();
        std::fs::create_dir_all(&tmp_dir).ok();
        let timeout = std::time::Duration::from_secs(self.config.agent_timeout_s.max(300) as u64);
        let output = tokio::time::timeout(
            timeout,
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .current_dir(dir)
                .env("TMPDIR", tmp_dir.to_string_lossy().to_string())
                .output(),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "run_test_command timed out after {}s: {cmd}",
                timeout.as_secs()
            )
        })?
        .context("run test command")?;

        Ok(TestOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(1),
        })
    }

    /// Run a test command inside a fresh Docker container (for validate phase in Docker mode).
    /// Clones the task branch and runs `cmd` directly via bash — no claude agent involved.
    async fn run_test_in_container(&self, task: &Task, cmd: &str) -> Result<TestOutput> {
        self.ensure_tmp_capacity(task.id, "run_test_in_container")?;
        let timeout = std::time::Duration::from_secs(self.config.agent_timeout_s.max(300) as u64);
        let repo_name = std::path::Path::new(&task.repo_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let branch = format!("task-{}", task.id);
        let host_mirror = format!("{}/mirrors/{repo_name}.git", self.config.data_dir);
        let container_mirror = format!("/mirrors/{repo_name}.git");

        // Shallow clone — test containers only need the branch tip.
        // Wrap a value in single quotes with internal single quotes escaped.
        fn sq(s: &str) -> String {
            format!("'{}'", s.replace('\'', "'\\''"))
        }
        let repo_url_q = sq(&task.repo_path);
        let branch_q = sq(&branch);
        let cmd_q = sq(cmd);
        let container_mirror_q = sq(&container_mirror);
        let clone_cmd = if std::path::Path::new(&host_mirror).exists() {
            format!(
                "git clone --depth 1 --single-branch --reference {container_mirror_q} {repo_url_q} /workspace/repo"
            )
        } else {
            format!("git clone --depth 1 --single-branch {repo_url_q} /workspace/repo")
        };
        let bash_script = format!(
            "set -e; {clone_cmd} && cd /workspace/repo && git checkout {branch_q} && {cmd_q}"
        );
        let bash_cmd = vec!["bash".to_string(), "-c".to_string(), bash_script];

        let mut binds: Vec<(String, String, bool)> = Vec::new();
        if std::path::Path::new(&host_mirror).exists() {
            binds.push((host_mirror, container_mirror, true));
        }
        let binds_ref: Vec<(&str, &str, bool)> = binds
            .iter()
            .map(|(h, c, ro)| (h.as_str(), c.as_str(), *ro))
            .collect();
        let volumes_owned: Vec<(String, String)> = vec![
            (
                format!("borg-cache-{repo_name}-target"),
                "/workspace/repo/target".to_string(),
            ),
            (
                format!("borg-cache-{repo_name}-cargo-registry"),
                "/home/bun/.cargo/registry".to_string(),
            ),
        ];
        let volumes_ref: Vec<(&str, &str)> = volumes_owned
            .iter()
            .map(|(n, c)| (n.as_str(), c.as_str()))
            .collect();

        let network = if self.agent_network_available {
            Some(Sandbox::AGENT_NETWORK)
        } else {
            None
        };
        let output = tokio::time::timeout(
            timeout,
            Sandbox::docker_command(
                &self.config.container_image,
                &binds_ref,
                &volumes_ref,
                "",
                &bash_cmd,
                &[],
                self.config.container_memory_mb,
                self.config.container_cpus,
                network,
            )
            .kill_on_drop(true)
            .output(),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "run_test_in_container timed out after {}s",
                timeout.as_secs()
            )
        })?
        .context("run_test_in_container")?;

        Ok(TestOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(1),
        })
    }

    // ── Integration merge ─────────────────────────────────────────────────

    pub async fn check_integration(self: &Arc<Self>) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let last = self.db.get_ts("last_release_ts");
        let min_interval = if self.config.continuous_mode {
            60i64
        } else {
            self.config.release_interval_mins as i64 * 60
        };
        if now - last < min_interval {
            return Ok(());
        }

        let mut any_merged = false;
        for repo in &self.config.watched_repos {
            let queued = self.db.get_queued_branches_for_repo(&repo.path)?;
            if queued.is_empty() {
                continue;
            }
            if repo.repo_slug.is_empty() {
                warn!("Integration: no repo_slug for {}, skipping", repo.path);
                continue;
            }
            info!("Integration: {} branches for {}", queued.len(), repo.path);
            match self
                .run_integration(queued, &repo.repo_slug, repo.auto_merge)
                .await
            {
                Ok(merged) => any_merged |= merged,
                Err(e) => warn!("Integration error for {}: {e}", repo.path),
            }
        }

        // Only reset the release timer when merges actually happened.
        // If integration ran but only sent branches to rebase (no merges),
        // skip resetting so we re-check promptly after rebase completes.
        if any_merged {
            self.db
                .set_ts("last_release_ts", chrono::Utc::now().timestamp());
        }
        Ok(())
    }

    /// Run a `gh` command without a working directory.
    async fn gh(&self, args: &[&str]) -> Result<TestOutput> {
        let timeout = std::time::Duration::from_secs(self.config.agent_timeout_s.max(300) as u64);
        let mut cmd = tokio::process::Command::new("gh");
        cmd.args(args);
        if !self.config.github_token.is_empty() {
            cmd.env("GH_TOKEN", &self.config.github_token);
        }
        let output = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "gh {} timed out after {}s",
                args.join(" "),
                timeout.as_secs()
            )
        })?
        .context("gh command")?;
        Ok(TestOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(1),
        })
    }

    /// Returns true if any branches were actually merged.
    async fn run_integration(
        &self,
        queued: Vec<crate::types::QueueEntry>,
        slug: &str,
        auto_merge: bool,
    ) -> Result<bool> {
        let mut live = Vec::new();
        for entry in queued {
            let check = self
                .gh(&[
                    "api",
                    "--silent",
                    &format!("repos/{slug}/branches/{}", entry.branch),
                ])
                .await;
            if check.map(|r| r.exit_code == 0).unwrap_or(false) {
                live.push(entry);
            } else {
                warn!(
                    "Excluding {} from integration: branch not found on remote",
                    entry.branch
                );
                self.db
                    .update_queue_status_with_error(entry.id, "excluded", "branch not found")?;
            }
        }
        if live.is_empty() {
            return Ok(false);
        }

        let mut excluded_ids: HashSet<i64> = HashSet::new();
        let mut freshly_created: HashSet<i64> = HashSet::new();

        for entry in &live {
            // Check if already merged on GitHub
            let state_out = self
                .gh(&[
                    "pr",
                    "view",
                    &entry.branch,
                    "--repo",
                    slug,
                    "--json",
                    "state",
                    "--jq",
                    ".state",
                ])
                .await;
            if let Ok(ref o) = state_out {
                let s = o.stdout.trim();
                if s == "MERGED" {
                    info!(
                        "Task #{} {}: PR already merged",
                        entry.task_id, entry.branch
                    );
                    self.db.update_queue_status(entry.id, "merged")?;
                    self.db.update_task_status(entry.task_id, "merged", None)?;
                    excluded_ids.insert(entry.id);
                    continue;
                }
                // CLOSED + identical to main → squash-merged
                if s == "CLOSED" {
                    let cmp = self
                        .gh(&[
                            "api",
                            &format!("repos/{slug}/compare/main...{}", entry.branch),
                            "--jq",
                            ".status",
                        ])
                        .await;
                    if cmp.map(|r| r.stdout.trim() == "identical").unwrap_or(false) {
                        info!(
                            "Task #{} {}: identical to main, marking merged",
                            entry.task_id, entry.branch
                        );
                        self.db.update_queue_status(entry.id, "merged")?;
                        self.db.update_task_status(entry.task_id, "merged", None)?;
                        excluded_ids.insert(entry.id);
                        continue;
                    }
                    // Closed but not identical: attempt reopen so the branch can re-enter merge flow.
                    let pr_num = self
                        .gh(&[
                            "pr",
                            "view",
                            &entry.branch,
                            "--repo",
                            slug,
                            "--json",
                            "number",
                            "--jq",
                            ".number",
                        ])
                        .await
                        .ok()
                        .map(|o| o.stdout.trim().to_string())
                        .filter(|s| !s.is_empty());
                    if let Some(num) = pr_num {
                        let reopened = self
                            .gh(&["pr", "reopen", &num, "--repo", slug])
                            .await
                            .ok()
                            .filter(|o| o.exit_code == 0);
                        if reopened.is_some() {
                            info!(
                                "Task #{} {}: reopened closed PR #{}",
                                entry.task_id, entry.branch, num
                            );
                            continue;
                        }
                    }
                }
            }

            // Check if PR already exists
            let view_out = self
                .gh(&[
                    "pr",
                    "view",
                    &entry.branch,
                    "--repo",
                    slug,
                    "--json",
                    "number,state",
                    "--jq",
                    ".state + \" \" + (.number|tostring)",
                ])
                .await;
            let view_out = match view_out {
                Ok(o) => o,
                Err(e) => {
                    warn!("gh pr view {}: {e}", entry.branch);
                    continue;
                },
            };
            if view_out.exit_code == 0 && !view_out.stdout.trim().is_empty() {
                let mut parts = view_out.stdout.split_whitespace();
                let state = parts.next().unwrap_or_default();
                let number = parts.next().unwrap_or_default();
                if state == "OPEN" {
                    continue;
                }
                if state == "CLOSED" && !number.is_empty() {
                    let reopened = self
                        .gh(&["pr", "reopen", number, "--repo", slug])
                        .await
                        .ok()
                        .filter(|o| o.exit_code == 0);
                    if reopened.is_some() {
                        info!(
                            "Task #{} {}: reopened PR #{}",
                            entry.task_id, entry.branch, number
                        );
                        continue;
                    }
                }
            }

            // Get task title for PR
            let title = self
                .db
                .get_task(entry.task_id)?
                .map(|t| t.title.chars().take(100).collect::<String>())
                .unwrap_or_else(|| entry.branch.clone());

            let create_out = self
                .gh(&[
                    "pr",
                    "create",
                    "--repo",
                    slug,
                    "--base",
                    "main",
                    "--head",
                    &entry.branch,
                    "--title",
                    &title,
                    "--body",
                    "Automated implementation.",
                ])
                .await;
            let create_out = match create_out {
                Ok(o) => o,
                Err(e) => {
                    warn!("gh pr create {}: {e}", entry.branch);
                    continue;
                },
            };

            if create_out.exit_code != 0 {
                let err = &create_out.stderr[..create_out.stderr.len().min(300)];
                if err.contains("No commits between") {
                    info!(
                        "Task #{} {}: no commits vs main, marking merged",
                        entry.task_id, entry.branch
                    );
                    self.db.update_queue_status(entry.id, "merged")?;
                    self.db.update_task_status(entry.task_id, "merged", None)?;
                    excluded_ids.insert(entry.id);
                } else {
                    warn!("gh pr create {}: {}", entry.branch, err);
                }
            } else {
                info!("Created PR for {}", entry.branch);
                freshly_created.insert(entry.id);
            }
        }

        let mut merged_branches: Vec<String> = Vec::new();

        if !auto_merge {
            for entry in &live {
                if excluded_ids.contains(&entry.id) {
                    continue;
                }
                self.db.update_queue_status(entry.id, "pending_review")?;
                info!(
                    "Task #{} {}: PR ready for manual review",
                    entry.task_id, entry.branch
                );
            }
        } else {
            // ── Merge queue: serialize to one merge per cycle ──────────────
            //
            // Pick the oldest non-excluded, non-freshly-created entry. Verify
            // it is current with main (behind_by == 0) before merging. A branch
            // rebased onto main N has behind_by=0 and will fast-forward onto N,
            // producing an identical file tree to what the compile check tested.
            // If any other PR was merged since the rebase, behind_by > 0 and we
            // send the branch back to rebase rather than risk a corrupted merge.
            let candidate = live
                .iter()
                .find(|e| !excluded_ids.contains(&e.id) && !freshly_created.contains(&e.id));

            if let Some(entry) = candidate {
                // Check if PR is already merged (picked up from a prior run)
                let state_check = self
                    .gh(&[
                        "pr",
                        "view",
                        &entry.branch,
                        "--repo",
                        slug,
                        "--json",
                        "state",
                        "--jq",
                        ".state",
                    ])
                    .await;
                let pr_state = state_check
                    .as_ref()
                    .map(|o| o.stdout.trim().to_string())
                    .unwrap_or_default();

                if pr_state == "MERGED" {
                    info!("Task #{} {}: already merged", entry.task_id, entry.branch);
                    self.db.update_queue_status(entry.id, "merged")?;
                    self.db.update_task_status(entry.task_id, "merged", None)?;
                    merged_branches.push(entry.branch.clone());
                } else {
                    // Check how far behind main this branch is.
                    // behind_by == 0 means the branch was rebased onto current main tip.
                    // A fast-forward merge then produces exactly what the rebase compile
                    // check tested — no new conflicts can arise.
                    let compare = self
                        .gh(&[
                            "api",
                            &format!("repos/{slug}/compare/main...{}", entry.branch),
                            "--jq",
                            ".behind_by",
                        ])
                        .await;
                    let behind_by: u64 = compare
                        .as_ref()
                        .ok()
                        .and_then(|o| o.stdout.trim().parse().ok())
                        .unwrap_or(1); // default conservative: treat unknown as stale

                    if behind_by > 0 {
                        info!(
                            "Task #{} {}: behind main by {}, sending to rebase",
                            entry.task_id, entry.branch, behind_by
                        );
                        self.db.update_queue_status_with_error(
                            entry.id,
                            "excluded",
                            "behind main — rebase required",
                        )?;
                        self.db.update_task_status(entry.task_id, "rebase", None)?;
                    } else {
                        // behind_by == 0 → safe to fast-forward merge
                        self.db.update_queue_status(entry.id, "merging")?;
                        let merge_out = self
                            .gh(&["pr", "merge", &entry.branch, "--repo", slug, "--merge"])
                            .await;

                        match merge_out {
                            Err(e) => {
                                warn!("gh pr merge {}: {e}", entry.branch);
                                self.db.update_queue_status(entry.id, "queued")?;
                            },
                            Ok(out) if out.exit_code != 0 => {
                                let err = &out.stderr[..out.stderr.len().min(200)];
                                warn!("gh pr merge {}: {}", entry.branch, err);
                                if err.contains("not mergeable")
                                    || err.contains("cannot be cleanly created")
                                {
                                    self.db.update_queue_status_with_error(
                                        entry.id,
                                        "excluded",
                                        "merge conflict with main",
                                    )?;
                                    self.db.update_task_status(entry.task_id, "rebase", None)?;
                                    info!("Task #{} has conflicts, sent to rebase", entry.task_id);
                                } else {
                                    self.db.update_queue_status(entry.id, "queued")?;
                                }
                            },
                            Ok(_) => {
                                self.db.update_queue_status(entry.id, "merged")?;
                                self.db.update_task_status(entry.task_id, "merged", None)?;
                                merged_branches.push(entry.branch.clone());
                                let _ = self
                                    .gh(&[
                                        "api",
                                        "-X",
                                        "DELETE",
                                        &format!("repos/{slug}/git/refs/heads/{}", entry.branch),
                                    ])
                                    .await;
                                if let Ok(Some(task)) = self.db.get_task(entry.task_id) {
                                    let duration_str = task
                                        .duration_secs
                                        .map(|s| {
                                            if s >= 3600 {
                                                format!(" ({}h{}m)", s / 3600, (s % 3600) / 60)
                                            } else if s >= 60 {
                                                format!(" ({}m{}s)", s / 60, s % 60)
                                            } else {
                                                format!(" ({}s)", s)
                                            }
                                        })
                                        .unwrap_or_default();
                                    self.notify(
                                        &task.notify_chat,
                                        &format!(
                                            "Task #{} \"{}\" merged via PR{}.",
                                            task.id, task.title, duration_str
                                        ),
                                    );
                                }
                            },
                        }
                    }
                }
            }
        }

        if !merged_branches.is_empty() {
            let digest = self.generate_digest(&merged_branches);
            self.notify(&self.config.pipeline_admin_chat, &digest);
            info!("Integration complete: {} merged", merged_branches.len());
        }

        Ok(!merged_branches.is_empty())
    }

    fn generate_digest(&self, merged: &[String]) -> String {
        let mut s = format!("*{} PR(s) merged*\n", merged.len());
        for branch in merged {
            s.push_str(&format!("  + {branch}\n"));
        }
        s
    }

    // ── Seed ─────────────────────────────────────────────────────────────

    async fn seed_if_idle(&self) -> Result<()> {
        if !self.config.continuous_mode {
            return Ok(());
        }

        let active = self.db.list_active_tasks()?.len() as u32;
        if active >= self.config.pipeline_max_backlog {
            return Ok(());
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let cooldown = self.config.pipeline_seed_cooldown_s;

        for repo in &self.config.watched_repos {
            if repo.is_self {
                let key = (repo.path.clone(), "github_open_issues".to_string());
                {
                    let mut cooldowns = self.seed_cooldowns.lock().await;
                    if now - cooldowns.get(&key).copied().unwrap_or(0) >= cooldown {
                        cooldowns.insert(key.clone(), now);
                        drop(cooldowns);
                        let _ = self.db.set_seed_cooldown(&key.0, &key.1, now);
                        info!("seed scan: 'github_open_issues' for {}", repo.path);
                        if let Err(e) = self.seed_from_open_issues(repo) {
                            warn!("seed github_open_issues for {}: {e}", repo.path);
                        }
                    }
                }
            }

            let mode = match self.resolve_mode(&repo.mode) {
                Some(m) => m,
                None => continue,
            };
            for seed_cfg in mode.seed_modes.clone() {
                // Non-primary repos only get proposal seeds — skip task seeds to avoid
                // creating automated pipeline tasks for repos we don't auto-merge.
                if !repo.is_self && seed_cfg.output_type == SeedOutputType::Task {
                    continue;
                }
                // Re-check backlog limit between seeds to avoid blocking for ages
                if let Ok(active) = self.db.list_active_tasks() {
                    if active.len() as u32 >= self.config.pipeline_max_backlog {
                        info!("seed: backlog full, stopping seed scan early");
                        return Ok(());
                    }
                }
                let key = (repo.path.clone(), seed_cfg.name.clone());
                {
                    let mut cooldowns = self.seed_cooldowns.lock().await;
                    if now - cooldowns.get(&key).copied().unwrap_or(0) < cooldown {
                        continue;
                    }
                    cooldowns.insert(key.clone(), now);
                }
                let _ = self.db.set_seed_cooldown(&key.0, &key.1, now);
                info!("seed scan: '{}' for {}", seed_cfg.name, repo.path);
                if let Err(e) = self.run_seed(repo, &mode.name, &seed_cfg).await {
                    warn!("seed {} for {}: {e}", seed_cfg.name, repo.path);
                }
            }
        }

        Ok(())
    }

    fn seed_from_open_issues(&self, repo: &RepoConfig) -> Result<()> {
        let gh_available = Command::new("gh")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !gh_available {
            info!(
                "seed github_open_issues skipped for {}: gh CLI not available",
                repo.path
            );
            return Ok(());
        }

        let mode_name = match self.resolve_mode(&repo.mode) {
            Some(m) => m.name,
            None => {
                warn!(
                    "seed_from_open_issues: unknown pipeline mode {:?}, skipping",
                    repo.mode
                );
                return Ok(());
            },
        };

        let active = self.db.list_active_tasks()?.len() as i64;
        let available_slots = (self.config.pipeline_max_backlog as i64 - active).max(0) as usize;
        if available_slots == 0 {
            return Ok(());
        }

        let issues = self.fetch_open_issues(repo)?;
        if issues.is_empty() {
            return Ok(());
        }

        let existing_tasks = self.db.list_all_tasks(Some(&repo.path))?;
        let existing_proposals = self.db.list_all_proposals(Some(&repo.path))?;
        let mut created = 0usize;
        let mut skipped_existing = 0usize;

        for issue in issues {
            if created >= available_slots {
                break;
            }
            let marker = issue_seed_marker(&issue.url);
            let already_exists = existing_tasks
                .iter()
                .any(|t| t.description.contains(&marker))
                || existing_proposals
                    .iter()
                    .any(|p| p.rationale.contains(&marker));
            if already_exists {
                skipped_existing += 1;
                continue;
            }

            let labels = issue
                .labels
                .iter()
                .map(|l| l.name.trim())
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
            let label_line = if labels.is_empty() {
                String::new()
            } else {
                format!("Labels: {labels}\n\n")
            };

            let mut description = format!(
                "Imported from GitHub issue #{}.\n\n{}{}",
                issue.number,
                label_line,
                trim_issue_body(&issue.body)
            );
            description.push_str("\n\n");
            description.push_str(&marker);

            let task = Task {
                id: 0,
                title: format!("Issue #{}: {}", issue.number, issue.title.trim()),
                description,
                repo_path: repo.path.clone(),
                branch: String::new(),
                status: "backlog".to_string(),
                attempt: 0,
                max_attempts: 5,
                last_error: String::new(),
                created_by: "issue_seed".to_string(),
                notify_chat: String::new(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                session_id: String::new(),
                mode: mode_name.clone(),
                backend: String::new(),
                workspace_id: 0,
                project_id: 0,
                task_type: String::new(),
                requires_exhaustive_corpus_review: false,
                started_at: None,
                completed_at: None,
                duration_secs: None,
                review_status: None,
                revision_count: 0,
                chat_thread: String::new(),
            };
            match self.db.insert_task(&task) {
                Ok(id) => {
                    created += 1;
                    info!("seed github_open_issues created task #{id}: {}", task.title);
                },
                Err(e) => warn!("seed github_open_issues insert_task: {e}"),
            }
        }

        if created > 0 || skipped_existing > 0 {
            info!(
                "seed github_open_issues for {}: created={}, skipped_existing={}",
                repo.path, created, skipped_existing
            );
        }
        Ok(())
    }

    fn fetch_open_issues(&self, repo: &RepoConfig) -> Result<Vec<GithubIssue>> {
        let mut cmd = Command::new("gh");
        cmd.args([
            "issue",
            "list",
            "--state",
            "open",
            "--limit",
            "100",
            "--json",
            "number,title,body,url,labels",
        ]);
        if !repo.repo_slug.is_empty() {
            cmd.args(["--repo", &repo.repo_slug]);
        } else if std::path::Path::new(&repo.path).exists() {
            cmd.current_dir(&repo.path);
        } else {
            anyhow::bail!("no repo_slug or local checkout for {}", repo.path);
        }
        let output = cmd
            .output()
            .with_context(|| format!("failed to execute `gh issue list` for {}", repo.path))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            anyhow::bail!("gh issue list failed for {}: {}", repo.path, stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let issues: Vec<GithubIssue> = serde_json::from_str(&stdout)
            .with_context(|| format!("failed to parse gh issue list JSON for {}", repo.path))?;
        Ok(issues)
    }

    async fn run_seed(
        &self,
        repo: &RepoConfig,
        mode_name: &str,
        seed_cfg: &crate::types::SeedConfig,
    ) -> Result<()> {
        let session_dir = std::fs::canonicalize("store/sessions/seed")
            .unwrap_or_else(|_| {
                std::fs::create_dir_all("store/sessions/seed").ok();
                std::fs::canonicalize("store/sessions/seed")
                    .unwrap_or_else(|_| std::path::PathBuf::from("store/sessions/seed"))
            })
            .to_string_lossy()
            .to_string();
        tokio::fs::create_dir_all(&session_dir).await.ok();

        let task = Task {
            id: 0,
            title: format!("seed:{}", seed_cfg.name),
            description: String::new(),
            repo_path: repo.path.clone(),
            branch: String::new(),
            status: "seed".to_string(),
            attempt: 0,
            max_attempts: 1,
            last_error: String::new(),
            created_by: "seed".to_string(),
            notify_chat: String::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            session_id: String::new(),
            mode: mode_name.to_string(),
            backend: String::new(),
            workspace_id: 0,
            project_id: 0,
            task_type: String::new(),
            requires_exhaustive_corpus_review: false,
            started_at: None,
            completed_at: None,
            duration_secs: None,
            review_status: None,
            revision_count: 0,
            chat_thread: String::new(),
        };

        let task_suffix =
            "\n\nFor each improvement, output EXACTLY this format (one per task):\n\n\
            TASK_START\n\
            Title: <short imperative title, max 80 chars>\n\
            Description: <2-4 sentences explaining what to change and why>\n\
            TASK_END\n\n\
            Output ONLY the task blocks above. No other text.";
        let proposal_suffix = "\n\nFor each proposal, output EXACTLY this format:\n\n\
            PROPOSAL_START\n\
            Title: <short imperative title, max 80 chars>\n\
            Description: <2-4 sentences explaining the feature or change>\n\
            Rationale: <1-2 sentences on why this would be valuable>\n\
            PROPOSAL_END\n\n\
            Output ONLY the proposal blocks above. No other text.";
        let preamble = "First, thoroughly explore the codebase before making any suggestions. \
            Use Read to examine key source files, Grep to search for patterns, \
            and Glob to discover the project structure. Understand the architecture, \
            existing patterns, and current state of the code.\n\nThen, based on your exploration:\n\n";
        let suffix = match seed_cfg.output_type {
            SeedOutputType::Task => task_suffix,
            SeedOutputType::Proposal => proposal_suffix,
        };
        let instruction = format!("{preamble}{}{suffix}", seed_cfg.prompt);

        let phase = PhaseConfig {
            name: format!("seed_{}", seed_cfg.name),
            label: seed_cfg.label.clone(),
            instruction,
            fresh_session: true,
            include_file_listing: true,
            allowed_tools: if seed_cfg.allowed_tools.is_empty() {
                "Read,Glob,Grep,Bash".to_string()
            } else {
                seed_cfg.allowed_tools.clone()
            },
            ..Default::default()
        };

        let ctx = self.make_context(&task, repo.path.clone(), session_dir, Vec::new());

        info!("running seed '{}' for {}", seed_cfg.name, repo.path);
        let backend = self
            .resolve_backend(&task)
            .ok_or_else(|| anyhow::anyhow!("no backends configured for seed"))?;
        let result = self.run_backend_phase(&backend, &task, &phase, ctx).await?;

        if !result.success {
            warn!(
                "seed '{}' for {} failed (output: {:?})",
                seed_cfg.name, repo.path, &result.output
            );
        } else {
            info!("seed '{}' output: {:?}", seed_cfg.name, &result.output);
        }

        let target_repo = if seed_cfg.target_primary_repo {
            self.config
                .watched_repos
                .iter()
                .find(|r| r.is_self)
                .map(|r| r.path.as_str())
                .unwrap_or(&repo.path)
        } else {
            &repo.path
        };
        self.parse_seed_output(&result.output, target_repo, mode_name, seed_cfg.output_type)?;
        Ok(())
    }

    fn parse_seed_output(
        &self,
        output: &str,
        repo_path: &str,
        mode_name: &str,
        output_type: SeedOutputType,
    ) -> Result<()> {
        match output_type {
            SeedOutputType::Task => {
                for block in extract_blocks(output, "TASK_START", "TASK_END") {
                    let title = extract_field(&block, "Title:").unwrap_or_default();
                    let description = extract_field(&block, "Description:").unwrap_or_default();
                    if title.is_empty() {
                        continue;
                    }
                    let task = Task {
                        id: 0,
                        title,
                        description,
                        repo_path: repo_path.to_string(),
                        branch: String::new(),
                        status: "backlog".to_string(),
                        attempt: 0,
                        max_attempts: 5,
                        last_error: String::new(),
                        created_by: "seed".to_string(),
                        notify_chat: String::new(),
                        created_at: Utc::now(),
                        updated_at: Utc::now(),
                        session_id: String::new(),
                        mode: mode_name.to_string(),
                        backend: String::new(),
                        workspace_id: 0,
                        project_id: 0,
                        task_type: String::new(),
                        requires_exhaustive_corpus_review: false,
                        started_at: None,
                        completed_at: None,
                        duration_secs: None,
                        review_status: None,
                        revision_count: 0,
                        chat_thread: String::new(),
                    };
                    match self.db.insert_task(&task) {
                        Ok(id) => info!("seed created task #{id}: {}", task.title),
                        Err(e) => warn!("seed insert_task: {e}"),
                    }
                }
            },
            SeedOutputType::Proposal => {
                for block in extract_blocks(output, "PROPOSAL_START", "PROPOSAL_END") {
                    let title = extract_field(&block, "Title:").unwrap_or_default();
                    let description = extract_field(&block, "Description:").unwrap_or_default();
                    let rationale = extract_field(&block, "Rationale:").unwrap_or_default();
                    if title.is_empty() {
                        continue;
                    }
                    let proposal = Proposal {
                        id: 0,
                        repo_path: repo_path.to_string(),
                        title,
                        description,
                        rationale,
                        status: "proposed".to_string(),
                        created_at: Utc::now(),
                        triage_score: 0,
                        triage_impact: 0,
                        triage_feasibility: 0,
                        triage_risk: 0,
                        triage_effort: 0,
                        triage_reasoning: String::new(),
                    };
                    match self.db.insert_proposal(&proposal) {
                        Ok(id) => info!("seed created proposal #{id}: {}", proposal.title),
                        Err(e) => warn!("seed insert_proposal: {e}"),
                    }
                }
            },
        }
        Ok(())
    }

    // ── Mirror refresh ────────────────────────────────────────────────────

    /// Refresh bare mirrors for all watched repos at the configured interval.
    /// Mirrors are mounted read-only into containers to accelerate `git clone`.
    async fn refresh_mirrors(&self) {
        let interval = self.config.mirror_refresh_interval_s;
        if interval <= 0 {
            return;
        }
        let now = chrono::Utc::now().timestamp();
        if now - self.db.get_ts("last_mirror_refresh_ts") < interval {
            return;
        }
        self.db.set_ts("last_mirror_refresh_ts", now);

        let mirrors_dir = format!("{}/mirrors", self.config.data_dir);
        if let Err(e) = std::fs::create_dir_all(&mirrors_dir) {
            warn!("refresh_mirrors: cannot create mirrors dir: {e}");
            return;
        }

        for repo in &self.config.watched_repos {
            let repo_name = std::path::Path::new(&repo.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if repo_name.is_empty() {
                continue;
            }
            let mirror_path = format!("{mirrors_dir}/{repo_name}.git");
            let path = repo.path.clone();
            let mirror = mirror_path.clone();
            tokio::spawn(async move {
                if !std::path::Path::new(&mirror).exists() {
                    let out = tokio::process::Command::new("git")
                        .args(["clone", "--mirror", &path, &mirror])
                        .output()
                        .await;
                    match out {
                        Ok(o) if o.status.success() => info!("mirrored {path} → {mirror}"),
                        Ok(o) => warn!(
                            "git clone --mirror failed for {path}: {}",
                            String::from_utf8_lossy(&o.stderr).trim()
                        ),
                        Err(e) => warn!("git clone --mirror spawn failed for {path}: {e}"),
                    }
                } else {
                    let out = tokio::process::Command::new("git")
                        .args(["-C", &mirror, "fetch", "--prune", "--tags"])
                        .output()
                        .await;
                    if let Ok(o) = out {
                        if !o.status.success() {
                            warn!(
                                "git fetch on mirror {mirror} failed: {}",
                                String::from_utf8_lossy(&o.stderr).trim()
                            );
                        }
                    }
                }
            });
        }
    }

    // ── Auto-promote + auto-triage ────────────────────────────────────────

    pub fn maybe_auto_promote_proposals(&self) {
        let active = self.db.active_task_count();
        let max = self.config.pipeline_max_backlog as i64;
        if active >= max {
            return;
        }
        let slots = max - active;
        let proposals = match self
            .db
            .get_top_scored_proposals(self.config.proposal_promote_threshold, slots)
        {
            Ok(p) => p,
            Err(e) => {
                warn!("auto_promote: {e}");
                return;
            },
        };
        for p in proposals {
            let repo_cfg = self
                .config
                .watched_repos
                .iter()
                .find(|r| r.path == p.repo_path);
            // Only auto-promote for repos that allow auto-merge
            if let Some(repo) = repo_cfg {
                if !repo.auto_merge {
                    continue;
                }
            }
            let mode = repo_cfg.map(|r| r.mode.as_str()).unwrap_or("sweborg");
            let task = crate::types::Task {
                id: 0,
                title: p.title.clone(),
                description: p.description.clone(),
                repo_path: p.repo_path.clone(),
                branch: String::new(),
                status: "backlog".into(),
                attempt: 0,
                max_attempts: 5,
                last_error: String::new(),
                created_by: "proposal".into(),
                notify_chat: String::new(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                session_id: String::new(),
                mode: mode.to_string(),
                backend: String::new(),
                workspace_id: 0,
                project_id: 0,
                task_type: String::new(),
                requires_exhaustive_corpus_review: false,
                started_at: None,
                completed_at: None,
                duration_secs: None,
                review_status: None,
                revision_count: 0,
                chat_thread: String::new(),
            };
            match self.db.insert_task(&task) {
                Ok(id) => {
                    self.db.update_proposal_status(p.id, "approved").ok();
                    info!(
                        "Auto-promoted proposal #{} (score={}) → task #{}: {}",
                        p.id, p.triage_score, id, p.title
                    );
                },
                Err(e) => warn!("auto_promote insert_task: {e}"),
            }
        }
    }

    pub async fn maybe_auto_triage(&self) {
        const TRIAGE_INTERVAL_S: i64 = 6 * 3600;
        let now = chrono::Utc::now().timestamp();
        if now - self.db.get_ts("last_triage_ts") < TRIAGE_INTERVAL_S {
            return;
        }
        if self.db.count_unscored_proposals() == 0 {
            return;
        }
        self.db.set_ts("last_triage_ts", now);

        let proposals = match self.db.list_untriaged_proposals() {
            Ok(p) if !p.is_empty() => p,
            _ => return,
        };
        let merged = self.db.get_recent_merged_tasks(50).unwrap_or_default();

        let mut prompt = String::from(
            "Rate each proposal on 4 dimensions (1-5 scale), and flag proposals that should be auto-dismissed.\n\n\
            Dimensions:\n\
            - impact: How much value does this deliver? (5=critical, 1=cosmetic)\n\
            - feasibility: How likely is an AI agent to implement this correctly? (5=trivial, 1=needs human)\n\
            - risk: How likely to break existing functionality? (5=very risky, 1=safe)\n\
            - effort: How many agent cycles will this need? (5=massive, 1=simple)\n\n\
            Overall score formula: (impact*2 + feasibility*2 - risk - effort) mapped to 1-10.\n\n\
            Set \"dismiss\": true if: already implemented, duplicate, nonsensical, vague, or irrelevant.\n\n\
            Reply with ONLY a JSON array, no markdown fences:\n\
            [{\"id\": <n>, \"impact\": <1-5>, \"feasibility\": <1-5>, \"risk\": <1-5>, \"effort\": <1-5>, \"score\": <1-10>, \"reasoning\": \"<one sentence>\", \"dismiss\": <bool>}]\n\n",
        );
        if !merged.is_empty() {
            prompt.push_str("Recently merged tasks (for duplicate detection):\n");
            for t in &merged {
                prompt.push_str(&format!("- {}\n", t.title));
            }
            prompt.push('\n');
        }
        prompt.push_str("Proposals to evaluate:\n\n");
        for p in &proposals {
            prompt.push_str(&format!(
                "- ID {}: {}\n  Description: {}\n  Rationale: {}\n\n",
                p.id,
                p.title,
                if p.description.is_empty() {
                    "(none)"
                } else {
                    &p.description
                },
                if p.rationale.is_empty() {
                    "(none)"
                } else {
                    &p.rationale
                },
            ));
        }

        let output = self
            .run_claude_print("claude-haiku-4-5-20251001", &prompt)
            .await;
        let output = match output {
            Ok(o) => o,
            Err(e) => {
                warn!("auto_triage: {e}");
                return;
            },
        };

        let arr_start = match output.find('[') {
            Some(i) => i,
            None => {
                warn!("auto_triage: no JSON array in output");
                return;
            },
        };
        let arr_end = match output.rfind(']') {
            Some(i) => i + 1,
            None => return,
        };
        let json_slice = &output[arr_start..arr_end];

        let items: Vec<serde_json::Value> = match serde_json::from_str(json_slice) {
            Ok(v) => v,
            Err(e) => {
                warn!("auto_triage: JSON parse failed: {e}");
                return;
            },
        };

        let mut scored = 0u32;
        let mut dismissed = 0u32;
        for item in &items {
            let Some((p_id, impact, feasibility, risk, effort, score, reasoning, should_dismiss)) =
                parse_triage_item(item)
            else {
                continue;
            };

            if let Err(e) = self.db.update_proposal_triage(
                p_id,
                score,
                impact,
                feasibility,
                risk,
                effort,
                reasoning,
            ) {
                warn!("auto_triage: update_proposal_triage #{p_id}: {e}");
                continue;
            }
            scored += 1;
            if should_dismiss {
                self.db.update_proposal_status(p_id, "auto_dismissed").ok();
                dismissed += 1;
                info!("Auto-triage: dismissed proposal #{p_id}: {reasoning}");
            }
        }
        info!(
            "Auto-triage: scored {scored}/{} proposals, dismissed {dismissed}",
            proposals.len()
        );
    }

    async fn maybe_prune_cache_volumes(&self) {
        const PRUNE_INTERVAL_S: i64 = 24 * 3600;
        let now = chrono::Utc::now().timestamp();
        let last = self
            .last_cache_prune_secs
            .load(std::sync::atomic::Ordering::Relaxed);
        if now - last < PRUNE_INTERVAL_S {
            return;
        }
        self.last_cache_prune_secs
            .store(now, std::sync::atomic::Ordering::Relaxed);
        Sandbox::prune_stale_cache_volumes(7).await;
    }

    async fn maybe_prune_session_dirs(&self) {
        const PRUNE_INTERVAL_S: i64 = 3600;
        let now = chrono::Utc::now().timestamp();
        let last = self
            .last_session_prune_secs
            .load(std::sync::atomic::Ordering::Relaxed);
        if now - last < PRUNE_INTERVAL_S {
            return;
        }
        self.last_session_prune_secs
            .store(now, std::sync::atomic::Ordering::Relaxed);

        let max_age_secs = self.config.session_max_age_hours * 3600;
        if max_age_secs <= 0 {
            return;
        }

        let sessions_dir = format!("{}/sessions", self.config.data_dir);
        let in_flight_ids: HashSet<i64> = self
            .in_flight
            .try_lock()
            .map(|g| g.clone())
            .unwrap_or_default();

        let to_remove = collect_stale_session_dirs(
            &sessions_dir,
            now,
            max_age_secs,
            &in_flight_ids,
            |task_id| {
                self.db
                    .get_task(task_id)
                    .ok()
                    .flatten()
                    .map(|t| t.created_at.timestamp())
            },
        );

        let mut pruned = 0usize;
        for path in to_remove {
            match tokio::fs::remove_dir_all(&path).await {
                Ok(()) => pruned += 1,
                Err(e) => warn!("failed to remove session dir {}: {e}", path.display()),
            }
        }
        if pruned > 0 {
            info!("pruned {pruned} stale session dir(s) from {sessions_dir}");
        }
    }

    fn maybe_alert_guardrails(&self) {
        const ALERT_INTERVAL_S: i64 = 5 * 60;
        let now = chrono::Utc::now().timestamp();
        let last = self.db.get_ts("last_guardrail_check_ts");
        if now - last < ALERT_INTERVAL_S {
            return;
        }
        self.db.set_ts("last_guardrail_check_ts", now);

        let rebase_count = self.db.count_tasks_with_status("rebase").unwrap_or(0);
        if rebase_count >= 50 {
            let last_alert = self.db.get_ts("last_alert_rebase_backlog_ts");
            if now - last_alert >= 15 * 60 {
                self.db.set_ts("last_alert_rebase_backlog_ts", now);
                let msg = format!(
                    "Guardrail alert: rebase backlog is high ({rebase_count} tasks in rebase)."
                );
                warn!("{msg}");
                self.notify(&self.config.pipeline_admin_chat, &msg);
            }
        }

        let queued_count = self.db.count_queue_with_status("queued").unwrap_or(0)
            + self.db.count_queue_with_status("merging").unwrap_or(0);
        let last_merge_ts = self.db.get_ts("last_release_ts");
        let backlog_started_ts = self.db.get_ts("last_no_merge_backlog_started_ts");
        let (baseline_ts, next_backlog_started_ts) = no_merge_guardrail_baseline(
            queued_count,
            last_merge_ts,
            backlog_started_ts,
            now,
        );
        if next_backlog_started_ts != backlog_started_ts {
            self.db
                .set_ts("last_no_merge_backlog_started_ts", next_backlog_started_ts);
        }
        if let Some(baseline_ts) = baseline_ts.filter(|baseline| now - baseline >= 60 * 60) {
            let last_alert = self.db.get_ts("last_alert_no_merge_ts");
            if now - last_alert >= 15 * 60 {
                self.db.set_ts("last_alert_no_merge_ts", now);
                let mins = (now - baseline_ts) / 60;
                let msg = format!(
                    "Guardrail alert: {queued_count} queued/merging entries and no merge for {mins} minutes."
                );
                warn!("{msg}");
                self.notify(&self.config.pipeline_admin_chat, &msg);
            }
        }

        if let Some(inode_used_pct) = tmp_inode_usage_percent("/tmp") {
            if inode_used_pct >= 90.0 {
                let last_alert = self.db.get_ts("last_alert_tmp_inode_ts");
                if now - last_alert >= 15 * 60 {
                    self.db.set_ts("last_alert_tmp_inode_ts", now);
                    let msg = format!(
                        "Guardrail alert: /tmp inode usage is high ({inode_used_pct:.1}%)."
                    );
                    warn!("{msg}");
                    self.notify(&self.config.pipeline_admin_chat, &msg);
                }
            }
        }
    }

    /// Run `claude --print --model <model>` with prompt on stdin, return stdout.
    async fn run_claude_print(&self, model: &str, prompt: &str) -> Result<String> {
        use tokio::io::AsyncWriteExt;
        let mut child = tokio::process::Command::new("claude")
            .args([
                "--print",
                "--model",
                model,
                "--permission-mode",
                "bypassPermissions",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .env("CLAUDE_CODE_OAUTH_TOKEN", &self.config.oauth_token)
            .spawn()
            .context("spawn claude --print")?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes()).await.ok();
        }
        let out = child
            .wait_with_output()
            .await
            .context("wait claude --print")?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    // ── Notify + event broadcast ──────────────────────────────────────────

    pub fn notify(&self, chat_id: &str, message: &str) {
        if chat_id.is_empty() {
            return;
        }
        self.emit(PipelineEvent::Notify {
            chat_id: chat_id.to_string(),
            message: message.to_string(),
        });
    }

    fn emit(&self, event: PipelineEvent) {
        let _ = self.event_tx.send(event);
    }
}

fn issue_seed_marker(url: &str) -> String {
    format!("Source issue: {}", url.trim())
}

fn trim_issue_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "No issue body provided.".to_string();
    }
    const MAX_CHARS: usize = 2000;
    if trimmed.chars().count() <= MAX_CHARS {
        return trimmed.to_string();
    }
    let clipped: String = trimmed.chars().take(MAX_CHARS).collect();
    format!("{clipped}...")
}

// ── Private helpers ───────────────────────────────────────────────────────────

pub(crate) struct TestOutput {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) exit_code: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryClass {
    Resource,
    Transient,
    Conflict,
    Other,
}

fn container_result_as_test_output(
    results: &[ContainerTestResult],
    phase: &str,
) -> Option<TestOutput> {
    results
        .iter()
        .find(|r| r.phase == phase)
        .map(|r| TestOutput {
            stdout: r.output.clone(),
            stderr: String::new(),
            exit_code: r.exit_code,
        })
}

fn extract_blocks(text: &str, start_marker: &str, end_marker: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut remaining = text;
    while let Some(start) = remaining.find(start_marker) {
        remaining = &remaining[start + start_marker.len()..];
        if let Some(end) = remaining.find(end_marker) {
            blocks.push(remaining[..end].trim().to_string());
            remaining = &remaining[end + end_marker.len()..];
        } else {
            break;
        }
    }
    blocks
}

fn tmp_inode_usage_percent(path: &str) -> Option<f64> {
    tmp_health(path).map(|h| h.inode_used_pct)
}

#[derive(Debug, Clone, Copy)]
struct TmpHealth {
    inode_used_pct: f64,
    free_inodes: u64,
    free_bytes: u64,
}

fn tmp_health(path: &str) -> Option<TmpHealth> {
    let c_path = CString::new(path).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat as *mut libc::statvfs) };
    if rc != 0 || stat.f_files == 0 {
        return None;
    }
    let used = stat.f_files.saturating_sub(stat.f_ffree);
    let inode_used_pct = (used as f64) * 100.0 / (stat.f_files as f64);
    Some(TmpHealth {
        inode_used_pct,
        free_inodes: stat.f_ffree,
        free_bytes: stat.f_bavail.saturating_mul(stat.f_frsize),
    })
}

fn cleanup_tmp_prefixes(base: &str, prefixes: &[&str]) -> usize {
    let mut removed = 0usize;
    let Ok(entries) = std::fs::read_dir(base) else {
        return removed;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !prefixes.iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let path = entry.path();
        let res = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if res.is_ok() {
            removed += 1;
        }
    }
    removed
}

fn classify_retry_error(error: &str) -> RetryClass {
    let err = error.to_ascii_lowercase();
    if err.contains("no space left on device")
        || err.contains("failed to copy file")
        || err.contains("inode")
        || err.contains("cannot create temp")
        || err.contains("resource temporarily unavailable")
        || err.contains("too many open files")
    {
        return RetryClass::Resource;
    }
    if err.contains("could not resolve host")
        || err.contains("temporary failure in name resolution")
        || err.contains("network is unreachable")
        || err.contains("connection reset")
        || err.contains("timed out")
        || err.contains("timeout")
        || err.contains("rate limit")
        || err.contains("http 502")
        || err.contains("http 503")
    {
        return RetryClass::Transient;
    }
    if err.contains("merge conflict")
        || err.contains("behind main")
        || err.contains("not mergeable")
        || err.contains("could not apply")
        || err.contains("conflict")
    {
        return RetryClass::Conflict;
    }
    RetryClass::Other
}

#[derive(Debug, Clone)]
struct ComplianceFinding {
    check_id: String,
    severity: &'static str,
    issue: String,
    source_url: String,
    as_of: String,
}

#[derive(Debug, Clone, Default)]
struct LegalRetrievalTrace {
    enforced: bool,
    passed: bool,
    trigger_source: String,
    inventory_calls: usize,
    category_calls: usize,
    search_calls: usize,
    coverage_calls: usize,
    full_document_reads: usize,
    search_queries: Vec<String>,
    distinct_search_queries: Vec<String>,
    coverage_queries: Vec<String>,
    mcp_servers: Vec<serde_json::Value>,
    missing_steps: Vec<String>,
}

fn legal_retrieval_protocol_trigger(
    task: &Task,
    phase: &PhaseConfig,
    stats: &crate::db::ProjectFileStats,
) -> Option<&'static str> {
    if phase.name != "implement" {
        return None;
    }
    if !matches!(task.mode.as_str(), "lawborg" | "legal") {
        return None;
    }
    if task.project_id <= 0 || stats.text_files <= 0 {
        return None;
    }
    if task.requires_exhaustive_corpus_review {
        return Some("explicit");
    }

    let task_type = task.task_type.trim().to_ascii_lowercase();
    if matches!(
        task_type.as_str(),
        "contract_analysis"
            | "contract_review"
            | "nda_triage"
            | "nda"
            | "compliance"
            | "regulatory_analysis"
            | "vendor_check"
            | "clause_review"
    ) {
        return Some("heuristic_task_type");
    }

    let haystack =
        format!("{} {} {}", task.title, task.description, task.task_type).to_ascii_lowercase();
    [
        "review the legal documents",
        "review all documents",
        "review the documents in this repository",
        "reviewing project documents",
        "uploaded documents",
        "project documents",
        "document corpus",
        "clause extraction",
        "contract review",
        "contract analysis",
        "compliance audit",
        "due diligence",
        "all agreements",
        "across all documents",
    ]
    .iter()
    .find(|needle| haystack.contains(**needle))
    .map(|_| "heuristic_description")
}

fn inspect_legal_retrieval_trace(raw_stream: &str) -> LegalRetrievalTrace {
    let mut trace = LegalRetrievalTrace::default();
    let mut distinct_queries = HashSet::new();

    for line in raw_stream.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        if value.get("type").and_then(|v| v.as_str()) == Some("system")
            && value.get("subtype").and_then(|v| v.as_str()) == Some("init")
        {
            if let Some(servers) = value.get("mcp_servers").and_then(|v| v.as_array()) {
                trace.mcp_servers = servers.clone();
            }
            continue;
        }

        if value.get("type").and_then(|v| v.as_str()) != Some("assistant") {
            continue;
        }
        let Some(blocks) = value
            .get("message")
            .and_then(|v| v.get("content"))
            .and_then(|v| v.as_array())
        else {
            continue;
        };

        for block in blocks {
            if block.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
                continue;
            }
            let name = block
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let input = block
                .get("input")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));

            match normalize_tool_name(name) {
                "list_documents" => trace.inventory_calls += 1,
                "get_document_categories" => trace.category_calls += 1,
                "search_documents" => {
                    trace.search_calls += 1;
                    if let Some(query) = extract_trace_query(&input, &["query", "q"]) {
                        let normalized = normalize_trace_query(&query);
                        if distinct_queries.insert(normalized.clone()) {
                            trace.distinct_search_queries.push(query.clone());
                        }
                        trace.search_queries.push(query);
                    }
                },
                "check_coverage" => {
                    trace.coverage_calls += 1;
                    if let Some(query) = extract_trace_query(&input, &["query", "q"]) {
                        trace.coverage_queries.push(query);
                    }
                },
                "read_document" => trace.full_document_reads += 1,
                "Read" => {
                    if block
                        .get("input")
                        .and_then(|v| v.get("file_path"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.contains("/project_files/") || s.starts_with("project_files/"))
                        .unwrap_or(false)
                    {
                        trace.full_document_reads += 1;
                    }
                },
                "WebFetch" => classify_borg_webfetch(&input, &mut trace, &mut distinct_queries),
                _ => {},
            }
        }
    }

    trace
}

fn normalize_tool_name(name: &str) -> &str {
    name.rsplit("__")
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(name)
}

fn classify_borg_webfetch(
    input: &serde_json::Value,
    trace: &mut LegalRetrievalTrace,
    distinct_queries: &mut HashSet<String>,
) {
    let Some(url) = input.get("url").and_then(|v| v.as_str()) else {
        return;
    };
    if !url.contains("/api/borgsearch/") {
        return;
    }

    if url.contains("/api/borgsearch/query?") {
        trace.search_calls += 1;
        if let Some(query) = extract_query_param(url, "q") {
            let normalized = normalize_trace_query(&query);
            if distinct_queries.insert(normalized) {
                trace.distinct_search_queries.push(query.clone());
            }
            trace.search_queries.push(query);
        }
        return;
    }
    if url.contains("/api/borgsearch/files?") {
        trace.inventory_calls += 1;
        return;
    }
    if url.contains("/api/borgsearch/coverage?") {
        trace.coverage_calls += 1;
        if let Some(query) = extract_query_param(url, "q") {
            trace.coverage_queries.push(query);
        }
        return;
    }
    if url.contains("/api/borgsearch/facets?") {
        trace.category_calls += 1;
        return;
    }
    if url.contains("/api/borgsearch/file/") {
        trace.full_document_reads += 1;
    }
}

fn extract_trace_query(input: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| input.get(key))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn extract_query_param(url: &str, key: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k == key {
            let decoded = v.replace('+', " ");
            let trimmed = decoded.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn normalize_trace_query(query: &str) -> String {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn run_compliance_pack(profile: &str, text: &str) -> Vec<ComplianceFinding> {
    let as_of = chrono::Utc::now().format("%Y-%m-%d").to_string();
    if text.trim().is_empty() {
        return vec![ComplianceFinding {
            check_id: "output_present".into(),
            severity: "high",
            issue: "No prior phase output found to evaluate.".into(),
            source_url: "".into(),
            as_of,
        }];
    }

    let lower = text.to_lowercase();
    let mut findings = Vec::new();

    if !lower.contains("regulatory considerations") {
        findings.push(ComplianceFinding {
            check_id: "regulatory_section".into(),
            severity: "medium",
            issue: "Missing `Regulatory Considerations` section.".into(),
            source_url: "".into(),
            as_of: as_of.clone(),
        });
    }
    if !(lower.contains("as of ") || lower.contains("as-of")) {
        findings.push(ComplianceFinding {
            check_id: "as_of_date".into(),
            severity: "medium",
            issue: "Missing an explicit as-of date for regulatory statements.".into(),
            source_url: "".into(),
            as_of: as_of.clone(),
        });
    }
    if !(lower.contains("http://") || lower.contains("https://")) {
        findings.push(ComplianceFinding {
            check_id: "source_links".into(),
            severity: "high",
            issue: "Missing source URLs for regulatory references.".into(),
            source_url: "".into(),
            as_of: as_of.clone(),
        });
    }

    match profile {
        "uk_sra" => {
            if !(lower.contains("sra") || lower.contains("solicitors regulation authority")) {
                findings.push(ComplianceFinding {
                    check_id: "uk_sra_reference".into(),
                    severity: "high",
                    issue: "UK profile selected but no SRA reference found.".into(),
                    source_url: "https://www.sra.org.uk/solicitors/standards-regulations/".into(),
                    as_of: as_of.clone(),
                });
            }
        },
        "us_prof_resp" => {
            if !(lower.contains("model rule")
                || lower.contains("professional conduct")
                || lower.contains("state bar"))
            {
                findings.push(ComplianceFinding {
                    check_id: "us_model_rules_reference".into(),
                    severity: "high",
                    issue: "US profile selected but no Model Rules/state professional-conduct reference found.".into(),
                    source_url: "https://www.americanbar.org/groups/professional_responsibility/publications/model_rules_of_professional_conduct/".into(),
                    as_of: as_of.clone(),
                });
            }
        },
        _ => {
            findings.push(ComplianceFinding {
                check_id: "profile_supported".into(),
                severity: "high",
                issue: format!(
                    "Unknown compliance profile `{profile}` (supported: uk_sra, us_prof_resp)."
                ),
                source_url: "".into(),
                as_of,
            });
        },
    }

    findings
}

fn compliance_should_block(enforcement: &str, findings: &[ComplianceFinding]) -> bool {
    !findings.is_empty() && enforcement == "block"
}

fn extract_field(block: &str, field: &str) -> Option<String> {
    let mut lines = block.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(rest) = line.strip_prefix(field) {
            let mut parts = vec![rest.trim()];
            // Collect continuation lines until the next field (word: pattern)
            while let Some(&next) = lines.peek() {
                if looks_like_field_key(next) {
                    break;
                }
                let trimmed = next.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed);
                }
                lines.next();
            }
            let val: Vec<&str> = parts.into_iter().filter(|s| !s.is_empty()).collect();
            if !val.is_empty() {
                return Some(val.join("\n"));
            }
        }
    }
    None
}

fn parse_triage_item(
    item: &serde_json::Value,
) -> Option<(i64, i64, i64, i64, i64, i64, &str, bool)> {
    let get_i64 = |k: &str| item.get(k).and_then(|v| v.as_i64());
    let p_id = get_i64("id")?;
    let impact = get_i64("impact")?;
    let feasibility = get_i64("feasibility")?;
    let risk = get_i64("risk")?;
    let effort = get_i64("effort")?;
    let score = get_i64("score")?;
    let reasoning = item.get("reasoning").and_then(|v| v.as_str()).unwrap_or("");
    let should_dismiss = item
        .get("dismiss")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Some((
        p_id,
        impact,
        feasibility,
        risk,
        effort,
        score,
        reasoning,
        should_dismiss,
    ))
}

fn no_merge_guardrail_baseline(
    queued_count: i64,
    last_release_ts: i64,
    backlog_started_ts: i64,
    now: i64,
) -> (Option<i64>, i64) {
    if queued_count <= 0 {
        return (None, 0);
    }
    if last_release_ts > 0 {
        return (Some(last_release_ts), 0);
    }
    let started = if backlog_started_ts > 0 {
        backlog_started_ts
    } else {
        now
    };
    (Some(started), started)
}

/// Collect session directory paths under `sessions_dir` that are stale and
/// eligible for removal.
///
/// A directory named `task-{N}` is stale when:
/// - It is not in `skip_ids` (i.e. not currently in-flight), AND
/// - Its age (seconds since task creation, or since mtime if the task is not
///   in the DB) is >= `max_age_secs`.
///
/// Exposed as a free function so it can be unit-tested without a Pipeline.
pub fn collect_stale_session_dirs(
    sessions_dir: &str,
    now_secs: i64,
    max_age_secs: i64,
    skip_ids: &HashSet<i64>,
    task_created_at: impl Fn(i64) -> Option<i64>,
) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return vec![];
    };
    let mut stale = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let Some(id_str) = name_str.strip_prefix("task-") else {
            continue;
        };
        let Ok(task_id) = id_str.parse::<i64>() else {
            continue;
        };
        if skip_ids.contains(&task_id) {
            continue;
        }
        let age_secs = match task_created_at(task_id) {
            Some(created_at) => now_secs.saturating_sub(created_at),
            None => {
                // Orphaned dir: fall back to filesystem mtime
                entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| now_secs.saturating_sub(d.as_secs() as i64))
                    .unwrap_or(max_age_secs + 1) // unknown age → treat as stale
            },
        };
        if age_secs >= max_age_secs {
            stale.push(entry.path());
        }
    }
    stale
}

fn looks_like_field_key(line: &str) -> bool {
    let trimmed = line.trim();
    if let Some(colon) = trimmed.find(':') {
        let key = &trimmed[..colon];
        !key.is_empty()
            && !key.contains(' ')
            && key.chars().next().map_or(false, |c| c.is_alphabetic())
    } else {
        false
    }
}

#[cfg(test)]
mod seeding_toctou_tests {
    use std::{
        collections::HashSet,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    };

    use tokio::sync::Mutex;

    /// Replicates the fixed "check-and-set" logic so we can test it in
    /// isolation without constructing a full Pipeline.
    async fn try_activate_seeding(
        in_flight: &Mutex<HashSet<i64>>,
        seeding_active: &AtomicBool,
    ) -> bool {
        let guard = in_flight.lock().await;
        if guard.is_empty() {
            seeding_active
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
        } else {
            false
        }
    }

    #[tokio::test]
    async fn seeding_does_not_start_when_in_flight_is_nonempty() {
        let in_flight = Mutex::new(HashSet::from([42i64]));
        let seeding_active = AtomicBool::new(false);

        let activated = try_activate_seeding(&in_flight, &seeding_active).await;

        assert!(
            !activated,
            "should not activate seeding while tasks are in-flight"
        );
        assert!(
            !seeding_active.load(Ordering::Acquire),
            "seeding_active must stay false"
        );
    }

    #[tokio::test]
    async fn seeding_starts_when_in_flight_is_empty() {
        let in_flight = Mutex::new(HashSet::new());
        let seeding_active = AtomicBool::new(false);

        let activated = try_activate_seeding(&in_flight, &seeding_active).await;

        assert!(
            activated,
            "should activate seeding when no tasks are in-flight"
        );
        assert!(
            seeding_active.load(Ordering::Acquire),
            "seeding_active must be set to true"
        );
    }

    #[tokio::test]
    async fn seeding_does_not_double_start_when_already_active() {
        let in_flight = Mutex::new(HashSet::new());
        let seeding_active = AtomicBool::new(true); // already running

        let activated = try_activate_seeding(&in_flight, &seeding_active).await;

        assert!(!activated, "CAS must fail when seeding is already active");
        assert!(
            seeding_active.load(Ordering::Acquire),
            "seeding_active must remain true"
        );
    }

    /// Regression: the in_flight lock must be held during the CAS.
    /// Simulate the race: after acquiring the lock and confirming emptiness,
    /// a concurrent task insertion should not be possible before the CAS
    /// completes because we hold the same lock.
    #[tokio::test]
    async fn in_flight_lock_held_prevents_concurrent_insertion() {
        let in_flight = Arc::new(Mutex::new(HashSet::new()));
        let seeding_active = Arc::new(AtomicBool::new(false));

        // Spawn a task that holds the in_flight lock and tries to insert
        // while try_activate_seeding is in its critical section.
        let in_flight2 = Arc::clone(&in_flight);
        let seeding_active2 = Arc::clone(&seeding_active);

        // First: activate seeding (acquires + holds lock, does CAS, drops lock).
        let activated = try_activate_seeding(&in_flight, &seeding_active).await;
        assert!(activated);

        // Now insert a task into in_flight to simulate a concurrent dispatch.
        in_flight2.lock().await.insert(99);

        // seeding_active is already true; a second call must fail even though
        // in_flight is now non-empty (belt-and-suspenders).
        let activated2 = try_activate_seeding(&in_flight2, &seeding_active2).await;
        assert!(
            !activated2,
            "must not activate again while seeding is running"
        );
    }
}

#[cfg(test)]
mod legal_retrieval_protocol_tests {
    use chrono::Utc;

    use super::{inspect_legal_retrieval_trace, legal_retrieval_protocol_trigger};
    use crate::{
        db::ProjectFileStats,
        types::{PhaseConfig, Task},
    };

    fn sample_task(task_type: &str, title: &str, description: &str) -> Task {
        Task {
            id: 1,
            title: title.to_string(),
            description: description.to_string(),
            repo_path: String::new(),
            branch: String::new(),
            status: "implement".into(),
            attempt: 0,
            max_attempts: 3,
            last_error: String::new(),
            created_by: "test".into(),
            notify_chat: String::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            session_id: String::new(),
            mode: "lawborg".into(),
            backend: String::new(),
            workspace_id: 0,
            project_id: 42,
            task_type: task_type.to_string(),
            requires_exhaustive_corpus_review: false,
            started_at: None,
            completed_at: None,
            duration_secs: None,
            review_status: None,
            revision_count: 0,
            chat_thread: String::new(),
        }
    }

    #[test]
    fn inspects_mcp_tool_trace_for_exhaustive_protocol() {
        let raw = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"get_document_categories","input":{"project_id":42}}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"list_documents","input":{"project_id":42}}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"search_documents","input":{"query":"indemnification clause","project_id":42}}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"search_documents","input":{"query":"hold harmless obligation","project_id":42}}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"check_coverage","input":{"query":"indemnification clause","project_id":42}}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"read_document","input":{"file_id":7,"project_id":42}}]}}"#;

        let trace = inspect_legal_retrieval_trace(raw);

        assert_eq!(trace.category_calls, 1);
        assert_eq!(trace.inventory_calls, 1);
        assert_eq!(trace.search_calls, 2);
        assert_eq!(trace.coverage_calls, 1);
        assert_eq!(trace.full_document_reads, 1);
        assert_eq!(trace.distinct_search_queries.len(), 2);
    }

    #[test]
    fn inspects_webfetch_and_staged_reads_for_exhaustive_protocol() {
        let raw = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"WebFetch","input":{"url":"http://127.0.0.1:3131/api/borgsearch/files?project_id=42&limit=50"}}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"WebFetch","input":{"url":"http://127.0.0.1:3131/api/borgsearch/query?q=limitation%20of%20liability&project_id=42"}}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"WebFetch","input":{"url":"http://127.0.0.1:3131/api/borgsearch/query?q=consequential%20damages&project_id=42"}}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"WebFetch","input":{"url":"http://127.0.0.1:3131/api/borgsearch/coverage?q=limitation%20of%20liability&project_id=42"}}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"project_files/01-master-services-agreement.txt"}}]}}"#;

        let trace = inspect_legal_retrieval_trace(raw);

        assert_eq!(trace.inventory_calls, 1);
        assert_eq!(trace.search_calls, 2);
        assert_eq!(trace.coverage_calls, 1);
        assert_eq!(trace.full_document_reads, 1);
        assert_eq!(trace.distinct_search_queries.len(), 2);
        assert!(trace
            .search_queries
            .iter()
            .any(|q| q.contains("limitation%20of%20liability")));
    }

    #[test]
    fn only_enforces_for_exhaustive_legal_tasks_with_project_corpus() {
        let phase = PhaseConfig {
            name: "implement".into(),
            ..Default::default()
        };
        let stats = ProjectFileStats {
            project_id: 42,
            text_files: 5,
            ..Default::default()
        };
        let task = sample_task(
            "contract_review",
            "Review vendor agreement",
            "Review the legal documents in this repository for playbook deviations.",
        );
        assert_eq!(
            legal_retrieval_protocol_trigger(&task, &phase, &stats),
            Some("heuristic_task_type")
        );

        let non_exhaustive = sample_task(
            "",
            "Research Delaware implied covenant doctrine",
            "Find recent authorities on the implied covenant of good faith.",
        );
        assert_eq!(
            legal_retrieval_protocol_trigger(&non_exhaustive, &phase, &stats),
            None
        );
    }

    #[test]
    fn explicit_exhaustive_flag_overrides_heuristics() {
        let phase = PhaseConfig {
            name: "implement".into(),
            ..Default::default()
        };
        let stats = ProjectFileStats {
            project_id: 42,
            text_files: 5,
            ..Default::default()
        };
        let mut task = sample_task(
            "",
            "Research Delaware implied covenant doctrine",
            "Find recent authorities on the implied covenant of good faith.",
        );
        task.requires_exhaustive_corpus_review = true;

        assert_eq!(
            legal_retrieval_protocol_trigger(&task, &phase, &stats),
            Some("explicit")
        );
    }
}

#[cfg(test)]
mod phase_completion_verdict_tests {
    use std::fs;

    use chrono::Utc;
    use tempfile::tempdir;

    use super::Pipeline;
    use crate::types::{PhaseCompletionVerdict, PhaseConfig, Task};

    fn sample_task() -> Task {
        Task {
            id: 7,
            title: "Implement gate".into(),
            description: "Ensure the phase gate is robust.".into(),
            repo_path: String::new(),
            branch: String::new(),
            status: "implement".into(),
            attempt: 2,
            max_attempts: 5,
            last_error: String::new(),
            created_by: "test".into(),
            notify_chat: String::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            session_id: String::new(),
            mode: "sweborg".into(),
            backend: String::new(),
            workspace_id: 0,
            project_id: 0,
            task_type: String::new(),
            requires_exhaustive_corpus_review: false,
            started_at: None,
            completed_at: None,
            duration_secs: None,
            review_status: None,
            revision_count: 0,
            chat_thread: String::new(),
        }
    }

    fn sample_phase() -> PhaseConfig {
        PhaseConfig {
            name: "implement".into(),
            ..Default::default()
        }
    }

    #[test]
    fn reads_phase_completion_verdict_from_workdir() {
        let dir = tempdir().expect("tempdir");
        let borg_dir = dir.path().join(".borg");
        fs::create_dir_all(&borg_dir).expect("create .borg");
        fs::write(
            borg_dir.join("phase-verdict.json"),
            r#"{"task_id":7,"phase":"implement","attempt":2,"gate_token":"gate-123","ready_to_advance":true,"rationale":"checked request","missing_requirements":[]}"#,
        )
        .expect("write verdict");

        let verdict = Pipeline::read_phase_completion_verdict(dir.path().to_str().unwrap())
            .expect("verdict should parse");

        assert_eq!(verdict.task_id, 7);
        assert_eq!(verdict.phase, "implement");
        assert_eq!(verdict.attempt, 2);
        assert_eq!(verdict.gate_token, "gate-123");
        assert!(verdict.ready_to_advance);
        assert_eq!(verdict.rationale, "checked request");
        assert!(verdict.missing_requirements.is_empty());
        assert!(
            !borg_dir.join("phase-verdict.json").exists(),
            "verdict file should be consumed so stale approvals cannot be reused"
        );
    }

    #[test]
    fn invalid_phase_completion_verdict_returns_none() {
        let dir = tempdir().expect("tempdir");
        let borg_dir = dir.path().join(".borg");
        fs::create_dir_all(&borg_dir).expect("create .borg");
        fs::write(borg_dir.join("phase-verdict.json"), "{not-json").expect("write invalid verdict");

        let verdict = Pipeline::read_phase_completion_verdict(dir.path().to_str().unwrap());

        assert!(verdict.is_none(), "malformed verdict should be rejected");
        assert!(
            !borg_dir.join("phase-verdict.json").exists(),
            "invalid verdict file should be removed after evaluation"
        );
    }

    #[test]
    fn mismatched_phase_completion_verdict_is_rejected() {
        let task = sample_task();
        let phase = sample_phase();
        let verdict = PhaseCompletionVerdict {
            task_id: task.id,
            phase: phase.name.clone(),
            attempt: task.attempt,
            gate_token: "old-token".into(),
            ready_to_advance: true,
            rationale: "checked request".into(),
            missing_requirements: Vec::new(),
        };

        let err =
            Pipeline::validate_phase_completion_verdict(&verdict, &task, &phase, "fresh-token")
                .expect_err("stale gate token must be rejected");

        assert!(
            err.contains("gate token mismatch"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn positive_phase_completion_verdict_cannot_list_missing_requirements() {
        let task = sample_task();
        let phase = sample_phase();
        let verdict = PhaseCompletionVerdict {
            task_id: task.id,
            phase: phase.name.clone(),
            attempt: task.attempt,
            gate_token: "fresh-token".into(),
            ready_to_advance: true,
            rationale: "checked request".into(),
            missing_requirements: vec!["still need tests".into()],
        };

        let err =
            Pipeline::validate_phase_completion_verdict(&verdict, &task, &phase, "fresh-token")
                .expect_err("inconsistent positive verdict must be rejected");

        assert!(
            err.contains("missing_requirements"),
            "unexpected error: {err}"
        );
    }
}

#[cfg(test)]
mod guardrail_alert_tests {
    use super::no_merge_guardrail_baseline;

    #[test]
    fn queue_absence_clears_no_merge_baseline() {
        let (baseline, next_started) = no_merge_guardrail_baseline(0, 0, 123, 900);

        assert_eq!(baseline, None);
        assert_eq!(next_started, 0);
    }

    #[test]
    fn first_backlog_without_merge_starts_timer_now() {
        let (baseline, next_started) = no_merge_guardrail_baseline(5, 0, 0, 900);

        assert_eq!(baseline, Some(900));
        assert_eq!(next_started, 900);
    }

    #[test]
    fn existing_backlog_without_merge_preserves_first_seen_time() {
        let (baseline, next_started) = no_merge_guardrail_baseline(5, 0, 600, 900);

        assert_eq!(baseline, Some(600));
        assert_eq!(next_started, 600);
    }

    #[test]
    fn last_release_takes_precedence_and_clears_backlog_timer() {
        let (baseline, next_started) = no_merge_guardrail_baseline(5, 750, 600, 900);

        assert_eq!(baseline, Some(750));
        assert_eq!(next_started, 0);
    }
}
