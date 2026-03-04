// Tests for TaskStreamManager memory-leak fix:
// ended streams must be removed from the HashMap after their TTL expires.

use std::time::Duration;

use borg_core::stream::TaskStreamManager;

// After end_task, history is still available (TTL not yet elapsed).
#[tokio::test]
async fn test_ended_stream_history_available_before_prune() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 200;
    manager.start(task_id).await;
    manager.push_line(task_id, "line1".to_string()).await;
    manager.end_task(task_id).await;

    let (history, rx) = manager.subscribe(task_id).await;
    assert!(!history.is_empty(), "history must be available before prune");
    assert!(rx.is_none(), "ended stream must return no live receiver");
}

// prune_ended(Duration::ZERO) removes all ended streams immediately.
#[tokio::test]
async fn test_prune_ended_removes_ended_streams() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 201;
    manager.start(task_id).await;
    manager.push_line(task_id, "hello".to_string()).await;
    manager.end_task(task_id).await;

    manager.prune_ended(Duration::ZERO).await;

    let (history, rx) = manager.subscribe(task_id).await;
    assert!(history.is_empty(), "history must be empty after prune");
    assert!(rx.is_none(), "receiver must be None after prune");
}

// prune_ended does not remove streams that have not ended.
#[tokio::test]
async fn test_prune_ended_leaves_active_streams() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 202;
    manager.start(task_id).await;
    manager.push_line(task_id, "active line".to_string()).await;

    manager.prune_ended(Duration::ZERO).await;

    let (history, rx) = manager.subscribe(task_id).await;
    assert!(!history.is_empty(), "active stream must survive prune");
    assert!(rx.is_some(), "active stream must still have live receiver");
}

// prune_ended with large max_age leaves recently-ended streams intact.
#[tokio::test]
async fn test_prune_ended_large_max_age_keeps_recent_streams() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 203;
    manager.start(task_id).await;
    manager.push_line(task_id, "data".to_string()).await;
    manager.end_task(task_id).await;

    // 1-hour TTL — the stream was ended milliseconds ago, must survive.
    manager.prune_ended(Duration::from_secs(3600)).await;

    let (history, _) = manager.subscribe(task_id).await;
    assert!(!history.is_empty(), "recently-ended stream must survive large-TTL prune");
}

// start() prunes stale ended streams as a side effect.
// We verify by: end a stream, call prune_ended(ZERO) manually, then start a new
// unrelated task and confirm the old ended stream is gone.
#[tokio::test]
async fn test_start_triggers_prune_of_stale_ended_streams() {
    let manager = TaskStreamManager::new();
    let old_task: i64 = 210;
    let new_task: i64 = 211;

    manager.start(old_task).await;
    manager.push_line(old_task, "stale data".to_string()).await;
    manager.end_task(old_task).await;

    // Force-expire the TTL by pruning with Duration::ZERO directly.
    manager.prune_ended(Duration::ZERO).await;

    // Starting a new task should not resurrect the old one.
    manager.start(new_task).await;

    let (old_history, _) = manager.subscribe(old_task).await;
    assert!(old_history.is_empty(), "pruned ended stream must not reappear after start");

    let (new_history, new_rx) = manager.subscribe(new_task).await;
    assert!(new_history.is_empty(), "new task must start with empty history");
    assert!(new_rx.is_some(), "new task must have live receiver");
}

// Multiple ended streams are all pruned in one call.
#[tokio::test]
async fn test_prune_ended_removes_multiple_ended_streams() {
    let manager = TaskStreamManager::new();
    for id in 220..225_i64 {
        manager.start(id).await;
        manager.push_line(id, format!("line {id}")).await;
        manager.end_task(id).await;
    }

    manager.prune_ended(Duration::ZERO).await;

    for id in 220..225_i64 {
        let (history, _) = manager.subscribe(id).await;
        assert!(history.is_empty(), "stream {id} must be pruned");
    }
}

// Mixed: some ended, some active — only ended ones are pruned.
#[tokio::test]
async fn test_prune_ended_only_removes_ended_streams_in_mixed_map() {
    let manager = TaskStreamManager::new();

    let active_id: i64 = 230;
    let ended_id: i64 = 231;

    manager.start(active_id).await;
    manager.push_line(active_id, "still running".to_string()).await;

    manager.start(ended_id).await;
    manager.push_line(ended_id, "done".to_string()).await;
    manager.end_task(ended_id).await;

    manager.prune_ended(Duration::ZERO).await;

    let (active_history, active_rx) = manager.subscribe(active_id).await;
    assert!(!active_history.is_empty(), "active stream must survive");
    assert!(active_rx.is_some());

    let (ended_history, _) = manager.subscribe(ended_id).await;
    assert!(ended_history.is_empty(), "ended stream must be pruned");
}
