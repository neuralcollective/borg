use borg_agent::instruction::build_instruction;
use borg_core::{
    db::KnowledgeFile,
    types::{PhaseConfig, PhaseContext, RepoConfig, Task},
};
use chrono::Utc;
use std::collections::HashMap;

fn make_task() -> Task {
    Task {
        id: 1,
        title: "My Task".into(),
        description: "Task desc.".into(),
        repo_path: "/repo".into(),
        branch: "main".into(),
        status: "implement".into(),
        attempt: 1,
        max_attempts: 3,
        last_error: String::new(),
        created_by: "test".into(),
        notify_chat: String::new(),
        created_at: Utc::now(),
        session_id: String::new(),
        mode: "sweborg".into(),
        backend: String::new(),
        project_id: 0,
        task_type: String::new(),
    }
}

fn make_phase() -> PhaseConfig {
    PhaseConfig {
        instruction: "Do the work.".into(),
        ..Default::default()
    }
}

fn make_ctx() -> PhaseContext {
    PhaseContext {
        task: make_task(),
        repo_config: RepoConfig {
            path: "/nonexistent-repo".into(),
            test_cmd: String::new(),
            prompt_file: String::new(),
            mode: "sweborg".into(),
            is_self: false,
            auto_merge: false,
            lint_cmd: String::new(),
            backend: String::new(),
            repo_slug: String::new(),
        },
        data_dir: String::new(),
        session_dir: String::new(),
        worktree_path: "/nonexistent-worktree".into(),
        oauth_token: String::new(),
        model: String::new(),
        pending_messages: Vec::new(),
        system_prompt_suffix: String::new(),
        user_coauthor: String::new(),
        stream_tx: None,
        setup_script: String::new(),
        api_keys: HashMap::new(),
        disallowed_tools: String::new(),
        knowledge_files: Vec::new(),
        knowledge_dir: String::new(),
        agent_network: None,
    }
}

fn make_knowledge_file(file_name: &str, description: &str, inline: bool) -> KnowledgeFile {
    KnowledgeFile {
        id: 1,
        file_name: file_name.into(),
        description: description.into(),
        size_bytes: 0,
        inline,
        created_at: String::new(),
    }
}

// ── Phase instruction is always present ──────────────────────────────────────

#[test]
fn test_phase_instruction_always_present() {
    let task = make_task();
    let phase = PhaseConfig {
        instruction: "Write unit tests.".into(),
        ..Default::default()
    };
    let ctx = make_ctx();
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(result.contains("Write unit tests."), "instruction missing from output");
}

// ── Task context included/excluded ────────────────────────────────────────────

#[test]
fn test_task_context_included() {
    let task = make_task();
    let phase = PhaseConfig {
        instruction: "Do it.".into(),
        include_task_context: true,
        ..Default::default()
    };
    let ctx = make_ctx();
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(result.contains("My Task"), "task title missing");
    assert!(result.contains("Task desc."), "task description missing");
}

#[test]
fn test_task_context_excluded() {
    let task = make_task();
    let phase = PhaseConfig {
        instruction: "Do it.".into(),
        include_task_context: false,
        ..Default::default()
    };
    let ctx = make_ctx();
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(!result.contains("My Task"), "task title should not appear");
    assert!(!result.contains("Task desc."), "task description should not appear");
}

// ── File listing included/excluded ────────────────────────────────────────────

#[test]
fn test_file_listing_included() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx();
    let result = build_instruction(&task, &phase, &ctx, Some("src/main.rs\nsrc/lib.rs\n"));
    assert!(result.contains("src/main.rs"), "file listing missing");
    assert!(result.contains("Files in repository:"), "file listing header missing");
}

#[test]
fn test_file_listing_excluded_when_none() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx();
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(!result.contains("Files in repository:"), "should have no file listing");
}

#[test]
fn test_file_listing_excluded_when_empty() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx();
    let result = build_instruction(&task, &phase, &ctx, Some(""));
    assert!(!result.contains("Files in repository:"), "empty file listing should be omitted");
}

// ── {ERROR} substitution ──────────────────────────────────────────────────────

#[test]
fn test_error_substitution() {
    let mut task = make_task();
    task.last_error = "compilation failed: expected `;`".into();
    let phase = PhaseConfig {
        instruction: "Fix the code.".into(),
        error_instruction: "Previous error: {ERROR}".into(),
        ..Default::default()
    };
    let ctx = make_ctx();
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(
        result.contains("Previous error: compilation failed: expected `;`"),
        "{{ERROR}} not replaced; got: {result}"
    );
    assert!(!result.contains("{ERROR}"), "raw {{ERROR}} placeholder should be gone");
}

