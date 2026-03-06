use std::{collections::HashMap, process::Command};

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
        work_dir: &str,
        message: &str,
        author: Option<(&str, &str)>,
    ) -> Result<bool> {
        let add = self.exec(work_dir, &["add", "-A"])?;
        if !add.success() {
            return Err(anyhow!(
                "git add -A failed in {work_dir}: {}",
                add.combined_output()
            ));
        }

        let status = self.exec(work_dir, &["status", "--porcelain"])?;
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

        let result = self.exec(work_dir, &args)?;
        if !result.success() {
            return Err(anyhow!(
                "git commit failed in {work_dir}: {}",
                result.combined_output()
            ));
        }
        Ok(true)
    }

    pub fn ls_files(&self, work_dir: &str) -> Result<String> {
        let result = self.exec(work_dir, &["ls-files"])?;
        if !result.success() {
            return Err(anyhow!(
                "git ls-files failed in {work_dir}: {}",
                result.combined_output()
            ));
        }
        Ok(result.stdout)
    }

    pub fn ls_files_manifest(
        &self,
        work_dir: &str,
        max_entries: usize,
        max_bytes: usize,
    ) -> Result<String> {
        let listing = self.ls_files(work_dir)?;
        if listing.trim().is_empty() {
            return Ok(String::new());
        }

        let files = listing.lines().collect::<Vec<_>>();
        let total = files.len();
        let mut top_dirs: HashMap<String, usize> = HashMap::new();
        for file in &files {
            let top = file.split('/').next().unwrap_or(*file).to_string();
            *top_dirs.entry(top).or_insert(0) += 1;
        }
        let mut dir_counts = top_dirs.into_iter().collect::<Vec<_>>();
        dir_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        let mut out = format!("Repository manifest (bounded)\n- total_tracked_files: {total}\n");
        if !dir_counts.is_empty() {
            let dirs = dir_counts
                .iter()
                .take(12)
                .map(|(dir, count)| format!("{dir} ({count})"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("- top_level_entries: {dirs}\n"));
        }
        out.push_str("- sample_paths:\n");

        let mut shown = 0usize;
        for file in files.iter().take(max_entries.max(1)) {
            let line = format!("  - {file}\n");
            if out.len() + line.len() > max_bytes.max(256) {
                break;
            }
            out.push_str(&line);
            shown += 1;
        }
        if shown < total {
            out.push_str(&format!("- omitted_paths: {}\n", total - shown));
        }
        Ok(out)
    }

    /// Create and checkout a new branch from a given start point.
    pub fn checkout_new_branch(&self, branch: &str, start: &str) -> Result<()> {
        // Delete the branch if it already exists locally (stale leftover from a previous attempt)
        let _ = self.exec(&self.repo_path, &["branch", "-D", branch]);
        let result = self.exec(&self.repo_path, &["checkout", "-b", branch, start])?;
        if !result.success() {
            return Err(anyhow!(
                "git checkout -b {branch} {start} failed: {}",
                result.combined_output()
            ));
        }
        Ok(())
    }

    /// Push a branch to origin.
    pub fn push_branch(&self, branch: &str) -> Result<()> {
        let result = self.exec(&self.repo_path, &["push", "-u", "origin", branch, "--force-with-lease"])?;
        if !result.success() {
            return Err(anyhow!(
                "git push origin {branch} failed: {}",
                result.combined_output()
            ));
        }
        Ok(())
    }

    /// Checkout an existing branch.
    pub fn checkout(&self, branch: &str) -> Result<()> {
        let result = self.exec(&self.repo_path, &["checkout", branch])?;
        if !result.success() {
            return Err(anyhow!(
                "git checkout {branch} failed: {}",
                result.combined_output()
            ));
        }
        Ok(())
    }

}
