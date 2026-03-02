use borg_core::{db::Db, types::Task};
use chrono::Utc;
use serde_json::json;

fn open_db() -> Db {
    let mut db = Db::open(":memory:").expect("open in-memory db");
    db.migrate().expect("migrate");
    db
}

fn make_task(db: &Db) -> i64 {
    let task = Task {
        id: 0,
        title: "Timing test task".into(),
        description: String::new(),
        repo_path: "/repo".into(),
        branch: String::new(),
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
    };
    db.insert_task(&task).expect("insert_task")
}

#[test]
fn test_get_phase_timings_empty_for_new_task() {
    let db = open_db();
    let task_id = make_task(&db);
    let timings = db.get_phase_timings(task_id).expect("get_phase_timings");
    assert!(timings.is_empty(), "new task should have no timings");
}

#[test]
fn test_get_phase_timings_single_completed_phase() {
    let db = open_db();
    let task_id = make_task(&db);

    db.log_event(Some(task_id), None, "phase_started", &json!({"phase": "implement", "attempt": 1}))
        .expect("log phase_started");
    db.log_event(Some(task_id), None, "phase_completed", &json!({"phase": "implement", "attempt": 1, "duration_ms": 5000}))
        .expect("log phase_completed");

    let timings = db.get_phase_timings(task_id).expect("get_phase_timings");
    assert_eq!(timings.len(), 1);
    assert_eq!(timings[0].phase, "implement");
    assert_eq!(timings[0].attempt, 1);
    assert_eq!(timings[0].duration_ms, Some(5000));
    assert!(timings[0].ended_at.is_some());
}

#[test]
fn test_get_phase_timings_in_progress_phase() {
    let db = open_db();
    let task_id = make_task(&db);

    db.log_event(Some(task_id), None, "phase_started", &json!({"phase": "implement", "attempt": 1}))
        .expect("log phase_started");
    // No phase_completed logged — phase is still running.

    let timings = db.get_phase_timings(task_id).expect("get_phase_timings");
    assert_eq!(timings.len(), 1);
    assert_eq!(timings[0].phase, "implement");
    assert!(timings[0].ended_at.is_none());
    assert!(timings[0].duration_ms.is_none());
}

#[test]
fn test_get_phase_timings_multiple_phases() {
    let db = open_db();
    let task_id = make_task(&db);

    db.log_event(Some(task_id), None, "phase_started", &json!({"phase": "implement", "attempt": 1})).unwrap();
    db.log_event(Some(task_id), None, "phase_completed", &json!({"phase": "implement", "attempt": 1, "duration_ms": 3000})).unwrap();
    db.log_event(Some(task_id), None, "phase_started", &json!({"phase": "validate", "attempt": 1})).unwrap();
    db.log_event(Some(task_id), None, "phase_completed", &json!({"phase": "validate", "attempt": 1, "duration_ms": 1500})).unwrap();

    let timings = db.get_phase_timings(task_id).expect("get_phase_timings");
    assert_eq!(timings.len(), 2);

    let impl_timing = timings.iter().find(|t| t.phase == "implement").unwrap();
    assert_eq!(impl_timing.duration_ms, Some(3000));

    let val_timing = timings.iter().find(|t| t.phase == "validate").unwrap();
    assert_eq!(val_timing.duration_ms, Some(1500));
}

#[test]
fn test_get_phase_timings_multiple_attempts() {
    let db = open_db();
    let task_id = make_task(&db);

    // Two separate implement attempts.
    db.log_event(Some(task_id), None, "phase_started", &json!({"phase": "implement", "attempt": 1})).unwrap();
    db.log_event(Some(task_id), None, "phase_completed", &json!({"phase": "implement", "attempt": 1, "duration_ms": 2000})).unwrap();
    db.log_event(Some(task_id), None, "phase_started", &json!({"phase": "implement", "attempt": 2})).unwrap();
    db.log_event(Some(task_id), None, "phase_completed", &json!({"phase": "implement", "attempt": 2, "duration_ms": 4000})).unwrap();

    let timings = db.get_phase_timings(task_id).expect("get_phase_timings");
    assert_eq!(timings.len(), 2);

    let a1 = timings.iter().find(|t| t.attempt == 1).unwrap();
    assert_eq!(a1.duration_ms, Some(2000));

    let a2 = timings.iter().find(|t| t.attempt == 2).unwrap();
    assert_eq!(a2.duration_ms, Some(4000));
}

#[test]
fn test_get_phase_timings_excludes_other_tasks() {
    let db = open_db();
    let task_a = make_task(&db);
    let task_b = make_task(&db);

    db.log_event(Some(task_a), None, "phase_started", &json!({"phase": "implement", "attempt": 1})).unwrap();
    db.log_event(Some(task_a), None, "phase_completed", &json!({"phase": "implement", "attempt": 1, "duration_ms": 1000})).unwrap();
    db.log_event(Some(task_b), None, "phase_started", &json!({"phase": "implement", "attempt": 1})).unwrap();
    db.log_event(Some(task_b), None, "phase_completed", &json!({"phase": "implement", "attempt": 1, "duration_ms": 9999})).unwrap();

    let timings_a = db.get_phase_timings(task_a).expect("get_phase_timings task_a");
    assert_eq!(timings_a.len(), 1);
    assert_eq!(timings_a[0].duration_ms, Some(1000));
}

#[test]
fn test_phase_timing_struct_serializes() {
    use borg_core::types::PhaseTiming;
    let t = PhaseTiming {
        phase: "implement".into(),
        attempt: 1,
        started_at: "2024-01-01T00:00:00".into(),
        ended_at: Some("2024-01-01T00:01:00".into()),
        duration_ms: Some(60000),
    };
    let json = serde_json::to_string(&t).expect("serialize");
    let restored: PhaseTiming = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored.phase, "implement");
    assert_eq!(restored.attempt, 1);
    assert_eq!(restored.duration_ms, Some(60000));
}

#[test]
fn test_phase_timing_no_ended_at_serializes_as_null() {
    use borg_core::types::PhaseTiming;
    let t = PhaseTiming {
        phase: "implement".into(),
        attempt: 1,
        started_at: "2024-01-01T00:00:00".into(),
        ended_at: None,
        duration_ms: None,
    };
    let json = serde_json::to_string(&t).expect("serialize");
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(v["ended_at"].is_null());
    assert!(v["duration_ms"].is_null());
}