#[test]
fn test_error_substitution_multiple_placeholders() {
    let mut task = make_task();
    task.last_error = "panic at main.rs:10".into();
    let phase = PhaseConfig {
        instruction: "Retry.".into(),
        error_instruction: "Error: {ERROR}. Fix this: {ERROR}".into(),
        ..Default::default()
    };
    let ctx = make_ctx();
    let result = build_instruction(&task, &phase, &ctx, None);
    // Both occurrences replaced
    assert_eq!(
        result.matches("panic at main.rs:10").count(),
        2,
        "both {{ERROR}} placeholders should be replaced"
    );
}

#[test]
fn test_error_section_excluded_when_no_error() {
    let task = make_task(); // last_error is empty
    let phase = PhaseConfig {
        instruction: "Do it.".into(),
        error_instruction: "Previous error: {ERROR}".into(),
        ..Default::default()
    };
    let ctx = make_ctx();
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(!result.contains("Previous error:"), "error section should be absent when no error");
}

#[test]
fn test_error_section_excluded_when_no_error_instruction() {
    let mut task = make_task();
    task.last_error = "some error".into();
    let phase = PhaseConfig {
        instruction: "Do it.".into(),
        error_instruction: String::new(),
        ..Default::default()
    };
    let ctx = make_ctx();
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(!result.contains("some error"), "error should not appear without error_instruction");
}

// ── Pending messages ──────────────────────────────────────────────────────────

#[test]
fn test_pending_messages_formatted_with_role_prefix() {
    let task = make_task();
    let phase = make_phase();
    let mut ctx = make_ctx();
    ctx.pending_messages = vec![
        ("user".into(), "Please add tests.".into()),
        ("director".into(), "Focus on edge cases.".into()),
    ];
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(result.contains("[user]: Please add tests."), "user message missing");
    assert!(result.contains("[director]: Focus on edge cases."), "director message missing");
}

#[test]
fn test_pending_messages_header_present() {
    let task = make_task();
    let phase = make_phase();
    let mut ctx = make_ctx();
    ctx.pending_messages = vec![("user".into(), "A question.".into())];
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(result.contains("messages were sent"), "pending messages header missing");
}

#[test]
fn test_pending_messages_excluded_when_empty() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx();
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(!result.contains("messages were sent"), "pending messages section should be absent");
}

// ── Section separators ────────────────────────────────────────────────────────

#[test]
fn test_separator_between_task_context_and_instruction() {
    let task = make_task();
    let phase = PhaseConfig {
        instruction: "Do the work.".into(),
        include_task_context: true,
        ..Default::default()
    };
    let ctx = make_ctx();
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(result.contains("---"), "separator missing");
    let task_pos = result.find("Task:").expect("task context missing");
    let sep_pos = result.find("---").expect("separator missing");
    let instr_pos = result.find("Do the work.").expect("instruction missing");
    assert!(task_pos < sep_pos, "task context should come before separator");
    assert!(sep_pos < instr_pos, "separator should come before instruction");
}

#[test]
fn test_separator_before_file_listing() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx();
    let result = build_instruction(&task, &phase, &ctx, Some("foo.rs\n"));
    let listing_pos = result.find("Files in repository:").expect("listing missing");
    // There must be a `---` somewhere before the listing
    result[..listing_pos].find("---").expect("separator before file listing missing");
}

// ── Knowledge section included/excluded ───────────────────────────────────────

#[test]
fn test_knowledge_section_included_with_non_inline_file() {
    let task = make_task();
    let phase = make_phase();
    let mut ctx = make_ctx();
    ctx.knowledge_files = vec![make_knowledge_file("style_guide.md", "Coding standards", false)];
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(result.contains("## Knowledge Base"), "knowledge section missing");
    assert!(result.contains("style_guide.md"), "knowledge file name missing");
    assert!(result.contains("Coding standards"), "knowledge description missing");
}

#[test]
fn test_knowledge_section_excluded_when_no_files() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx();
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(!result.contains("## Knowledge Base"), "knowledge section should be absent");
}

