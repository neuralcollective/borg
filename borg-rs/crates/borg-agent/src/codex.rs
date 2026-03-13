use std::{path::Path, process::Stdio};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use borg_core::{
    agent::AgentBackend,
    types::{PhaseConfig, PhaseContext, PhaseOutput, Task},
};
use serde_json::json;
use tracing::{info, warn};

use crate::drain::{drain_child, DrainConfig};

/// Runs Codex (openai/codex) as the agent backend.
///
/// Codex is invoked via `codex exec` in non-interactive mode.
/// The codex app-server JSON-RPC protocol is planned but not yet wired up.
pub struct CodexBackend {
    pub api_key: String,
    pub model: String,
    pub reasoning_effort: String,
    pub codex_bin: String,
    pub timeout_s: u64,
    pub git_author_name: String,
    pub git_author_email: String,
    pub git_committer_name: String,
    pub git_committer_email: String,
    pub credentials_path: String,
}

impl CodexBackend {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            reasoning_effort: "medium".into(),
            codex_bin: "codex".into(),
            timeout_s: 0,
            git_author_name: "Borg".into(),
            git_author_email: "borg@localhost".into(),
            git_committer_name: "Borg".into(),
            git_committer_email: "borg@localhost".into(),
            credentials_path: String::new(),
        }
    }

    pub fn with_reasoning_effort(mut self, effort: impl Into<String>) -> Self {
        self.reasoning_effort = effort.into();
        self
    }

    pub fn with_credentials_path(mut self, path: impl Into<String>) -> Self {
        self.credentials_path = path.into();
        self
    }

    pub fn with_bin(mut self, bin: impl Into<String>) -> Self {
        self.codex_bin = bin.into();
        self
    }

    pub fn with_timeout(mut self, timeout_s: u64) -> Self {
        self.timeout_s = timeout_s;
        self
    }

    pub fn with_git_identity(
        mut self,
        author_name: &str,
        author_email: &str,
        committer_name: &str,
        committer_email: &str,
    ) -> Self {
        if !author_name.is_empty() {
            self.git_author_name = author_name.into();
        }
        if !author_email.is_empty() {
            self.git_author_email = author_email.into();
        }
        if !committer_name.is_empty() {
            self.git_committer_name = committer_name.into();
        }
        if !committer_email.is_empty() {
            self.git_committer_email = committer_email.into();
        }
        self
    }

    pub async fn is_available(&self) -> bool {
        tokio::process::Command::new(&self.codex_bin)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn is_warning_stderr(line: &str) -> bool {
        let l = line.trim().to_ascii_lowercase();
        l.starts_with("error:")
            || l.starts_with("error ")
            || l.starts_with("fatal:")
            || l.contains("unexpected status")
            || l.contains("unauthorized")
            || l.contains("failed to")
            || l.contains("panic!")
            || l.contains("panicked at")
            || (l.contains("thread '") && l.contains(" panicked"))
    }

    /// Copy host Codex credentials into an isolated session home when no
    /// session auth exists and no explicit API key was provided.
    fn ensure_session_auth(credentials_path: &str, codex_home: &str) -> bool {
        if credentials_path.is_empty() {
            return false;
        }
        let session_auth = Path::new(codex_home).join("auth.json");
        if session_auth.exists() {
            return false;
        }
        if !borg_core::config::codex_has_credentials(credentials_path) {
            return false;
        }
        let Ok(contents) = std::fs::read(credentials_path) else {
            return false;
        };
        if std::fs::write(&session_auth, contents).is_err() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                &session_auth,
                std::fs::Permissions::from_mode(0o600),
            );
        }
        info!(
            codex_home,
            "copied host codex credentials into isolated session"
        );
        true
    }

    fn push_config_arg(args: &mut Vec<String>, key: &str, value: serde_json::Value) {
        args.push("-c".into());
        args.push(format!("{key}={value}"));
    }

    fn append_mcp_config(
        &self,
        args: &mut Vec<String>,
        task: &Task,
        ctx: &PhaseContext,
    ) -> Result<()> {
        if ctx.borg_api_token.is_empty() || ctx.borg_api_url.is_empty() {
            return Ok(());
        }

        if let Some(borg_server) = crate::mcp::resolve_mcp_server_path(
            "BORG_MCP_SERVER",
            "../../../sidecar/borg-mcp/server.js",
        ) {
            let server_str = borg_server.to_string_lossy().to_string();
            Self::push_config_arg(args, "mcp_servers.borg.command", json!("bun"));
            Self::push_config_arg(args, "mcp_servers.borg.args", json!(["run", server_str]));
            Self::push_config_arg(
                args,
                "mcp_servers.borg.env.API_BASE_URL",
                json!(&ctx.borg_api_url),
            );
            Self::push_config_arg(
                args,
                "mcp_servers.borg.env.API_TOKEN",
                json!(&ctx.borg_api_token),
            );
            if task.project_id > 0 {
                Self::push_config_arg(
                    args,
                    "mcp_servers.borg.env.PROJECT_ID",
                    json!(task.project_id.to_string()),
                );
                Self::push_config_arg(args, "mcp_servers.borg.env.PROJECT_MODE", json!(&task.mode));
            }
        } else {
            warn!(task_id = task.id, "failed to resolve borg-mcp server path");
            return Ok(());
        }

        if matches!(task.mode.as_str(), "lawborg" | "legal") {
            if let Some(legal_server) = crate::mcp::resolve_mcp_server_path(
                "LAWBORG_MCP_SERVER",
                "../../../sidecar/lawborg-mcp/server.js",
            ) {
                let server_str = legal_server.to_string_lossy().to_string();
                Self::push_config_arg(args, "mcp_servers.legal.command", json!("bun"));
                Self::push_config_arg(args, "mcp_servers.legal.args", json!(["run", server_str]));
                for (provider, key) in &ctx.api_keys {
                    if let Some(env_name) = crate::mcp::legal_provider_env_name(provider) {
                        Self::push_config_arg(
                            args,
                            &format!("mcp_servers.legal.env.{env_name}"),
                            json!(key),
                        );
                    }
                }
            } else {
                warn!(
                    task_id = task.id,
                    "failed to resolve lawborg-mcp server path"
                );
            }
        }

        Ok(())
    }
}

