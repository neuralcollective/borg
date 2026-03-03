use borg_agent::instruction::build_instruction;
use borg_core::types::{PhaseConfig, PhaseContext, RepoConfig, Task};
use chrono::Utc;
use std::collections::HashMap;

// ── Fixtures ─────────────────────────────────────────────────────────────────

fn make_task() -> Task {
    Task {
        id: 1,
        title: "Test task".to_string(),
        description: "Task description".to_string(),
        repo_path: String::new(),
        branch: String::new(),
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

fn make_ctx() -> PhaseContext {
    // Use a path that does not exist on disk so read_repo_prompt returns None.
    let nonexistent = "/tmp/nonexistent-borg-test-instruction".to_string();
    PhaseContext {
        task: make_task(),
        repo_config: RepoConfig {
            path: nonexistent.clone(),
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
        worktree_path: nonexistent,
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

fn make_phase(instruction: &str) -> PhaseConfig {
    PhaseConfig {
        instruction: instruction.to_string(),
        ..Default::default()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

// With every optional section absent, the output is exactly the phase instruction.
#[test]
fn test_only_phase_instruction_when_all_empty() {
    let task = make_task();
    let phase = make_phase("Do the thing.");
    let ctx = make_ctx();

    let out = build_instruction(&task, &phase, &ctx, None);
    assert_eq!(out, "Do the thing.");
}

// include_task_context=true prepends "Task: <title>\n\n<description>\n\n---\n\n".
#[test]
fn test_include_task_context_prepends_title_and_description() {
    let mut task = make_task();
    task.title = "Implement feature X".to_string();
    task.description = "Add the frobnication module.".to_string();

    let phase = PhaseConfig {
        instruction: "Do it.".to_string(),
        include_task_context: true,
        ..Default::default()
    };
    let ctx = make_ctx();

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(
        out.contains("Task: Implement feature X"),
        "expected task title, got: {out}"
    );
    assert!(
        out.contains("Add the frobnication module."),
        "expected description, got: {out}"
    );
    // The separator and instruction must follow.
    assert!(
        out.contains("---\n\nDo it."),
        "expected instruction after separator, got: {out}"
    );
}

// Without include_task_context, title and description are absent.
#[test]
fn test_no_task_context_when_flag_false() {
    let mut task = make_task();
    task.title = "Secret title".to_string();
    task.description = "Secret description".to_string();

    let phase = make_phase("Phase instruction.");
    let ctx = make_ctx();

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(!out.contains("Secret title"), "title must not appear: {out}");
    assert!(!out.contains("Secret description"), "description must not appear: {out}");
}

// Non-empty last_error + error_instruction substitutes {ERROR} correctly.
#[test]
fn test_error_instruction_substitutes_error_placeholder() {
    let mut task = make_task();
    task.last_error = "panic: index out of bounds".to_string();

    let phase = PhaseConfig {
        instruction: "Fix the code.".to_string(),
        error_instruction: "Previous run failed with: {ERROR}".to_string(),
        ..Default::default()
    };
    let ctx = make_ctx();

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(
        out.contains("Previous run failed with: panic: index out of bounds"),
        "expected substituted error, got: {out}"
    );
}

// If last_error is empty, the error_instruction is not appended even if set.
#[test]
fn test_no_error_section_when_last_error_empty() {
    let task = make_task(); // last_error is empty

    let phase = PhaseConfig {
        instruction: "Do it.".to_string(),
        error_instruction: "Error: {ERROR}".to_string(),
        ..Default::default()
    };
    let ctx = make_ctx();

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(!out.contains("Error:"), "error section must not appear: {out}");
}

// If error_instruction is empty, no error section even with a non-empty last_error.
#[test]
fn test_no_error_section_when_error_instruction_empty() {
    let mut task = make_task();
    task.last_error = "something went wrong".to_string();

    let phase = make_phase("Do it.");
    let ctx = make_ctx();

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(
        !out.contains("something went wrong"),
        "error text must not appear: {out}"
    );
}

// revision_count > 0 in pending_messages renders the revision header.
#[test]
fn test_revision_header_when_revision_count_positive() {
    let task = make_task();
    let phase = make_phase("Do it.");
    let mut ctx = make_ctx();
    ctx.revision_count = 2;
    ctx.pending_messages = vec![("user".to_string(), "fix the formatting".to_string())];

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(
        out.contains("Revision #2"),
        "expected revision header, got: {out}"
    );
    assert!(
        out.contains("fix the formatting"),
        "expected feedback content, got: {out}"
    );
    // Plain queue header must NOT appear.
    assert!(
        !out.contains("messages were sent"),
        "plain queue header must not appear in revision: {out}"
    );
}

// revision_count == 0 renders the plain queue header.
#[test]
fn test_plain_queue_header_when_revision_count_zero() {
    let task = make_task();
    let phase = make_phase("Do it.");
    let mut ctx = make_ctx();
    ctx.revision_count = 0;
    ctx.pending_messages = vec![("user".to_string(), "please add tests".to_string())];

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(
        out.contains("messages were sent"),
        "expected plain queue header, got: {out}"
    );
    assert!(
        !out.contains("Revision #"),
        "revision header must not appear: {out}"
    );
}

// No pending_messages → neither header appears.
#[test]
fn test_no_messages_section_when_pending_empty() {
    let task = make_task();
    let phase = make_phase("Do it.");
    let ctx = make_ctx();

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(!out.contains("messages were sent"), "no message section: {out}");
    assert!(!out.contains("Revision #"), "no revision section: {out}");
}

// file_listing = None → no file listing section.
#[test]
fn test_file_listing_omitted_when_none() {
    let task = make_task();
    let phase = make_phase("Do it.");
    let ctx = make_ctx();

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(!out.contains("Files in repository"), "listing must not appear: {out}");
}

// file_listing = Some("") → no file listing section.
#[test]
fn test_file_listing_omitted_when_empty_string() {
    let task = make_task();
    let phase = make_phase("Do it.");
    let ctx = make_ctx();

    let out = build_instruction(&task, &phase, &ctx, Some(""));
    assert!(!out.contains("Files in repository"), "listing must not appear for empty str: {out}");
}

// file_listing with content → section included.
#[test]
fn test_file_listing_included_when_non_empty() {
    let task = make_task();
    let phase = make_phase("Do it.");
    let ctx = make_ctx();

    let out = build_instruction(&task, &phase, &ctx, Some("src/main.rs\nsrc/lib.rs"));
    assert!(out.contains("Files in repository"), "listing must appear: {out}");
    assert!(out.contains("src/main.rs"), "file path must appear: {out}");
}

// prior_research chunks are appended with a numbered list.
#[test]
fn test_prior_research_appended() {
    let task = make_task();
    let phase = make_phase("Do it.");
    let mut ctx = make_ctx();
    ctx.prior_research = vec!["Finding about jurisdiction.".to_string()];

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(out.contains("Prior Research"), "prior research header must appear: {out}");
    assert!(
        out.contains("Finding about jurisdiction."),
        "research chunk must appear: {out}"
    );
}

// No prior_research → section absent.
#[test]
fn test_no_prior_research_section_when_empty() {
    let task = make_task();
    let phase = make_phase("Do it.");
    let ctx = make_ctx();

    let out = build_instruction(&task, &phase, &ctx, None);
    assert!(!out.contains("Prior Research"), "research section must not appear: {out}");
}
