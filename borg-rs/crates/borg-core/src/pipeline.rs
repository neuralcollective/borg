use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{broadcast, Mutex};
use tracing::{error, info, warn};

use chrono::Utc;

pub use crate::types::PipelineEvent;

use crate::{
    agent::AgentBackend,
    config::Config,
    db::Db,
    git::Git,
    modes::get_mode,
    sandbox::Sandbox,
    stream::TaskStreamManager,
    types::{IntegrationType, PhaseConfig, PhaseContext, PhaseHistoryEntry, PhaseOutput, PhaseType, PipelineMode, PipelineStateSnapshot, Proposal, RepoConfig, SeedOutputType, Task},
};

pub struct Pipeline {
    pub db: Arc<Db>,
    pub backends: HashMap<String, Arc<dyn AgentBackend>>,
    pub config: Arc<Config>,
    pub sandbox: Sandbox,
    pub event_tx: broadcast::Sender<PipelineEvent>,
    pub stream_manager: Arc<TaskStreamManager>,
    pub force_restart: Arc<std::sync::atomic::AtomicBool>,
    /// Per-(repo_path, seed_name) last-run timestamp for independent per-seed cooldowns.
    seed_cooldowns: Mutex<HashMap<(String, String), i64>>,
    last_self_update_secs: std::sync::atomic::AtomicI64,
    startup_heads: HashMap<String, String>,
    in_flight: Mutex<HashSet<i64>>,
    /// Serializes git worktree creation to avoid .git/config lock contention.
    worktree_create_lock: Mutex<()>,
}

impl Pipeline {
    pub fn new(
        db: Arc<Db>,
        backends: HashMap<String, Arc<dyn AgentBackend>>,
        config: Arc<Config>,
        force_restart: Arc<std::sync::atomic::AtomicBool>,
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
        let p = Self {
            db,
            backends,
            config,
            sandbox: Sandbox,
            event_tx: tx,
            stream_manager: TaskStreamManager::new(),
            force_restart,
            seed_cooldowns: Mutex::new(HashMap::new()),
            last_self_update_secs: std::sync::atomic::AtomicI64::new(0),
            startup_heads,
            in_flight: Mutex::new(HashSet::new()),
            worktree_create_lock: Mutex::new(()),
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
        if let Some(repo) = self.config.watched_repos.iter().find(|r| r.path == task.repo_path) {
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
            })
    }