#[async_trait]
impl AgentBackend for CodexBackend {
    async fn run_phase(
        &self,
        task: &Task,
        phase: &PhaseConfig,
        ctx: PhaseContext,
    ) -> Result<PhaseOutput> {
        if !self.is_available().await {
            bail!("codex binary not found: {}", self.codex_bin);
        }

        let instruction = crate::instruction::build_instruction(task, phase, &ctx, None);

        info!(
            task_id = task.id,
            phase = %phase.name,
            model = %self.model,
            "spawning codex subprocess"
        );

        let mut codex_args = vec![
            "exec".to_string(),
            "--model".to_string(),
            self.model.clone(),
            "-c".to_string(),
            format!("model_reasoning_effort=\"{}\"", self.reasoning_effort),
            "--full-auto".to_string(),
        ];
        self.append_mcp_config(&mut codex_args, task, &ctx)?;
        codex_args.push(instruction.clone());

        let mut cmd = tokio::process::Command::new(&self.codex_bin);
        let codex_home = format!("{}/.codex", ctx.session_dir);
        std::fs::create_dir_all(&codex_home)
            .with_context(|| format!("failed to create Codex home: {codex_home}"))?;
        if self.api_key.is_empty() {
            Self::ensure_session_auth(&self.credentials_path, &codex_home);
        }
        let has_linked_auth = Path::new(&codex_home).join("auth.json").exists();
        cmd.args(&codex_args)
            .current_dir(&ctx.work_dir)
            .env_remove("CLAUDECODE")
            .env("HOME", &ctx.session_dir)
            .env("CODEX_HOME", &codex_home)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.env("GIT_AUTHOR_NAME", &self.git_author_name);
        cmd.env("GIT_AUTHOR_EMAIL", &self.git_author_email);
        cmd.env("GIT_COMMITTER_NAME", &self.git_committer_name);
        cmd.env("GIT_COMMITTER_EMAIL", &self.git_committer_email);
        if !has_linked_auth && !self.api_key.is_empty() {
            cmd.env("OPENAI_API_KEY", &self.api_key);
        }
        let mut child = cmd
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("failed to spawn codex binary: {}", self.codex_bin))?;

