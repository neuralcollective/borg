use std::collections::HashSet;

use borg_core::pipeline::collect_stale_session_dirs;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

// Create a task-N dir inside sessions_dir and return its path.
fn make_session_dir(sessions_dir: &std::path::Path, task_id: i64) -> std::path::PathBuf {
    let dir = sessions_dir.join(format!("task-{task_id}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_stale_dir_is_collected() {
    let tmp = tempfile::tempdir().unwrap();
    let sessions = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    make_session_dir(&sessions, 1);

    let max_age = 3600i64;
    let now = now_secs();
    // Task created 2 hours ago → stale
    let created_at = now - 7200;

    let stale = collect_stale_session_dirs(
        sessions.to_str().unwrap(),
        now,
        max_age,
        &HashSet::new(),
        |_id| Some(created_at),
    );

    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0], sessions.join("task-1"));
}

#[test]
fn test_recent_dir_is_not_collected() {
    let tmp = tempfile::tempdir().unwrap();
    let sessions = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    make_session_dir(&sessions, 2);

    let max_age = 3600i64;
    let now = now_secs();
    // Task created 30 minutes ago → not stale
    let created_at = now - 1800;

    let stale = collect_stale_session_dirs(
        sessions.to_str().unwrap(),
        now,
        max_age,
        &HashSet::new(),
        |_id| Some(created_at),
    );

    assert!(stale.is_empty());
}

#[test]
fn test_in_flight_dir_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let sessions = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    make_session_dir(&sessions, 3);

    let max_age = 3600i64;
    let now = now_secs();
    let created_at = now - 7200; // would be stale

    let mut skip = HashSet::new();
    skip.insert(3i64);

    let stale = collect_stale_session_dirs(
        sessions.to_str().unwrap(),
        now,
        max_age,
        &skip,
        |_id| Some(created_at),
    );

    assert!(stale.is_empty(), "in-flight task dir must not be collected");
}

#[test]
fn test_orphaned_dir_uses_mtime_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let sessions = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    make_session_dir(&sessions, 4);

    let max_age = 3600i64;
    // Set now far into the future so the real mtime appears very old
    let now = now_secs() + 7 * 86_400; // 7 days from now

    // task_created_at returns None → orphaned
    let stale = collect_stale_session_dirs(
        sessions.to_str().unwrap(),
        now,
        max_age,
        &HashSet::new(),
        |_id| None,
    );

    assert_eq!(stale.len(), 1, "orphaned stale dir must be collected");
}

#[test]
fn test_non_task_dirs_are_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let sessions = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    // "seed" and "other" dirs should be ignored
    std::fs::create_dir_all(sessions.join("seed")).unwrap();
    std::fs::create_dir_all(sessions.join("other")).unwrap();

    let max_age = 0i64; // anything is stale
    let now = now_secs();

    let stale = collect_stale_session_dirs(
        sessions.to_str().unwrap(),
        now,
        max_age,
        &HashSet::new(),
        |_id| Some(0), // very old
    );

    assert!(stale.is_empty(), "non-task-N dirs must not be collected");
}

#[test]
fn test_multiple_tasks_only_stale_collected() {
    let tmp = tempfile::tempdir().unwrap();
    let sessions = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    make_session_dir(&sessions, 10);
    make_session_dir(&sessions, 11);
    make_session_dir(&sessions, 12);

    let max_age = 3600i64;
    let now = now_secs();

    let stale = collect_stale_session_dirs(
        sessions.to_str().unwrap(),
        now,
        max_age,
        &HashSet::new(),
        |id| match id {
            10 => Some(now - 7200), // stale
            11 => Some(now - 1800), // fresh
            12 => Some(now - 3601), // just over threshold → stale
            _ => None,
        },
    );

    let stale_names: HashSet<String> = stale
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();

    assert!(stale_names.contains("task-10"), "task-10 must be stale");
    assert!(!stale_names.contains("task-11"), "task-11 must not be stale");
    assert!(stale_names.contains("task-12"), "task-12 must be stale");
    assert_eq!(stale.len(), 2);
}

#[test]
fn test_missing_sessions_dir_returns_empty() {
    let stale = collect_stale_session_dirs(
        "/tmp/borg-test-nonexistent-sessions-dir",
        now_secs(),
        3600,
        &HashSet::new(),
        |_| None,
    );
    assert!(stale.is_empty());
}

#[test]
fn test_exactly_at_threshold_is_stale() {
    let tmp = tempfile::tempdir().unwrap();
    let sessions = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    make_session_dir(&sessions, 5);

    let max_age = 3600i64;
    let now = now_secs();
    // Created exactly max_age seconds ago
    let created_at = now - max_age;

    let stale = collect_stale_session_dirs(
        sessions.to_str().unwrap(),
        now,
        max_age,
        &HashSet::new(),
        |_id| Some(created_at),
    );

    assert_eq!(stale.len(), 1, "dir at exact threshold must be collected");
}
