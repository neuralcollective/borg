/// Tests for parse_ts error propagation (db.rs).
///
/// parse_ts must return an error for malformed timestamps instead of
/// silently substituting Utc::now(), which would corrupt ordering and audit records.
use borg_core::db::Db;
use borg_core::types::Task;
use chrono::Utc;

fn open_db() -> Db {
    let mut db = Db::open(":memory:").expect("open in-memory db");
    db.migrate().expect("migrate");
    db
}

fn make_task() -> Task {
    Task {
        id: 0,
        title: "ts-test".into(),
        description: String::new(),
        repo_path: "/repo".into(),
        branch: "task-1".into(),
        status: "backlog".into(),
        attempt: 0,
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
    }
}

/// Valid timestamps round-trip correctly through the DB.
#[test]
fn test_valid_timestamp_roundtrips() {
    let db = open_db();
    let task = make_task();
    let id = db.insert_task(&task).expect("insert");
    let loaded = db.get_task(id).expect("get").expect("exists");
    // created_at must be within a second of the inserted value.
    let delta = loaded.created_at.signed_duration_since(task.created_at);
    assert!(
        delta.num_seconds().abs() < 2,
        "created_at drifted by {delta:?} — parse_ts corrupted the timestamp"
    );
}

/// A row with a malformed created_at returns an error, not a silently-wrong timestamp.
#[test]
fn test_malformed_created_at_returns_error() {
    let db = open_db();
    let task = make_task();
    let id = db.insert_task(&task).expect("insert");

    // Corrupt the stored timestamp directly via raw connection.
    {
        let conn = db.raw_conn().lock().unwrap();
        conn.execute(
            "UPDATE pipeline_tasks SET created_at = 'not-a-date' WHERE id = ?1",
            rusqlite::params![id],
        )
        .expect("corrupt timestamp");
    }

    let result = db.get_task(id);
    assert!(
        result.is_err(),
        "expected an error for malformed created_at, got: {:?}",
        result
    );
}

/// A row with a malformed started_at returns an error.
#[test]
fn test_malformed_started_at_returns_error() {
    let db = open_db();
    let task = make_task();
    let id = db.insert_task(&task).expect("insert");

    {
        let conn = db.raw_conn().lock().unwrap();
        conn.execute(
            "UPDATE pipeline_tasks SET started_at = 'BOGUS' WHERE id = ?1",
            rusqlite::params![id],
        )
        .expect("corrupt started_at");
    }

    let result = db.get_task(id);
    assert!(
        result.is_err(),
        "expected an error for malformed started_at, got: {:?}",
        result
    );
}

/// A NULL started_at is fine (optional field).
#[test]
fn test_null_started_at_is_ok() {
    let db = open_db();
    let task = make_task();
    let id = db.insert_task(&task).expect("insert");

    let loaded = db.get_task(id).expect("get").expect("exists");
    assert!(loaded.started_at.is_none(), "started_at should be None");
}
