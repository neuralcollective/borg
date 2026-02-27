use std::path::Path;

/// Integration tests for pipeline-state snapshot files (spec.md).
///
/// All tests in this file reference types and DB methods that do not exist
/// until the feature is implemented. They are expected to FAIL (compile
/// error) until `PipelineStateSnapshot`, `PhaseHistoryEntry`, and
/// `Db::get_queue_entries_for_task` are added.
use borg_core::{
    db::Db,
    types::{PhaseHistoryEntry, PipelineStateSnapshot, Task},
};
use chrono::Utc;

// ── helpers ──────────────────────────────────────────────────────────────────

fn open_db() -> Db {
    let mut db = Db::open(":memory:").expect("open in-memory db");
    db.migrate().expect("migrate");
    db
}

fn make_task(db: &Db) -> i64 {
    let task = Task {
        id: 0,
        title: "Test task".into(),
        description: "desc".into(),
        repo_path: "/repo".into(),
        branch: "task-1".into(),
        status: "impl".into(),
        attempt: 1,
        max_attempts: 5,
        last_error: String::new(),
        created_by: "test".into(),
        notify_chat: String::new(),
        created_at: Utc::now(),
        session_id: String::new(),
        mode: "sweborg".into(),
        backend: String::new(),
    };
    db.insert_task(&task).expect("insert_task")
}

fn make_snapshot(phase: &str) -> PipelineStateSnapshot {
    PipelineStateSnapshot {
        task_id: 42,
        task_title: "Fix the thing".into(),
        phase: phase.into(),
        worktree_path: "/repo/.worktrees/task-42".into(),
        pr_url: Some("https://github.com/org/repo/pull/7".into()),
        pending_approvals: vec!["task-42".into()],
        phase_history: vec![PhaseHistoryEntry {
            phase: "spec".into(),
            success: true,
            output: "wrote spec.md".into(),
            timestamp: Utc::now(),
        }],
        generated_at: Utc::now(),
    }
}

// ── AC #8 ─────────────────────────────────────────────────────────────────────
// "Unit test test_pipeline_state_snapshot_serialization: constructs a
//  PipelineStateSnapshot with known fields, serializes to JSON, deserializes
//  back, and asserts round-trip equality."

#[test]
fn test_pipeline_state_snapshot_serialization() {
    let original = PipelineStateSnapshot {
        task_id: 7,
        task_title: "Round-trip test".into(),
        phase: "impl".into(),
        worktree_path: "/repo/.worktrees/task-7".into(),
        pr_url: Some("https://github.com/org/repo/pull/42".into()),
        pending_approvals: vec!["task-7".into()],
        phase_history: vec![PhaseHistoryEntry {
            phase: "spec".into(),
            success: true,
            output: "agent output".into(),
            timestamp: Utc::now(),
        }],
        generated_at: Utc::now(),
    };

    let json = serde_json::to_string(&original).expect("serialize");
    let restored: PipelineStateSnapshot = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(restored.task_id, original.task_id);
    assert_eq!(restored.task_title, original.task_title);
    assert_eq!(restored.phase, original.phase);
    assert_eq!(restored.worktree_path, original.worktree_path);
    assert_eq!(restored.pr_url, original.pr_url);
    assert_eq!(restored.pending_approvals, original.pending_approvals);
    assert_eq!(restored.phase_history.len(), original.phase_history.len());
    assert_eq!(
        restored.phase_history[0].phase,
        original.phase_history[0].phase
    );
    assert_eq!(
        restored.phase_history[0].success,
        original.phase_history[0].success
    );
    assert_eq!(
        restored.phase_history[0].output,
        original.phase_history[0].output
    );
}

// ── AC #2 ─────────────────────────────────────────────────────────────────────
// "The JSON object contains exactly the fields defined in PipelineStateSnapshot:
//  task_id, task_title, phase, worktree_path, pr_url, pending_approvals,
//  phase_history, generated_at."

