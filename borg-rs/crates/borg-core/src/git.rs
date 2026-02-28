use std::{path::Path, process::Command};

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

    pub fn worktree_path(&self, branch: &str) -> String {
        let parent = Path::new(&self.repo_path)
            .parent()
            .unwrap_or(Path::new("/tmp"));
        parent
            .join("worktrees")
            .join(branch)
            .to_string_lossy()
            .into_owned()
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

    pub fn create_worktree(&self, branch: &str, base: &str) -> Result<String> {
        let wt_path = self.worktree_path(branch);
        let result = self.exec(
            &self.repo_path,
            &["worktree", "add", &wt_path, "-b", branch, base],
        )?;
        if !result.success() {
            return Err(anyhow!(
                "git worktree add failed for branch={branch} base={base}: {}",
                result.combined_output()
            ));
        }
        Ok(wt_path)
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

    pub fn pull(&self) -> Result<()> {
        // Try fast-forward first; if diverged, reset hard to origin/main
        let result = self.exec(&self.repo_path, &["pull", "--ff-only", "origin", "main"])?;
        if result.success() {
            return Ok(());
        }
        // Diverged: discard local commits and follow origin
        let reset = self.exec(&self.repo_path, &["reset", "--hard", "origin/main"])?;
        if !reset.success() {
            return Err(anyhow!(
                "git pull --ff-only then reset --hard both failed: {}",
                reset.combined_output()
            ));
        }
        Ok(())
    }

    pub fn push_force(&self, branch: &str) -> Result<ExecResult> {
        self.exec(&self.repo_path, &["push", "--force", "origin", branch])
    }

    pub fn delete_remote_branch(&self, branch: &str) -> Result<()> {
        let result = self.exec(&self.repo_path, &["push", "origin", "--delete", branch])?;
        if !result.success() {
            return Err(anyhow!(
                "git push origin --delete {branch} failed: {}",
                result.combined_output()
            ));
        }
        Ok(())
    }

    pub fn delete_branch(&self, branch: &str) -> Result<()> {
        let result = self.exec(&self.repo_path, &["branch", "-D", branch])?;
        if !result.success() {
            return Err(anyhow!(
                "git branch -D {branch} failed: {}",
                result.combined_output()
            ));
        }
        Ok(())
    }

    pub fn stash(&self) -> Result<()> {
        let result = self.exec(&self.repo_path, &["stash"])?;
        if !result.success() {
            return Err(anyhow!("git stash failed: {}", result.combined_output()));
        }
        Ok(())
    }

    pub fn stash_pop(&self) -> Result<()> {
        let result = self.exec(&self.repo_path, &["stash", "pop"])?;
        if !result.success() {
            return Err(anyhow!(
                "git stash pop failed: {}",
                result.combined_output()
            ));
        }
        Ok(())
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
            return Ok(false);
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

    pub fn push_branch(&self, worktree_path: &str, branch: &str) -> Result<()> {
        let result = self.exec(worktree_path, &["push", "origin", branch])?;
        if !result.success() {
            return Err(anyhow!(
                "git push origin {branch} failed: {}",
                result.combined_output()
            ));
        }
        Ok(())
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

    pub fn merge_delete(&self, branch: &str) -> Result<()> {
        let checkout = self.exec(&self.repo_path, &["checkout", "main"])?;
        if !checkout.success() {
            return Err(anyhow!(
                "git checkout main failed: {}",
                checkout.combined_output()
            ));
        }

        let merge = self.exec(&self.repo_path, &["merge", "--no-ff", branch])?;
        if !merge.success() {
            return Err(anyhow!(
                "git merge --no-ff {branch} failed: {}",
                merge.combined_output()
            ));
        }

        let delete = self.exec(&self.repo_path, &["branch", "-D", branch])?;
        if !delete.success() {
            return Err(anyhow!(
                "git branch -D {branch} failed: {}",
                delete.combined_output()
            ));
        }
        Ok(())
    }

    pub fn status_clean(&self, dir: &str) -> Result<bool> {
        let result = self.exec(dir, &["status", "--porcelain"])?;
        Ok(result.stdout.trim().is_empty() && result.exit_code == 0)
    }

    pub fn current_branch(&self, dir: &str) -> Result<String> {
        let result = self.exec(dir, &["rev-parse", "--abbrev-ref", "HEAD"])?;
        if !result.success() {
            return Err(anyhow!(
                "git rev-parse --abbrev-ref HEAD failed in {dir}: {}",
                result.combined_output()
            ));
        }
        Ok(result.stdout.trim().to_string())
    }

    pub fn abort_rebase(&self, dir: &str) -> Result<ExecResult> {
        self.exec(dir, &["rebase", "--abort"])
    }

    pub fn abort_merge(&self, dir: &str) -> Result<ExecResult> {
        self.exec(dir, &["merge", "--abort"])
    }

    pub fn diff_name_only(&self, dir: &str) -> Result<String> {
        let result = self.exec(dir, &["diff", "--name-only", "HEAD"])?;
        if !result.success() {
            return Err(anyhow!(
                "git diff --name-only HEAD failed in {dir}: {}",
                result.combined_output()
            ));
        }
        Ok(result.stdout)
    }

    pub fn log_oneline(&self, dir: &str, range: &str) -> Result<String> {
        let result = self.exec(dir, &["log", "--oneline", range])?;
        if !result.success() {
            return Err(anyhow!(
                "git log --oneline {range} failed in {dir}: {}",
                result.combined_output()
            ));
        }
        Ok(result.stdout)
    }

    pub fn reset_hard(&self, dir: &str, ref_: &str) -> Result<()> {
        let result = self.exec(dir, &["reset", "--hard", ref_])?;
        if !result.success() {
            return Err(anyhow!(
                "git reset --hard {ref_} failed in {dir}: {}",
                result.combined_output()
            ));
        }
        Ok(())
    }

    pub fn set_author_config(&self, dir: &str, name: &str, email: &str) -> Result<()> {
        let name_result = self.exec(dir, &["config", "user.name", name])?;
        if !name_result.success() {
            return Err(anyhow!(
                "git config user.name failed: {}",
                name_result.combined_output()
            ));
        }
        let email_result = self.exec(dir, &["config", "user.email", email])?;
        if !email_result.success() {
            return Err(anyhow!(
                "git config user.email failed: {}",
                email_result.combined_output()
            ));
        }
        Ok(())
    }
}
