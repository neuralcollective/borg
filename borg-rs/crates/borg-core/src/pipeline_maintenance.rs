use anyhow::Result;
use tracing::{info, warn};

use crate::{git::Git, pipeline::Pipeline, sandbox::SandboxMode, types::Task};

impl Pipeline {
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
        let desc = format!(
            "Health check detected {kind} failure on main branch.\n\nError output:\n```\n{tail}\n```"
        );
        let task = Task {
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
            updated_at: chrono::Utc::now(),
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
                    &format!("Health check: {kind} failing for {repo_path}, created fix task #{id}"),
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
            }
            Err(e) => {
                warn!("Self-update: git pull spawn failed: {e}");
                return;
            }
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
            }
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
            }
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
            }
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
}
