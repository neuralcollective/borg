use borg_agent::instruction::build_instruction;
use borg_core::{
    db::KnowledgeFile,
    types::{PhaseConfig, PhaseContext, RepoConfig, Task},
};
use chrono::Utc;

fn make_task(title: &str, description: &str, last_error: &str) -> Task {
    Task {
        id: 1,
        title: title.to_string(),
        description: description.to_string(),
        repo_path: String::new(),
        branch: String::new(),
        status: "impl".to_string(),
        attempt: 1,
        max_attempts: 3,
        last_error: last_error.to_string(),
        created_by: String::new(),
        notify_chat: String::new(),
        created_at: Utc::now(),
        session_id: String::new(),
        mode: String::new(),
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

fn make_ctx() -> PhaseContext {
    PhaseContext {
        task: make_task("", "", ""),
        repo_config: RepoConfig {
            path: "/nonexistent".to_string(),
            test_cmd: String::new(),
            prompt_file: String::new(),
            mode: String::new(),
            is_self: false,
            auto_merge: false,
            lint_cmd: String::new(),
            backend: String::new(),
            repo_slug: String::new(),
        },
        data_dir: String::new(),
        session_dir: String::new(),
        work_dir: "/nonexistent".to_string(),
        oauth_token: String::new(),
        model: String::new(),
        pending_messages: vec![],
        system_prompt_suffix: String::new(),
        user_coauthor: String::new(),
        stream_tx: None,
        setup_script: String::new(),
        api_keys: Default::default(),
        disallowed_tools: String::new(),
        knowledge_files: vec![],
        knowledge_dir: String::new(),
        agent_network: None,
        prior_research: vec![],
        revision_count: 0,
        experimental_domains: false,
    }
}

fn make_knowledge_file(file_name: &str, description: &str) -> KnowledgeFile {
    KnowledgeFile {
        id: 1,
        file_name: file_name.to_string(),
        description: description.to_string(),
        size_bytes: 100,
        inline: false,
        tags: String::new(),
        category: String::new(),
        jurisdiction: String::new(),
        project_id: None,
        created_at: String::new(),
    }
}

// =============================================================================
// Knowledge section: prepended with separator when non-empty
// =============================================================================

#[test]
fn test_knowledge_section_prepended_when_non_empty() {
    let task = make_task("Title", "Desc", "");
    let phase = PhaseConfig {
        instruction: "Do the thing.".to_string(),
        ..PhaseConfig::default()
    };
    let mut ctx = make_ctx();
    ctx.knowledge_files = vec![make_knowledge_file("guide.md", "A guide")];

    let result = build_instruction(&task, &phase, &ctx, None);

    assert!(
        result.starts_with("## Knowledge Base"),
        "knowledge section should be at start: {result}"
    );
    let sep_pos = result.find("\n\n---\n\n").expect("separator must follow knowledge section");
    let inst_pos = result.find("Do the thing.").expect("instruction must be present");
    assert!(sep_pos < inst_pos, "separator must appear before the instruction");
}

#[test]
fn test_no_knowledge_section_when_empty() {
    let task = make_task("Title", "Desc", "");
    let phase = PhaseConfig {
        instruction: "Do the thing.".to_string(),
        ..PhaseConfig::default()
    };
    let ctx = make_ctx();

    let result = build_instruction(&task, &phase, &ctx, None);

    assert!(
        !result.contains("## Knowledge Base"),
        "no knowledge section when files are empty: {result}"
    );
}

// =============================================================================
// include_task_context: controls presence of task title and description
// =============================================================================

#[test]
fn test_include_task_context_true_shows_title_and_description() {
    let task = make_task("My Task Title", "Task description here.", "");
    let phase = PhaseConfig {
        instruction: "Phase instruction.".to_string(),
        include_task_context: true,
        ..PhaseConfig::default()
    };
    let ctx = make_ctx();

    let result = build_instruction(&task, &phase, &ctx, None);

    assert!(result.contains("Task: My Task Title"), "expected task title: {result}");
    assert!(result.contains("Task description here."), "expected task description: {result}");
}

#[test]
fn test_include_task_context_false_omits_title_and_description() {
    let task = make_task("My Task Title", "Task description here.", "");
    let phase = PhaseConfig {
        instruction: "Phase instruction.".to_string(),
        include_task_context: false,
        ..PhaseConfig::default()
    };
    let ctx = make_ctx();

    let result = build_instruction(&task, &phase, &ctx, None);

    assert!(!result.contains("My Task Title"), "title should be absent: {result}");
    assert!(!result.contains("Task description here."), "description should be absent: {result}");
}

// =============================================================================
// last_error + error_instruction: {ERROR} placeholder substitution
// =============================================================================

#[test]
fn test_error_placeholder_substituted() {
    let task = make_task("Title", "Desc", "test suite panicked");
    let phase = PhaseConfig {
        instruction: "Do work.".to_string(),
        error_instruction: "Previous attempt failed: {ERROR}".to_string(),
        ..PhaseConfig::default()
    };
    let ctx = make_ctx();

    let result = build_instruction(&task, &phase, &ctx, None);

    assert!(
        result.contains("Previous attempt failed: test suite panicked"),
        "expected substituted error: {result}"
    );
}

#[test]
fn test_no_error_section_when_last_error_empty() {
    let task = make_task("Title", "Desc", "");
    let phase = PhaseConfig {
        instruction: "Do work.".to_string(),
        error_instruction: "Previous attempt failed: {ERROR}".to_string(),
        ..PhaseConfig::default()
    };
    let ctx = make_ctx();

    let result = build_instruction(&task, &phase, &ctx, None);

    assert!(
        !result.contains("Previous attempt failed"),
        "no error section when last_error is empty: {result}"
    );
}

#[test]
fn test_no_error_section_when_error_instruction_empty() {
    let task = make_task("Title", "Desc", "some error occurred");
    let phase = PhaseConfig {
        instruction: "Do work.".to_string(),
        // error_instruction is empty (default)
        ..PhaseConfig::default()
    };
    let ctx = make_ctx();

    let result = build_instruction(&task, &phase, &ctx, None);

    assert!(
        !result.contains("some error occurred"),
        "no error section when error_instruction is empty: {result}"
    );
}

// =============================================================================
// Revision messages: include revision header and count
// =============================================================================

#[test]
fn test_revision_messages_include_header_and_count() {
    let task = make_task("Title", "Desc", "");
    let phase = PhaseConfig {
        instruction: "Write document.".to_string(),
        ..PhaseConfig::default()
    };
    let mut ctx = make_ctx();
    ctx.revision_count = 3;
    ctx.pending_messages = vec![("reviewer".to_string(), "Fix section 2.".to_string())];

    let result = build_instruction(&task, &phase, &ctx, None);

    assert!(result.contains("Revision #3"), "expected revision header with count: {result}");
    assert!(result.contains("Reviewer Feedback"), "expected reviewer feedback header: {result}");
    assert!(result.contains("Fix section 2."), "expected message content: {result}");
    assert!(result.contains("[reviewer]"), "expected role in message: {result}");
}

#[test]
fn test_revision_count_reflected_in_header() {
    let task = make_task("Title", "Desc", "");
    let phase = PhaseConfig {
        instruction: "Write document.".to_string(),
        ..PhaseConfig::default()
    };
    let mut ctx = make_ctx();
    ctx.revision_count = 1;
    ctx.pending_messages = vec![("reviewer".to_string(), "Add citations.".to_string())];

    let result = build_instruction(&task, &phase, &ctx, None);

    assert!(result.contains("Revision #1"), "revision count should be 1: {result}");
}

// =============================================================================
// Queue messages: simpler prefix format (revision_count == 0)
// =============================================================================

#[test]
fn test_queue_messages_use_simple_prefix_format() {
    let task = make_task("Title", "Desc", "");
    let phase = PhaseConfig {
        instruction: "Do work.".to_string(),
        ..PhaseConfig::default()
    };
    let mut ctx = make_ctx();
    ctx.revision_count = 0;
    ctx.pending_messages = vec![("user".to_string(), "Please add more tests.".to_string())];

    let result = build_instruction(&task, &phase, &ctx, None);

    assert!(
        result.contains("[user]: Please add more tests."),
        "expected queue message format: {result}"
    );
    assert!(!result.contains("Revision #"), "no revision header for queue messages: {result}");
    assert!(
        !result.contains("Reviewer Feedback"),
        "no reviewer feedback header for queue messages: {result}"
    );
}

#[test]
fn test_queue_messages_intro_text() {
    let task = make_task("Title", "Desc", "");
    let phase = PhaseConfig {
        instruction: "Do work.".to_string(),
        ..PhaseConfig::default()
    };
    let mut ctx = make_ctx();
    ctx.revision_count = 0;
    ctx.pending_messages = vec![("director".to_string(), "Prioritize speed.".to_string())];

    let result = build_instruction(&task, &phase, &ctx, None);

    assert!(
        result.contains("messages were sent by the user or director"),
        "expected queue intro text: {result}"
    );
}