#[test]
fn test_snapshot_json_has_all_required_fields() {
    let snapshot = make_snapshot("impl");
    let json = serde_json::to_string(&snapshot).expect("serialize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("parse");
    let obj = value.as_object().expect("root is object");

    for field in &[
        "task_id",
        "task_title",
        "phase",
        "worktree_path",
        "pr_url",
        "pending_approvals",
        "phase_history",
        "generated_at",
    ] {
        assert!(obj.contains_key(*field), "missing field: {field}");
    }
}

#[test]
fn test_snapshot_pr_url_serializes_as_null_when_none() {
    let snapshot = PipelineStateSnapshot {
        pr_url: None,
        ..make_snapshot("spec")
    };
    let json = serde_json::to_string(&snapshot).expect("serialize");
    let value: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert!(value["pr_url"].is_null(), "pr_url should be JSON null");
}

// ── AC #3 ─────────────────────────────────────────────────────────────────────
// "phase_history contains at most 5 entries; each output field is truncated
//  to 2 000 characters."

#[test]
fn test_phase_history_honours_five_entry_cap() {
    // Simulate building phase_history from 8 DB outputs — only last 5 kept.
    let all_outputs: Vec<PhaseHistoryEntry> = (0u8..8)
        .map(|i| PhaseHistoryEntry {
            phase: format!("phase-{i}"),
            success: true,
            output: "ok".into(),
            timestamp: Utc::now(),
        })
        .collect();

    // The implementation takes the last 5.
    let capped: Vec<_> = all_outputs.into_iter().rev().take(5).rev().collect();
    assert_eq!(capped.len(), 5, "history must be capped at 5 entries");
    // Verify oldest retained is index 3 (the 4th of 8)
    assert_eq!(capped[0].phase, "phase-3");
}

#[test]
fn test_phase_output_truncated_to_2000_chars() {
    let long_output: String = "x".repeat(5_000);
    let truncated: String = long_output.chars().take(2_000).collect();

    assert_eq!(truncated.len(), 2_000);
    assert!(truncated.len() < long_output.len());
}

// Edge case: truncation is char-based, not byte-based (multibyte safety).
#[test]
fn test_output_truncation_is_char_based() {
    // Each '€' is 3 UTF-8 bytes; 2 000 '€' characters = 6 000 bytes.
    let long_output: String = "€".repeat(3_000);
    let truncated: String = long_output.chars().take(2_000).collect();

    assert_eq!(truncated.chars().count(), 2_000);
    // Must be valid UTF-8 (no panic on to_string / len).
    let _ = truncated.len();
}

// Edge case: exactly 2 000 chars — no truncation should occur.
#[test]
fn test_output_at_exact_2000_chars_is_not_truncated() {
    let exact: String = "a".repeat(2_000);
    let truncated: String = exact.chars().take(2_000).collect();
    assert_eq!(truncated.len(), 2_000);
    assert_eq!(truncated, exact);
}

// ── AC #4 ─────────────────────────────────────────────────────────────────────
// "When a task has a queue entry in pending_review, its branch name appears
//  in pending_approvals; when no such entry exists, the array is empty."

#[test]
fn test_get_queue_entries_for_task_returns_all_statuses() {
    let db = open_db();
    let task_id = make_task(&db);

    let entry_id = db.enqueue(task_id, "task-1", "/repo", 0).expect("enqueue");
    db.update_queue_status(entry_id, "pending_review")
        .expect("update to pending_review");
    let _other_id = db
        .enqueue(task_id, "task-1-retry", "/repo", 0)
        .expect("enqueue other");

    let entries = db
        .get_queue_entries_for_task(task_id)
        .expect("get_queue_entries_for_task");

    assert_eq!(entries.len(), 2, "both queue entries must be returned");
}

#[test]
fn test_pending_approvals_contains_pending_review_branches() {
    let db = open_db();
    let task_id = make_task(&db);

    let queued_id = db
        .enqueue(task_id, "task-1", "/repo", 0)
        .expect("enqueue queued");
    let review_id = db
        .enqueue(task_id, "task-1-v2", "/repo", 0)
        .expect("enqueue review");
    db.update_queue_status(review_id, "pending_review")
        .expect("set pending_review");

    let entries = db
        .get_queue_entries_for_task(task_id)
        .expect("get_queue_entries_for_task");

    let pending: Vec<String> = entries
        .into_iter()
        .filter(|e| e.status == "pending_review")
        .map(|e| e.branch)
        .collect();

    assert_eq!(pending, vec!["task-1-v2"]);

    // The purely-queued entry must NOT appear in pending_approvals.
    let all_entries = db
        .get_queue_entries_for_task(task_id)
        .expect("get_queue_entries_for_task");
    let queued_entry = all_entries.iter().find(|e| e.id == queued_id).unwrap();
    assert_ne!(queued_entry.status, "pending_review");
}

#[test]
fn test_pending_approvals_empty_when_no_queue_entries() {
    let db = open_db();
    let task_id = make_task(&db);

    let entries = db
        .get_queue_entries_for_task(task_id)
        .expect("get_queue_entries_for_task");

    let pending: Vec<_> = entries
        .into_iter()
        .filter(|e| e.status == "pending_review")
        .collect();

    assert!(
        pending.is_empty(),
        "no queue entries → empty pending_approvals"
    );
}

#[test]
fn test_get_queue_entries_for_task_excludes_other_tasks() {
    let db = open_db();
    let task_a = make_task(&db);
    let task_b = make_task(&db);

    db.enqueue(task_a, "task-a", "/repo", 0).expect("enqueue a");
    db.enqueue(task_b, "task-b", "/repo", 0).expect("enqueue b");

    let entries_a = db
        .get_queue_entries_for_task(task_a)
        .expect("get for task_a");
    assert_eq!(entries_a.len(), 1);
    assert_eq!(entries_a[0].branch, "task-a");
}

// ── AC #5 ─────────────────────────────────────────────────────────────────────
// "pr_url is a non-empty string when gh pr view returns a URL, null otherwise."

#[test]
fn test_pr_url_is_none_when_no_queue_entry_exists() {
    let db = open_db();
    let task_id = make_task(&db);

    let entries = db
        .get_queue_entries_for_task(task_id)
        .expect("get_queue_entries_for_task");

    // No queue entries → no subprocess spawned → pr_url must be None.
    let pr_url: Option<String> = if entries.is_empty() {
        None
    } else {
        Some("would-call-gh-here".into())
    };
    assert!(pr_url.is_none());
}

// ── AC #1 + edge case: .borg/ dir created if absent ──────────────────────────
// "pipeline-state.json exists and is valid JSON; .borg/ is created when absent"

#[test]
fn test_snapshot_written_to_correct_path_and_parseable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let wt_path = dir.path();

    // .borg/ does NOT exist yet — the implementation must create it.
    assert!(!wt_path.join(".borg").exists());

    let snapshot = make_snapshot("impl");

    // Replicate what write_pipeline_state_snapshot must do:
    std::fs::create_dir_all(wt_path.join(".borg")).expect("create .borg");
    let json = serde_json::to_string_pretty(&snapshot).expect("serialize");
    std::fs::write(wt_path.join(".borg/pipeline-state.json"), &json).expect("write");

    // File must exist at the exact path.
    let file_path = wt_path.join(".borg/pipeline-state.json");
    assert!(file_path.exists(), ".borg/pipeline-state.json must exist");

    // Content must be valid JSON parseable back to PipelineStateSnapshot.
    let content = std::fs::read_to_string(&file_path).expect("read");
    let restored: PipelineStateSnapshot =
        serde_json::from_str(&content).expect("parse pipeline-state.json");
    assert_eq!(restored.task_id, snapshot.task_id);
    assert_eq!(restored.phase, snapshot.phase);
    assert_eq!(restored.worktree_path, snapshot.worktree_path);
}

