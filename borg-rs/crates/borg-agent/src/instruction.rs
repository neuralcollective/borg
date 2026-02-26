use borg_core::types::{PhaseConfig, PhaseContext, Task};

/// Build the instruction string passed to any agent backend.
///
/// Composes task context, the phase instruction, an optional file listing,
/// error context from the previous attempt, and any pending user messages.
/// All backends use this so the prompt format stays consistent.
pub fn build_instruction(
    task: &Task,
    phase: &PhaseConfig,
    ctx: &PhaseContext,
    file_listing: Option<&str>,
) -> String {
    let mut s = String::new();

    if let Some(repo_prompt) = read_repo_prompt(ctx) {
        s.push_str("## Project Context\n\n");
        s.push_str(&repo_prompt);
        s.push_str("\n\n---\n\n");
    }

    if phase.include_task_context {
        s.push_str(&format!("Task: {}\n\n{}\n\n---\n\n", task.title, task.description));
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

/// Read the per-repo prompt from the explicit prompt_file config, or by
/// auto-detecting `.borg/prompt.md` in the worktree / repo root.
fn read_repo_prompt(ctx: &PhaseContext) -> Option<String> {
    // 1. Explicit prompt_file from config
    if !ctx.repo_config.prompt_file.is_empty() {
        if let Ok(content) = std::fs::read_to_string(&ctx.repo_config.prompt_file) {
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }

    // 2. .borg/prompt.md in the worktree (may differ from repo root during tasks)
    let worktree_prompt = format!("{}/.borg/prompt.md", ctx.worktree_path);
    if let Ok(content) = std::fs::read_to_string(&worktree_prompt) {
        let trimmed = content.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }

    // 3. .borg/prompt.md in the repo root
    let repo_prompt = format!("{}/.borg/prompt.md", ctx.repo_config.path);
    if repo_prompt != worktree_prompt {
        if let Ok(content) = std::fs::read_to_string(&repo_prompt) {
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }

    None
}
