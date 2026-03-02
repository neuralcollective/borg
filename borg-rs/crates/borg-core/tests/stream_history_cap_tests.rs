use borg_core::stream::TaskStreamManager;

const MAX_HISTORY_LINES: usize = 10_000;

#[tokio::test]
async fn test_history_cap_does_not_exceed_max() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 1;
    manager.start(task_id).await;

    for i in 0..MAX_HISTORY_LINES + 500 {
        manager.push_line(task_id, format!("line {i}")).await;
    }

    let (history, _rx) = manager.subscribe(task_id).await;
    assert_eq!(
        history.len(),
        MAX_HISTORY_LINES,
        "history must be capped at MAX_HISTORY_LINES"
    );
}

#[tokio::test]
async fn test_history_cap_drops_oldest_not_newest() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 2;
    manager.start(task_id).await;

    manager.push_line(task_id, "OLDEST_LINE".to_string()).await;

    for i in 0..MAX_HISTORY_LINES {
        manager.push_line(task_id, format!("middle {i}")).await;
    }

    let last = "NEWEST_LINE".to_string();
    manager.push_line(task_id, last.clone()).await;

    let (history, _rx) = manager.subscribe(task_id).await;

    assert_eq!(history.len(), MAX_HISTORY_LINES);
    assert!(
        !history.iter().any(|l| l == "OLDEST_LINE"),
        "oldest line must have been evicted"
    );
    assert_eq!(history.last().unwrap(), &last, "newest line must be retained");
}
