use borg_core::{
    db::KnowledgeFile,
    types::{PhaseConfig, PhaseContext, Task},
};

/// Build the instruction string passed to any agent backend.
///
/// Composes task context, the phase instruction, an optional file listing,
/// error context from the previous attempt, and any pending user messages.
/// All backends use this so the prompt format stays consistent.
pub fn build_instruction(task: &Task, phase: &PhaseConfig, ctx: &PhaseContext, file_listing: Option<&str>) -> String {
    let mut s = String::new();

    let kb = build_knowledge_section(&ctx.knowledge_files, &ctx.knowledge_dir);
    if !kb.is_empty() {
        s.push_str(&kb);
        s.push_str("\n\n---\n\n");
    }

    if let Some(repo_prompt) = read_repo_prompt(ctx) {
        s.push_str("## Project Context\n\n");
        s.push_str(&repo_prompt);
        s.push_str("\n\n---\n\n");
    }

    if phase.include_task_context {
        s.push_str(&format!(
            "Task: {}\n\n{}\n\n---\n\n",
            task.title, task.description
        ));
    }

    s.push_str(&phase.instruction);

    if let Some(files) = file_listing.filter(|f| !f.is_empty()) {
        s.push_str("\n\n---\n\nFiles in repository:\n```\n");
        s.push_str(files);
        s.push_str("```\n");
    }

    if !task.last_error.is_empty() && !phase.error_instruction.is_empty() {
        s.push('\n');
        s.push_str(&phase.error_instruction.replace("{ERROR}", &task.last_error));
    }

    if !ctx.pending_messages.is_empty() {
        s.push_str("\n\n---\nThe following messages were sent by the user or director while this task was queued:\n");
        for (role, content) in &ctx.pending_messages {
            s.push_str(&format!("\n[{}]: {}", role, content));
        }
    }

    s
}

/// Build the `## Knowledge Base` section prepended to agent instructions.
pub fn build_knowledge_section(files: &[KnowledgeFile], knowledge_dir: &str) -> String {
    if files.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "## Knowledge Base\nYou have access to the following knowledge files at /knowledge/:\n",
    );
    for file in files {
        if file.inline {
            let path = format!("{}/{}", knowledge_dir, file.file_name);
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let content = content.trim();
            if content.is_empty() {
                s.push_str(&format!("- **{}**", file.file_name));
                if !file.description.is_empty() {
                    s.push_str(&format!(": {}", file.description));
                }
                s.push('\n');
            } else {
                s.push_str(&format!("- **{}**", file.file_name));
                if !file.description.is_empty() {
                    s.push_str(&format!(" ({})", file.description));
                }
                s.push_str(":\n```\n");
                s.push_str(content);
                s.push_str("\n```\n");
            }
        } else {
            s.push_str(&format!("- `/knowledge/{}`", file.file_name));
            if !file.description.is_empty() {
                s.push_str(&format!(": {}", file.description));
            }
            s.push('\n');
        }
    }
    s
}

