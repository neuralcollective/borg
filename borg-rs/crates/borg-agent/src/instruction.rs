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
        let stored = if file.stored_path.is_empty() { &file.file_name } else { &file.stored_path };
        if file.inline {
            let path = format!("{}/{}", knowledge_dir, stored);
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
            s.push_str(&format!("- `/knowledge/{}`", stored));
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
