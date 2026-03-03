use std::sync::Arc;

use borg_core::db::Db;

fn open_db() -> Db {
    let mut db = Db::open(":memory:").expect("open in-memory db");
    db.migrate().expect("migrate");
    db
}

// Simulate a thread panicking mid-transaction while holding the DB lock.
// After recovery, lock_conn must roll back the stale transaction so the
// connection is clean for the next caller.
#[test]
fn test_poisoned_lock_recovery_rolls_back_open_transaction() {
    let db = Arc::new(open_db());

    let db2 = Arc::clone(&db);
    let _ = std::thread::spawn(move || {
        let conn = db2.raw_conn().lock().unwrap();
        conn.execute_batch("BEGIN").unwrap();
        // Panic with an open transaction — poisons the mutex.
        panic!("simulated mid-transaction panic");
    })
    .join();

    // The mutex must be poisoned now.
    assert!(
        db.raw_conn().lock().is_err(),
        "mutex should be poisoned after thread panic"
    );

    // A normal DB call should succeed: lock_conn recovers the guard and
    // rolls back the stale transaction before returning it.
    let tasks = db
        .list_active_tasks()
        .expect("DB operation succeeds after poisoned-mutex recovery");
    assert!(tasks.is_empty());

    // The mutex stays poisoned indefinitely in Rust — that is expected.
    // Subsequent calls through lock_conn must still succeed.
    let tasks2 = db
        .list_active_tasks()
        .expect("second call also succeeds");
    assert!(tasks2.is_empty());
}

// If there is no open transaction when the mutex is recovered, ROLLBACK is a
// no-op and the connection is still usable.
#[test]
fn test_poisoned_lock_recovery_without_open_transaction() {
    let db = Arc::new(open_db());

    let db2 = Arc::clone(&db);
    let _ = std::thread::spawn(move || {
        // Lock but do NOT begin a transaction — just panic.
        let _conn = db2.raw_conn().lock().unwrap();
        panic!("panic without transaction");
    })
    .join();

    assert!(db.raw_conn().lock().is_err(), "mutex must be poisoned");

    // Recovery with no open transaction should still work cleanly.
    let tasks = db
        .list_active_tasks()
        .expect("DB usable after recovery without open tx");
    assert!(tasks.is_empty());
}
