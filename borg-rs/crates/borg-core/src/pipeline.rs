use std::{
    collections::{HashMap, HashSet},
    process::Command,
    sync::Arc,
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
    knowledge::EmbeddingClient,
    modes::get_mode,
    sandbox::{Sandbox, SandboxMode},
    stream::TaskStreamManager,
    types::{
        ContainerTestResult, IntegrationType, PhaseConfig, PhaseContext, PhaseHistoryEntry,
        PhaseOutput, PhaseType, PipelineMode, PipelineStateSnapshot, Proposal, RepoConfig,
        SeedOutputType, Task,
    },
};

/// Derive a compile-only check command from a test command, if possible.
/// For `cargo test` commands, returns the same command with `--no-run` appended.
pub fn derive_compile_check(test_cmd: &str) -> Option<String> {
    let trimmed = test_cmd.trim();
    if trimmed.contains("cargo test") {
        Some(format!("{trimmed} --no-run"))
    } else {
        None
    }
}

pub struct Pipeline {
    pub db: Arc<Db>,
    pub backends: HashMap<String, Arc<dyn AgentBackend>>,
    pub config: Arc<Config>,
    pub sandbox: Sandbox,
    pub sandbox_mode: SandboxMode,
    pub event_tx: broadcast::Sender<PipelineEvent>,
    pub stream_manager: Arc<TaskStreamManager>,
    pub force_restart: Arc<std::sync::atomic::AtomicBool>,
    /// Per-(repo_path, seed_name) last-run timestamp for independent per-seed cooldowns.
    seed_cooldowns: Mutex<HashMap<(String, String), i64>>,
    last_self_update_secs: std::sync::atomic::AtomicI64,
    last_cache_prune_secs: std::sync::atomic::AtomicI64,
    last_session_prune_secs: std::sync::atomic::AtomicI64,
    startup_heads: HashMap<String, String>,
    in_flight: Mutex<HashSet<i64>>,
    /// Per-task last agent dispatch timestamp (epoch seconds) for rate limiting.
    last_agent_dispatch: Mutex<HashMap<i64, i64>>,
    /// Prevents overlapping seed runs (seeding is spawned in background).
    seeding_active: std::sync::atomic::AtomicBool,
    /// Whether the borg-agent-net Docker bridge network was successfully created at startup.
    pub agent_network_available: bool,
    pub embed_client: EmbeddingClient,
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
    fn custom_modes_from_db(&self) -> Vec<PipelineMode> {
        let raw = match self.db.get_config("custom_modes") {
            Ok(Some(v)) => v,
            _ => return Vec::new(),
        };
        serde_json::from_str::<Vec<PipelineMode>>(&raw).unwrap_or_default()
    }

    fn resolve_mode(&self, name: &str) -> Option<PipelineMode> {
        get_mode(name)
            .or_else(|| {
                self.custom_modes_from_db()
                    .into_iter()
                    .find(|m| m.name == name)
            })
            .or_else(|| {
                warn!("resolve_mode: unknown mode {name:?}, falling back to sweborg");
                get_mode("sweborg")
            })
    }

