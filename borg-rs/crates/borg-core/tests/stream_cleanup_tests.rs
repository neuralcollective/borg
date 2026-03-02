use borg_core::stream::TaskStreamManager;

// cleanup_ended_streams(0) removes all ended streams immediately (elapsed >= 0s always)
#[tokio::test]
async fn test_cleanup_removes_ended_stream() {
    let manager = TaskStreamManager::new();
    manager.start(1).await;
    manager.end_task(1).await;

    let removed = manager.cleanup_ended_streams(0).await;
    assert_eq!(removed, 1);

    // Subscribe returns empty history — stream is gone.
    let (history, rx) = manager.subscribe(1).await;
    assert!(history.is_empty());
    assert!(rx.is_none());
}

// Active (non-ended) streams are never removed.
#[tokio::test]
async fn test_cleanup_does_not_remove_active_stream() {
    let manager = TaskStreamManager::new();
    manager.start(2).await;
    manager.push_line(2, "hello".to_string()).await;

    let removed = manager.cleanup_ended_streams(0).await;
    assert_eq!(removed, 0);

    let (history, _rx) = manager.subscribe(2).await;
    assert!(!history.is_empty());
}

// Streams below the age threshold are not removed.
#[tokio::test]
async fn test_cleanup_respects_age_threshold() {
    let manager = TaskStreamManager::new();
    manager.start(3).await;
    manager.end_task(3).await;

    // Ended just now — still within u64::MAX seconds.
    let removed = manager.cleanup_ended_streams(u64::MAX).await;
    assert_eq!(removed, 0);

    // Stream should still be accessible.
    let (history, _rx) = manager.subscribe(3).await;
    assert!(!history.is_empty(), "stream_end line should still be in history");
}

// Only ended streams matching the age threshold are removed; active ones survive.
#[tokio::test]
async fn test_cleanup_mixed_active_and_ended() {
    let manager = TaskStreamManager::new();
    manager.start(10).await; // active
    manager.start(11).await; // will be ended
    manager.push_line(10, "still running".to_string()).await;
    manager.end_task(11).await;

    let removed = manager.cleanup_ended_streams(0).await;
    assert_eq!(removed, 1);

    // Active stream still accessible.
    let (history_active, rx_active) = manager.subscribe(10).await;
    assert!(!history_active.is_empty());
    assert!(rx_active.is_some());

    // Ended stream is gone.
    let (history_ended, _) = manager.subscribe(11).await;
    assert!(history_ended.is_empty());
}

// Multiple ended streams are all cleaned up.
#[tokio::test]
async fn test_cleanup_removes_multiple_ended_streams() {
    let manager = TaskStreamManager::new();
    for id in 20..25_i64 {
        manager.start(id).await;
        manager.end_task(id).await;
    }

    let removed = manager.cleanup_ended_streams(0).await;
    assert_eq!(removed, 5);
}

// cleanup_ended_streams on an empty manager is a no-op.
#[tokio::test]
async fn test_cleanup_empty_manager_is_noop() {
    let manager = TaskStreamManager::new();
    let removed = manager.cleanup_ended_streams(0).await;
    assert_eq!(removed, 0);
}

// History is preserved up until cleanup removes the stream.
#[tokio::test]
async fn test_cleanup_removes_history_with_stream() {
    let manager = TaskStreamManager::new();
    manager.start(30).await;
    manager.push_line(30, "line1".to_string()).await;
    manager.push_line(30, "line2".to_string()).await;
    manager.end_task(30).await;

    // History exists before cleanup.
    let (history_before, _) = manager.subscribe(30).await;
    assert_eq!(history_before.len(), 3); // 2 lines + stream_end

    manager.cleanup_ended_streams(0).await;

    // History is gone after cleanup.
    let (history_after, _) = manager.subscribe(30).await;
    assert!(history_after.is_empty());
}
