/// Verifies that a poisoned Mutex in Db propagates as a panic rather than
/// silently resuming with a potentially-corrupt connection state.
use borg_core::db::Db;
use borg_core::types::Task;
use chrono::Utc;
use std::panic;
use std::sync::Arc;

fn open_db() -> Db {
    let mut db = Db::open(":memory:").expect("open in-memory db");
    db.migrate().expect("migrate");
    db
}

fn make_task() -> Task {
    Task {
        id: 0,
        title: "Poison test".into(),
        description: "".into(),
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
    }
}

/// If a thread panics while holding the Db's connection lock, subsequent
/// callers must panic rather than silently proceed with a potentially-corrupt
/// connection. This test verifies that behavior by directly poisoning the
/// underlying Mutex.
#[test]
fn poisoned_db_mutex_panics_on_access() {
    let db = Arc::new(open_db());
    let db2 = Arc::clone(&db);

    // Poison the mutex by panicking while holding it.
    let handle = std::thread::spawn(move || {
        let _guard = db2.raw_conn().lock().unwrap();
        panic!("intentional panic to poison the mutex");
    });

    // The spawned thread must have panicked.
    assert!(handle.join().is_err(), "thread should have panicked");

    // Now the mutex is poisoned; any Db method must panic.
    let result = panic::catch_unwind(|| {
        let _ = db.get_task(1);
    });

    assert!(
        result.is_err(),
        "Db::get_task must panic when the connection mutex is poisoned"
    );
}

/// A non-poisoned Db continues to work normally after an unrelated thread panics.
#[test]
fn healthy_db_unaffected_by_unrelated_panic() {
    let db = open_db();
    // Unrelated panic in another thread.
    let _ = std::thread::spawn(|| panic!("unrelated")).join();

    // db's mutex is still healthy — insert must succeed.
    let id = db.insert_task(&make_task()).expect("insert should succeed");
    assert!(id > 0);
}