    pub fn new(
        db: Arc<Db>,
        backends: HashMap<String, Arc<dyn AgentBackend>>,
        config: Arc<Config>,
        sandbox_mode: SandboxMode,
        force_restart: Arc<std::sync::atomic::AtomicBool>,
        agent_network_available: bool,
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
            sandbox: Sandbox,
            sandbox_mode,
            event_tx: tx,
            stream_manager: TaskStreamManager::new(),
            force_restart,
            seed_cooldowns: Mutex::new(seed_cooldowns),
            last_self_update_secs: std::sync::atomic::AtomicI64::new(0),
            last_cache_prune_secs: std::sync::atomic::AtomicI64::new(0),
            last_session_prune_secs: std::sync::atomic::AtomicI64::new(0),
            startup_heads,
            in_flight: Mutex::new(HashSet::new()),
            last_agent_dispatch: Mutex::new(HashMap::new()),
            seeding_active: std::sync::atomic::AtomicBool::new(false),
            agent_network_available,
            embed_client: EmbeddingClient::from_env(),
        };
        (p, rx)
    }

    // ── Backend resolution ────────────────────────────────────────────────

    /// Select the agent backend for a task: task override → repo override → global → any.
    fn resolve_backend(&self, task: &Task) -> Option<Arc<dyn AgentBackend>> {
        if !task.backend.is_empty() {
            if let Some(b) = self.backends.get(&task.backend) {
                return Some(Arc::clone(b));
            }
        }
        if let Some(repo) = self
            .config
            .watched_repos
            .iter()
            .find(|r| r.path == task.repo_path)
        {
            if !repo.backend.is_empty() {
                if let Some(b) = self.backends.get(&repo.backend) {
                    return Some(Arc::clone(b));
                }
            }
        }
        if let Some(b) = self.backends.get(&self.config.backend) {
            return Some(Arc::clone(b));
        }
        self.backends.values().next().map(Arc::clone)
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
        let key_owner = if task.created_by.is_empty() {
            "global"
        } else {
            &task.created_by
        };
        for provider in [
            "lexisnexis", "lexmachina", "intelligize", "westlaw",
            "clio", "imanage", "netdocuments", "congress", "openstates",
            "canlii", "regulations_gov", "shovels",
            "plaid_client_id", "plaid_secret", "plaid_env",
        ] {
            if let Ok(Some(key)) = self.db.get_api_key(key_owner, provider) {
                api_keys.insert(provider.to_string(), key);
            }
        }
        let disallowed_tools = self.db.get_config("pipeline_disallowed_tools")
            .ok().flatten().unwrap_or_default();
        let knowledge_files = self.db.list_knowledge_files().unwrap_or_default();
        let knowledge_dir = format!("{}/knowledge", self.config.data_dir);
        PhaseContext {
            task: task.clone(),
            repo_config: self.repo_config(task),
            data_dir: self.config.data_dir.clone(),
            session_dir,
            work_dir,
            oauth_token: self.config.oauth_token.clone(),
            model: self.config.model.clone(),
            pending_messages,
            system_prompt_suffix,
            user_coauthor,
            stream_tx: None,
            setup_script,
            api_keys,
            disallowed_tools,
            knowledge_files,
            knowledge_dir,
            agent_network: if self.agent_network_available {
                Some(Sandbox::AGENT_NETWORK.to_string())
            } else {
                None
            },
            prior_research: Vec::new(),
            revision_count: task.revision_count,
        }
    }

    /// Increment attempt and set the retry status, or fail if attempts exhausted.
    /// After 3 failed attempts, clears the session ID to force a fresh start and
    /// builds a summary of previous attempts so the new session has context.
    fn fail_or_retry(&self, task: &Task, retry_status: &str, error: &str) -> Result<()> {
        self.db.increment_attempt(task.id)?;
        let current = self.db.get_task(task.id)?.unwrap_or_else(|| {
            // Fallback: use stale snapshot but with incremented attempt so check is correct
            let mut t = task.clone();
            t.attempt += 1;
            t
        });
        if current.attempt >= current.max_attempts {
            self.db.update_task_status(task.id, "failed", Some(error))?;
        } else {
            // After 3 attempts, force a fresh session with a summary of what was tried
            let error_ctx = if current.attempt >= 3 {
                self.db.update_task_session(task.id, "").ok();
                info!(
                    "task #{} attempt {} — clearing session for fresh start",
                    task.id, current.attempt
                );
                self.build_retry_summary(task.id, error)
            } else {
                error.to_string()
            };
            self.db
                .update_task_status(task.id, retry_status, Some(&error_ctx))?;
        }
        Ok(())
    }

    /// Build a summary of previous failed attempts for fresh-session retries.
    fn build_retry_summary(&self, task_id: i64, current_error: &str) -> String {
        let outputs = self.db.get_task_outputs(task_id).unwrap_or_default();
        let mut summary = String::from("FRESH RETRY — previous approaches failed. Summary of attempts:\n");
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
                        if let Err(e) = self.db.enqueue(task.id, &branch, &task.repo_path, 0) {
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
            let mut guard = self.in_flight.lock().await;
            if guard.len() >= max_agents {
                break;
            }
            if guard.contains(&task.id) {
                continue;
            }
            guard.insert(task.id);
            drop(guard);

            dispatched += 1;
            let pipeline = Arc::clone(&self);
            let inner_pipeline = Arc::clone(&self);
            let task_id = task.id;
            let task_for_recovery = task.clone();
            tokio::spawn(async move {
                // Drop guard ensures in_flight slot is released even if this future is cancelled.
                struct InFlightGuard {
                    pipeline: Arc<Pipeline>,
                    task_id: i64,
                }
                impl Drop for InFlightGuard {
                    fn drop(&mut self) {
                        let pipeline = Arc::clone(&self.pipeline);
                        let task_id = self.task_id;
                        tokio::spawn(async move {
                            pipeline.in_flight.lock().await.remove(&task_id);
                        });
                    }
                }
                let _guard = InFlightGuard { pipeline: Arc::clone(&pipeline), task_id };

                let handle = tokio::spawn(async move {
                    Arc::clone(&inner_pipeline)
                        .process_task(task)
                        .await
                });
                match handle.await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => error!("process_task #{task_id} error: {e}"),
                    Err(join_err) => {
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
                    }
                }
            });
        }

        if dispatched == 0
            && self.in_flight.lock().await.is_empty()
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
        self.maybe_prune_cache_volumes().await;
        self.maybe_prune_session_dirs().await;

        // Check if main loop should exit for self-update restart
        if self
            .force_restart
            .load(std::sync::atomic::Ordering::Acquire)
        {
            info!("force_restart flag set — exiting for systemd restart");
            std::process::exit(0);
        }

        Ok(())
    }

    // ── Task dispatch ─────────────────────────────────────────────────────

    /// Process a single task through its current phase.
    async fn process_task(self: Arc<Self>, task: Task) -> Result<()> {
        let mode = self
            .resolve_mode(&task.mode)
            .ok_or_else(|| anyhow::anyhow!("no pipeline mode found for task #{}", task.id))?;

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
            PhaseType::HumanReview => {
                // Task sits in this status until a human acts via the API.
                // Do not dispatch to any backend — just return.
                return Ok(());
            }
        }

        // Async embedding indexing for completed tasks
        if phase.next == "done" && !task.repo_path.is_empty() {
            let db = Arc::clone(&self.db);
            let embed = &self.embed_client;
            let pid = if task.project_id > 0 { Some(task.project_id) } else { None };
            crate::knowledge::index_task_embeddings(&db, embed, task.id, pid, &task.repo_path).await;
        }

        // Async embedding indexing for completed tasks
        if phase.next == "done" && !task.repo_path.is_empty() {
            let db = Arc::clone(&self.db);
            let embed = &self.embed_client;
            let pid = if task.project_id > 0 { Some(task.project_id) } else { None };
            crate::knowledge::index_task_embeddings(&db, embed, task.id, pid, &task.repo_path).await;
        }

        // Async embedding indexing for completed tasks
        if phase.next == "done" && !task.repo_path.is_empty() {
            let db = Arc::clone(&self.db);
            let embed = &self.embed_client;
            let pid = if task.project_id > 0 { Some(task.project_id) } else { None };
            crate::knowledge::index_task_embeddings(&db, embed, task.id, pid, &task.repo_path).await;
        }

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

    /// Setup phase: record branch name and advance to first agent phase.
    /// In non-Docker mode, also creates the git branch so the agent works on it.
    async fn setup_branch(&self, task: &Task, mode: &PipelineMode) -> Result<()> {
        let next = mode
            .phases
            .iter()
            .find(|p| p.phase_type != PhaseType::Setup)
            .map(|p| p.name.as_str())
            .unwrap_or("spec");

        let branch = format!("task-{}", task.id);
        self.db.update_task_branch(task.id, &branch)?;

        self.db.update_task_status(task.id, next, None)?;

        self.emit(PipelineEvent::Phase {
            task_id: Some(task.id),
            message: format!("task #{} started branch {}", task.id, branch),
        });

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
        let session_dir = make_session_dir(&session_dir_rel).await?;

        let work_dir = session_dir.clone();

        let pending_messages = self
            .db
            .get_pending_task_messages(task.id)
            .unwrap_or_default()
            .into_iter()
            .map(|m| (m.role, m.content))
            .collect::<Vec<_>>();

        let mut ctx = self.make_context(task, work_dir.clone(), session_dir, pending_messages);
        let had_pending = !ctx.pending_messages.is_empty();
        let test_cmd = ctx.repo_config.test_cmd.clone();

        // Inject prior research from knowledge graph for lawborg tasks
        if task.mode == "lawborg" || task.mode == "legal" {
            let pid = if task.project_id > 0 { Some(task.project_id) } else { None };
            let query = format!("{} {}", task.title, task.description);
            let results = crate::knowledge::get_prior_research(
                &self.db, &self.embed_client, &query, pid, 5,
            ).await;
            ctx.prior_research = results.into_iter().map(|r| {
                format!("[{}] {}", r.file_path, r.chunk_text)
            }).collect();
        }

        // Wire live NDJSON stream for the dashboard LiveTerminal.
        let (stream_tx, mut stream_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        ctx.stream_tx = Some(stream_tx);
        self.stream_manager.start(task.id).await;
        let sm = Arc::clone(&self.stream_manager);
        let stream_task_id = task.id;
        tokio::spawn(async move {
            while let Some(line) = stream_rx.recv().await {
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
        if let Err(e) = self
            .write_pipeline_state_snapshot(task, &phase.name, &work_dir)
            .await
        {
            warn!("task #{}: write_pipeline_state_snapshot: {e}", task.id);
        }
        let result = backend
            .run_phase(task, phase, ctx)
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
        if had_pending {
            if let Err(e) = self.db.mark_messages_delivered(task.id, &phase.name) {
                warn!("task #{}: mark_messages_delivered: {e}", task.id);
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

        // Read agent signal from .borg/signal.json (if present).
        let signal = Self::read_agent_signal(&work_dir);
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
                self.fail_or_retry(
                    task,
                    &phase.name,
                    &format!("missing artifact: {artifact}"),
                )?;
                return Ok(());
            }
        }

        if phase.compile_check && !test_cmd.is_empty() {
            if let Some(check_cmd) = derive_compile_check(&test_cmd) {
                let out = if result.ran_in_docker {
                    container_result_as_test_output(
                        &result.container_test_results,
                        "compileCheck",
                    )
                } else {
                    match self.run_test_command(&work_dir, &check_cmd).await {
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

        if phase.runs_tests && mode.uses_test_cmd && !test_cmd.is_empty() {
            let out = if result.ran_in_docker {
                container_result_as_test_output(&result.container_test_results, "test")
            } else {
                match self.run_test_command(&work_dir, &test_cmd).await {
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

        self.advance_phase(task, phase, mode)?;
        Ok(())
    }

    /// Read `.borg/signal.json` from the work dir. Returns default (done) if missing or malformed.
    fn read_agent_signal(work_dir: &str) -> crate::types::AgentSignal {
        let path = format!("{work_dir}/.borg/signal.json");
        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                // Clean up the signal file so it doesn't carry over to next run
                std::fs::remove_file(&path).ok();
                serde_json::from_str(&contents).unwrap_or_default()
            },
            Err(_) => crate::types::AgentSignal::default(),
        }
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
                self.run_test_command(&work_dir, &check_cmd).await?
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
            match self.run_test_command(&work_dir, &test_cmd).await {
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

    /// Run a rebase phase — uses GitHub API to update the PR branch.
    async fn run_rebase_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        mode: &PipelineMode,
    ) -> Result<()> {
        self.run_rebase_phase_docker(task, phase, mode).await
    }

    /// Rebase: try GitHub update-branch API first; on conflict spawn a Docker agent.
    async fn run_rebase_phase_docker(&self, task: &Task, phase: &PhaseConfig, mode: &PipelineMode) -> Result<()> {
        let repo = self.repo_config(task);
        if repo.repo_slug.is_empty() {
            warn!("task #{} rebase: no repo_slug, skipping", task.id);
            self.advance_phase(task, phase, mode)?;
            return Ok(());
        }

        let branch = format!("task-{}", task.id);
        let slug = &repo.repo_slug;

        // Find the PR number for this branch
        let pr_num_out = self.gh(&[
            "pr", "view", &branch, "--repo", slug,
            "--json", "number", "--jq", ".number",
        ]).await;
        let pr_num = pr_num_out
            .ok()
            .filter(|o| o.exit_code == 0)
            .and_then(|o| o.stdout.trim().parse::<u64>().ok());

        if let Some(num) = pr_num {
            let update_out = self.gh(&[
                "api", "-X", "PUT",
                &format!("repos/{slug}/pulls/{num}/update-branch"),
            ]).await;
            match update_out {
                Ok(o) if o.exit_code == 0 => {
                    info!("task #{} rebase: update-branch succeeded", task.id);
                    self.advance_phase(task, phase, mode)?;
                    return Ok(());
                },
                Ok(o) => {
                    let err = o.stderr.trim().chars().take(300).collect::<String>();
                    warn!("task #{} rebase: update-branch failed, spawning agent: {err}", task.id);
                },
                Err(e) => {
                    warn!("task #{} rebase: update-branch error, spawning agent: {e}", task.id);
                },
            }
        } else {
            info!("task #{} rebase: no PR found, advancing", task.id);
            self.advance_phase(task, phase, mode)?;
            return Ok(());
        }

        // GitHub API couldn't auto-merge — spawn a Docker agent to resolve conflicts
        self.run_rebase_agent(task, phase, mode, &branch).await
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
            }
        };

        let result = backend
            .run_phase(task, &rebase_phase, ctx)
            .await
            .unwrap_or_else(|e| {
                error!("rebase agent for task #{}: {e}", task.id);
                PhaseOutput::failed(String::new())
            });

        if let Some(ref sid) = result.new_session_id {
            self.db.update_task_session(task.id, sid).ok();
        }

        self.db
            .insert_task_output(task.id, "rebase_fix", &result.output, &result.raw_stream, if result.success { 0 } else { 1 })
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
                    self.db.insert_task_output(
                        task.id,
                        "rebase_compile_fail",
                        &errors,
                        "",
                        1,
                    ).ok();
                    self.fail_or_retry(
                        task,
                        "rebase",
                        &format!("Compile failed after rebase:\n{errors}"),
                    )?;
                    return Ok(());
                }
            }
            info!("task #{} rebase: agent resolved conflicts", task.id);
            self.advance_phase(task, phase, mode)?;
        } else {
            warn!("task #{} rebase: agent failed to resolve conflicts", task.id);
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
        self.advance_phase(task, phase, mode)?;
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
        let session_dir_rel = format!("store/sessions/task-{}", task.id);
        let session_dir = std::fs::canonicalize(&session_dir_rel)
            .unwrap_or_else(|_| std::path::PathBuf::from(&session_dir_rel))
            .to_string_lossy()
            .to_string();

        let mut errors = initial_errors.to_string();

        for attempt in 0..2u32 {
            info!(
                "task #{} compile_fix: attempt {}",
                task.id,
                attempt + 1
            );

            let fix_phase = PhaseConfig {
                name: format!("compile_fix_{attempt}"),
                label: "Compile Fix".into(),
                system_prompt: "You are a compile-error fix agent. Fix compile errors with minimal changes.".into(),
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

            let ctx = self.make_context(task, work_dir.to_string(), session_dir.clone(), Vec::new());

            let result = match self.resolve_backend(task) {
                Some(b) => b.run_phase(task, &fix_phase, ctx).await.unwrap_or_else(|e| {
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

            match self.run_test_command(work_dir, check_cmd).await {
                Ok(ref out) if out.exit_code == 0 => {
                    info!("task #{} compile_fix: resolved after {} attempt(s)", task.id, attempt + 1);
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
        if next == "done" {
            self.read_structured_output(task);
            self.read_task_deadlines(task);
            self.index_task_documents(task);

            self.db.update_task_status(task.id, "done", Some(""))?;
            let _ = self.db.mark_task_completed(task.id);
            let pid = if task.project_id > 0 { Some(task.project_id) } else { None };
            let _ = self.db.log_event_full(Some(task.id), None, pid, "pipeline", "task.completed", &serde_json::json!({ "title": task.title }));
            match mode.integration {
                IntegrationType::GitPr => {
                    let branch = format!("task-{}", task.id);
                    if let Err(e) = self.db.enqueue(task.id, &branch, &task.repo_path, 0) {
                        warn!("enqueue for task #{}: {}", task.id, e);
                    } else {
                        info!("task #{} done, queued for integration", task.id);
                    }
                }
                IntegrationType::GitBranch => {
                    info!("task #{} done, branch preserved", task.id);
                }
                IntegrationType::None => {}
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

    fn read_structured_output(&self, task: &Task) {
        if task.repo_path.is_empty() { return; }
        let branch = format!("task-{}", task.id);
        let path = std::path::Path::new(&task.repo_path);
        if !path.join(".git").exists() { return; }
        let out = std::process::Command::new("git")
            .args(["-C", &task.repo_path, "show", &format!("{branch}:structured.json")])
            .stderr(std::process::Stdio::null())
            .output();
        if let Ok(output) = out {
            if output.status.success() {
                let data = String::from_utf8_lossy(&output.stdout);
                let trimmed = data.trim();
                if !trimmed.is_empty() {
                    if let Err(e) = self.db.update_task_structured_data(task.id, trimmed) {
                        tracing::warn!("task #{}: failed to save structured data: {e}", task.id);
                    } else {
                        tracing::info!("task #{}: saved structured output ({} bytes)", task.id, trimmed.len());
                    }
                }
            }
        }
    }

    fn read_task_deadlines(&self, task: &Task) {
        if task.repo_path.is_empty() || task.project_id == 0 { return; }
        let branch = format!("task-{}", task.id);
        let path = std::path::Path::new(&task.repo_path);
        if !path.join(".git").exists() { return; }
        let out = std::process::Command::new("git")
            .args(["-C", &task.repo_path, "show", &format!("{branch}:deadlines.json")])
            .stderr(std::process::Stdio::null())
            .output();
        if let Ok(output) = out {
            if output.status.success() {
                let data = String::from_utf8_lossy(&output.stdout);
                if let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(data.trim()) {
                    for item in items {
                        let label = item.get("label").and_then(|v| v.as_str()).unwrap_or("Deadline");
                        let due = item.get("due_date").or_else(|| item.get("date")).and_then(|v| v.as_str()).unwrap_or("");
                        let basis = item.get("rule_basis").and_then(|v| v.as_str()).unwrap_or("");
                        if due.is_empty() { continue; }
                        if let Err(e) = self.db.insert_deadline(task.project_id, label, due, basis) {
                            tracing::warn!("task #{}: failed to insert deadline: {e}", task.id);
                        }
                    }
                    tracing::info!("task #{}: imported deadlines from branch", task.id);
                }
            }
        }
    }

    fn index_task_documents(&self, task: &Task) {
        if task.repo_path.is_empty() || task.project_id == 0 { return; }
        let branch = format!("task-{}", task.id);
        let path = std::path::Path::new(&task.repo_path);
        if !path.join(".git").exists() { return; }
        // List .md files on the task branch
        let out = std::process::Command::new("git")
            .args(["-C", &task.repo_path, "ls-tree", "-r", "--name-only", &branch])
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
            if !file.ends_with(".md") { continue; }
            let show = std::process::Command::new("git")
                .args(["-C", &task.repo_path, "show", &format!("{branch}:{file}")])
                .stderr(std::process::Stdio::null())
                .output();
            if let Ok(o) = show {
                if o.status.success() {
                    let content = String::from_utf8_lossy(&o.stdout);
                    let title = content.lines().next().unwrap_or(file).trim_start_matches('#').trim();
                    if let Err(e) = self.db.fts_index_document(task.project_id, task.id, file, title, &content) {
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

    async fn run_test_command(&self, dir: &str, cmd: &str) -> Result<TestOutput> {
        let timeout = std::time::Duration::from_secs(self.config.agent_timeout_s.max(300) as u64);
        let output = tokio::time::timeout(
            timeout,
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .current_dir(dir)
                .output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("run_test_command timed out after {}s: {cmd}", timeout.as_secs()))?
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
        let clone_cmd = if std::path::Path::new(&host_mirror).exists() {
            format!(
                "git clone --depth 1 --single-branch --reference {container_mirror} {repo_url_q} /workspace/repo"
            )
        } else {
            format!(
                "git clone --depth 1 --single-branch {repo_url_q} /workspace/repo"
            )
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
            (format!("borg-cache-{repo_name}-target"), "/workspace/repo/target".to_string()),
            (format!("borg-cache-{repo_name}-cargo-registry"), "/home/bun/.cargo/registry".to_string()),
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
        .map_err(|_| anyhow::anyhow!("run_test_in_container timed out after {}s", timeout.as_secs()))?
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
        let timeout =
            std::time::Duration::from_secs(self.config.agent_timeout_s.max(300) as u64);
        let output = tokio::time::timeout(
            timeout,
            tokio::process::Command::new("gh").args(args).output(),
        )
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
                    if cmp
                        .map(|r| r.stdout.trim() == "identical")
                        .unwrap_or(false)
                    {
                        info!(
                            "Task #{} {}: identical to main, marking merged",
                            entry.task_id, entry.branch
                        );
                        self.db.update_queue_status(entry.id, "merged")?;
                        self.db.update_task_status(entry.task_id, "merged", None)?;
                        excluded_ids.insert(entry.id);
                        continue;
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
                    "number",
                    "--jq",
                    ".number",
                ])
                .await;
            let view_out = match view_out {
                Ok(o) => o,
                Err(e) => { warn!("gh pr view {}: {e}", entry.branch); continue; }
            };
            if view_out.exit_code == 0 && !view_out.stdout.trim().is_empty() {
                continue;
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
                Err(e) => { warn!("gh pr create {}: {e}", entry.branch); continue; }
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
            let candidate = live.iter().find(|e| {
                !excluded_ids.contains(&e.id) && !freshly_created.contains(&e.id)
            });

            if let Some(entry) = candidate {
                // Check if PR is already merged (picked up from a prior run)
                let state_check = self
                    .gh(&[
                        "pr", "view", &entry.branch, "--repo", slug,
                        "--json", "state", "--jq", ".state",
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
                            &format!(
                                "repos/{slug}/compare/main...{}",
                                entry.branch
                            ),
                            "--jq", ".behind_by",
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
                            .gh(&[
                                "pr", "merge", &entry.branch, "--repo", slug, "--merge",
                            ])
                            .await;

                        match merge_out {
                            Err(e) => {
                                warn!("gh pr merge {}: {e}", entry.branch);
                                self.db.update_queue_status(entry.id, "queued")?;
                            }
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
                                    self.db.update_task_status(
                                        entry.task_id,
                                        "rebase",
                                        None,
                                    )?;
                                    info!(
                                        "Task #{} has conflicts, sent to rebase",
                                        entry.task_id
                                    );
                                } else {
                                    self.db.update_queue_status(entry.id, "queued")?;
                                }
                            }
                            Ok(_) => {
                                self.db.update_queue_status(entry.id, "merged")?;
                                self.db.update_task_status(
                                    entry.task_id,
                                    "merged",
                                    None,
                                )?;
                                merged_branches.push(entry.branch.clone());
                                let _ = self
                                    .gh(&[
                                        "api",
                                        "-X",
                                        "DELETE",
                                        &format!(
                                            "repos/{slug}/git/refs/heads/{}",
                                            entry.branch
                                        ),
                                    ])
                                    .await;
                                if let Ok(Some(task)) = self.db.get_task(entry.task_id) {
                                    self.notify(
                                        &task.notify_chat,
                                        &format!(
                                            "Task #{} \"{}\" merged via PR.",
                                            task.id, task.title
                                        ),
                                    );
                                }
                            }
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

        let mode_name = self
            .resolve_mode(&repo.mode)
            .map(|m| m.name)
            .unwrap_or_else(|| "sweborg".to_string());

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
                session_id: String::new(),
                mode: mode_name.clone(),
                backend: String::new(),
                project_id: 0,
                task_type: String::new(),
                started_at: None,
                completed_at: None,
                duration_secs: None,
                review_status: None,
                revision_count: 0,
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
            session_id: String::new(),
            mode: mode_name.to_string(),
            backend: String::new(),
                project_id: 0,
                task_type: String::new(),
                started_at: None,
                completed_at: None,
                duration_secs: None,
                review_status: None,
                revision_count: 0,
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
        let result = backend.run_phase(&task, &phase, ctx).await?;

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
                        session_id: String::new(),
                        mode: mode_name.to_string(),
                        backend: String::new(),
                project_id: 0,
                task_type: String::new(),
                started_at: None,
                completed_at: None,
                duration_secs: None,
                review_status: None,
                revision_count: 0,
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

    // ── Health monitoring ─────────────────────────────────────────────────

    pub async fn check_health(&self) -> Result<()> {
        // In Docker mode, repos are not checked out on the host; skip host-side health checks.
        if self.sandbox_mode == SandboxMode::Docker {
            return Ok(());
        }

        const HEALTH_INTERVAL_S: i64 = 1800;
        let now = chrono::Utc::now().timestamp();
        if now - self.db.get_ts("last_health_ts") < HEALTH_INTERVAL_S {
            return Ok(());
        }
        self.db.set_ts("last_health_ts", now);

        for repo in &self.config.watched_repos {
            if !repo.is_self {
                continue;
            }
            match self.run_test_command(&repo.path, &repo.test_cmd).await {
                Ok(out) if out.exit_code != 0 => {
                    warn!("Health: tests failed for {}", repo.path);
                    self.create_health_task(&repo.path, "tests", &out.stderr)
                        .await;
                },
                Ok(_) => info!("Health: {} OK", repo.path),
                Err(e) => warn!("Health: test command error for {}: {e}", repo.path),
            }
        }
        Ok(())
    }

    async fn create_health_task(&self, repo_path: &str, kind: &str, stderr: &str) {
        if let Ok(active) = self.db.list_active_tasks() {
            if active
                .iter()
                .any(|t| t.title.starts_with("Fix failing ") && t.repo_path == repo_path)
            {
                return;
            }
        }
        let tail = if stderr.len() > 500 {
            &stderr[stderr.floor_char_boundary(stderr.len() - 500)..]
        } else {
            stderr
        };
        let title = format!("Fix failing {kind} on main");
        let desc = format!("Health check detected {kind} failure on main branch.\n\nError output:\n```\n{tail}\n```");
        let task = crate::types::Task {
            id: 0,
            title: title.clone(),
            description: desc,
            repo_path: repo_path.to_string(),
            branch: String::new(),
            status: "backlog".into(),
            attempt: 0,
            max_attempts: 5,
            last_error: String::new(),
            created_by: "health-check".into(),
            notify_chat: String::new(),
            created_at: chrono::Utc::now(),
            session_id: String::new(),
            mode: "sweborg".into(),
            backend: String::new(),
                project_id: 0,
                task_type: String::new(),
                started_at: None,
                completed_at: None,
                duration_secs: None,
                review_status: None,
                revision_count: 0,
        };
        match self.db.insert_task(&task) {
            Ok(id) => {
                info!("Health: created fix task #{id} for {repo_path} {kind} failure");
                self.notify(
                    &self.config.pipeline_admin_chat,
                    &format!(
                        "Health check: {kind} failing for {repo_path}, created fix task #{id}"
                    ),
                );
            },
            Err(e) => warn!("Health: insert_task: {e}"),
        }
    }

    // ── Self-update ───────────────────────────────────────────────────────

    pub async fn check_remote_updates(&self) {
        if !self.config.self_update_enabled {
            return;
        }
        let now = chrono::Utc::now().timestamp();
        if now - self.db.get_ts("last_remote_check_ts") < self.config.remote_check_interval_s {
            return;
        }
        self.db.set_ts("last_remote_check_ts", now);

        for repo in &self.config.watched_repos {
            if !repo.is_self {
                continue;
            }
            if !std::path::Path::new(&repo.path).exists() {
                continue;
            }

            let repo_name = std::path::Path::new(&repo.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if repo_name.is_empty() {
                continue;
            }

            // Compare the clone's HEAD against the bare mirror (kept fresh by refresh_mirrors).
            // Falls back to fetching origin directly if the mirror doesn't exist yet.
            let mirror_path = format!("{}/mirrors/{}.git", self.config.data_dir, repo_name);
            let remote = if std::path::Path::new(&mirror_path).exists() {
                tokio::process::Command::new("git")
                    .args(["-C", &mirror_path, "rev-parse", "HEAD"])
                    .output()
                    .await
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                let git = Git::new(&repo.path);
                if git.fetch_origin().is_err() {
                    return;
                }
                git.rev_parse("origin/main").ok()
            };

            let Some(remote) = remote else { return };

            let local = match Git::new(&repo.path).rev_parse_head() {
                Ok(h) => h,
                Err(_) => return,
            };

            if local == remote {
                return;
            }

            info!(
                "Remote update detected on {} (local={}, remote={})",
                repo.path,
                &local[..8.min(local.len())],
                &remote[..8.min(remote.len())]
            );
            self.check_self_update(&repo.path).await;
            return;
        }
    }

    async fn check_self_update(&self, repo_path: &str) {
        // Prevent concurrent self-updates via a pid lock file (atomic create_new).
        let lock_path = format!("{}/self-update.lock", self.config.data_dir);
        let lock_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path);
        match lock_file {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                warn!("Self-update: lock file exists, skipping");
                return;
            }
            Err(e) => {
                warn!("Self-update: could not create lock file: {e}");
                return;
            }
            Ok(mut f) => {
                use std::io::Write;
                let _ = f.write_all(std::process::id().to_string().as_bytes());
            }
        }
        struct LockGuard(String);
        impl Drop for LockGuard {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _guard = LockGuard(lock_path);

        // Only pull if the working tree is clean — never stash user's uncommitted work.
        let status_out = tokio::process::Command::new("git")
            .args(["-C", repo_path, "status", "--porcelain"])
            .output()
            .await;
        let is_dirty = status_out
            .as_ref()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(true);
        if is_dirty {
            info!("Self-update: working tree has local changes, skipping pull");
            return;
        }

        let pull_out = tokio::process::Command::new("git")
            .args(["-C", repo_path, "pull", "--ff-only", "origin", "main"])
            .output()
            .await;

        let pulled = match pull_out {
            Ok(o) if o.status.success() => Git::new(repo_path).rev_parse_head().ok(),
            Ok(o) => {
                warn!(
                    "Self-update: git pull failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                );
                return;
            },
            Err(e) => {
                warn!("Self-update: git pull spawn failed: {e}");
                return;
            },
        };

        let startup = match self.startup_heads.get(repo_path) {
            Some(h) => h,
            None => return,
        };
        let current = pulled.as_deref().unwrap_or("");
        if current == startup.as_str() {
            return;
        }

        info!(
            "Self-update: HEAD at {}, rebuilding...",
            &current[..8.min(current.len())]
        );
        self.notify(
            &self.config.pipeline_admin_chat,
            "Self-update: new commits detected, rebuilding...",
        );

        let build_cmd = self
            .db
            .get_config("build_cmd")
            .ok()
            .flatten()
            .unwrap_or_else(|| self.config.build_cmd.clone());

        let binary_path = format!("{}/target/release/borg-server", repo_path);
        let mtime_before = std::fs::metadata(&binary_path)
            .and_then(|m| m.modified())
            .ok();

        let out = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&build_cmd)
            .current_dir(repo_path)
            .output()
            .await;

        match out {
            Err(e) => {
                warn!("Self-update: build spawn failed: {e}");
                self.notify(
                    &self.config.pipeline_admin_chat,
                    "Self-update: build FAILED (spawn error).",
                );
            },
            Ok(o) if !o.status.success() => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                warn!(
                    "Self-update: build failed: {}",
                    &stderr[..stderr.len().min(500)]
                );
                self.notify(
                    &self.config.pipeline_admin_chat,
                    "Self-update: build FAILED, continuing with old binary.",
                );
            },
            Ok(_) => {
                let mtime_after = std::fs::metadata(&binary_path)
                    .and_then(|m| m.modified())
                    .ok();
                if mtime_after.is_none() {
                    warn!("Self-update: binary not found at {binary_path} after build");
                    self.notify(
                        &self.config.pipeline_admin_chat,
                        "Self-update: build succeeded but binary missing — not restarting.",
                    );
                    return;
                }
                if mtime_after == mtime_before {
                    warn!("Self-update: binary mtime unchanged after build");
                }
                info!("Self-update: build succeeded, restart scheduled");
                self.notify(
                    &self.config.pipeline_admin_chat,
                    "Self-update: new build ready. Will restart in 3h or on director command.",
                );
                self.last_self_update_secs.store(
                    chrono::Utc::now().timestamp(),
                    std::sync::atomic::Ordering::Relaxed,
                );
            },
        }
    }

    pub fn maybe_apply_self_update(&self) {
        let ts = self
            .last_self_update_secs
            .load(std::sync::atomic::Ordering::Relaxed);
        if ts == 0 {
            return;
        }
        let forced = self
            .force_restart
            .load(std::sync::atomic::Ordering::Acquire);
        let age = chrono::Utc::now().timestamp() - ts;
        if !forced && age < 3 * 3600 {
            return;
        }
        info!("Self-update: applying restart (forced={forced}, age={age}s)");
        self.notify(
            &self.config.pipeline_admin_chat,
            "Self-update: restarting now...",
        );
        // Signal main loop to exit; systemd restarts the process
        self.force_restart
            .store(true, std::sync::atomic::Ordering::Release);
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
                        Ok(o) if o.status.success() =>
                            info!("mirrored {path} → {mirror}"),
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
            let repo_cfg = self.config.watched_repos.iter().find(|r| r.path == p.repo_path);
            // Only auto-promote for repos that allow auto-merge
            if let Some(repo) = repo_cfg {
                if !repo.auto_merge {
                    continue;
                }
            }
            let mode = repo_cfg
                .map(|r| r.mode.as_str())
                .unwrap_or("sweborg");
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
                session_id: String::new(),
                mode: mode.to_string(),
                backend: String::new(),
                project_id: 0,
                task_type: String::new(),
                started_at: None,
                completed_at: None,
                duration_secs: None,
                review_status: None,
                revision_count: 0,
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
        let last = self.last_cache_prune_secs.load(std::sync::atomic::Ordering::Relaxed);
        if now - last < PRUNE_INTERVAL_S {
            return;
        }
        self.last_cache_prune_secs.store(now, std::sync::atomic::Ordering::Relaxed);
        Sandbox::prune_stale_cache_volumes(7).await;
    }

    async fn maybe_prune_session_dirs(&self) {
        const PRUNE_INTERVAL_S: i64 = 3600;
        let now = chrono::Utc::now().timestamp();
        let last = self.last_session_prune_secs.load(std::sync::atomic::Ordering::Relaxed);
        if now - last < PRUNE_INTERVAL_S {
            return;
        }
        self.last_session_prune_secs.store(now, std::sync::atomic::Ordering::Relaxed);

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

struct TestOutput {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

fn container_result_as_test_output(
    results: &[ContainerTestResult],
    phase: &str,
) -> Option<TestOutput> {
    results.iter().find(|r| r.phase == phase).map(|r| TestOutput {
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

fn parse_triage_item(item: &serde_json::Value) -> Option<(i64, i64, i64, i64, i64, i64, &str, bool)> {
    let get_i64 = |k: &str| item.get(k).and_then(|v| v.as_i64());
    let p_id = get_i64("id")?;
    let impact = get_i64("impact")?;
    let feasibility = get_i64("feasibility")?;
    let risk = get_i64("risk")?;
    let effort = get_i64("effort")?;
    let score = get_i64("score")?;
    let reasoning = item.get("reasoning").and_then(|v| v.as_str()).unwrap_or("");
    let should_dismiss = item.get("dismiss").and_then(|v| v.as_bool()).unwrap_or(false);
    Some((p_id, impact, feasibility, risk, effort, score, reasoning, should_dismiss))
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
            }
        };
        if age_secs >= max_age_secs {
            stale.push(entry.path());
        }
    }
    stale
}

async fn make_session_dir(rel: &str) -> Result<String> {
    tokio::fs::create_dir_all(rel)
        .await
        .with_context(|| format!("failed to create session dir {rel}"))?;
    let abs = std::fs::canonicalize(rel).unwrap_or_else(|_| std::path::PathBuf::from(rel));
    Ok(abs.to_string_lossy().to_string())
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
mod tests {
    use super::*;

    #[test]
    fn test_build_system_prompt_suffix_no_coauthor_no_user() {
        let s = Pipeline::build_system_prompt_suffix(false, "");
        assert!(s.contains("Do not add Co-Authored-By trailers to commit messages."));
    }

    #[test]
    fn test_build_system_prompt_suffix_claude_coauthor_no_user() {
        let s = Pipeline::build_system_prompt_suffix(true, "");
        assert!(s.is_empty());
    }

    #[test]
    fn test_build_system_prompt_suffix_no_coauthor_with_user() {
        let s = Pipeline::build_system_prompt_suffix(false, "User Name <u@e.com>");
        assert!(s.contains("Do not add Co-Authored-By trailers to commit messages."));
        assert!(s.contains("Git author is configured via environment variables"));
    }

    #[test]
    fn test_build_system_prompt_suffix_claude_coauthor_with_user() {
        let s = Pipeline::build_system_prompt_suffix(true, "User Name <u@e.com>");
        assert!(!s.contains("Do not add Co-Authored-By trailers"));
        assert!(s.contains("Git author is configured via environment variables"));
    }

    #[test]
    fn test_with_user_coauthor_empty_returns_message_unchanged() {
        let msg = "fix: some bug";
        assert_eq!(Pipeline::with_user_coauthor(msg, ""), msg);
    }

    #[test]
    fn test_with_user_coauthor_appends_trailer() {
        let result = Pipeline::with_user_coauthor("fix: some bug", "Name <email>");
        assert_eq!(result, "fix: some bug\n\nCo-Authored-By: Name <email>");
    }

    #[test]
    fn derive_compile_check_cargo_test_appends_no_run() {
        let result = derive_compile_check("cargo test");
        assert_eq!(result, Some("cargo test --no-run".to_string()));
    }

    #[test]
    fn derive_compile_check_just_test_returns_none() {
        assert_eq!(derive_compile_check("just test"), None);
    }

    #[test]
    fn derive_compile_check_bun_test_returns_none() {
        assert_eq!(derive_compile_check("bun test"), None);
    }

    #[test]
    fn derive_compile_check_cargo_test_release_preserves_full_command() {
        let result = derive_compile_check("cargo test --release");
        assert_eq!(result, Some("cargo test --release --no-run".to_string()));
    }

    #[test]
    fn derive_compile_check_trims_whitespace() {
        let result = derive_compile_check("  cargo test  ");
        assert_eq!(result, Some("cargo test --no-run".to_string()));
    }

    #[tokio::test]
    async fn test_make_session_dir_creates_dir_and_returns_absolute_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let sub = tmp.path().join("task-999");
        let result = make_session_dir(sub.to_str().expect("utf8")).await;
        assert!(result.is_ok(), "should succeed for valid path: {result:?}");
        let path = result.expect("ok");
        assert!(std::path::Path::new(&path).is_absolute(), "path must be absolute");
        assert!(std::path::Path::new(&path).is_dir(), "directory must exist after creation");
    }

    #[tokio::test]
    async fn test_make_session_dir_propagates_error_on_invalid_parent() {
        // /dev/null is a character device, not a directory; subdirs cannot be created under it.
        let result = make_session_dir("/dev/null/borg_session_test").await;
        assert!(result.is_err(), "must return Err when directory cannot be created");
        let msg = format!("{:#}", result.expect_err("is err"));
        assert!(msg.contains("session dir"), "error message must name the operation");
    }
}
