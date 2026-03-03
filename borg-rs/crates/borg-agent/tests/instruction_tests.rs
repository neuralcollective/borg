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
        title: "Test Task".into(),
        description: "Task description.".into(),
        repo_path: "/tmp/test-repo".into(),
        branch: "test-branch".into(),
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
        started_at: None,
        completed_at: None,
        duration_secs: None,
        review_status: None,
        revision_count: 0,
    }
}

fn make_phase() -> PhaseConfig {
    PhaseConfig {
        name: "implement".into(),
        label: "Implement".into(),
        instruction: "Write the implementation.".into(),
        ..PhaseConfig::default()
    }
}

fn make_ctx(worktree: &str) -> PhaseContext {
    PhaseContext {
        task: make_task(),
        repo_config: RepoConfig {
            path: worktree.to_string(),
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
        worktree_path: worktree.to_string(),
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
        prior_research: vec![],
        revision_count: 0,
    }
}

fn kb_file(name: &str, desc: &str, inline: bool) -> KnowledgeFile {
    KnowledgeFile {
        id: 1,
        file_name: name.into(),
        description: desc.into(),
        size_bytes: 0,
        inline,
        tags: String::new(),
        category: String::new(),
        jurisdiction: String::new(),
        project_id: None,
        created_at: String::new(),
    }
}

// =============================================================================
// Base case: instruction only
// =============================================================================

#[test]
fn test_instruction_only() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx("/nonexistent-path-for-tests");
    let result = build_instruction(&task, &phase, &ctx, None);
    assert_eq!(result, "Write the implementation.");
}

// =============================================================================
// Task context
// =============================================================================

#[test]
fn test_task_context_included() {
    let task = make_task();
    let phase = PhaseConfig {
        include_task_context: true,
        ..make_phase()
    };
    let ctx = make_ctx("/nonexistent-path-for-tests");
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(result.contains("Task: Test Task"), "title missing: {result}");
    assert!(result.contains("Task description."), "description missing: {result}");
    let title_pos = result.find("Task: Test Task").unwrap();
    let instr_pos = result.find("Write the implementation.").unwrap();
    assert!(title_pos < instr_pos, "task context must precede instruction");
}

#[test]
fn test_task_context_omitted_by_default() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx("/nonexistent-path-for-tests");
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(!result.contains("Task: Test Task"));
    assert!(!result.contains("Task description."));
}

// =============================================================================
// File listing
// =============================================================================

#[test]
fn test_file_listing_included() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx("/nonexistent-path-for-tests");
    let result = build_instruction(&task, &phase, &ctx, Some("src/main.rs\nsrc/lib.rs\n"));
    assert!(result.contains("Files in repository:"), "header missing: {result}");
    assert!(result.contains("src/main.rs"), "file missing: {result}");
    let instr_pos = result.find("Write the implementation.").unwrap();
    let files_pos = result.find("Files in repository:").unwrap();
    assert!(instr_pos < files_pos, "instruction must precede file listing");
}

#[test]
fn test_empty_file_listing_omitted() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx("/nonexistent-path-for-tests");
    let result = build_instruction(&task, &phase, &ctx, Some(""));
    assert!(!result.contains("Files in repository:"), "empty listing must be omitted: {result}");
}

#[test]
fn test_none_file_listing_omitted() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx("/nonexistent-path-for-tests");
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(!result.contains("Files in repository:"));
}

// =============================================================================
// Error instruction with {ERROR} substitution
// =============================================================================

#[test]
fn test_error_instruction_with_substitution() {
    let mut task = make_task();
    task.last_error = "compilation failed: undefined symbol".into();
    let phase = PhaseConfig {
        error_instruction: "Fix this error: {ERROR}".into(),
        ..make_phase()
    };
    let ctx = make_ctx("/nonexistent-path-for-tests");
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(
        result.contains("Fix this error: compilation failed: undefined symbol"),
        "error not substituted: {result}"
    );
    let instr_pos = result.find("Write the implementation.").unwrap();
    let err_pos = result.find("Fix this error:").unwrap();
    assert!(instr_pos < err_pos, "instruction must precede error instruction");
}

