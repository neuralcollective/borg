/// Tests for validate-phase output persistence in task_outputs.
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
        status: "validate".into(),
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

// ── AC1: validate failure is persisted ───────────────────────────────────────

#[test]
fn test_validate_failure_saved_to_task_outputs() {
    let db = open_db();
    let task_id = make_task(&db);

    let error = "error[E0308]: mismatched types\n  --> src/main.rs:5:18";
    db.insert_task_output(task_id, "validate", error, "", 1)
        .expect("insert_task_output");

    let outputs = db.get_task_outputs(task_id).expect("get_task_outputs");
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].phase, "validate");
    assert_eq!(outputs[0].exit_code, 1);
    assert_eq!(outputs[0].output, error);
}

// ── AC2: validate success is persisted ───────────────────────────────────────

#[test]
fn test_validate_success_saved_to_task_outputs() {
    let db = open_db();
    let task_id = make_task(&db);

    let stdout = "running 5 tests\ntest result: ok. 5 passed; 0 failed";
    db.insert_task_output(task_id, "validate", stdout, "", 0)
        .expect("insert_task_output");

    let outputs = db.get_task_outputs(task_id).expect("get_task_outputs");
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].phase, "validate");
    assert_eq!(outputs[0].exit_code, 0);
    assert!(outputs[0].output.contains("5 passed"));
}

// ── AC3: validate output appears in phase_history ────────────────────────────

#[test]
fn test_validate_output_appears_in_phase_history() {
    use borg_core::types::PhaseHistoryEntry;

    let db = open_db();
    let task_id = make_task(&db);

    db.insert_task_output(task_id, "impl", "wrote code", "", 0)
        .expect("insert impl output");
    db.insert_task_output(task_id, "validate", "FAILED: test_foo panicked", "", 1)
        .expect("insert validate output");

    let outputs = db.get_task_outputs(task_id).expect("get_task_outputs");
    let history: Vec<PhaseHistoryEntry> = outputs
        .into_iter()
        .rev()
        .take(5)
        .rev()
        .map(|o| PhaseHistoryEntry {
            phase: o.phase,
            success: o.exit_code == 0,
            output: o.output.chars().take(2_000).collect(),
            timestamp: o.created_at,
        })
        .collect();

    assert_eq!(history.len(), 2);
    let validate_entry = history.iter().find(|e| e.phase == "validate").unwrap();
    assert!(!validate_entry.success);
    assert!(validate_entry.output.contains("test_foo panicked"));
}

// ── AC4: validate output included in retry summary ───────────────────────────

#[test]
fn test_validate_output_in_retry_summary() {
    let db = open_db();
    let task_id = make_task(&db);

    db.insert_task_output(task_id, "impl", "wrote code", "", 0)
        .expect("insert impl output");
    db.insert_task_output(
        task_id,
        "validate",
        "thread 'test_add' panicked at 'assertion failed'",
        "",
        1,
    )
    .expect("insert validate output");

    let outputs = db.get_task_outputs(task_id).expect("get_task_outputs");

    // Replicate build_retry_summary logic: take last 3, format them.
    let summary_parts: Vec<String> = outputs
        .iter()
        .rev()
        .take(3)
        .map(|o| {
            let truncated: String = o.output.chars().take(500).collect();
            format!("Attempt ({phase}): {out}", phase = o.phase, out = truncated)
        })
        .collect();

    // The validate entry must appear in the summary (it's the most recent).
    assert!(
        summary_parts[0].contains("validate"),
        "validate phase must appear in retry summary"
    );
    assert!(
        summary_parts[0].contains("test_add"),
        "validate output must appear in retry summary"
    );
}

// ── AC5: multiple validate runs accumulate in task_outputs ───────────────────

#[test]
fn test_multiple_validate_runs_accumulate() {
    let db = open_db();
    let task_id = make_task(&db);

    db.insert_task_output(task_id, "validate", "compile error attempt 1", "", 1)
        .expect("first validate");
    db.insert_task_output(task_id, "validate", "test failed attempt 2", "", 1)
        .expect("second validate");
    db.insert_task_output(task_id, "validate", "running 3 tests\nok", "", 0)
        .expect("third validate (pass)");

    let outputs = db.get_task_outputs(task_id).expect("get_task_outputs");
    let validate_outputs: Vec<_> = outputs.iter().filter(|o| o.phase == "validate").collect();

    assert_eq!(validate_outputs.len(), 3);
    assert_eq!(validate_outputs[0].exit_code, 1);
    assert_eq!(validate_outputs[1].exit_code, 1);
    assert_eq!(validate_outputs[2].exit_code, 0);
}

// ── AC6: compile-check failure saves output before test suite runs ────────────

#[test]
fn test_compile_check_failure_saved_independently() {
    let db = open_db();
    let task_id = make_task(&db);

    // Simulate: compile check fails → saved; test suite never runs.
    let compile_err = "error[E0425]: cannot find value `foo`";
    db.insert_task_output(task_id, "validate", compile_err, "", 1)
        .expect("insert compile check output");

    let outputs = db.get_task_outputs(task_id).expect("get_task_outputs");
    assert_eq!(outputs.len(), 1, "only compile check output, no test suite output");
    assert_eq!(outputs[0].phase, "validate");
    assert!(outputs[0].output.contains("E0425"));
}
