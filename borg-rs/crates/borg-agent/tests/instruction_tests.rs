use std::{collections::HashMap, fs};

use borg_agent::instruction::build_instruction;
use borg_core::types::{PhaseConfig, PhaseContext, RepoConfig, Task};
use chrono::Utc;
use tempfile::TempDir;

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_task(last_error: &str) -> Task {
    Task {
        id: 1,
        title: "Test Task".into(),
        description: "A test task.".into(),
        repo_path: "/tmp/repo".into(),
        branch: "task-1".into(),
        status: "implement".into(),
        attempt: 1,
        max_attempts: 3,
        last_error: last_error.into(),
        created_by: "test".into(),
        notify_chat: String::new(),
        created_at: Utc::now(),
        session_id: String::new(),
        mode: "sweborg".into(),
        backend: String::new(),
        project_id: 0,
    }
}

fn make_phase(error_instruction: &str) -> PhaseConfig {
    PhaseConfig {
        instruction: "Do the work.".into(),
        error_instruction: error_instruction.into(),
        ..PhaseConfig::default()
    }
}

fn make_ctx(prompt_file: &str, worktree_path: &str, repo_path: &str) -> PhaseContext {
    PhaseContext {
        task: make_task(""),
        repo_config: RepoConfig {
            path: repo_path.into(),
            test_cmd: String::new(),
            prompt_file: prompt_file.into(),
            mode: "sweborg".into(),
            is_self: false,
            auto_merge: false,
            lint_cmd: String::new(),
            backend: String::new(),
            repo_slug: String::new(),
        },
        data_dir: String::new(),
        session_dir: String::new(),
        worktree_path: worktree_path.into(),
        oauth_token: String::new(),
        model: "claude".into(),
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

fn write_borg_prompt(dir: &TempDir, content: &str) {
    let borg_dir = dir.path().join(".borg");
    fs::create_dir_all(&borg_dir).expect("create .borg dir");
    fs::write(borg_dir.join("prompt.md"), content).expect("write prompt.md");
}

fn dir_path(dir: &TempDir) -> String {
    dir.path().to_str().expect("utf-8 path").to_string()
}

// ── priority: explicit config path wins ──────────────────────────────────────

#[test]
fn config_prompt_wins_over_worktree_and_repo() {
    let config_dir = TempDir::new().expect("tmpdir");
    let worktree_dir = TempDir::new().expect("tmpdir");
    let repo_dir = TempDir::new().expect("tmpdir");

    let config_file = config_dir.path().join("custom-prompt.md");
    fs::write(&config_file, "config-prompt-content").expect("write config prompt");
    write_borg_prompt(&worktree_dir, "worktree-prompt-content");
    write_borg_prompt(&repo_dir, "repo-root-prompt-content");

    let ctx = make_ctx(
        config_file.to_str().expect("utf-8"),
        &dir_path(&worktree_dir),
        &dir_path(&repo_dir),
    );
    let output = build_instruction(&make_task(""), &make_phase(""), &ctx, None);

    assert!(output.contains("config-prompt-content"), "output:\n{output}");
    assert!(!output.contains("worktree-prompt-content"), "output:\n{output}");
    assert!(!output.contains("repo-root-prompt-content"), "output:\n{output}");
}

// ── priority: worktree beats repo root when no config path ───────────────────

#[test]
fn worktree_prompt_wins_over_repo_root() {
    let worktree_dir = TempDir::new().expect("tmpdir");
    let repo_dir = TempDir::new().expect("tmpdir");

    write_borg_prompt(&worktree_dir, "worktree-prompt-content");
    write_borg_prompt(&repo_dir, "repo-root-prompt-content");

    let ctx = make_ctx("", &dir_path(&worktree_dir), &dir_path(&repo_dir));
    let output = build_instruction(&make_task(""), &make_phase(""), &ctx, None);

    assert!(output.contains("worktree-prompt-content"), "output:\n{output}");
    assert!(!output.contains("repo-root-prompt-content"), "output:\n{output}");
}

// ── priority: repo root fallback ─────────────────────────────────────────────

#[test]
fn repo_root_prompt_used_when_no_config_or_worktree() {
    let worktree_dir = TempDir::new().expect("tmpdir"); // no .borg/prompt.md
    let repo_dir = TempDir::new().expect("tmpdir");

    write_borg_prompt(&repo_dir, "repo-root-only-content");

    let ctx = make_ctx("", &dir_path(&worktree_dir), &dir_path(&repo_dir));
    let output = build_instruction(&make_task(""), &make_phase(""), &ctx, None);

    assert!(output.contains("repo-root-only-content"), "output:\n{output}");
}

// ── fallback: no prompt anywhere → no Project Context section ────────────────

#[test]
fn no_prompt_file_produces_no_project_context_section() {
    let worktree_dir = TempDir::new().expect("tmpdir");
    let repo_dir = TempDir::new().expect("tmpdir");

    let ctx = make_ctx("", &dir_path(&worktree_dir), &dir_path(&repo_dir));
    let output = build_instruction(&make_task(""), &make_phase(""), &ctx, None);

    assert!(!output.contains("## Project Context"), "output:\n{output}");
}

// ── absent config path falls through to worktree ─────────────────────────────

#[test]
fn absent_config_path_falls_through_to_worktree() {
    let worktree_dir = TempDir::new().expect("tmpdir");
    let repo_dir = TempDir::new().expect("tmpdir");

    write_borg_prompt(&worktree_dir, "worktree-fallback-content");

    let ctx = make_ctx(
        "/nonexistent/borg-test-path/that/cannot/exist.md",
        &dir_path(&worktree_dir),
        &dir_path(&repo_dir),
    );
    let output = build_instruction(&make_task(""), &make_phase(""), &ctx, None);

    assert!(output.contains("worktree-fallback-content"), "output:\n{output}");
}

// ── worktree == repo root → no duplicate Project Context section ──────────────

#[test]
fn same_worktree_and_repo_path_reads_prompt_exactly_once() {
    let repo_dir = TempDir::new().expect("tmpdir");
    write_borg_prompt(&repo_dir, "shared-dir-prompt-content");

    let p = dir_path(&repo_dir);
    let ctx = make_ctx("", &p, &p); // worktree == repo root
    let output = build_instruction(&make_task(""), &make_phase(""), &ctx, None);

    assert!(output.contains("shared-dir-prompt-content"), "output:\n{output}");
    let section_count = output.matches("## Project Context").count();
    assert_eq!(section_count, 1, "expected one Project Context section, output:\n{output}");
}

// ── whitespace-only config content falls through ──────────────────────────────

#[test]
fn whitespace_only_config_content_falls_through_to_worktree() {
    let config_dir = TempDir::new().expect("tmpdir");
    let worktree_dir = TempDir::new().expect("tmpdir");
    let repo_dir = TempDir::new().expect("tmpdir");

    let config_file = config_dir.path().join("blank-prompt.md");
    fs::write(&config_file, "   \n\t\n   ").expect("write blank config prompt");
    write_borg_prompt(&worktree_dir, "worktree-after-blank-config");

    let ctx = make_ctx(
        config_file.to_str().expect("utf-8"),
        &dir_path(&worktree_dir),
        &dir_path(&repo_dir),
    );
    let output = build_instruction(&make_task(""), &make_phase(""), &ctx, None);

    assert!(output.contains("worktree-after-blank-config"), "output:\n{output}");
}

// ── {ERROR} placeholder substituted ──────────────────────────────────────────

#[test]
fn error_placeholder_substituted_in_output() {
    let worktree_dir = TempDir::new().expect("tmpdir");
    let repo_dir = TempDir::new().expect("tmpdir");

    let ctx = make_ctx("", &dir_path(&worktree_dir), &dir_path(&repo_dir));
    let task = make_task("compilation failed: missing semicolon");
    let phase = make_phase("Previous attempt failed: {ERROR}\nPlease fix it.");
    let output = build_instruction(&task, &phase, &ctx, None);

    assert!(
        output.contains("Previous attempt failed: compilation failed: missing semicolon"),
        "output:\n{output}"
    );
}

// ── no error section when last_error is empty ────────────────────────────────

#[test]
fn no_error_section_when_last_error_empty() {
    let worktree_dir = TempDir::new().expect("tmpdir");
    let repo_dir = TempDir::new().expect("tmpdir");

    let ctx = make_ctx("", &dir_path(&worktree_dir), &dir_path(&repo_dir));
    let task = make_task(""); // empty last_error
    let phase = make_phase("Previous attempt failed: {ERROR}");
    let output = build_instruction(&task, &phase, &ctx, None);

    assert!(!output.contains("Previous attempt failed"), "output:\n{output}");
}

// ── no error section when error_instruction is empty ─────────────────────────

#[test]
fn no_error_section_when_error_instruction_empty() {
    let worktree_dir = TempDir::new().expect("tmpdir");
    let repo_dir = TempDir::new().expect("tmpdir");

    let ctx = make_ctx("", &dir_path(&worktree_dir), &dir_path(&repo_dir));
    let task = make_task("some error occurred");
    let phase = make_phase(""); // empty error_instruction
    let output = build_instruction(&task, &phase, &ctx, None);

    assert!(!output.contains("some error occurred"), "output:\n{output}");
}
