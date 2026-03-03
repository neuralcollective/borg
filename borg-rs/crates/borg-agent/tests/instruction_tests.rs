use borg_agent::instruction::build_instruction;
use borg_core::{
    db::KnowledgeFile,
    types::{PhaseConfig, PhaseContext, RepoConfig, Task},
};
use chrono::Utc;
use std::collections::HashMap;

fn make_task(title: &str, description: &str, last_error: &str) -> Task {
    Task {
        id: 1,
        title: title.to_string(),
        description: description.to_string(),
        repo_path: String::new(),
        branch: String::new(),
        status: String::new(),
        attempt: 0,
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

fn make_repo_config() -> RepoConfig {
    RepoConfig {
        path: String::new(),
        test_cmd: String::new(),
        prompt_file: String::new(),
        mode: String::new(),
        is_self: false,
        auto_merge: false,
        lint_cmd: String::new(),
        backend: String::new(),
        repo_slug: String::new(),
    }
}

fn make_ctx(revision_count: i64, pending: Vec<(String, String)>) -> PhaseContext {
    PhaseContext {
        task: make_task("", "", ""),
        repo_config: make_repo_config(),
        data_dir: String::new(),
        session_dir: String::new(),
        worktree_path: String::new(),
        oauth_token: String::new(),
        model: String::new(),
        pending_messages: pending,
        system_prompt_suffix: String::new(),
        user_coauthor: String::new(),
        stream_tx: None,
        setup_script: String::new(),
        api_keys: HashMap::new(),
        disallowed_tools: String::new(),
        knowledge_files: Vec::<KnowledgeFile>::new(),
        knowledge_dir: String::new(),
        agent_network: None,
        prior_research: Vec::new(),
        revision_count,
    }
}

// =============================================================================
// revision_count > 0 path
// =============================================================================

#[test]
fn test_revision_header_emitted_when_revision_count_positive() {
    let task = make_task("My Task", "Do something", "");
    let phase = PhaseConfig::default();
    let ctx = make_ctx(1, vec![("reviewer".to_string(), "Fix the typo.".to_string())]);

    let out = build_instruction(&task, &phase, &ctx, None);

    assert!(
        out.contains("Revision #1 — Reviewer Feedback"),
        "expected revision header, got: {out}"
    );
}

#[test]
fn test_revision_header_uses_correct_count() {
    let task = make_task("T", "D", "");
    let phase = PhaseConfig::default();
    let ctx = make_ctx(3, vec![("user".to_string(), "Please rewrite.".to_string())]);

    let out = build_instruction(&task, &phase, &ctx, None);

    assert!(out.contains("Revision #3"), "expected Revision #3, got: {out}");
}

#[test]
fn test_revision_messages_formatted_with_bold_brackets() {
    let task = make_task("T", "D", "");
    let phase = PhaseConfig::default();
    let ctx = make_ctx(
        2,
        vec![
            ("reviewer".to_string(), "Shorten section 2.".to_string()),
            ("user".to_string(), "Add a conclusion.".to_string()),
        ],
    );

    let out = build_instruction(&task, &phase, &ctx, None);

    assert!(
        out.contains("**[reviewer]**: Shorten section 2."),
        "expected bold-bracket format for reviewer, got: {out}"
    );
    assert!(
        out.contains("**[user]**: Add a conclusion."),
        "expected bold-bracket format for user, got: {out}"
    );
}

#[test]
fn test_revision_path_includes_important_footer() {
    let task = make_task("T", "D", "");
    let phase = PhaseConfig::default();
    let ctx = make_ctx(1, vec![("reviewer".to_string(), "Feedback.".to_string())]);

    let out = build_instruction(&task, &phase, &ctx, None);

    assert!(
        out.contains("IMPORTANT: Focus on the reviewer's feedback"),
        "expected IMPORTANT footer in revision path, got: {out}"
    );
}

// =============================================================================
// revision_count == 0 path (plain format)
// =============================================================================

#[test]
fn test_no_revision_header_when_revision_count_zero() {
    let task = make_task("T", "D", "");
    let phase = PhaseConfig::default();
    let ctx = make_ctx(0, vec![("user".to_string(), "Please hurry.".to_string())]);

    let out = build_instruction(&task, &phase, &ctx, None);

    assert!(
        !out.contains("Reviewer Feedback"),
        "should not contain revision header at revision_count=0, got: {out}"
    );
    assert!(
        !out.contains("**[user]**"),
        "should not use bold-bracket format at revision_count=0, got: {out}"
    );
}

#[test]
fn test_plain_pending_messages_format_at_revision_zero() {
    let task = make_task("T", "D", "");
    let phase = PhaseConfig::default();
    let ctx = make_ctx(
        0,
        vec![
            ("user".to_string(), "Do X.".to_string()),
            ("director".to_string(), "Also Y.".to_string()),
        ],
    );

    let out = build_instruction(&task, &phase, &ctx, None);

    assert!(
        out.contains("messages were sent by the user or director"),
        "expected plain intro header, got: {out}"
    );
    assert!(out.contains("[user]: Do X."), "expected plain format, got: {out}");
    assert!(out.contains("[director]: Also Y."), "expected plain format, got: {out}");
}

#[test]
fn test_no_pending_messages_section_when_empty() {
    let task = make_task("T", "D", "");
    let phase = PhaseConfig::default();
    let ctx = make_ctx(0, vec![]);

    let out = build_instruction(&task, &phase, &ctx, None);

    assert!(!out.contains("messages were sent"), "no pending section when empty, got: {out}");
    assert!(!out.contains("Reviewer Feedback"), "no pending section when empty, got: {out}");
}

// =============================================================================
// {ERROR} substitution in error_instruction
// =============================================================================

#[test]
fn test_error_placeholder_substituted_with_last_error() {
    let task = make_task("T", "D", "compilation failed: undefined symbol");
    let phase = PhaseConfig {
        error_instruction: "Previous attempt failed:\n{ERROR}\nPlease fix it.".to_string(),
        ..PhaseConfig::default()
    };
    let ctx = make_ctx(0, vec![]);

    let out = build_instruction(&task, &phase, &ctx, None);

    assert!(
        out.contains("compilation failed: undefined symbol"),
        "expected last_error substituted, got: {out}"
    );
    assert!(
        !out.contains("{ERROR}"),
        "raw {{ERROR}} placeholder should be replaced, got: {out}"
    );
}

#[test]
fn test_error_section_omitted_when_last_error_empty() {
    let task = make_task("T", "D", "");
    let phase = PhaseConfig {
        error_instruction: "Previous attempt failed:\n{ERROR}".to_string(),
        ..PhaseConfig::default()
    };
    let ctx = make_ctx(0, vec![]);

    let out = build_instruction(&task, &phase, &ctx, None);

    assert!(
        !out.contains("Previous attempt failed"),
        "error section should be omitted when last_error is empty, got: {out}"
    );
}

#[test]
fn test_error_section_omitted_when_error_instruction_empty() {
    let task = make_task("T", "D", "something went wrong");
    let phase = PhaseConfig {
        error_instruction: String::new(),
        ..PhaseConfig::default()
    };
    let ctx = make_ctx(0, vec![]);

    let out = build_instruction(&task, &phase, &ctx, None);

    // last_error is non-empty but error_instruction is empty — section omitted
    assert!(
        !out.contains("something went wrong"),
        "error section omitted when error_instruction is empty, got: {out}"
    );
}

// =============================================================================
// include_task_context flag
// =============================================================================

#[test]
fn test_task_context_included_when_flag_true() {
    let task = make_task("Refactor auth module", "Improve the login flow.", "");
    let phase = PhaseConfig {
        include_task_context: true,
        ..PhaseConfig::default()
    };
    let ctx = make_ctx(0, vec![]);

    let out = build_instruction(&task, &phase, &ctx, None);

    assert!(out.contains("Refactor auth module"), "title should be present, got: {out}");
    assert!(out.contains("Improve the login flow."), "description should be present, got: {out}");
}

#[test]
fn test_task_context_omitted_when_flag_false() {
    let task = make_task("Refactor auth module", "Improve the login flow.", "");
    let phase = PhaseConfig {
        include_task_context: false,
        ..PhaseConfig::default()
    };
    let ctx = make_ctx(0, vec![]);

    let out = build_instruction(&task, &phase, &ctx, None);

    assert!(
        !out.contains("Refactor auth module"),
        "title should be omitted when include_task_context=false, got: {out}"
    );
    assert!(
        !out.contains("Improve the login flow."),
        "description should be omitted when include_task_context=false, got: {out}"
    );
}