#[test]
fn test_error_instruction_omitted_when_no_error() {
    let task = make_task(); // last_error is empty
    let phase = PhaseConfig {
        error_instruction: "Fix this error: {ERROR}".into(),
        ..make_phase()
    };
    let ctx = make_ctx("/nonexistent-path-for-tests");
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(!result.contains("Fix this error:"), "error instr must be omitted: {result}");
}

#[test]
fn test_error_instruction_omitted_when_no_template() {
    let mut task = make_task();
    task.last_error = "some error".into();
    let phase = make_phase(); // error_instruction is empty
    let ctx = make_ctx("/nonexistent-path-for-tests");
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(!result.contains("some error"), "error must not appear without template: {result}");
}

// =============================================================================
// Prior research
// =============================================================================

#[test]
fn test_prior_research_included() {
    let task = make_task();
    let phase = make_phase();
    let mut ctx = make_ctx("/nonexistent-path-for-tests");
    ctx.prior_research = vec!["Research finding A.".into(), "Research finding B.".into()];
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(result.contains("Prior Research"), "header missing: {result}");
    assert!(result.contains("Research finding A."), "chunk 1 missing: {result}");
    assert!(result.contains("Research finding B."), "chunk 2 missing: {result}");
    let instr_pos = result.find("Write the implementation.").unwrap();
    let research_pos = result.find("Prior Research").unwrap();
    assert!(instr_pos < research_pos, "instruction must precede prior research");
}

#[test]
fn test_prior_research_omitted_when_empty() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx("/nonexistent-path-for-tests");
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(!result.contains("Prior Research"));
}

// =============================================================================
// Pending messages — regular (revision_count == 0)
// =============================================================================

#[test]
fn test_pending_messages_regular() {
    let task = make_task();
    let phase = make_phase();
    let mut ctx = make_ctx("/nonexistent-path-for-tests");
    ctx.pending_messages = vec![("user".into(), "Please add logging.".into())];
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(result.contains("[user]: Please add logging."), "message missing: {result}");
    assert!(result.contains("messages were sent"), "regular header missing: {result}");
    assert!(!result.contains("Revision #"), "must not use revision format: {result}");
}

#[test]
fn test_pending_messages_multiple_regular() {
    let task = make_task();
    let phase = make_phase();
    let mut ctx = make_ctx("/nonexistent-path-for-tests");
    ctx.pending_messages = vec![
        ("user".into(), "First message.".into()),
        ("director".into(), "Second message.".into()),
    ];
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(result.contains("[user]: First message."), "first missing: {result}");
    assert!(result.contains("[director]: Second message."), "second missing: {result}");
}

#[test]
fn test_pending_messages_omitted_when_empty() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx("/nonexistent-path-for-tests");
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(!result.contains("messages were sent"));
    assert!(!result.contains("Revision #"));
}

// =============================================================================
// Pending messages — revision (revision_count > 0)
// =============================================================================

#[test]
fn test_pending_messages_revision() {
    let task = make_task();
    let phase = make_phase();
    let mut ctx = make_ctx("/nonexistent-path-for-tests");
    ctx.revision_count = 2;
    ctx.pending_messages = vec![("reviewer".into(), "Section 3 needs more detail.".into())];
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(result.contains("Revision #2"), "revision header missing: {result}");
    assert!(
        result.contains("**[reviewer]**: Section 3 needs more detail."),
        "message missing: {result}"
    );
    assert!(
        result.contains("IMPORTANT: Focus on the reviewer's feedback."),
        "revision footer missing: {result}"
    );
    assert!(!result.contains("messages were sent"), "must not use regular format: {result}");
}

#[test]
fn test_revision_count_one() {
    let task = make_task();
    let phase = make_phase();
    let mut ctx = make_ctx("/nonexistent-path-for-tests");
    ctx.revision_count = 1;
    ctx.pending_messages = vec![("reviewer".into(), "Fix the intro.".into())];
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(result.contains("Revision #1"), "revision #1 missing: {result}");
}

// =============================================================================
// Repo prompt injection
// =============================================================================