#[test]
fn test_knowledge_section_separator_present() {
    let task = make_task();
    let phase = make_phase();
    let mut ctx = make_ctx();
    ctx.knowledge_files = vec![make_knowledge_file("kb.md", "KB", false)];
    let result = build_instruction(&task, &phase, &ctx, None);
    // After the knowledge section there must be a separator before the instruction
    let kb_pos = result.find("## Knowledge Base").expect("KB missing");
    let instr_pos = result.find("Do the work.").expect("instruction missing");
    let sep_in_between = result[kb_pos..instr_pos].contains("---");
    assert!(sep_in_between, "separator between KB section and instruction missing");
}

#[test]
fn test_knowledge_section_comes_first() {
    let task = make_task();
    let phase = PhaseConfig {
        instruction: "Do it.".into(),
        include_task_context: true,
        ..Default::default()
    };
    let mut ctx = make_ctx();
    ctx.knowledge_files = vec![make_knowledge_file("api.md", "API reference", false)];
    let result = build_instruction(&task, &phase, &ctx, None);
    let kb_pos = result.find("## Knowledge Base").expect("KB section missing");
    let task_pos = result.find("Task:").expect("task context missing");
    assert!(kb_pos < task_pos, "knowledge base should come before task context");
}

// ── Repo prompt included/excluded ─────────────────────────────────────────────

#[test]
fn test_repo_prompt_excluded_when_no_file() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx(); // worktree_path is /nonexistent-worktree, no prompt.md
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(!result.contains("## Project Context"), "repo prompt section should be absent");
}

#[test]
fn test_repo_prompt_included_when_file_exists() {
    let tmp = std::env::temp_dir().join("borg-test-build-instruction-prompt");
    let borg_dir = tmp.join(".borg");
    std::fs::create_dir_all(&borg_dir).expect("create .borg dir");
    std::fs::write(borg_dir.join("prompt.md"), "Repo context for tests.").expect("write prompt.md");

    let task = make_task();
    let phase = make_phase();
    let mut ctx = make_ctx();
    ctx.worktree_path = tmp.to_string_lossy().into_owned();
    ctx.repo_config.path = "/different-path".into();

    let result = build_instruction(&task, &phase, &ctx, None);
    let _ = std::fs::remove_dir_all(&tmp);

    assert!(result.contains("## Project Context"), "repo prompt section missing");
    assert!(result.contains("Repo context for tests."), "repo prompt content missing");
}

// ── Full section ordering ─────────────────────────────────────────────────────

#[test]
fn test_full_section_ordering() {
    let tmp = std::env::temp_dir().join("borg-test-section-ordering");
    let borg_dir = tmp.join(".borg");
    std::fs::create_dir_all(&borg_dir).expect("create .borg dir");
    std::fs::write(borg_dir.join("prompt.md"), "Repo context.").expect("write prompt.md");

    let mut task = make_task();
    task.last_error = "build error".into();
    let phase = PhaseConfig {
        instruction: "Phase instruction.".into(),
        include_task_context: true,
        error_instruction: "Error was: {ERROR}".into(),
        ..Default::default()
    };
    let mut ctx = make_ctx();
    ctx.worktree_path = tmp.to_string_lossy().into_owned();
    ctx.repo_config.path = "/different".into();
    ctx.knowledge_files = vec![make_knowledge_file("kb.md", "KB desc", false)];
    ctx.pending_messages = vec![("user".into(), "Check this.".into())];

    let result = build_instruction(&task, &phase, &ctx, Some("src/main.rs\n"));
    let _ = std::fs::remove_dir_all(&tmp);

    let kb_pos = result.find("## Knowledge Base").expect("KB missing");
    let repo_pos = result.find("## Project Context").expect("repo prompt missing");
    let task_pos = result.find("Task:").expect("task context missing");
    let instr_pos = result.find("Phase instruction.").expect("instruction missing");
    let files_pos = result.find("Files in repository:").expect("file listing missing");
    let error_pos = result.find("Error was: build error").expect("error section missing");
    let msg_pos = result.find("[user]: Check this.").expect("pending message missing");

    assert!(kb_pos < repo_pos, "KB before repo prompt");
    assert!(repo_pos < task_pos, "repo prompt before task context");
    assert!(task_pos < instr_pos, "task context before instruction");
    assert!(instr_pos < files_pos, "instruction before file listing");
    assert!(files_pos < error_pos, "file listing before error section");
    assert!(error_pos < msg_pos, "error section before pending messages");
}
