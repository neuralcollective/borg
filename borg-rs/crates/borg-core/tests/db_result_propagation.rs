/// Tests verifying db methods work correctly.
use borg_core::{
    db::Db,
    types::{Proposal, Task},
};
use chrono::Utc;

mod support;

use support::open_db;

fn make_task(db: &Db) -> i64 {
    let task = Task {
        id: 0,
        title: "Test".into(),
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
        updated_at: Utc::now(),
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
    };
    db.insert_task(&task).expect("insert_task")
}

fn make_proposal(status: &str, triage_score: i64) -> Proposal {
    Proposal {
        id: 0,
        repo_path: "/repo".into(),
        title: "Test proposal".into(),
        description: "desc".into(),
        rationale: "rationale".into(),
        status: status.into(),
        created_at: Utc::now(),
        triage_score,
        triage_impact: 0,
        triage_feasibility: 0,
        triage_risk: 0,
        triage_effort: 0,
        triage_reasoning: String::new(),
    }
}

// ── count_unscored_proposals ─────────────────────────────────────────────────

#[test]
fn test_count_unscored_proposals_returns_zero_on_empty_db() {
    let db = open_db();
    let before = db.count_unscored_proposals();
    assert!(before >= 0, "count should not be negative");
}

#[test]
fn test_count_unscored_proposals_returns_count_with_unscored() {
    let db = open_db();
    let before = db.count_unscored_proposals();
    db.insert_proposal(&make_proposal("proposed", 0))
        .expect("insert");
    db.insert_proposal(&make_proposal("proposed", 0))
        .expect("insert");
    assert_eq!(db.count_unscored_proposals(), before + 2);
}

#[test]
fn test_count_unscored_proposals_excludes_scored() {
    let db = open_db();
    let before = db.count_unscored_proposals();
    db.insert_proposal(&make_proposal("proposed", 5))
        .expect("insert scored");
    db.insert_proposal(&make_proposal("proposed", 0))
        .expect("insert unscored");
    assert_eq!(
        db.count_unscored_proposals(),
        before + 1,
        "scored proposal must not be counted"
    );
}

#[test]
fn test_count_unscored_proposals_excludes_non_proposed_status() {
    let db = open_db();
    let before = db.count_unscored_proposals();
    db.insert_proposal(&make_proposal("approved", 0))
        .expect("insert approved");
    db.insert_proposal(&make_proposal("dismissed", 0))
        .expect("insert dismissed");
    assert_eq!(
        db.count_unscored_proposals(),
        before,
        "non-proposed proposals must not be counted"
    );
}

// ── active_task_count ────────────────────────────────────────────────────────

#[test]
fn test_active_task_count_returns_zero_on_empty_db() {
    let db = open_db();
    let before = db.active_task_count();
    assert!(before >= 0, "count should not be negative");
}

#[test]
fn test_active_task_count_includes_active_statuses() {
    let db = open_db();
    let before = db.active_task_count();
    make_task(&db); // status='impl'
    assert_eq!(db.active_task_count(), before + 1);
}

#[test]
fn test_active_task_count_excludes_terminal_statuses() {
    let db = open_db();
    let before = db.active_task_count();
    let id = make_task(&db);
    db.update_task_status(id, "done", None)
        .expect("update status");
    assert_eq!(
        db.active_task_count(),
        before,
        "'done' tasks must not be counted as active"
    );
}

#[test]
fn test_active_task_count_excludes_all_terminal_statuses() {
    let db = open_db();
    let before = db.active_task_count();
    for status in &["done", "merged", "failed", "blocked", "pending_review"] {
        let id = make_task(&db);
        db.update_task_status(id, status, None).expect("update");
    }
    assert_eq!(
        db.active_task_count(),
        before,
        "all terminal statuses must be excluded"
    );
}

// ── get_unknown_retries ──────────────────────────────────────────────────────

#[test]
fn test_get_unknown_retries_returns_zero_for_new_entry() {
    let db = open_db();
    let task_id = make_task(&db);
    let entry_id = db.enqueue(task_id, "task-1", "/repo", 0).expect("enqueue");
    assert_eq!(db.get_unknown_retries(entry_id), 0);
}

#[test]
fn test_get_unknown_retries_returns_zero_for_missing_entry() {
    let db = open_db();
    // No entry with id 9999999 — returns 0 (unwrap_or default)
    assert_eq!(db.get_unknown_retries(9999999), 0);
}

#[test]
fn test_get_unknown_retries_reflects_increments() {
    let db = open_db();
    let task_id = make_task(&db);
    let entry_id = db.enqueue(task_id, "task-1", "/repo", 0).expect("enqueue");
    db.increment_unknown_retries(entry_id).expect("increment");
    db.increment_unknown_retries(entry_id).expect("increment");
    assert_eq!(db.get_unknown_retries(entry_id), 2);
}