#[test]
fn test_repo_prompt_injected() {
    let tmp = std::env::temp_dir().join("borg-instr-test-repo-prompt");
    std::fs::create_dir_all(tmp.join(".borg")).unwrap();
    std::fs::write(tmp.join(".borg/prompt.md"), "This is the repo prompt.").unwrap();
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx(tmp.to_str().unwrap());
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(result.contains("## Project Context"), "header missing: {result}");
    assert!(result.contains("This is the repo prompt."), "content missing: {result}");
    let prompt_pos = result.find("## Project Context").unwrap();
    let instr_pos = result.find("Write the implementation.").unwrap();
    assert!(prompt_pos < instr_pos, "repo prompt must precede instruction");
}

#[test]
fn test_repo_prompt_omitted_when_absent() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx("/nonexistent-path-for-tests");
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(!result.contains("## Project Context"));
}

// =============================================================================
// Knowledge section
// =============================================================================

#[test]
fn test_knowledge_section_non_inline() {
    let task = make_task();
    let phase = make_phase();
    let mut ctx = make_ctx("/nonexistent-path-for-tests");
    ctx.knowledge_files = vec![kb_file("guide.md", "Style guide", false)];
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(result.contains("## Knowledge Base"), "KB header missing: {result}");
    assert!(result.contains("guide.md"), "file name missing: {result}");
    assert!(result.contains("Style guide"), "description missing: {result}");
    let kb_pos = result.find("## Knowledge Base").unwrap();
    let instr_pos = result.find("Write the implementation.").unwrap();
    assert!(kb_pos < instr_pos, "knowledge section must precede instruction");
}

#[test]
fn test_knowledge_section_omitted_when_empty() {
    let task = make_task();
    let phase = make_phase();
    let ctx = make_ctx("/nonexistent-path-for-tests");
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(!result.contains("## Knowledge Base"));
}

#[test]
fn test_knowledge_inline_file_content_embedded() {
    let tmp = std::env::temp_dir().join("borg-instr-test-kb-inline");
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("guide.md"), "Always use snake_case.").unwrap();
    let task = make_task();
    let phase = make_phase();
    let mut ctx = make_ctx("/nonexistent-path-for-tests");
    ctx.knowledge_dir = tmp.to_str().unwrap().to_string();
    ctx.knowledge_files = vec![kb_file("guide.md", "Style guide", true)];
    let result = build_instruction(&task, &phase, &ctx, None);
    assert!(result.contains("Always use snake_case."), "inline content missing: {result}");
}

// =============================================================================
// Ordering regression: all sections present
// =============================================================================

#[test]
fn test_section_ordering() {
    let tmp = std::env::temp_dir().join("borg-instr-test-ordering");
    std::fs::create_dir_all(tmp.join(".borg")).unwrap();
    std::fs::write(tmp.join(".borg/prompt.md"), "Repo prompt content.").unwrap();

    let mut task = make_task();
    task.last_error = "build error".into();

    let phase = PhaseConfig {
        include_task_context: true,
        error_instruction: "Error was: {ERROR}".into(),
        ..make_phase()
    };

    let mut ctx = make_ctx(tmp.to_str().unwrap());
    ctx.knowledge_files = vec![kb_file("guide.md", "Style guide", false)];
    ctx.prior_research = vec!["Research chunk.".into()];
    ctx.pending_messages = vec![("user".into(), "Please check formatting.".into())];

    let result = build_instruction(&task, &phase, &ctx, Some("src/main.rs\n"));

    let kb_pos = result.find("## Knowledge Base").unwrap();
    let prompt_pos = result.find("## Project Context").unwrap();
    let task_pos = result.find("Task: Test Task").unwrap();
    let instr_pos = result.find("Write the implementation.").unwrap();
    let files_pos = result.find("Files in repository:").unwrap();
    let error_pos = result.find("Error was: build error").unwrap();
    let research_pos = result.find("Prior Research").unwrap();
    let msg_pos = result.find("[user]: Please check formatting.").unwrap();

    // Expected order: KB → repo prompt → task → instruction → files → error → research → messages
    assert!(kb_pos < prompt_pos, "KB must precede repo prompt");
    assert!(prompt_pos < task_pos, "repo prompt must precede task");
    assert!(task_pos < instr_pos, "task must precede instruction");
    assert!(instr_pos < files_pos, "instruction must precede files");
    assert!(files_pos < error_pos, "files must precede error");
    assert!(error_pos < research_pos, "error must precede research");
    assert!(research_pos < msg_pos, "research must precede messages");
}
