use std::io::Read;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};

pub const DEFAULT_GIT_OUTPUT_LIMIT: usize = 10 * 1024 * 1024; // 10 MB

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
    /// Max bytes to collect from stdout or stderr of a single git command.
    pub output_limit: usize,
}

/// Read up to `limit` bytes from `reader`, returning the data and whether it was truncated.
fn read_limited<R: Read>(reader: R, limit: usize) -> (String, bool) {
    let mut buf = Vec::new();
    // Read one extra byte so we can detect truncation without reading the whole stream.
    reader
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut buf)
        .ok();
    let truncated = buf.len() > limit;
    if truncated {
        buf.truncate(limit);
    }
    (String::from_utf8_lossy(&buf).into_owned(), truncated)
}

impl Git {
    pub fn new(repo_path: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
            output_limit: DEFAULT_GIT_OUTPUT_LIMIT,
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
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn git -C {dir} {}", args.join(" ")))?;

        let stdout_pipe = child.stdout.take().expect("stdout pipe missing");
        let stderr_pipe = child.stderr.take().expect("stderr pipe missing");

        let limit = self.output_limit;
        let stdout_handle = std::thread::spawn(move || read_limited(stdout_pipe, limit));
        let stderr_handle = std::thread::spawn(move || read_limited(stderr_pipe, limit));

        let status = child
            .wait()
            .with_context(|| format!("failed to wait for git -C {dir} {}", args.join(" ")))?;

        let (stdout, stdout_truncated) = stdout_handle.join().expect("stdout thread panicked");
        let (stderr, stderr_truncated) = stderr_handle.join().expect("stderr thread panicked");

        if stdout_truncated || stderr_truncated {
            tracing::warn!(
                "git output truncated at {} bytes for: git -C {dir} {}",
                limit,
                args.join(" ")
            );
        }

        Ok(ExecResult {
            stdout,
            stderr,
            exit_code: status.code().unwrap_or(1),
        })
    }

    pub fn remove_worktree(&self, worktree_path: &str) -> Result<()> {
        let result = self.exec(
            &self.repo_path,
            &["worktree", "remove", "--force", worktree_path],
        )?;
        if !result.success() {
            return Err(anyhow!(
                "git worktree remove failed for {worktree_path}: {}",
                result.combined_output()
            ));
        }
        Ok(())
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

    pub fn rebase_onto_main(&self, worktree_path: &str) -> Result<()> {
        let result = self.exec(worktree_path, &["rebase", "origin/main"])?;
        if !result.success() {
            return Err(anyhow!(
                "git rebase origin/main failed in {worktree_path}: {}",
                result.combined_output()
            ));
        }
        Ok(())
    }

    pub fn rebase_abort(&self, worktree_path: &str) -> Result<()> {
        let result = self.exec(worktree_path, &["rebase", "--abort"])?;
        if !result.success() {
            return Err(anyhow!(
                "git rebase --abort failed in {worktree_path}: {}",
                result.combined_output()
            ));
        }
        Ok(())
    }

    pub fn rebase_in_progress(&self, worktree_path: &str) -> Result<bool> {
        let merge = self.exec(worktree_path, &["rev-parse", "--git-path", "rebase-merge"])?;
        let apply = self.exec(worktree_path, &["rev-parse", "--git-path", "rebase-apply"])?;
        if !merge.success() || !apply.success() {
            anyhow::bail!("git rev-parse --git-path failed in {worktree_path}");
        }
        let merge_path = std::path::PathBuf::from(merge.stdout.trim());
        let apply_path = std::path::PathBuf::from(apply.stdout.trim());
        Ok(merge_path.exists() || apply_path.exists())
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
