// Tests for TaskStreamManager::subscribe() contract.
//
// Covers:
//   - non-existent task_id returns empty history and None receiver
//   - active stream returns Some receiver
//   - after end_task() returns full history snapshot and None receiver
//   - start() on same task_id resets history and ended flag

use borg_core::stream::TaskStreamManager;

#[tokio::test]
async fn test_subscribe_nonexistent_task_returns_empty_history_and_none_receiver() {
    let manager = TaskStreamManager::new();
    let (history, rx) = manager.subscribe(9999).await;
    assert!(history.is_empty(), "history must be empty for non-existent task");
    assert!(rx.is_none(), "receiver must be None for non-existent task");
}

#[tokio::test]
async fn test_subscribe_active_stream_returns_some_receiver() {
    let manager = TaskStreamManager::new();
    manager.start(1).await;
    let (_history, rx) = manager.subscribe(1).await;
    assert!(rx.is_some(), "receiver must be Some for active stream");
}

#[tokio::test]
async fn test_subscribe_after_end_task_returns_full_history_and_none_receiver() {
    let manager = TaskStreamManager::new();
    manager.start(2).await;
    manager.push_line(2, "line one".to_string()).await;
    manager.push_line(2, "line two".to_string()).await;
    manager.end_task(2).await;

    let (history, rx) = manager.subscribe(2).await;

    assert!(rx.is_none(), "receiver must be None after stream has ended");
    // history includes both pushed lines plus the stream_end sentinel
    assert!(
        history.iter().any(|l| l.contains("line one")),
        "history must contain 'line one'"
    );
    assert!(
        history.iter().any(|l| l.contains("line two")),
        "history must contain 'line two'"
    );
    assert!(
        history.iter().any(|l| l.contains("stream_end")),
        "history must contain stream_end"
    );
}

#[tokio::test]
async fn test_start_second_time_resets_history_and_ended_flag() {
    let manager = TaskStreamManager::new();

    // First run: push some lines, end the stream.
    manager.start(3).await;
    manager.push_line(3, "old line".to_string()).await;
    manager.end_task(3).await;

    // Second start on same task_id.
    manager.start(3).await;

    let (history, rx) = manager.subscribe(3).await;

    assert!(
        history.is_empty(),
        "history must be empty after second start(), got: {history:?}"
    );
    assert!(
        rx.is_some(),
        "receiver must be Some after second start() (stream is active again)"
    );
}
