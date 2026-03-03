use borg_core::db::Db;

fn open_db() -> Db {
    let mut db = Db::open(":memory:").expect("open in-memory db");
    db.migrate().expect("migrate");
    db
}

/// Poisons the db's internal mutex by locking it from another thread and panicking.
fn poison(db: &Db) {
    let mutex = db.raw_conn();
    // Wrap in Arc to share with the panicking thread.
    // Safety: we only use this borrow inside this function's scope.
    let ptr = mutex as *const _ as usize;
    let handle = std::thread::spawn(move || {
        let mutex = unsafe { &*(ptr as *const std::sync::Mutex<rusqlite::Connection>) };
        let _guard = mutex.lock().unwrap();
        panic!("deliberate panic to poison mutex");
    });
    // The thread panicked; its panic is expected.
    let _ = handle.join(); // returns Err because the thread panicked
}

#[test]
fn get_task_returns_err_on_poisoned_lock() {
    let db = open_db();
    poison(&db);
    let result = db.get_task(1);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("poisoned"), "expected 'poisoned' in error, got: {msg}");
}

#[test]
fn list_active_tasks_returns_err_on_poisoned_lock() {
    let db = open_db();
    poison(&db);
    let result = db.list_active_tasks();
    assert!(result.is_err());
}

#[test]
fn insert_task_returns_err_on_poisoned_lock() {
    use borg_core::types::Task;
    use chrono::Utc;

    let db = open_db();
    poison(&db);

    let task = Task {
        id: 0,
        title: "test".into(),
        description: "desc".into(),
        repo_path: "/repo".into(),
        branch: "main".into(),
        status: "pending".into(),
        attempt: 0,
        max_attempts: 3,
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
    let result = db.insert_task(&task);
    assert!(result.is_err());
}

#[test]
fn active_task_count_returns_err_on_poisoned_lock() {
    let db = open_db();
    poison(&db);
    let result = db.active_task_count();
    assert!(result.is_err());
}
