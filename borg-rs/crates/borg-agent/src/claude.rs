use std::path::Path;
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use borg_core::{
    agent::AgentBackend,
    sandbox::{Sandbox, SandboxMode},
    types::{ContainerTestResult, PhaseConfig, PhaseContext, PhaseOutput, Task},
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tracing::{debug, error, info, warn};

const BORG_SIGNAL_MARKER: &str = "BORG_SIGNAL:";

/// Utility to extract phase results from Claude output.
pub fn extract_phase_result(output: &str) -> Option<String> {
    output
        .lines()
        .rev()
        .find(|l| l.starts_with(BORG_SIGNAL_MARKER))
        .and_then(|l| l.strip_prefix(BORG_SIGNAL_MARKER))
        .map(|s| s.trim().to_string())
}

fn derive_compile_check(test_cmd: &str) -> Option<String> {
    let trimmed = test_cmd.trim();
    if trimmed.starts_with("cargo test") {
        Some(format!("{trimmed} --no-run"))
    } else {
        None
    }
}

/// Runs Claude Code as a subprocess, with configurable sandbox isolation.
pub struct ClaudeBackend {
    /// Path to the `claude` CLI binary.
    pub claude_bin: String,
    /// Which sandbox backend to use for `phase.use_docker` phases.
    /// Use `Sandbox::detect()` at startup to pick the best available option.
    pub sandbox_mode: SandboxMode,
    /// Docker image name (only used when `sandbox_mode == Docker`).
    pub docker_image: String,
    /// Kill subprocess and return failure after this many seconds (0 = no limit).
    pub timeout_s: u64,
    /// Path to Claude credentials file for fresh token reads.
    pub credentials_path: String,
    /// Memory limit for Docker containers in MiB (0 = no limit).
    pub container_memory_mb: u64,
    /// CPU quota for Docker containers (0.0 = no limit).
    pub container_cpus: f64,
    pub git_author_name: String,
    pub git_author_email: String,
    pub base_url: String,
}

impl ClaudeBackend {
    pub fn new(
        claude_bin: impl Into<String>,
        sandbox_mode: SandboxMode,
        docker_image: impl Into<String>,
    ) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        Self {
            claude_bin: claude_bin.into(),
            sandbox_mode,
            docker_image: docker_image.into(),
            timeout_s: 0,
            credentials_path: format!("{home}/.claude/.credentials.json"),
            container_memory_mb: 0,
            container_cpus: 0.0,
            git_author_name: "Borg".into(),
            git_author_email: "borg@localhost".into(),
            base_url: String::new(),
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    pub fn with_timeout(mut self, timeout_s: u64) -> Self {
        self.timeout_s = timeout_s;
        self
    }

    pub fn with_resource_limits(mut self, memory_mb: u64, cpus: f64) -> Self {
        self.container_memory_mb = memory_mb;
        self.container_cpus = cpus;
        self
    }

    pub fn with_git_author(mut self, name: &str, email: &str) -> Self {
        self.git_author_name = name.to_string();
        self.git_author_email = email.to_string();
        self
    }

    fn resolve_gh_token() -> String {
        std::process::Command::new("gh")
            .args(["auth", "token"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }

    fn host_mirror_path(task: &Task, data_dir: &str) -> String {
        let repo_name = std::path::Path::new(&task.repo_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let path = std::path::Path::new(data_dir).join("mirrors").join(format!("{repo_name}.git"));
        std::fs::canonicalize(path)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    fn container_mirror_path(task: &Task) -> String {
        let repo_name = std::path::Path::new(&task.repo_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        format!("/mirrors/{repo_name}.git")
    }
}

#[async_trait]
impl AgentBackend for ClaudeBackend {
    async fn run_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        ctx: PhaseContext,
    ) -> Result<PhaseOutput> {
        let file_listing = if phase.include_file_listing {
            let git = borg_core::git::Git::new(&ctx.work_dir);
            git.ls_files(&ctx.work_dir).ok()
        } else {
            None
        };
        let instruction =
            crate::instruction::build_instruction(task, phase, &ctx, file_listing.as_deref());

        let mut claude_args = vec![
            "--print".to_string(),
            "--dangerously-skip-permissions".to_string(),
        ];
        if phase.fresh_session {
            claude_args.push("--no-resume".into());
        } else if !task.session_id.is_empty() {
            claude_args.push("--resume".into());
            claude_args.push(task.session_id.clone());
        }
        claude_args.push(instruction);

        let full_cmd: Vec<String> = std::iter::once(self.claude_bin.clone())
            .chain(claude_args)
            .collect();

        info!(
            task_id = task.id,
            phase = %phase.name,
            mode = ?self.sandbox_mode,
            "spawning claude"
        );

        let effective_mode = if phase.use_docker {
            self.sandbox_mode.clone()
        } else {
            SandboxMode::Direct
        };

        let is_docker = matches!(effective_mode, SandboxMode::Docker);
        let oauth_token = ctx.oauth_token.clone();

        let stream_tx = ctx.stream_tx.clone();
        if let Some(tx) = &stream_tx {
            let evt = serde_json::json!({
                "type": "status",
                "status": format!("Spawning agent ({:?})...", effective_mode),
            })
            .to_string();
            let _ = tx.send(evt);
        }

        let cidfile_path = if is_docker {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let p = format!("/tmp/borg-cid-{}-{}.txt", task.id, ms);
            let _ = std::fs::remove_file(&p);
            Some(p)
        } else {
            None
        };

        let real_home = std::env::var("HOME").unwrap_or_default();
        let rustup_home = std::env::var("RUSTUP_HOME")
            .unwrap_or_else(|_| format!("{real_home}/.rustup"));
        let cargo_home = std::env::var("CARGO_HOME")
            .unwrap_or_else(|_| format!("{real_home}/.cargo"));

        let gh_token = if is_docker {
            tokio::task::spawn_blocking(Self::resolve_gh_token)
                .await
                .unwrap_or_default()
        } else {
            String::new()
        };

        let effective_base_url = if ctx.isolated {
            "http://172.31.0.1:3131".to_string()
        } else if !self.base_url.is_empty() {
            self.base_url.clone()
        } else {
            String::new()
        };

        let mut child: tokio::process::Child = match effective_mode {
            SandboxMode::Bwrap => {
                let git_dir = Path::new(&task.repo_path).join(".git");
                let git_dir_str = git_dir.to_string_lossy().to_string();
                let writable: Vec<&str> = vec![ctx.work_dir.as_str(), ctx.session_dir.as_str(), &git_dir_str];
                let mut cmd = Sandbox::bwrap_command(&writable, &ctx.work_dir, &full_cmd);
                cmd.kill_on_drop(true)
                    .env("HOME", &ctx.session_dir)
                    .env("RUSTUP_HOME", &rustup_home)
                    .env("CARGO_HOME", &cargo_home)
                    .env("CLAUDE_CODE_OAUTH_TOKEN", &oauth_token);
                
                if !effective_base_url.is_empty() {
                    cmd.env("ANTHROPIC_BASE_URL", &effective_base_url);
                }
                
                cmd.stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .context("failed to spawn bwrap")?
            },
            SandboxMode::Docker => {
                let binds = vec![
                    (ctx.work_dir.clone(), "/workspace".to_string(), false),
                    (ctx.session_dir.clone(), "/home/bun".to_string(), false),
                ];
                let volumes_owned = vec![
                    ("rustup-cache".to_string(), "/home/bun/.rustup".to_string()),
                    ("cargo-cache".to_string(), "/home/bun/.cargo".to_string()),
                ];
                let mut env_kv = vec![
                    ("HOME".to_string(), "/home/bun".to_string()),
                    ("RUSTUP_HOME".to_string(), "/home/bun/.rustup".to_string()),
                    ("CARGO_HOME".to_string(), "/home/bun/.cargo".to_string()),
                    ("CLAUDE_CODE_OAUTH_TOKEN".to_string(), oauth_token.clone()),
                ];
                if !gh_token.is_empty() {
                    env_kv.push(("GH_TOKEN".to_string(), gh_token));
                }
                if !effective_base_url.is_empty() {
                    env_kv.push(("ANTHROPIC_BASE_URL".to_string(), effective_base_url));
                }

                let binds_ref: Vec<(&str, &str, bool)> = binds
                    .iter()
                    .map(|(h, c, r)| (h.as_str(), c.as_str(), *r))
                    .collect();
                let volumes_ref: Vec<(&str, &str)> = volumes_owned
                    .iter()
                    .map(|(n, c)| (n.as_str(), c.as_str()))
                    .collect();
                let env_ref: Vec<(&str, &str)> = env_kv
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();

                let mut docker_cmd = Sandbox::docker_command(
                    &self.docker_image,
                    &binds_ref,
                    &volumes_ref,
                    "",
                    &[],
                    &env_ref,
                    self.container_memory_mb,
                    self.container_cpus,
                    ctx.agent_network.as_deref(),
                );
                if let Some(ref cid_path) = cidfile_path {
                    let existing_args: Vec<_> = docker_cmd
                        .as_std()
                        .get_args()
                        .map(|a| a.to_os_string())
                        .collect();
                    let mut new_cmd = Command::new("docker");
                    new_cmd.arg("run").arg("--cidfile").arg(cid_path);
                    for arg in existing_args.into_iter().skip(1) {
                        new_cmd.arg(arg);
                    }
                    docker_cmd = new_cmd;
                }

                docker_cmd
                    .kill_on_drop(true)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .stdin(Stdio::piped())
                    .spawn()
                    .context("failed to spawn docker")?
            }
            SandboxMode::Direct => {
                let augmented_path = format!(
                    "{}/bin:{}:{}",
                    cargo_home,
                    std::env::var("PATH").unwrap_or_default(),
                    "/usr/local/bin:/usr/bin:/bin"
                );
                let mut cmd = Command::new(&self.claude_bin);
                cmd.args(&full_cmd[1..])
                    .kill_on_drop(true)
                    .current_dir(&ctx.work_dir)
                    .env("HOME", &ctx.session_dir)
                    .env("RUSTUP_HOME", &rustup_home)
                    .env("CARGO_HOME", &cargo_home)
                    .env("PATH", &augmented_path)
                    .env("CLAUDE_CODE_OAUTH_TOKEN", &oauth_token);

                if !effective_base_url.is_empty() {
                    cmd.env("ANTHROPIC_BASE_URL", &effective_base_url);
                }

                cmd.stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .with_context(|| format!("failed to spawn claude: {}", self.claude_bin))?
            },
            _ => bail!("unsupported sandbox mode"),
        };

        if is_docker {
            if let Some(mut stdin) = child.stdin.take() {
                let repo_test_cmd = ctx.repo_config.test_cmd.clone();
                let compile_check_cmd = if phase.compile_check {
                    derive_compile_check(&repo_test_cmd).unwrap_or_default()
                } else {
                    String::new()
                };
                let input = serde_json::json!({
                    "test_cmd": repo_test_cmd,
                    "compile_check_cmd": compile_check_cmd,
                    "lint_cmd": ctx.repo_config.lint_cmd,
                    "git_user_name": self.git_author_name,
                    "git_user_email": self.git_author_email,
                });
                let payload = serde_json::to_vec(&input).unwrap_or_default();
                let _ = stdin.write_all(&payload).await;
                drop(stdin);
            }
        }

        let stdout = child.stdout.take().context("failed to take stdout")?;
        let stderr = child.stderr.take().context("failed to take stderr")?;

        let timeout_s = self.timeout_s;
        let io_future = async move {
            let mut signal_json: Option<String> = None;
            let mut output_lines: Vec<String> = Vec::new();
            let mut container_test_results: Vec<ContainerTestResult> = Vec::new();

            let mut stdout_reader = BufReader::new(stdout).lines();
            let mut stderr_reader = BufReader::new(stderr).lines();

            let mut stdout_done = false;
            let mut stderr_done = false;

            while !stdout_done || !stderr_done {
                tokio::select! {
                    line = stdout_reader.next_line(), if !stdout_done => {
                        match line {
                            Ok(Some(l)) => {
                                if let Some(sig) = l.strip_prefix(BORG_SIGNAL_MARKER) {
                                    signal_json = Some(sig.to_string());
                                }
                                if let Some(tx) = &stream_tx {
                                    let _ = tx.send(l.clone());
                                }
                                output_lines.push(l);
                            }
                            Ok(None) => stdout_done = true,
                            Err(e) => {
                                warn!("stdout read error: {e}");
                                stdout_done = true;
                            }
                        }
                    }
                    line = stderr_reader.next_line(), if !stderr_done => {
                        match line {
                            Ok(Some(l)) => {
                                if !l.is_empty() {
                                    if let Ok(res) = serde_json::from_str::<ContainerTestResult>(&l) {
                                        container_test_results.push(res);
                                    } else {
                                        if let Some(tx) = &stream_tx {
                                            let evt = serde_json::json!({
                                                "type": "stderr",
                                                "content": l,
                                            }).to_string();
                                            let _ = tx.send(evt);
                                        }
                                        debug!("claude stderr: {l}");
                                    }
                                }
                            }
                            Ok(None) => stderr_done = true,
                            Err(e) => {
                                warn!("stderr read error: {e}");
                                stderr_done = true;
                            }
                        }
                    }
                }
            }

            let exit_status = child.wait().await.ok();
            let success = exit_status.map(|s| s.success()).unwrap_or(false);
            (output_lines.join("\n"), signal_json, container_test_results, success)
        };

        let (raw_stream, signal_json, container_test_results, success) = if timeout_s > 0 {
            match tokio::time::timeout(std::time::Duration::from_secs(timeout_s), io_future).await {
                Ok(res) => res,
                Err(_) => {
                    warn!("claude timed out after {}s", timeout_s);
                    (String::new(), None, Vec::new(), false)
                }
            }
        } else {
            io_future.await
        };

        if let Some(cid_path) = cidfile_path {
            if let Ok(cid) = std::fs::read_to_string(&cid_path) {
                let cid = cid.trim();
                if !cid.is_empty() {
                    info!("cleaning up container {cid}");
                    let _ = std::process::Command::new("docker")
                        .args(["rm", "-f", cid])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            }
            let _ = std::fs::remove_file(cid_path);
        }

        Ok(PhaseOutput {
            output: raw_stream.clone(),
            new_session_id: signal_json,
            raw_stream,
            success,
            signal_json: None,
            ran_in_docker: is_docker,
            container_test_results,
        })
    }
}
