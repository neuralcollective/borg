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
        title: "Test Task".to_string(),
        description: "Test description.".to_string(),
        repo_path: "/nonexistent".to_string(),
        branch: "task-1".to_string(),
        status: "implement".to_string(),
        attempt: 1,
        max_attempts: 3,
        last_error: String::new(),
        created_by: "test".to_string(),
        notify_chat: String::new(),
        created_at: Utc::now(),
        session_id: String::new(),
        mode: "sweborg".to_string(),
        backend: String::new(),
        project_id: 0,
        task_type: String::new(),
        started_at: None,
        completed_at: None,
        duration_secs: None,
        review_status: None,
        revision_count: 0,
    }
}

fn make_repo_config() -> RepoConfig {
    RepoConfig {
        path: "/nonexistent-repo".to_string(),
        test_cmd: String::new(),
        prompt_file: String::new(),
        mode: "sweborg".to_string(),
        is_self: false,
        auto_merge: false,
        lint_cmd: String::new(),
        backend: String::new(),
        repo_slug: String::new(),
    }
}

fn make_ctx() -> PhaseContext {
    PhaseContext {
        task: make_task(),
        repo_config: make_repo_config(),
        data_dir: "/tmp".to_string(),
        session_dir: "/tmp".to_string(),
        // Same as repo_config.path so the third prompt-file fallback is skipped.
        worktree_path: "/nonexistent-repo".to_string(),
        oauth_token: String::new(),
        model: "sonnet".to_string(),
        pending_messages: vec![],
        system_prompt_suffix: String::new(),
        user_coauthor: String::new(),
        stream_tx: None,
        setup_script: String::new(),
        api_keys: HashMap::new(),
        disallowed_tools: String::new(),
        knowledge_files: vec![],
        knowledge_dir: "/tmp".to_string(),
        agent_network: None,
        prior_research: vec![],
    }
}

fn make_knowledge_file(name: &str) -> KnowledgeFile {
    KnowledgeFile {
        id: 1,
        file_name: name.to_string(),
        description: "A test file".to_string(),
        size_bytes: 100,
        inline: false,
        tags: String::new(),
        category: String::new(),
        jurisdiction: String::new(),
        project_id: None,
        created_at: "2024-01-01".to_string(),
    }
}

// ── knowledge section ─────────────────────────────────────────────────────────

#[test]
fn knowledge_section_omitted_when_empty() {
    let task = make_task();
    let phase = PhaseConfig::default();
    let ctx = make_ctx(); // knowledge_files: vec![]

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(
        !out.contains("Knowledge Base"),
        "knowledge section must not appear when knowledge_files is empty"
    );
}

#[test]
fn knowledge_section_present_when_files_exist() {
    let task = make_task();
    let phase = PhaseConfig::default();
    let mut ctx = make_ctx();
    ctx.knowledge_files = vec![make_knowledge_file("guidelines.md")];

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(
        out.contains("Knowledge Base"),
        "knowledge section must appear when files are present"
    );
}

// ── error_instruction / {ERROR} placeholder ───────────────────────────────────

#[test]
fn error_placeholder_replaced_by_last_error() {
    let mut task = make_task();
    task.last_error = "compilation failed on line 42".to_string();
    let mut phase = PhaseConfig::default();
    phase.error_instruction = "Previous attempt failed: {ERROR}".to_string();
    let ctx = make_ctx();

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(
        out.contains("Previous attempt failed: compilation failed on line 42"),
        "{{ERROR}} must be replaced with task.last_error"
    );
    assert!(
        !out.contains("{ERROR}"),
        "literal {{ERROR}} must not remain in output"
    );
}

#[test]
fn error_instruction_omitted_when_last_error_empty() {
    let task = make_task(); // last_error is ""
    let mut phase = PhaseConfig::default();
    phase.error_instruction = "Previous attempt failed: {ERROR}".to_string();
    let ctx = make_ctx();

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(
        !out.contains("Previous attempt failed:"),
        "error_instruction must not appear when last_error is empty"
    );
}

// ── prior_research numbered list ──────────────────────────────────────────────

#[test]
fn prior_research_appears_as_numbered_list() {
    let task = make_task();
    let phase = PhaseConfig::default();
    let mut ctx = make_ctx();
    ctx.prior_research = vec![
        "First research chunk".to_string(),
        "Second research chunk".to_string(),
    ];

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(out.contains("1. First research chunk"), "first entry must be numbered 1");
    assert!(out.contains("2. Second research chunk"), "second entry must be numbered 2");
}

#[test]
fn prior_research_section_absent_when_empty() {
    let task = make_task();
    let phase = PhaseConfig::default();
    let ctx = make_ctx(); // prior_research: vec![]

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(
        !out.contains("Prior Research"),
        "prior research section must not appear when list is empty"
    );
}

// ── pending_messages formatting ───────────────────────────────────────────────

#[test]
fn pending_messages_formatted_as_role_colon_content() {
    let task = make_task();
    let phase = PhaseConfig::default();
    let mut ctx = make_ctx();
    ctx.pending_messages = vec![
        ("user".to_string(), "Focus on edge cases.".to_string()),
        ("director".to_string(), "Skip the migration step.".to_string()),
    ];

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(out.contains("[user]: Focus on edge cases."), "user message must be formatted");
    assert!(
        out.contains("[director]: Skip the migration step."),
        "director message must be formatted"
    );
}

#[test]
fn pending_messages_absent_when_empty() {
    let task = make_task();
    let phase = PhaseConfig::default();
    let ctx = make_ctx(); // pending_messages: vec![]

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(
        !out.contains("messages were sent"),
        "pending messages header must not appear when list is empty"
    );
}

// ── include_task_context ──────────────────────────────────────────────────────

#[test]
fn task_context_omitted_when_include_task_context_false() {
    let mut task = make_task();
    task.title = "UniqueTaskTitleXYZ".to_string();
    task.description = "UniqueDescriptionTextABC.".to_string();
    let phase = PhaseConfig::default(); // include_task_context: false
    let ctx = make_ctx();

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(
        !out.contains("UniqueTaskTitleXYZ"),
        "task title must not appear when include_task_context is false"
    );
    assert!(
        !out.contains("UniqueDescriptionTextABC"),
        "task description must not appear when include_task_context is false"
    );
}

#[test]
fn task_context_included_when_include_task_context_true() {
    let mut task = make_task();
    task.title = "UniqueTaskTitleXYZ".to_string();
    task.description = "UniqueDescriptionTextABC.".to_string();
    let mut phase = PhaseConfig::default();
    phase.include_task_context = true;
    let ctx = make_ctx();

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(out.contains("UniqueTaskTitleXYZ"), "task title must appear when include_task_context is true");
    assert!(
        out.contains("UniqueDescriptionTextABC"),
        "task description must appear when include_task_context is true"
    );
}
