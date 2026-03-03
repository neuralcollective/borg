use borg_core::stream::{TaskStreamManager, MAX_HISTORY_LINES};

// Pushing MAX_HISTORY_LINES + 1 lines evicts the oldest and retains the newest.
#[tokio::test]
async fn test_history_eviction_oldest_dropped_newest_retained() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 1001;
    manager.start(task_id).await;

    let first_line = "line_OLDEST_SENTINEL".to_string();
    manager.push_line(task_id, first_line.clone()).await;

    for i in 1..MAX_HISTORY_LINES {
        manager.push_line(task_id, format!("line_{i}")).await;
    }

    let last_line = "line_NEWEST_SENTINEL".to_string();
    manager.push_line(task_id, last_line.clone()).await;

    let (history, _rx) = manager.subscribe(task_id).await;

    assert_eq!(
        history.len(),
        MAX_HISTORY_LINES,
        "history must be capped at MAX_HISTORY_LINES"
    );
    assert!(
        !history.contains(&first_line),
        "oldest line must have been evicted"
    );
    assert_eq!(
        history.last().map(String::as_str),
        Some(last_line.as_str()),
        "newest line must be the last entry in history"
    );
}

// After end_task(), subscribe() returns non-empty history and None receiver.
#[tokio::test]
async fn test_ended_stream_subscribe_returns_history_and_no_receiver() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 1002;
    manager.start(task_id).await;

    manager.push_line(task_id, "some_line".to_string()).await;
    manager.end_task(task_id).await;

    let (history, rx) = manager.subscribe(task_id).await;

    assert!(!history.is_empty(), "history must be non-empty after end_task");
    assert!(rx.is_none(), "receiver must be None for an ended stream");
}

// Subscribing to a task that was never started returns empty history and None receiver.
#[tokio::test]
async fn test_never_started_subscribe_returns_empty_history_and_no_receiver() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 1003; // never started

    let (history, rx) = manager.subscribe(task_id).await;

    assert!(history.is_empty(), "history must be empty for unknown task");
    assert!(rx.is_none(), "receiver must be None for unknown task");
}
