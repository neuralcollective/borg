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

pub(crate) fn read_repo_prompt(ctx: &PhaseContext) -> Option<String> {
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
    use std::collections::HashMap;

    use chrono::Utc;
    use tempfile::TempDir;

    use borg_core::types::{PhaseContext, RepoConfig, Task};

    use super::read_repo_prompt;

    fn make_task() -> Task {
        Task {
            id: 1,
            title: "t".into(),
            description: "d".into(),
            repo_path: String::new(),
            branch: String::new(),
            status: "backlog".into(),
            attempt: 0,
            max_attempts: 3,
            last_error: String::new(),
            created_by: String::new(),
            notify_chat: String::new(),
            created_at: Utc::now(),
            session_id: String::new(),
            mode: "sweborg".into(),
            backend: String::new(),
        }
    }

    fn make_repo_config(path: &str, prompt_file: &str) -> RepoConfig {
        RepoConfig {
            path: path.to_string(),
            test_cmd: String::new(),
            prompt_file: prompt_file.to_string(),
            mode: "sweborg".into(),
            is_self: false,
            auto_merge: false,
            lint_cmd: String::new(),
            backend: String::new(),
            repo_slug: String::new(),
        }
    }

    fn make_ctx(worktree_path: &str, repo_config: RepoConfig) -> PhaseContext {
        PhaseContext {
            task: make_task(),
            repo_config,
            data_dir: String::new(),
            session_dir: String::new(),
            worktree_path: worktree_path.to_string(),
            oauth_token: String::new(),
            model: String::new(),
            pending_messages: vec![],
            system_prompt_suffix: String::new(),
            user_coauthor: String::new(),
            stream_tx: None,
            setup_script: String::new(),
            api_keys: HashMap::new(),
            disallowed_tools: String::new(),
            knowledge_files: vec![],
            knowledge_dir: String::new(),
            agent_network: None,
        }
    }

    fn write_prompt(dir: &TempDir, rel: &str, content: &str) {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    // Explicit prompt_file is returned instead of the worktree file.
    #[test]
    fn explicit_prompt_file_takes_priority() {
        let tmp = TempDir::new().unwrap();
        let explicit = tmp.path().join("explicit.txt");
        std::fs::write(&explicit, "explicit content").unwrap();
        write_prompt(&tmp, "wt/.borg/prompt.md", "worktree content");

        let repo_cfg = make_repo_config(
            &tmp.path().join("wt").to_string_lossy(),
            &explicit.to_string_lossy(),
        );
        let ctx = make_ctx(&tmp.path().join("wt").to_string_lossy(), repo_cfg);
        let result = read_repo_prompt(&ctx);
        assert_eq!(result.as_deref(), Some("explicit content"));
    }

    // Whitespace-only explicit file falls through to the worktree .borg/prompt.md.
    #[test]
    fn whitespace_only_prompt_file_falls_through_to_worktree() {
        let tmp = TempDir::new().unwrap();
        let explicit = tmp.path().join("blank.txt");
        std::fs::write(&explicit, "   \n\t  \n").unwrap();
        write_prompt(&tmp, "wt/.borg/prompt.md", "worktree prompt");

        let repo_cfg = make_repo_config(
            &tmp.path().join("wt").to_string_lossy(),
            &explicit.to_string_lossy(),
        );
        let ctx = make_ctx(&tmp.path().join("wt").to_string_lossy(), repo_cfg);
        let result = read_repo_prompt(&ctx);
        assert_eq!(result.as_deref(), Some("worktree prompt"));
    }

    // When worktree_path == repo_config.path the third lookup is skipped;
    // no double-read occurs and the result is None when no prompt exists.
    #[test]
    fn same_path_skips_third_lookup() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().to_string_lossy().into_owned();
        // No .borg/prompt.md written — both source 2 and (skipped) source 3 would be absent.
        let repo_cfg = make_repo_config(&dir, "");
        let ctx = make_ctx(&dir, repo_cfg);
        assert!(read_repo_prompt(&ctx).is_none());
    }

    // Confirm the third source IS used when paths differ.
    #[test]
    fn third_source_used_when_paths_differ() {
        let tmp = TempDir::new().unwrap();
        let wt = tmp.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        let repo_root = tmp.path().join("repo");
        write_prompt(&tmp, "repo/.borg/prompt.md", "repo root prompt");

        let repo_cfg = make_repo_config(&repo_root.to_string_lossy(), "");
        let ctx = make_ctx(&wt.to_string_lossy(), repo_cfg);
        let result = read_repo_prompt(&ctx);
        assert_eq!(result.as_deref(), Some("repo root prompt"));
    }
}