        let drain = drain_child(
            &mut child,
            DrainConfig {
                backend: "codex",
                task_id: task.id,
                phase_name: &phase.name,
                timeout_s: self.timeout_s,
                stream_tx: ctx.stream_tx.clone(),
                is_warning_stderr: Self::is_warning_stderr,
            },
        )
        .await?;

        let exit_status = child
            .wait()
            .await
            .context("failed to wait for codex process")?;

        info!(
            task_id = task.id,
            phase = %phase.name,
            success = exit_status.success() && !drain.had_fatal_stderr,
            output_len = drain.output.len(),
            "codex subprocess finished"
        );

        Ok(PhaseOutput {
            output: drain.output,
            new_session_id: None,
            raw_stream: String::new(),
            success: !drain.timed_out && exit_status.success() && !drain.had_fatal_stderr,
            signal_json: None,
            ran_in_docker: false,
            container_test_results: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_valid_credentials(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(
            path,
            r#"{"tokens":{"access_token":"tok_test"}}"#,
        )
        .unwrap();
    }

    #[test]
    fn ensure_session_auth_copies_valid_credentials() {
        let host_dir = TempDir::new().unwrap();
        let creds_path = host_dir.path().join("auth.json");
        write_valid_credentials(&creds_path);

        let session_dir = TempDir::new().unwrap();
        let codex_home = session_dir.path().join(".codex");
        fs::create_dir_all(&codex_home).unwrap();

        let copied = CodexBackend::ensure_session_auth(
            creds_path.to_str().unwrap(),
            codex_home.to_str().unwrap(),
        );
        assert!(copied);
        assert!(codex_home.join("auth.json").exists());

        let contents = fs::read_to_string(codex_home.join("auth.json")).unwrap();
        assert!(contents.contains("tok_test"));
    }

    #[test]
    fn ensure_session_auth_noop_when_session_auth_exists() {
        let host_dir = TempDir::new().unwrap();
        let creds_path = host_dir.path().join("auth.json");
        write_valid_credentials(&creds_path);

        let session_dir = TempDir::new().unwrap();
        let codex_home = session_dir.path().join(".codex");
        fs::create_dir_all(&codex_home).unwrap();
        fs::write(codex_home.join("auth.json"), r#"{"existing":true}"#).unwrap();

        let copied = CodexBackend::ensure_session_auth(
            creds_path.to_str().unwrap(),
            codex_home.to_str().unwrap(),
        );
        assert!(!copied);

        let contents = fs::read_to_string(codex_home.join("auth.json")).unwrap();
        assert!(contents.contains("existing"));
    }

    #[test]
    fn ensure_session_auth_noop_when_credentials_path_empty() {
        let session_dir = TempDir::new().unwrap();
        let codex_home = session_dir.path().join(".codex");
        fs::create_dir_all(&codex_home).unwrap();

        let copied = CodexBackend::ensure_session_auth("", codex_home.to_str().unwrap());
        assert!(!copied);
        assert!(!codex_home.join("auth.json").exists());
    }

    #[test]
    fn ensure_session_auth_noop_when_credentials_invalid() {
        let host_dir = TempDir::new().unwrap();
        let creds_path = host_dir.path().join("auth.json");
        fs::write(&creds_path, r#"{"no_tokens":true}"#).unwrap();

        let session_dir = TempDir::new().unwrap();
        let codex_home = session_dir.path().join(".codex");
        fs::create_dir_all(&codex_home).unwrap();

        let copied = CodexBackend::ensure_session_auth(
            creds_path.to_str().unwrap(),
            codex_home.to_str().unwrap(),
        );
        assert!(!copied);
        assert!(!codex_home.join("auth.json").exists());
    }
}
