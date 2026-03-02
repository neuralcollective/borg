use borg_core::stream::TaskStreamManager;

#[tokio::test]
async fn test_remove_clears_entry() {
    let manager = TaskStreamManager::new();
    manager.start(1).await;
    manager.push_line(1, "line".to_string()).await;

    manager.remove(1).await;

    // After removal, subscribe returns empty history with no live receiver.
    let (history, rx) = manager.subscribe(1).await;
    assert!(history.is_empty(), "history must be empty after remove");
    assert!(rx.is_none(), "no live receiver after remove");
}

#[tokio::test]
async fn test_remove_nonexistent_is_noop() {
    let manager = TaskStreamManager::new();
    // Must not panic.
    manager.remove(9999).await;
}

#[tokio::test]
async fn test_remove_does_not_affect_other_tasks() {
    let manager = TaskStreamManager::new();
    manager.start(10).await;
    manager.start(11).await;
    manager.push_line(10, "a".to_string()).await;
    manager.push_line(11, "b".to_string()).await;

    manager.remove(10).await;

    let (h10, _) = manager.subscribe(10).await;
    let (h11, _) = manager.subscribe(11).await;
    assert!(h10.is_empty(), "removed task history must be empty");
    assert_eq!(h11.len(), 1, "other task history must be intact");
}

#[tokio::test]
async fn test_remove_then_start_resets_stream() {
    let manager = TaskStreamManager::new();
    manager.start(20).await;
    manager.push_line(20, "old line".to_string()).await;

    manager.remove(20).await;
    manager.start(20).await;

    let (history, _) = manager.subscribe(20).await;
    assert!(history.is_empty(), "restarted stream must have empty history");
}

#[tokio::test]
async fn test_remove_after_end_task() {
    let manager = TaskStreamManager::new();
    manager.start(30).await;
    manager.push_line(30, "data".to_string()).await;
    manager.end_task(30).await;

    manager.remove(30).await;

    let (history, rx) = manager.subscribe(30).await;
    assert!(history.is_empty());
    assert!(rx.is_none());
}