    /// Build a PhaseContext for a task phase.
    fn make_context(
        &self,
        task: &Task,
        worktree_path: String,
        session_dir: String,
        pending_messages: Vec<(String, String)>,
    ) -> PhaseContext {
        let (claude_coauthor, user_coauthor) = self.git_coauthor_settings();
        let system_prompt_suffix = Self::build_system_prompt_suffix(claude_coauthor, &user_coauthor);
        let setup_script = if self.config.container_setup.is_empty() {
            String::new()
        } else {
            std::fs::canonicalize(&self.config.container_setup)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| self.config.container_setup.clone())
        };
        PhaseContext {
            task: task.clone(),
            repo_config: self.repo_config(task),
            session_dir,
            worktree_path,
            oauth_token: self.config.oauth_token.clone(),
            model: self.config.model.clone(),
            pending_messages,
            system_prompt_suffix,
            user_coauthor,
            stream_tx: None,
            setup_script,
        }
    }

    /// Increment attempt and set the retry status, or fail if attempts exhausted.
    fn fail_or_retry(&self, task: &Task, retry_status: &str, error: &str) -> Result<()> {
        self.db.increment_attempt(task.id)?;
        let current = self.db.get_task(task.id)?.unwrap_or_else(|| task.clone());
        if current.attempt >= current.max_attempts {
            self.db.update_task_status(task.id, "failed", Some(error))?;
            self.cleanup_worktree(task);
        } else {
            self.db.update_task_status(task.id, retry_status, Some(error))?;
        }
        Ok(())
    }

    /// Remove the git worktree for a task (best-effort, silent on error).
    fn cleanup_worktree(&self, task: &Task) {
        let wt_path = format!("{}/.worktrees/task-{}", task.repo_path, task.id);
        let git = Git::new(&task.repo_path);
        let _ = git.remove_worktree(&wt_path);
        std::fs::remove_dir_all(&wt_path).ok();
        let _ = git.exec(&task.repo_path, &["worktree", "prune"]);
        info!("cleaned up worktree {} for task #{}", wt_path, task.id);
    }

    /// Git author pair from config, or None if not configured.
    fn git_author(&self) -> Option<(&str, &str)> {
        if self.config.git_author_name.is_empty() {
            None
        } else {
            Some((self.config.git_author_name.as_str(), self.config.git_author_email.as_str()))
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
                if let Some(mode) = get_mode(&task.mode).or_else(|| get_mode("sweborg")) {
                    if mode.integration == IntegrationType::GitPr {
                        let branch = format!("task-{}", task.id);
                        if let Err(e) = self.db.enqueue(task.id, &branch, &task.repo_path, 0) {
                            warn!("re-enqueue orphaned done task #{}: {e}", task.id);
                        } else {
                            info!("re-enqueued orphaned done task #{}: {}", task.id, task.title);
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
            let task_id = task.id;
            tokio::spawn(async move {
                if let Err(e) = Arc::clone(&pipeline).process_task(task).await {
                    error!("process_task #{task_id} error: {e}");
                }
                pipeline.in_flight.lock().await.remove(&task_id);
            });
        }

        if dispatched == 0 && self.in_flight.lock().await.is_empty() {
            self.seed_if_idle().await?;
        }

        // Periodic background work (each is internally throttled)
        self.clone().check_integration().await.unwrap_or_else(|e| warn!("check_integration: {e}"));
        self.maybe_auto_promote_proposals();
        self.maybe_auto_triage().await;
        self.check_health().await.unwrap_or_else(|e| warn!("check_health: {e}"));
        self.check_remote_updates().await;
        self.maybe_apply_self_update();

        // Check if main loop should exit for self-update restart
        if self.force_restart.load(std::sync::atomic::Ordering::Acquire) {
            info!("force_restart flag set — exiting for systemd restart");
            std::process::exit(0);
        }

        Ok(())
    }

    // ── Task dispatch ─────────────────────────────────────────────────────

    /// Process a single task through its current phase.
    async fn process_task(self: Arc<Self>, task: Task) -> Result<()> {
        let mode = get_mode(&task.mode)
            .or_else(|| get_mode("sweborg"))
            .ok_or_else(|| anyhow::anyhow!("no pipeline mode found for task #{}", task.id))?;

        let phase = match mode.get_phase(&task.status) {
            Some(p) => p.clone(),
            None => {
                error!(
                    "task #{} has unknown phase '{}' for mode '{}'",
                    task.id, task.status, mode.name
                );
                return Ok(());
            }
        };

        info!(
            "pipeline dispatching task #{} [{}] in {}: {}",
            task.id, task.status, task.repo_path, task.title
        );

        match phase.phase_type {
            PhaseType::Setup => self.setup_branch(&task, &mode).await?,
            PhaseType::Agent => self.run_agent_phase(&task, &phase, &mode).await?,
            PhaseType::Rebase => self.run_rebase_phase(&task, &phase, &mode).await?,
            PhaseType::LintFix => self.run_lint_fix_phase(&task, &phase, &mode).await?,
        }

        Ok(())
    }

    /// Read git co-author settings from DB (runtime-editable), falling back to Config.
    fn git_coauthor_settings(&self) -> (bool, String) {
        let claude_coauthor = self.db.get_config("git_claude_coauthor")
            .ok().flatten()
            .map(|v| v == "true")
            .unwrap_or(self.config.git_claude_coauthor);
        let user_coauthor = self.db.get_config("git_user_coauthor")
            .ok().flatten()
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
            if !s.is_empty() { s.push(' '); }
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

    /// Setup phase: create git worktree and advance to first agent phase.
    async fn setup_branch(&self, task: &Task, mode: &PipelineMode) -> Result<()> {
        if !mode.uses_git_worktrees {
            let next = mode
                .phases
                .get(1)
                .map(|p| p.name.as_str())
                .unwrap_or("spec");
            self.db.update_task_status(task.id, next, None)?;
            return Ok(());
        }

        let git = Git::new(&task.repo_path);
        let _ = git.fetch_origin();

        let branch = format!("task-{}", task.id);
        let wt_dir = format!("{}/.worktrees", task.repo_path);
        tokio::fs::create_dir_all(&wt_dir).await.ok();
        let wt_path = format!("{wt_dir}/task-{}", task.id);

        // Serialize worktree creation to avoid .git/config lock contention.
        let _wt_lock = self.worktree_create_lock.lock().await;

        let _ = git.remove_worktree(&wt_path);
        tokio::fs::remove_dir_all(&wt_path).await.ok();
        let _ = git.exec(&task.repo_path, &["worktree", "prune"]);
        let _ = git.exec(&task.repo_path, &["branch", "-D", &branch]);

        let wt_result = git.exec(
            &task.repo_path,
            &["worktree", "add", &wt_path, "-b", &branch, "origin/main"],
        )?;

        drop(_wt_lock);

        if !wt_result.success() {
            self.db.update_task_status(task.id, "failed", Some(&wt_result.stderr))?;
            return Ok(());
        }

        self.db.update_task_branch(task.id, &branch)?;

        let next = mode
            .phases
            .iter()
            .find(|p| p.phase_type != PhaseType::Setup)
            .map(|p| p.name.as_str())
            .unwrap_or("spec");

        self.db.update_task_status(task.id, next, None)?;

        info!("created worktree {} (branch {}) for task #{}", wt_path, branch, task.id);
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
        let wt_path = if mode.uses_git_worktrees {
            format!("{}/.worktrees/task-{}", task.repo_path, task.id)
        } else {
            task.repo_path.clone()
        };

        let session_dir_rel = format!("store/sessions/task-{}", task.id);
        tokio::fs::create_dir_all(&session_dir_rel).await.ok();
        let session_dir = std::fs::canonicalize(&session_dir_rel)
            .unwrap_or_else(|_| std::path::PathBuf::from(&session_dir_rel))
            .to_string_lossy()
            .to_string();

        let pending_messages = self
            .db
            .get_pending_task_messages(task.id)
            .unwrap_or_default()
            .into_iter()
            .map(|m| (m.role, m.content))
            .collect::<Vec<_>>();

        let mut ctx = self.make_context(task, wt_path.clone(), session_dir, pending_messages);
        let had_pending = !ctx.pending_messages.is_empty();
        let test_cmd = ctx.repo_config.test_cmd.clone();

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
                warn!("task #{}: no backend configured, skipping phase {}", task.id, phase.name);
                return Ok(());
            }
        };
        if let Err(e) = self.write_pipeline_state_snapshot(task, &phase.name, &wt_path).await {
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
            task.id, &phase.name, &result.output, &result.raw_stream, exit_code,
        ) {
            warn!("task #{}: insert_task_output: {e}", task.id);
        }

        self.emit(PipelineEvent::Output {
            task_id: Some(task.id),
            message: format!("task #{} phase {} completed (success={})", task.id, phase.name, result.success),
        });

        if let Some(ref artifact) = phase.check_artifact {
            if !crate::ipc::check_artifact(&wt_path, artifact) && result.output.is_empty() {
                self.db.update_task_status(
                    task.id,
                    "failed",
                    Some(&format!("missing artifact: {artifact}")),
                )?;
                return Ok(());
            }
        }

        if phase.commits && mode.uses_git_worktrees {
            let git = Git::new(&task.repo_path);
            let (_, user_coauthor) = self.git_coauthor_settings();
            let commit_msg = Self::with_user_coauthor(&phase.commit_message, &user_coauthor);
            match git.commit_all(&wt_path, &commit_msg, self.git_author()) {
                Ok(changed) => {
                    if !changed && !phase.allow_no_changes {
                        self.db.update_task_status(task.id, "failed", Some("agent made no changes"))?;
                        return Ok(());
                    }
                }
                Err(e) => warn!("commit_all for task #{}: {}", task.id, e),
            }
        }

        if phase.runs_tests && mode.uses_test_cmd && !test_cmd.is_empty() {
            let test_result = self.run_test_command(&wt_path, &test_cmd).await;
            match test_result {
                Ok(ref out) if out.exit_code == 0 => {}
                Ok(out) => {
                    let error_msg = format!("{}\n{}", out.stdout, out.stderr);
                    if phase.has_qa_fix_routing && self.error_is_in_test_files(&out.stderr) {
                        self.db.update_task_status(task.id, "qa_fix", Some(&error_msg))?;
                    } else {
                        self.fail_or_retry(task, "retry", &error_msg)?;
                    }
                    return Ok(());
                }
                Err(e) => warn!("test command error for task #{}: {}", task.id, e),
            }
        }

        self.advance_phase(task, phase, mode)?;
        Ok(())
    }

    /// Run a rebase phase.
    async fn run_rebase_phase(&self, task: &Task, phase: &PhaseConfig, _mode: &PipelineMode) -> Result<()> {
        let wt_path = format!("{}/.worktrees/task-{}", task.repo_path, task.id);
        let git = Git::new(&task.repo_path);

        if !std::path::Path::new(&wt_path).exists() {
            warn!("task #{} rebase: worktree missing at {wt_path}, resetting to backlog", task.id);
            self.db.update_task_status(task.id, "backlog", None)?;
            return Ok(());
        }

        git.fetch_origin().ok();

        match git.rebase_onto_main(&wt_path) {
            Ok(()) => {
                self.db.update_task_status(task.id, &phase.next, None)?;
                info!("task #{} rebase succeeded", task.id);
            }
            Err(e) => {
                if !phase.fix_instruction.is_empty() {
                    info!("task #{} rebase failed, running fix agent", task.id);

                    let fix_phase = PhaseConfig {
                        name: "rebase_fix".into(),
                        label: "Rebase Fix".into(),
                        instruction: phase.fix_instruction.replace("{ERROR}", &e.to_string()),
                        fresh_session: true,
                        allowed_tools: "Read,Glob,Grep,Write,Edit,Bash".into(),
                        ..Default::default()
                    };

                    let session_dir_rel = format!("store/sessions/task-{}", task.id);
                    let session_dir = std::fs::canonicalize(&session_dir_rel)
                        .unwrap_or_else(|_| std::path::PathBuf::from(&session_dir_rel))
                        .to_string_lossy()
                        .to_string();
                    let ctx = self.make_context(task, wt_path.clone(), session_dir, Vec::new());

                    if let Some(backend) = self.resolve_backend(task) {
                        if let Err(e) = self.write_pipeline_state_snapshot(task, "rebase_fix", &wt_path).await {
                            warn!("task #{}: write_pipeline_state_snapshot: {e}", task.id);
                        }
                        if let Err(fix_err) = backend.run_phase(task, &fix_phase, ctx).await {
                            warn!("task #{} fix agent error: {fix_err}", task.id);
                        }
                    }

                    if let Ok(()) = git.rebase_onto_main(&wt_path) {
                        self.db.update_task_status(task.id, &phase.next, None)?;
                        info!("task #{} rebase succeeded after fix", task.id);
                        return Ok(());
                    }
                }

                self.fail_or_retry(task, "rebase", &e.to_string())?;
            }
        }

        Ok(())
    }

    /// Run a lint_fix phase: run lint command, spawn agent to fix if dirty, re-verify.
    async fn run_lint_fix_phase(&self, task: &Task, phase: &PhaseConfig, mode: &PipelineMode) -> Result<()> {
        let wt_path = format!("{}/.worktrees/task-{}", task.repo_path, task.id);

        let lint_cmd = match self.repo_lint_cmd(&task.repo_path, &wt_path) {
            Some(cmd) => cmd,
            None => {
                self.advance_phase(task, phase, mode)?;
                info!("task #{} lint_fix: no lint command, skipping", task.id);
                return Ok(());
            }
        };

        const LINT_FIX_SYSTEM: &str = "You are a lint-fix agent. Your only job is to make the \
codebase pass the project's linter with zero warnings or errors. Do not refactor, rename, or \
change logic — only fix what the linter reports. Read the lint output carefully and make the \
minimal changes needed. After editing, do not run the linter yourself — the pipeline will verify.";

        let mut lint_out = self.run_test_command(&wt_path, &lint_cmd).await?;
        if lint_out.exit_code == 0 {
            self.advance_phase(task, phase, mode)?;
            info!("task #{} lint_fix: already clean", task.id);
            return Ok(());
        }

        let session_dir_rel = format!("store/sessions/task-{}", task.id);
        let session_dir = std::fs::canonicalize(&session_dir_rel)
            .unwrap_or_else(|_| std::path::PathBuf::from(&session_dir_rel))
            .to_string_lossy()
            .to_string();

        for fix_attempt in 0..2u32 {
            let lint_output_text = format!("{}\n{}", lint_out.stdout, lint_out.stderr)
                .trim()
                .to_string();

            info!("task #{} lint_fix: running fix agent (attempt {})", task.id, fix_attempt + 1);

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
                    if let Err(e) = self.write_pipeline_state_snapshot(task, &fix_phase.name, &wt_path).await {
                        warn!("task #{}: write_pipeline_state_snapshot: {e}", task.id);
                    }
                    b.run_phase(task, &fix_phase, ctx).await.unwrap_or_else(|e| {
                        error!("lint-fix agent for task #{}: {e}", task.id);
                        PhaseOutput::failed(String::new())
                    })
                }
                None => {
                    warn!("task #{}: no backend, skipping lint fix attempt {}", task.id, fix_attempt);
                    self.advance_phase(task, phase, mode)?;
                    return Ok(());
                }
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

            lint_out = self.run_test_command(&wt_path, &lint_cmd).await?;
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

    // ── Phase transition ──────────────────────────────────────────────────

    /// Advance a task to the next phase, or enqueue for integration when done.
    fn advance_phase(&self, task: &Task, phase: &PhaseConfig, mode: &PipelineMode) -> Result<()> {
        let next = phase.next.as_str();
        if next == "done" {
            self.db.update_task_status(task.id, "done", None)?;
            if mode.integration == IntegrationType::GitPr {
                let branch = format!("task-{}", task.id);
                if let Err(e) = self.db.enqueue(task.id, &branch, &task.repo_path, 0) {
                    warn!("enqueue for task #{}: {}", task.id, e);
                } else {
                    info!("task #{} done, queued for integration", task.id);
                }
            } else if mode.uses_git_worktrees {
                self.cleanup_worktree(task);
            }
        } else {
            self.db.update_task_status(task.id, next, None)?;
        }
        self.emit(PipelineEvent::Phase {
            task_id: Some(task.id),
            message: format!("task #{} advanced to '{}'", task.id, next),
        });
        Ok(())
    }

    // ── Pipeline state snapshot ───────────────────────────────────────────

    /// Write `.borg/pipeline-state.json` into the worktree before agent launch.
    /// Logs a warning and returns Ok(()) on any error so phase execution is
    /// never aborted by snapshot failures.
    async fn write_pipeline_state_snapshot(
        &self,
        task: &Task,
        phase_name: &str,
        wt_path: &str,
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
            let out = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(format!(
                    "gh pr view {} --json url --jq .url 2>/dev/null",
                    entry.branch
                ))
                .output()
                .await
                .ok();
            out.and_then(|o| {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() { None } else { Some(s) }
            })
        } else {
            None
        };

        let snapshot = PipelineStateSnapshot {
            task_id: task.id,
            task_title: task.title.clone(),
            phase: phase_name.to_string(),
            worktree_path: wt_path.to_string(),
            pr_url,
            pending_approvals,
            phase_history,
            generated_at: Utc::now(),
        };

        let borg_dir = format!("{wt_path}/.borg");
        tokio::fs::create_dir_all(&borg_dir).await?;
        let json = serde_json::to_string_pretty(&snapshot)?;
        tokio::fs::write(format!("{borg_dir}/pipeline-state.json"), json).await?;

        Ok(())
    }

    // ── Lint helpers ──────────────────────────────────────────────────────

    /// Resolve lint command: explicit repo config → `.borg/lint.sh` fallback.
    fn repo_lint_cmd(&self, repo_path: &str, wt_path: &str) -> Option<String> {
        if let Some(repo) = self.config.watched_repos.iter().find(|r| r.path == repo_path) {
            if !repo.lint_cmd.is_empty() {
                return Some(repo.lint_cmd.clone());
            }
        }
        let script = format!("{wt_path}/.borg/lint.sh");
        if std::path::Path::new(&script).exists() {
            return Some(format!("bash {script}"));
        }
        None
    }

    // ── Test runner ───────────────────────────────────────────────────────

    async fn run_test_command(&self, dir: &str, cmd: &str) -> Result<TestOutput> {
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(dir)
            .output()
            .await
            .context("run test command")?;

        Ok(TestOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(1),
        })
    }

    fn error_is_in_test_files(&self, error: &str) -> bool {
        ["_test.", "/tests/", "test_", ".test.", "spec."]
            .iter()
            .any(|p| error.contains(p))
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

        let mut ran_any = false;
        for repo in &self.config.watched_repos {
            let queued = self.db.get_queued_branches_for_repo(&repo.path)?;
            if queued.is_empty() {
                continue;
            }
            info!("Integration: {} branches for {}", queued.len(), repo.path);
            if let Err(e) = self.run_integration(queued, &repo.path, repo.auto_merge).await {
                warn!("Integration error for {}: {e}", repo.path);
            }
            ran_any = true;
        }

        if ran_any {
            self.db.set_ts("last_release_ts", chrono::Utc::now().timestamp());
        }
        Ok(())
    }

    async fn run_integration(&self, queued: Vec<crate::types::QueueEntry>, repo_path: &str, auto_merge: bool) -> Result<()> {
        let git = Git::new(repo_path);
        // Clean up stale worktree refs before checkout (avoids "cannot change to" errors)
        let _ = git.exec(repo_path, &["worktree", "prune"]);
        git.checkout("main")?;
        git.pull()?;

        // Filter entries whose branches no longer exist
        let mut live = Vec::new();
        for entry in queued {
            let check = git.exec(repo_path, &["rev-parse", "--verify", &entry.branch]);
            if check.map(|r| r.success()).unwrap_or(false) {
                live.push(entry);
            } else {
                warn!("Excluding {} from integration: branch not found", entry.branch);
                self.db.update_queue_status_with_error(entry.id, "excluded", "branch not found")?;
            }
        }
        if live.is_empty() {
            return Ok(());
        }

        let mut excluded_ids: HashSet<i64> = HashSet::new();
        let mut freshly_pushed: HashSet<i64> = HashSet::new();

        for entry in &live {
            // Check if already merged on GitHub, or if changes already in main (squash-merge case)
            let state_out = self.run_test_command(repo_path, &format!(
                "gh pr view {0} --json state --jq .state 2>/dev/null", entry.branch
            )).await;
            if let Ok(ref o) = state_out {
                if o.stdout.trim() == "MERGED" {
                    info!("Task #{} {}: PR already merged", entry.task_id, entry.branch);
                    self.db.update_queue_status(entry.id, "merged")?;
                    self.db.update_task_status(entry.task_id, "merged", None)?;
                    excluded_ids.insert(entry.id);
                    continue;
                }
            }

            // If diff between branch and main is empty, the work is already in main
            // (e.g. squash-merged PR where GitHub PR state shows CLOSED not MERGED)
            let diff_empty = git.exec(repo_path, &["diff", &format!("{0}...origin/main", entry.branch)])
                .map(|r| r.success() && r.stdout.trim().is_empty())
                .unwrap_or(false);
            if diff_empty {
                info!("Task #{} {}: diff vs main is empty, marking merged", entry.task_id, entry.branch);
                self.db.update_queue_status(entry.id, "merged")?;
                self.db.update_task_status(entry.task_id, "merged", None)?;
                excluded_ids.insert(entry.id);
                continue;
            }

            // Reject if not rebased on main
            let rb = git.exec(repo_path, &["merge-base", "--is-ancestor", "origin/main", &entry.branch]);
            if rb.map(|r| !r.success()).unwrap_or(false) {
                info!("Task #{} {} not rebased on main, sending to rebase", entry.task_id, entry.branch);
                self.db.update_queue_status_with_error(entry.id, "excluded", "branch not rebased on main")?;
                self.db.update_task_status(entry.task_id, "rebase", None)?;
                excluded_ids.insert(entry.id);
                continue;
            }

            // Push branch (force, to handle post-rebase)
            let push = git.push_force(&entry.branch)?;
            if !push.success() {
                if push.stderr.contains("cannot lock ref") {
                    git.delete_remote_branch(&entry.branch).ok();
                    let push2 = git.push_force(&entry.branch)?;
                    if !push2.success() {
                        warn!("Failed to push {} after ref fix: {}", entry.branch, &push2.stderr[..push2.stderr.len().min(200)]);
                        continue;
                    }
                } else {
                    warn!("Failed to push {}: {}", entry.branch, &push.stderr[..push.stderr.len().min(200)]);
                    continue;
                }
            }

            // Check if PR already exists
            let view_out = self.run_test_command(repo_path, &format!(
                "gh pr view {0} --json number --jq .number 2>/dev/null", entry.branch
            )).await?;
            if view_out.exit_code == 0 && !view_out.stdout.trim().is_empty() {
                if !push.stderr.contains("Everything up-to-date") {
                    freshly_pushed.insert(entry.id);
                }
                continue;
            }

            // Get task title for PR
            let title = self.db.get_task(entry.task_id)?
                .map(|t| t.title.chars().take(100).map(|c| if "\"\\$`".contains(c) { ' ' } else { c }).collect::<String>())
                .unwrap_or_else(|| entry.branch.clone());

            let create_out = self.run_test_command(repo_path, &format!(
                r#"gh pr create --base main --head {0} --title "{1}" --body "Automated implementation.""#,
                entry.branch, title
            )).await?;

            if create_out.exit_code != 0 {
                let err = &create_out.stderr[..create_out.stderr.len().min(300)];
                if err.contains("No commits between") {
                    info!("Task #{} {}: no commits vs main, marking merged", entry.task_id, entry.branch);
                    self.db.update_queue_status(entry.id, "merged")?;
                    self.db.update_task_status(entry.task_id, "merged", None)?;
                    excluded_ids.insert(entry.id);
                } else {
                    warn!("gh pr create {}: {}", entry.branch, err);
                }
            } else {
                info!("Created PR for {}", entry.branch);
                freshly_pushed.insert(entry.id);
            }
        }

        let mut merged_branches: Vec<String> = Vec::new();

        if !auto_merge {
            for entry in &live {
                if excluded_ids.contains(&entry.id) { continue; }
                self.db.update_queue_status(entry.id, "pending_review")?;
                info!("Task #{} {}: PR ready for manual review", entry.task_id, entry.branch);
            }
        } else {
            for entry in &live {
                if excluded_ids.contains(&entry.id) { continue; }
                if freshly_pushed.contains(&entry.id) {
                    info!("Task #{} {}: skipping merge (just pushed)", entry.task_id, entry.branch);
                    continue;
                }

                let view_out = self.run_test_command(repo_path, &format!(
                    "gh pr view {0} --json state --jq .state", entry.branch
                )).await?;
                if view_out.exit_code != 0 { continue; }
                let pr_state = view_out.stdout.trim();

                if pr_state == "MERGED" {
                    info!("Task #{} {}: already merged", entry.task_id, entry.branch);
                    self.db.update_queue_status(entry.id, "merged")?;
                    self.db.update_task_status(entry.task_id, "merged", None)?;
                    merged_branches.push(entry.branch.clone());
                    continue;
                }

                // Check mergeability
                let mb_out = self.run_test_command(repo_path, &format!(
                    "gh pr view {0} --json mergeable --jq .mergeable", entry.branch
                )).await?;
                let mb = mb_out.stdout.trim().to_string();
                let mut force_merge = false;

                if mb == "UNKNOWN" {
                    let retries = self.db.get_unknown_retries(entry.id);
                    if retries >= 5 {
                        warn!("Task #{} {}: mergeability UNKNOWN after {} retries, forcing merge", entry.task_id, entry.branch, retries);
                        self.db.reset_unknown_retries(entry.id)?;
                        force_merge = true;
                    } else {
                        self.db.increment_unknown_retries(entry.id)?;
                        info!("Task #{} {}: UNKNOWN ({}/5), retrying next tick", entry.task_id, entry.branch, retries + 1);
                        continue;
                    }
                }
                if !force_merge && mb != "MERGEABLE" {
                    info!("Task #{} {}: mergeable={mb}, sending to rebase", entry.task_id, entry.branch);
                    self.db.update_queue_status_with_error(entry.id, "excluded", "merge conflict with main")?;
                    self.db.update_task_status(entry.task_id, "rebase", None)?;
                    continue;
                }

                self.db.update_queue_status(entry.id, "merging")?;
                let merge_out = self.run_test_command(repo_path, &format!(
                    "gh pr merge {0} --squash", entry.branch
                )).await;

                match merge_out {
                    Err(e) => {
                        warn!("gh pr merge {}: {e}", entry.branch);
                        self.db.update_queue_status(entry.id, "queued")?;
                    }
                    Ok(out) if out.exit_code != 0 => {
                        let err = &out.stderr[..out.stderr.len().min(200)];
                        warn!("gh pr merge {}: {}", entry.branch, err);
                        if err.contains("not mergeable") || err.contains("cannot be cleanly created") {
                            self.db.update_queue_status_with_error(entry.id, "excluded", "merge conflict with main")?;
                            self.db.update_task_status(entry.task_id, "rebase", None)?;
                            info!("Task #{} has conflicts, sent to rebase", entry.task_id);
                        } else {
                            self.db.update_queue_status(entry.id, "queued")?;
                        }
                    }
                    Ok(_) => {
                        self.db.update_queue_status(entry.id, "merged")?;
                        self.db.update_task_status(entry.task_id, "merged", None)?;
                        merged_branches.push(entry.branch.clone());
                        // Clean up worktree and local branch (gh --squash doesn't do local cleanup)
                        let wt_path = format!("{}/.worktrees/{}", repo_path, entry.branch);
                        if std::path::Path::new(&wt_path).exists() {
                            if let Err(e) = git.remove_worktree(&wt_path) {
                                warn!("remove_worktree {} after merge: {e}", entry.branch);
                            }
                        }
                        let _ = git.delete_branch(&entry.branch);
                        if let Ok(Some(task)) = self.db.get_task(entry.task_id) {
                            self.notify(&task.notify_chat, &format!("Task #{} \"{}\" merged via PR.", task.id, task.title));
                        }
                    }
                }
            }
        }

        if !merged_branches.is_empty() {
            git.pull().ok();
            let digest = self.generate_digest(&merged_branches);
            self.notify(&self.config.pipeline_admin_chat, &digest);
            info!("Integration complete: {} merged", merged_branches.len());
        }

        Ok(())
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
            let mode = match get_mode(&repo.mode).or_else(|| get_mode("sweborg")) {
                Some(m) => m,
                None => continue,
            };
            for seed_cfg in mode.seed_modes.clone() {
                let key = (repo.path.clone(), seed_cfg.name.clone());
                {
                    let cooldowns = self.seed_cooldowns.lock().await;
                    if now - cooldowns.get(&key).copied().unwrap_or(0) < cooldown {
                        continue;
                    }
                }
                self.seed_cooldowns.lock().await.insert(key, now);
                info!("seed scan: '{}' for {}", seed_cfg.name, repo.path);
                if let Err(e) = self.run_seed(repo, &mode.name, &seed_cfg).await {
                    warn!("seed {} for {}: {e}", seed_cfg.name, repo.path);
                }
            }
        }

        Ok(())
    }

    async fn run_seed(&self, repo: &RepoConfig, mode_name: &str, seed_cfg: &crate::types::SeedConfig) -> Result<()> {
        let session_dir = std::fs::canonicalize("store/sessions/seed")
            .unwrap_or_else(|_| {
                std::fs::create_dir_all("store/sessions/seed").ok();
                std::fs::canonicalize("store/sessions/seed").unwrap_or_else(|_| std::path::PathBuf::from("store/sessions/seed"))
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
        };

        let task_suffix = "\n\nFor each improvement, output EXACTLY this format (one per task):\n\n\
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
        let backend = self.resolve_backend(&task)
            .ok_or_else(|| anyhow::anyhow!("no backends configured for seed"))?;
        let result = backend.run_phase(&task, &phase, ctx).await?;

        if !result.success {
            warn!(
                "seed '{}' for {} failed (output: {:?})",
                seed_cfg.name,
                repo.path,
                &result.output
            );
        } else {
            info!("seed '{}' output: {:?}", seed_cfg.name, &result.output);
        }

        let target_repo = if seed_cfg.target_primary_repo {
            self.config.watched_repos.iter().find(|r| r.is_self).map(|r| r.path.as_str()).unwrap_or(&repo.path)
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
                    };
                    match self.db.insert_task(&task) {
                        Ok(id) => info!("seed created task #{id}: {}", task.title),
                        Err(e) => warn!("seed insert_task: {e}"),
                    }
                }
            }
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
            }
        }
        Ok(())
    }

    // ── Health monitoring ─────────────────────────────────────────────────

    pub async fn check_health(&self) -> Result<()> {
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
            let git = Git::new(&repo.path);
            if git.checkout("main").is_err() || git.pull().is_err() {
                continue;
            }
            match self.run_test_command(&repo.path, &repo.test_cmd).await {
                Ok(out) if out.exit_code != 0 => {
                    warn!("Health: tests failed for {}", repo.path);
                    self.create_health_task(&repo.path, "tests", &out.stderr).await;
                }
                Ok(_) => info!("Health: {} OK", repo.path),
                Err(e) => warn!("Health: test command error for {}: {e}", repo.path),
            }
        }
        Ok(())
    }

    async fn create_health_task(&self, repo_path: &str, kind: &str, stderr: &str) {
        // Dedup: skip if a fix task already exists for this repo
        if let Ok(active) = self.db.list_active_tasks() {
            if active.iter().any(|t| t.title.starts_with("Fix failing ") && t.repo_path == repo_path) {
                return;
            }
        }
        let tail = if stderr.len() > 500 { &stderr[stderr.len() - 500..] } else { stderr };
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
        };
        match self.db.insert_task(&task) {
            Ok(id) => {
                info!("Health: created fix task #{id} for {repo_path} {kind} failure");
                self.notify(&self.config.pipeline_admin_chat, &format!("Health check: {kind} failing for {repo_path}, created fix task #{id}"));
            }
            Err(e) => warn!("Health: insert_task: {e}"),
        }
    }

    // ── Self-update ───────────────────────────────────────────────────────

    pub async fn check_remote_updates(&self) {
        let now = chrono::Utc::now().timestamp();
        if now - self.db.get_ts("last_remote_check_ts") < self.config.remote_check_interval_s {
            return;
        }
        self.db.set_ts("last_remote_check_ts", now);

        for repo in &self.config.watched_repos {
            if !repo.is_self {
                continue;
            }
            let git = Git::new(&repo.path);
            if git.fetch_origin().is_err() {
                return;
            }
            let local = match git.rev_parse_head() { Ok(h) => h, Err(_) => return };
            let remote = match git.rev_parse("origin/main") { Ok(h) => h, Err(_) => return };
            if local == remote {
                return;
            }
            info!("Remote update detected on {}, pulling...", repo.path);
            if let Err(e) = git.pull() {
                warn!("Remote pull failed: {e}");
                return;
            }
            self.check_self_update(&repo.path).await;
            return;
        }
    }

    async fn check_self_update(&self, repo_path: &str) {
        let git = Git::new(repo_path);
        let current = match git.rev_parse_head() { Ok(h) => h, Err(_) => return };
        let startup = match self.startup_heads.get(repo_path) { Some(h) => h, None => return };
        if &current == startup {
            return;
        }

        info!("Self-update: HEAD changed, rebuilding...");
        self.notify(&self.config.pipeline_admin_chat, "Self-update: new commits detected, rebuilding...");

        let build_cmd = self.db.get_config("build_cmd").ok().flatten()
            .unwrap_or_else(|| self.config.build_cmd.clone());

        let out = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&build_cmd)
            .current_dir(repo_path)
            .output()
            .await;

        match out {
            Err(e) => {
                warn!("Self-update: build spawn failed: {e}");
                self.notify(&self.config.pipeline_admin_chat, "Self-update: build FAILED (spawn error).");
            }
            Ok(o) if !o.status.success() => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                warn!("Self-update: build failed: {}", &stderr[..stderr.len().min(500)]);
                self.notify(&self.config.pipeline_admin_chat, "Self-update: build FAILED, continuing with old binary.");
            }
            Ok(_) => {
                info!("Self-update: build succeeded, restart scheduled");
                self.notify(&self.config.pipeline_admin_chat, "Self-update: new build ready. Will restart in 3h or on director command.");
                self.last_self_update_secs.store(chrono::Utc::now().timestamp(), std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    pub fn maybe_apply_self_update(&self) {
        let ts = self.last_self_update_secs.load(std::sync::atomic::Ordering::Relaxed);
        if ts == 0 {
            return;
        }
        let forced = self.force_restart.load(std::sync::atomic::Ordering::Acquire);
        let age = chrono::Utc::now().timestamp() - ts;
        if !forced && age < 3 * 3600 {
            return;
        }
        info!("Self-update: applying restart (forced={forced}, age={age}s)");
        self.notify(&self.config.pipeline_admin_chat, "Self-update: restarting now...");
        // Signal main loop to exit; systemd restarts the process
        self.force_restart.store(true, std::sync::atomic::Ordering::Release);
    }

    // ── Auto-promote + auto-triage ────────────────────────────────────────

    pub fn maybe_auto_promote_proposals(&self) {
        let active = self.db.active_task_count();
        let max = self.config.pipeline_max_backlog as i64;
        if active >= max {
            return;
        }
        let slots = max - active;
        let proposals = match self.db.get_top_scored_proposals(self.config.proposal_promote_threshold, slots) {
            Ok(p) => p,
            Err(e) => { warn!("auto_promote: {e}"); return; }
        };
        for p in proposals {
            let mode = self.config.watched_repos.iter()
                .find(|r| r.path == p.repo_path)
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
            };
            match self.db.insert_task(&task) {
                Ok(id) => {
                    self.db.update_proposal_status(p.id, "approved").ok();
                    info!("Auto-promoted proposal #{} (score={}) → task #{}: {}", p.id, p.triage_score, id, p.title);
                }
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
            for t in &merged { prompt.push_str(&format!("- {}\n", t.title)); }
            prompt.push('\n');
        }
        prompt.push_str("Proposals to evaluate:\n\n");
        for p in &proposals {
            prompt.push_str(&format!(
                "- ID {}: {}\n  Description: {}\n  Rationale: {}\n\n",
                p.id, p.title,
                if p.description.is_empty() { "(none)" } else { &p.description },
                if p.rationale.is_empty() { "(none)" } else { &p.rationale },
            ));
        }

        let output = self.run_claude_print("claude-haiku-4-5-20251001", &prompt).await;
        let output = match output { Ok(o) => o, Err(e) => { warn!("auto_triage: {e}"); return; } };

        let arr_start = match output.find('[') { Some(i) => i, None => { warn!("auto_triage: no JSON array in output"); return; } };
        let arr_end = match output.rfind(']') { Some(i) => i + 1, None => return };
        let json_slice = &output[arr_start..arr_end];

        let items: Vec<serde_json::Value> = match serde_json::from_str(json_slice) {
            Ok(v) => v,
            Err(e) => { warn!("auto_triage: JSON parse failed: {e}"); return; }
        };

        let mut scored = 0u32;
        let mut dismissed = 0u32;
        for item in &items {
            let get_i64 = |k: &str| item.get(k).and_then(|v| v.as_i64());
            let p_id = match get_i64("id") { Some(v) => v, None => continue };
            let impact = match get_i64("impact") { Some(v) => v, None => continue };
            let feasibility = match get_i64("feasibility") { Some(v) => v, None => continue };
            let risk = match get_i64("risk") { Some(v) => v, None => continue };
            let effort = match get_i64("effort") { Some(v) => v, None => continue };
            let score = match get_i64("score") { Some(v) => v, None => continue };
            let reasoning = item.get("reasoning").and_then(|v| v.as_str()).unwrap_or("");
            let should_dismiss = item.get("dismiss").and_then(|v| v.as_bool()).unwrap_or(false);

            if let Err(e) = self.db.update_proposal_triage(p_id, score, impact, feasibility, risk, effort, reasoning) {
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
        info!("Auto-triage: scored {scored}/{} proposals, dismissed {dismissed}", proposals.len());
    }

    /// Run `claude --print --model <model>` with prompt on stdin, return stdout.
    async fn run_claude_print(&self, model: &str, prompt: &str) -> Result<String> {
        use tokio::io::AsyncWriteExt;
        let mut child = tokio::process::Command::new("claude")
            .args(["--print", "--model", model, "--permission-mode", "bypassPermissions"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .env("CLAUDE_CODE_OAUTH_TOKEN", &self.config.oauth_token)
            .spawn()
            .context("spawn claude --print")?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes()).await.ok();
        }
        let out = child.wait_with_output().await.context("wait claude --print")?;
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

// ── Private helpers ───────────────────────────────────────────────────────────

struct TestOutput {
    stdout: String,
    stderr: String,
    exit_code: i32,
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

fn looks_like_field_key(line: &str) -> bool {
    let trimmed = line.trim();
    if let Some(colon) = trimmed.find(':') {
        let key = &trimmed[..colon];
        !key.is_empty() && !key.contains(' ') && key.chars().next().map_or(false, |c| c.is_alphabetic())
    } else {
        false
    }
}