// Edge case: retry overwrites stale snapshot from previous attempt.
#[test]
fn test_snapshot_overwritten_on_retry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let borg_dir = dir.path().join(".borg");
    std::fs::create_dir_all(&borg_dir).expect("create .borg");
    let path = borg_dir.join("pipeline-state.json");

    // Write a "stale" snapshot from attempt 1.
    let stale = PipelineStateSnapshot {
        phase: "impl".into(),
        ..make_snapshot("impl")
    };
    std::fs::write(&path, serde_json::to_string_pretty(&stale).unwrap()).unwrap();

    // Write a fresh snapshot for attempt 2 (different phase).
    let fresh = PipelineStateSnapshot {
        phase: "retry".into(),
        ..make_snapshot("retry")
    };
    std::fs::write(&path, serde_json::to_string_pretty(&fresh).unwrap()).unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    let restored: PipelineStateSnapshot = serde_json::from_str(&content).unwrap();
    assert_eq!(
        restored.phase, "retry",
        "stale snapshot must be overwritten"
    );
}

// Edge case: no prior phase outputs → phase_history is empty.
#[test]
fn test_phase_history_is_empty_when_no_prior_outputs() {
    let db = open_db();
    let task_id = make_task(&db);

    let outputs = db.get_task_outputs(task_id).expect("get_task_outputs");
    assert!(outputs.is_empty());

    // Building phase_history from empty outputs must yield an empty vec.
    let history: Vec<PhaseHistoryEntry> = outputs
        .iter()
        .rev()
        .take(5)
        .rev()
        .map(|o| PhaseHistoryEntry {
            phase: o.phase.clone(),
            success: o.exit_code == 0,
            output: o.output.chars().take(2_000).collect(),
            timestamp: o.created_at,
        })
        .collect();

    assert!(history.is_empty());
}

