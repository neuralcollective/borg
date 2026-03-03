use std::process::Command;

use anyhow::{anyhow, Context, Result};

pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl ExecResult {
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }

    pub fn combined_output(&self) -> String {
        if self.stderr.is_empty() {
            self.stdout.clone()
        } else {
            format!("{}\n{}", self.stdout, self.stderr)
        }
    }
}

pub struct Git {
    pub repo_path: String,
}

impl Git {
    pub fn new(repo_path: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    pub fn exec(&self, dir: &str, args: &[&str]) -> Result<ExecResult> {
        self.exec_env(dir, args, &[])
    }

    pub fn exec_env(&self, dir: &str, args: &[&str], env: &[(&str, &str)]) -> Result<ExecResult> {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(dir);
        cmd.args(args);
        for (k, v) in env {
            cmd.env(k, v);
        }

        let output = cmd
            .output()
            .with_context(|| format!("failed to spawn git -C {dir} {}", args.join(" ")))?;

        Ok(ExecResult {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(1),
        })
    }

    pub fn rev_parse_head(&self) -> Result<String> {
        self.rev_parse("HEAD")
    }

    pub fn rev_parse(&self, refname: &str) -> Result<String> {
        let result = self.exec(&self.repo_path, &["rev-parse", refname])?;
        if !result.success() {
            return Err(anyhow!(
                "git rev-parse {refname} failed: {}",
                result.combined_output()
            ));
        }
        Ok(result.stdout.trim().to_string())
    }

    pub fn fetch_origin(&self) -> Result<()> {
        let result = self.exec(&self.repo_path, &["fetch", "origin"])?;
        if !result.success() {
            return Err(anyhow!(
                "git fetch origin failed: {}",
                result.combined_output()
            ));
        }
        Ok(())
    }

    pub fn commit_all(
        &self,
        worktree_path: &str,
        message: &str,
        author: Option<(&str, &str)>,
    ) -> Result<bool> {
        let add = self.exec(worktree_path, &["add", "-A"])?;
        if !add.success() {
            return Err(anyhow!(
                "git add -A failed in {worktree_path}: {}",
                add.combined_output()
            ));
        }

        let status = self.exec(worktree_path, &["status", "--porcelain"])?;
        if status.stdout.trim().is_empty() {
            return Ok(false);
        }

        let mut args = vec!["commit", "-m", message];
        let author_str;
        if let Some((name, email)) = author {
            author_str = format!("{name} <{email}>");
            args.push("--author");
            args.push(&author_str);
        }

        let result = self.exec(worktree_path, &args)?;
        if !result.success() {
            return Err(anyhow!(
                "git commit failed in {worktree_path}: {}",
                result.combined_output()
            ));
        }
        Ok(true)
    }

    pub fn ls_files(&self, worktree_path: &str) -> Result<String> {
        let result = self.exec(worktree_path, &["ls-files"])?;
        if !result.success() {
            return Err(anyhow!(
                "git ls-files failed in {worktree_path}: {}",
                result.combined_output()
            ));
        }
        Ok(result.stdout)
    }
}
