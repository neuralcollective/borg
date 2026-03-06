use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};

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
        let top_level = self.exec(work_dir, &["ls-tree", "--name-only", "HEAD"])?;
        if !top_level.success() {
            return Err(anyhow!(
                "git ls-tree HEAD failed in {work_dir}: {}",
                top_level.combined_output()
            ));
        }

        let mut child = Command::new("git")
            .arg("-C")
            .arg(work_dir)
            .args(["ls-files"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to spawn git -C {work_dir} ls-files"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("missing stdout for git ls-files in {work_dir}"))?;
        let reader = BufReader::new(stdout);
        let mut samples = Vec::new();
        for line in reader.lines().take(max_entries.max(1)) {
            let line =
                line.with_context(|| format!("failed to read git ls-files output in {work_dir}"))?;
            if !line.trim().is_empty() {
                samples.push(line);
            }
        }
        let _ = child.kill();
        let _ = child.wait();

        if samples.is_empty() && top_level.stdout.trim().is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("Repository manifest (sampled)\n");
        out.push_str(
            "- scope: sampled tracked files only; use Read/Glob/Grep for deeper discovery\n",
        );
        let top_entries = top_level
            .stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .take(16)
            .collect::<Vec<_>>();
        if !top_entries.is_empty() {
            out.push_str(&format!(
                "- top_level_entries: {}\n",
                top_entries.join(", ")
            ));
        }
        out.push_str("- sample_paths:\n");
        let mut shown = 0usize;
        for file in &samples {
            let line = format!("  - {file}\n");
            if out.len() + line.len() > max_bytes.max(256) {
                break;
            }
            out.push_str(&line);
            shown += 1;
        }
        out.push_str(&format!("- sampled_entries: {shown}\n"));
        if shown >= max_entries.max(1) {
            out.push_str("- note: additional tracked files omitted from the manifest sample\n");
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
        let result = self.exec(
            &self.repo_path,
            &["push", "-u", "origin", branch, "--force-with-lease"],
        )?;
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