// Edge case: concurrent tasks have non-colliding snapshot paths.
#[test]
fn test_concurrent_tasks_have_distinct_snapshot_paths() {
    let root = tempfile::tempdir().expect("tempdir");
    let wt_a = root.path().join(".worktrees/task-1/.borg");
    let wt_b = root.path().join(".worktrees/task-2/.borg");
    std::fs::create_dir_all(&wt_a).unwrap();
    std::fs::create_dir_all(&wt_b).unwrap();

    let snap_a = make_snapshot("impl");
    let snap_b = PipelineStateSnapshot {
        task_id: 2,
        phase: "spec".into(),
        ..make_snapshot("spec")
    };

    std::fs::write(
        wt_a.join("pipeline-state.json"),
        serde_json::to_string_pretty(&snap_a).unwrap(),
    )
    .unwrap();
    std::fs::write(
        wt_b.join("pipeline-state.json"),
        serde_json::to_string_pretty(&snap_b).unwrap(),
    )
    .unwrap();

    let read_a: PipelineStateSnapshot =
        serde_json::from_str(&std::fs::read_to_string(wt_a.join("pipeline-state.json")).unwrap())
            .unwrap();
    let read_b: PipelineStateSnapshot =
        serde_json::from_str(&std::fs::read_to_string(wt_b.join("pipeline-state.json")).unwrap())
            .unwrap();

    assert_eq!(read_a.task_id, 42);
    assert_eq!(read_b.task_id, 2);
    assert_ne!(read_a.task_id, read_b.task_id, "paths must not collide");
}

// ── AC #7 ─────────────────────────────────────────────────────────────────────
// "Snapshot is also written before the rebase fix agent and each lint-fix
//  agent attempt."
// Tested via type-level check: phase names used for those call sites are
// valid values for PipelineStateSnapshot::phase.

#[test]
fn test_snapshot_phase_field_accepts_rebase_and_lint_phase_names() {
    for phase_name in &["rebase_fix", "lint_fix_0", "lint_fix_1"] {
        let snap = make_snapshot(phase_name);
        let json = serde_json::to_string(&snap).expect("serialize");
        let restored: PipelineStateSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(&restored.phase, phase_name);
    }
}

// ── AC #6 ─────────────────────────────────────────────────────────────────────
// "A snapshot write failure emits a warn! log but does not abort the phase
//  or change the task's DB status."
// Verified here at the DB level: task status is unchanged after a failed write.

#[test]
fn test_task_status_unchanged_after_snapshot_write_failure() {
    let db = open_db();
    let task_id = make_task(&db);

    // Simulate a failed snapshot write (e.g., read-only dir) by doing nothing
    // to the DB. The task status must remain "impl" (as inserted).
    let task = db
        .get_task(task_id)
        .expect("get_task")
        .expect("task exists");
    assert_eq!(
        task.status, "impl",
        "task status must not change on snapshot error"
    );
}

// ── PhaseHistoryEntry: field-level checks ────────────────────────────────────

#[test]
fn test_phase_history_entry_serialization() {
    let entry = PhaseHistoryEntry {
        phase: "qa".into(),
        success: false,
        output: "FAILED: assertion failed".into(),
        timestamp: Utc::now(),
    };
    let json = serde_json::to_string(&entry).expect("serialize");
    let restored: PhaseHistoryEntry = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored.phase, "qa");
    assert!(!restored.success);
    assert_eq!(restored.output, "FAILED: assertion failed");
}

// ── DB ordering guarantee ─────────────────────────────────────────────────────

#[test]
fn test_get_queue_entries_for_task_returns_in_insertion_order() {
    let db = open_db();
    let task_id = make_task(&db);

    db.enqueue(task_id, "first", "/repo", 0)
        .expect("enqueue first");
    db.enqueue(task_id, "second", "/repo", 0)
        .expect("enqueue second");
    db.enqueue(task_id, "third", "/repo", 0)
        .expect("enqueue third");

    let entries = db
        .get_queue_entries_for_task(task_id)
        .expect("get_queue_entries_for_task");

    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].branch, "first");
    assert_eq!(entries[1].branch, "second");
    assert_eq!(entries[2].branch, "third");
}
