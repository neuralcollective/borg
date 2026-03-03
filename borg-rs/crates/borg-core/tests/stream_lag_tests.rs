// Tests for stream_lag notification behaviour.
//
// When a broadcast channel receiver falls behind (Lagged error), the SSE
// handler should emit a {"type":"stream_lag","dropped":N} JSON line instead
// of silently skipping, so the client knows events were lost.

use borg_core::stream::TaskStreamManager;
use tokio::sync::broadcast;

// =============================================================================
// Unit: lag JSON format is correct
// =============================================================================

#[test]
fn test_stream_lag_json_format_with_count() {
    let n: u64 = 42;
    let json = format!(r#"{{"type":"stream_lag","dropped":{n}}}"#);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["type"], "stream_lag");
    assert_eq!(parsed["dropped"], 42);
}

#[test]
fn test_stream_lag_json_format_zero() {
    let n: u64 = 0;
    let json = format!(r#"{{"type":"stream_lag","dropped":{n}}}"#);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["type"], "stream_lag");
    assert_eq!(parsed["dropped"], 0);
}

// =============================================================================
// Integration: broadcast channel raises Lagged when receiver falls behind
// =============================================================================

#[tokio::test]
async fn test_broadcast_channel_raises_lagged_when_behind() {
    // Small capacity so we can overflow it cheaply.
    let (tx, mut rx) = broadcast::channel::<String>(4);

    // Fill the channel beyond capacity without any receiver consuming.
    for i in 0..8u64 {
        let _ = tx.send(format!("line-{i}"));
    }

    // The receiver should now get Lagged.
    let result = rx.recv().await;
    assert!(
        matches!(result, Err(broadcast::error::RecvError::Lagged(_))),
        "expected Lagged error, got: {result:?}"
    );
}

#[tokio::test]
async fn test_broadcast_lagged_carries_dropped_count() {
    let (tx, mut rx) = broadcast::channel::<String>(4);

    // Send 8 items; 4 fit in the buffer, 4 are dropped.
    for i in 0..8u64 {
        let _ = tx.send(format!("msg-{i}"));
    }

    match rx.recv().await {
        Err(broadcast::error::RecvError::Lagged(n)) => {
            // At least some messages were reported dropped.
            assert!(n > 0, "dropped count must be positive, got {n}");
        },
        other => panic!("expected Lagged, got: {other:?}"),
    }
}

// =============================================================================
// Integration: lag event can be forwarded through an unbounded channel
// (mirrors the pattern in routes.rs)
// =============================================================================

#[tokio::test]
async fn test_lag_event_forwarded_through_mpsc() {
    let (tx, mut rx) = broadcast::channel::<String>(4);
    let (fwd_tx, mut fwd_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    // Overflow the broadcast channel.
    for i in 0..8u64 {
        let _ = tx.send(format!("event-{i}"));
    }

    // Simulate the SSE handler loop for one iteration.
    match rx.recv().await {
        Ok(line) => {
            fwd_tx.send(line).unwrap();
        },
        Err(broadcast::error::RecvError::Lagged(n)) => {
            let lag = format!(r#"{{"type":"stream_lag","dropped":{n}}}"#);
            fwd_tx.send(lag).unwrap();
        },
        Err(_) => {},
    }

    let forwarded = fwd_rx.recv().await.expect("should have received a message");
    let parsed: serde_json::Value = serde_json::from_str(&forwarded).unwrap();
    assert_eq!(parsed["type"], "stream_lag", "forwarded event must be stream_lag");
    assert!(
        parsed["dropped"].as_u64().unwrap_or(0) > 0,
        "dropped count must be positive"
    );
}

// =============================================================================
// Integration: after a lag the receiver can still receive subsequent messages
// =============================================================================

#[tokio::test]
async fn test_receiver_continues_after_lag() {
    // Capacity 4, send 5 to cause exactly 1 dropped message.
    let (tx, mut rx) = broadcast::channel::<String>(4);

    for i in 0..5u64 {
        let _ = tx.send(format!("old-{i}"));
    }

    // Consume the Lagged error (1 message dropped: old-0).
    assert!(matches!(rx.recv().await, Err(broadcast::error::RecvError::Lagged(1))));

    // Drain the 4 buffered messages (old-1 through old-4).
    for _ in 0..4 {
        assert!(matches!(rx.recv().await, Ok(_)));
    }

    // Now the channel is empty — a fresh message should be receivable.
    tx.send("fresh".to_string()).unwrap();
    let msg = rx.recv().await.expect("should receive fresh message");
    assert_eq!(msg, "fresh");
}

// =============================================================================
// Integration: TaskStreamManager — subscriber that lags gets Lagged error
// =============================================================================

#[tokio::test]
async fn test_task_stream_manager_subscriber_gets_lagged() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 1001;
    manager.start(task_id).await;

    let (_history, Some(mut live_rx)) = manager.subscribe(task_id).await else {
        panic!("expected a live receiver");
    };

    // Push enough lines to overflow the 512-capacity channel.
    // We push 513 lines without consuming from live_rx.
    for i in 0..513u64 {
        manager
            .push_line(task_id, format!(r#"{{"type":"text","i":{i}}}"#))
            .await;
    }

    // The receiver should now be lagged.
    let result = live_rx.recv().await;
    assert!(
        matches!(result, Err(broadcast::error::RecvError::Lagged(_))),
        "expected Lagged after overflowing 512-capacity channel, got: {result:?}"
    );
}
