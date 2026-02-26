use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::{broadcast, Mutex};
use tracing::{error, info, warn};

use chrono::Utc;

use crate::{
    agent::AgentBackend,
    config::Config,
    db::Db,
    git::Git,
    modes::get_mode,
    sandbox::Sandbox,
    stream::TaskStreamManager,
    types::{IntegrationType, PhaseConfig, PhaseContext, PhaseOutput, PhaseType, PipelineMode, Proposal, RepoConfig, SeedOutputType, Task},
};

/// Broadcast event emitted after each significant pipeline state change.
#[derive(Debug, Clone)]
pub struct PipelineEvent {
    pub kind: String,
    pub task_id: Option<i64>,
    pub message: String,
}

pub struct Pipeline {
    pub db: Arc<Db>,
    pub backends: HashMap<String, Arc<dyn AgentBackend>>,
    pub config: Arc<Config>,
    pub sandbox: Sandbox,
    pub event_tx: broadcast::Sender<PipelineEvent>,
    last_seed_secs: std::sync::atomic::AtomicI64,
    in_flight: Mutex<HashSet<i64>>,
}

impl Pipeline {
    pub fn new(
        db: Arc<Db>,
        backends: HashMap<String, Arc<dyn AgentBackend>>,
        config: Arc<Config>,
    ) -> (Self, broadcast::Receiver<PipelineEvent>) {
        let (tx, rx) = broadcast::channel(256);
        let p = Self {
            db,
            backends,
            config,
            sandbox: Sandbox,
            event_tx: tx,
            last_seed_secs: std::sync::atomic::AtomicI64::new(0),
            in_flight: Mutex::new(HashSet::new()),
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

    /// Main tick: dispatch ready tasks and run seed if idle.
    pub async fn tick(self: Arc<Self>) -> Result<()> {
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

        let _ = git.remove_worktree(&wt_path);
        tokio::fs::remove_dir_all(&wt_path).await.ok();
        let _ = git.exec(&task.repo_path, &["worktree", "prune"]);
        let _ = git.exec(&task.repo_path, &["branch", "-D", &branch]);

        let wt_result = git.exec(
            &task.repo_path,
            &["worktree", "add", &wt_path, "-b", &branch, "origin/main"],
        )?;
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
        self.emit(PipelineEvent {
            kind: "task_phase".into(),
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

        let session_dir = format!("store/sessions/task-{}", task.id);
        tokio::fs::create_dir_all(&session_dir).await.ok();

        let pending_messages = self
            .db
            .get_pending_task_messages(task.id)
            .unwrap_or_default()
            .into_iter()
            .map(|m| (m.role, m.content))
            .collect::<Vec<_>>();

        let ctx = self.make_context(task, wt_path.clone(), session_dir, pending_messages);
        let had_pending = !ctx.pending_messages.is_empty();
        let test_cmd = ctx.repo_config.test_cmd.clone();

        info!("running {} phase for task #{}", phase.name, task.id);

        let backend = match self.resolve_backend(task) {
            Some(b) => b,
            None => {
                warn!("task #{}: no backend configured, skipping phase {}", task.id, phase.name);
                return Ok(());
            }
        };
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

        self.emit(PipelineEvent {
            kind: "task_output".into(),
            task_id: Some(task.id),
            message: format!("task #{} phase {} completed (success={})", task.id, phase.name, result.success),
        });

        if let Some(ref artifact) = phase.check_artifact {
            let artifact_path = format!("{wt_path}/{artifact}");
            if !std::path::Path::new(&artifact_path).exists() && result.output.is_empty() {
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

                    let session_dir = format!("store/sessions/task-{}", task.id);
                    let ctx = self.make_context(task, wt_path.clone(), session_dir, Vec::new());

                    if let Some(backend) = self.resolve_backend(task) {
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

        let session_dir = format!("store/sessions/task-{}", task.id);

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
                Some(b) => b.run_phase(task, &fix_phase, ctx).await.unwrap_or_else(|e| {
                    error!("lint-fix agent for task #{}: {e}", task.id);
                    PhaseOutput::failed(String::new())
                }),
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
        self.emit(PipelineEvent {
            kind: "task_phase".into(),
            task_id: Some(task.id),
            message: format!("task #{} advanced to '{}'", task.id, next),
        });
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

    // ── Seed ─────────────────────────────────────────────────────────────

    async fn seed_if_idle(&self) -> Result<()> {
        if !self.config.continuous_mode {
            return Ok(());
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let last = self.last_seed_secs.load(std::sync::atomic::Ordering::Relaxed);
        if now - last < self.config.pipeline_seed_cooldown_s {
            return Ok(());
        }

        let active = self.db.list_active_tasks()?.len() as u32;
        if active >= self.config.pipeline_max_backlog {
            return Ok(());
        }

        self.last_seed_secs.store(now, std::sync::atomic::Ordering::Relaxed);
        info!("seed scan starting (idle pipeline)");

        for repo in &self.config.watched_repos {
            let mode = match get_mode(&repo.mode).or_else(|| get_mode("sweborg")) {
                Some(m) => m,
                None => continue,
            };
            for seed_cfg in mode.seed_modes.clone() {
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

        let phase = PhaseConfig {
            name: format!("seed_{}", seed_cfg.name),
            label: seed_cfg.label.clone(),
            instruction: seed_cfg.prompt.clone(),
            fresh_session: true,
            allowed_tools: "Read,Glob,Grep,Bash".to_string(),
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

        self.parse_seed_output(&result.output, &repo.path, mode_name, seed_cfg.output_type)?;
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

    // ── Event broadcast ───────────────────────────────────────────────────

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
    for line in block.lines() {
        if let Some(rest) = line.strip_prefix(field) {
            let val = rest.trim().to_string();
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}
