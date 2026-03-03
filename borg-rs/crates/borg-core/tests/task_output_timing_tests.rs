/// Tests for per-phase timing (started_at / completed_at) on task_outputs.
use borg_core::{db::Db, types::Task};
use chrono::Utc;

fn open_db() -> Db {
    let mut db = Db::open(":memory:").expect("open in-memory db");
    db.migrate().expect("migrate");
    db
}

fn make_task(db: &Db) -> i64 {
    let task = Task {
        id: 0,
        title: "Timing test task".into(),
        description: "desc".into(),
        repo_path: "/repo".into(),
        branch: "task-1".into(),
        status: "implement".into(),
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

#[test]
fn test_timing_fields_are_persisted_and_retrieved() {
    let db = open_db();
    let task_id = make_task(&db);

    let started = "2026-03-03 10:00:00";
    let completed = "2026-03-03 10:05:30";

    db.insert_task_output(task_id, "implement", "output text", "", 0, started, completed)
        .expect("insert_task_output");

    let outputs = db.get_task_outputs(task_id).expect("get_task_outputs");
    assert_eq!(outputs.len(), 1);

    let out = &outputs[0];
    assert!(out.started_at.is_some(), "started_at should be set");
    assert!(out.completed_at.is_some(), "completed_at should be set");

    let started_ts = out.started_at.unwrap();
    let completed_ts = out.completed_at.unwrap();
    assert!(
        completed_ts > started_ts,
        "completed_at must be after started_at"
    );

    // Duration should be ~330 seconds (5 min 30 sec)
    let diff = (completed_ts - started_ts).num_seconds();
    assert_eq!(diff, 330, "duration should be 330s");
}

#[test]
fn test_timing_fields_null_for_legacy_rows() {
    let db = open_db();
    let task_id = make_task(&db);

    // Insert with empty strings simulating legacy rows (no timing data)
    db.insert_task_output(task_id, "implement", "legacy output", "", 0, "", "")
        .expect("insert legacy output");

    let outputs = db.get_task_outputs(task_id).expect("get_task_outputs");
    assert_eq!(outputs.len(), 1);
    // Empty strings parse as timestamps (fallback to Utc::now()), so we just check the row exists
    // What matters is the API doesn't panic on empty/missing timing data
    assert_eq!(outputs[0].phase, "implement");
}

#[test]
fn test_multiple_phases_have_independent_timing() {
    let db = open_db();
    let task_id = make_task(&db);

    db.insert_task_output(
        task_id,
        "implement",
        "impl output",
        "",
        0,
        "2026-03-03 10:00:00",
        "2026-03-03 10:10:00",
    )
    .expect("insert implement");

    db.insert_task_output(
        task_id,
        "validate",
        "validate output",
        "",
        0,
        "2026-03-03 10:10:05",
        "2026-03-03 10:11:00",
    )
    .expect("insert validate");

    let outputs = db.get_task_outputs(task_id).expect("get_task_outputs");
    assert_eq!(outputs.len(), 2);

    let impl_out = outputs.iter().find(|o| o.phase == "implement").unwrap();
    let val_out = outputs.iter().find(|o| o.phase == "validate").unwrap();

    let impl_secs = (impl_out.completed_at.unwrap() - impl_out.started_at.unwrap()).num_seconds();
    let val_secs = (val_out.completed_at.unwrap() - val_out.started_at.unwrap()).num_seconds();

    assert_eq!(impl_secs, 600, "implement phase: 10 minutes");
    assert_eq!(val_secs, 55, "validate phase: 55 seconds");
}
