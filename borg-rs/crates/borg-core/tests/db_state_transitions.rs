/// Tests for critical DB state-transition functions that were previously
/// silently ignored with `let _ =` in pipeline.rs.
///
/// These tests verify that:
/// 1. The functions return Ok(()) and correctly update state on valid inputs.
/// 2. The functions produce observable side-effects (timestamps set, etc.) so
///    their failure would be genuinely load-bearing — justifying warn! logging.
use borg_core::{
    db::Db,
    types::Task,
};
use chrono::Utc;

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

// ── mark_task_started ─────────────────────────────────────────────────────────

#[test]
fn test_mark_task_started_sets_started_at() {
    let db = open_db();
    let id = make_task(&db);

    db.mark_task_started(id).expect("mark_task_started");

    let task = db.get_task(id).expect("get_task").expect("task exists");
    assert!(
        task.started_at.is_some(),
        "started_at must be set after mark_task_started"
    );
}

#[test]
fn test_mark_task_started_is_idempotent() {
    let db = open_db();
    let id = make_task(&db);

    db.mark_task_started(id).expect("first call");
    let task_after_first = db.get_task(id).expect("get").expect("exists");
    let ts1 = task_after_first.started_at.clone();

    // COALESCE ensures a second call does not overwrite started_at.
    db.mark_task_started(id).expect("second call");
    let task_after_second = db.get_task(id).expect("get").expect("exists");
    let ts2 = task_after_second.started_at;

    assert_eq!(ts1, ts2, "started_at must not change on repeated calls");
}

#[test]
fn test_mark_task_started_on_nonexistent_task_returns_ok() {
    // SQLite UPDATE against a missing row succeeds with 0 rows affected — no error.
    // This means silent discard was hiding a logic gap, not a DB error.
    let db = open_db();
    let result = db.mark_task_started(99999);
    assert!(result.is_ok(), "UPDATE on missing row must not error in SQLite");
}

// ── mark_task_completed ───────────────────────────────────────────────────────

#[test]
fn test_mark_task_completed_sets_completed_at() {
    let db = open_db();
    let id = make_task(&db);

    db.mark_task_completed(id).expect("mark_task_completed");

    let task = db.get_task(id).expect("get_task").expect("task exists");
    assert!(
        task.completed_at.is_some(),
        "completed_at must be set after mark_task_completed"
    );
}

#[test]
fn test_mark_task_completed_computes_duration_when_started() {
    let db = open_db();
    let id = make_task(&db);

    db.mark_task_started(id).expect("start");
    db.mark_task_completed(id).expect("complete");

    let task = db.get_task(id).expect("get_task").expect("task exists");
    // duration_secs is CASE-computed from started_at; 0 is valid for fast ops.
    assert!(
        task.duration_secs.is_some(),
        "duration_secs must be populated when started_at is set"
    );
}

#[test]
fn test_mark_task_completed_without_started_leaves_duration_null() {
    let db = open_db();
    let id = make_task(&db);

    // Do NOT call mark_task_started — started_at remains NULL.
    db.mark_task_completed(id).expect("complete without start");

    let task = db.get_task(id).expect("get_task").expect("task exists");
    assert!(
        task.duration_secs.is_none(),
        "duration_secs must be NULL when started_at was never set"
    );
}

// ── fts_remove_task ───────────────────────────────────────────────────────────

#[test]
fn test_fts_remove_task_succeeds_when_no_entries() {
    // Removing FTS entries for a task that has none should not error.
    let db = open_db();
    let id = make_task(&db);

    let result = db.fts_remove_task(id);
    assert!(result.is_ok(), "fts_remove_task must succeed even with no entries");
}

// ── set_seed_cooldown ─────────────────────────────────────────────────────────

#[test]
fn test_set_seed_cooldown_succeeds() {
    let db = open_db();
    let now = chrono::Utc::now().timestamp();

    let result = db.set_seed_cooldown("/repo/path", "github_open_issues", now);
    assert!(result.is_ok(), "set_seed_cooldown must succeed");
}

#[test]
fn test_set_seed_cooldown_is_overwritable() {
    let db = open_db();
    let t1 = 1_700_000_000i64;
    let t2 = 1_700_001_000i64;

    db.set_seed_cooldown("/repo", "myseed", t1).expect("first write");
    let result = db.set_seed_cooldown("/repo", "myseed", t2);
    assert!(result.is_ok(), "overwriting cooldown must succeed");
}

// ── log_event_full ────────────────────────────────────────────────────────────

#[test]
fn test_log_event_full_succeeds_for_task_event() {
    let db = open_db();
    let id = make_task(&db);

    let result = db.log_event_full(
        Some(id),
        None,
        None,
        "pipeline",
        "task.completed",
        &serde_json::json!({ "title": "Test task" }),
    );
    assert!(result.is_ok(), "log_event_full must succeed for a valid task");
}

#[test]
fn test_log_event_full_succeeds_with_no_task_id() {
    let db = open_db();

    let result = db.log_event_full(
        None,
        None,
        None,
        "pipeline",
        "system.startup",
        &serde_json::json!({}),
    );
    assert!(result.is_ok(), "log_event_full must succeed without a task_id");
}