/// Read the per-repo prompt from the explicit prompt_file config, or by
/// auto-detecting `.borg/prompt.md` in the worktree / repo root.
fn read_repo_prompt(ctx: &PhaseContext) -> Option<String> {
    use borg_core::ipc::{self, IpcReadResult};

    // 1. Explicit prompt_file from config (operator-trusted absolute path)
    if !ctx.repo_config.prompt_file.is_empty() {
        if let IpcReadResult::Ok(content) = ipc::read_trusted_path(&ctx.repo_config.prompt_file) {
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }

    // 2. .borg/prompt.md in the worktree (may differ from repo root during tasks)
    if let IpcReadResult::Ok(content) = ipc::read_file(&ctx.worktree_path, ".borg/prompt.md") {
        let trimmed = content.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }

    // 3. .borg/prompt.md in the repo root (skip if same path as worktree)
    if ctx.repo_config.path != ctx.worktree_path {
        if let IpcReadResult::Ok(content) = ipc::read_file(&ctx.repo_config.path, ".borg/prompt.md")
        {
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::read_repo_prompt;
    use borg_core::db::KnowledgeFile;
    use borg_core::types::{PhaseContext, RepoConfig, Task};

    fn make_ctx(prompt_file: &str, worktree_path: &str, repo_path: &str) -> PhaseContext {
        PhaseContext {
            task: Task {
                id: 0,
                title: String::new(),
                description: String::new(),
                repo_path: String::new(),
                branch: String::new(),
                status: String::new(),
                attempt: 0,
                max_attempts: 1,
                last_error: String::new(),
                created_by: String::new(),
                notify_chat: String::new(),
                created_at: chrono::Utc::now(),
                session_id: String::new(),
                mode: String::new(),
                backend: String::new(),
            },
            repo_config: RepoConfig {
                path: repo_path.to_string(),
                test_cmd: String::new(),
                prompt_file: prompt_file.to_string(),
                mode: String::new(),
                is_self: false,
                auto_merge: false,
                lint_cmd: String::new(),
                backend: String::new(),
                repo_slug: String::new(),
            },
            data_dir: String::new(),
            session_dir: String::new(),
            worktree_path: worktree_path.to_string(),
            oauth_token: String::new(),
            model: String::new(),
            pending_messages: Vec::new(),
            system_prompt_suffix: String::new(),
            user_coauthor: String::new(),
            stream_tx: None,
            setup_script: String::new(),
            api_keys: std::collections::HashMap::new(),
            disallowed_tools: String::new(),
            knowledge_files: Vec::new() as Vec<KnowledgeFile>,
            knowledge_dir: String::new(),
            agent_network: None,
        }
    }

    #[test]
    fn test_explicit_prompt_file_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("custom.md");
        std::fs::write(&file, "explicit content\n").unwrap();

        let ctx = make_ctx(file.to_str().unwrap(), "/nonexistent/wt", "/nonexistent/repo");
        assert_eq!(read_repo_prompt(&ctx), Some("explicit content".to_string()));
    }

    #[test]
    fn test_explicit_path_missing_falls_through_to_worktree() {
        let wt = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(wt.path().join(".borg")).unwrap();
        std::fs::write(wt.path().join(".borg/prompt.md"), "worktree content\n").unwrap();

        let wt_str = wt.path().to_str().unwrap();
        let ctx = make_ctx("/nonexistent/prompt.md", wt_str, wt_str);
        assert_eq!(read_repo_prompt(&ctx), Some("worktree content".to_string()));
    }

    #[test]
    fn test_equal_paths_skip_third_lookup() {
        let wt = tempfile::TempDir::new().unwrap();
        let repo = tempfile::TempDir::new().unwrap();
        // Only the repo root has .borg/prompt.md
        std::fs::create_dir_all(repo.path().join(".borg")).unwrap();
        std::fs::write(repo.path().join(".borg/prompt.md"), "repo root content").unwrap();

        let wt_str = wt.path().to_str().unwrap();
        let repo_str = repo.path().to_str().unwrap();

        // Different paths: level 3 reads repo root → content found
        let ctx_diff = make_ctx("", wt_str, repo_str);
        assert_eq!(read_repo_prompt(&ctx_diff), Some("repo root content".to_string()));

        // Equal paths (worktree == repo): level 3 skipped, level 2 finds nothing → None
        let ctx_same = make_ctx("", wt_str, wt_str);
        assert_eq!(read_repo_prompt(&ctx_same), None);
    }

    #[test]
    fn test_all_three_missing_returns_none() {
        let wt = tempfile::TempDir::new().unwrap();
        let repo = tempfile::TempDir::new().unwrap();
        let wt_str = wt.path().to_str().unwrap();
        let repo_str = repo.path().to_str().unwrap();

        let ctx = make_ctx("", wt_str, repo_str);
        assert_eq!(read_repo_prompt(&ctx), None);
    }

    #[test]
    fn test_whitespace_only_file_treated_as_absent() {
        let prompt_dir = tempfile::TempDir::new().unwrap();
        let wt = tempfile::TempDir::new().unwrap();

        // Explicit file contains only whitespace → treated as absent
        let prompt_file = prompt_dir.path().join("prompt.md");
        std::fs::write(&prompt_file, "   \n\t\n  ").unwrap();

        // Worktree has real content → should be returned as fallback
        std::fs::create_dir_all(wt.path().join(".borg")).unwrap();
        std::fs::write(wt.path().join(".borg/prompt.md"), "fallback content").unwrap();

        let wt_str = wt.path().to_str().unwrap();
        let ctx = make_ctx(prompt_file.to_str().unwrap(), wt_str, wt_str);
        assert_eq!(read_repo_prompt(&ctx), Some("fallback content".to_string()));
    }
}
