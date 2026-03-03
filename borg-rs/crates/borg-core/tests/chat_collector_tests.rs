use std::time::Duration;

use borg_core::chat::{ChatCollector, IncomingMessage};

fn msg(chat_key: &str, sender_name: &str, text: &str) -> IncomingMessage {
    IncomingMessage {
        chat_key: chat_key.to_string(),
        sender_name: sender_name.to_string(),
        text: text.to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    }
}

// ── flush_expired ─────────────────────────────────────────────────────────────

/// flush_expired dispatches an expired collection window.
#[tokio::test]
async fn flush_expired_dispatches_expired_window() {
    let collector = ChatCollector::new(5, 0, 0); // 5ms window, unlimited, no cooldown

    let result = collector.process(msg("chat:1", "Alice", "hello")).await;
    assert!(result.is_none(), "window should still be open");

    tokio::time::sleep(Duration::from_millis(20)).await;

    let batches = collector.flush_expired().await;
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].chat_key, "chat:1");
    assert_eq!(batches[0].sender_name, "Alice");
    assert_eq!(batches[0].messages, vec!["hello"]);
    assert_eq!(collector.active_count().await, 1);
}

/// flush_expired leaves a non-expired window untouched.
#[tokio::test]
async fn flush_expired_leaves_non_expired_window() {
    let collector = ChatCollector::new(60_000, 0, 0); // 60s window — won't expire

    let result = collector.process(msg("chat:2", "Bob", "hey")).await;
    assert!(result.is_none());

    let batches = collector.flush_expired().await;
    assert!(batches.is_empty(), "non-expired window must not be dispatched");
    assert_eq!(collector.active_count().await, 0);
}

/// flush_expired transitions an expired cooldown back to Idle so the next
/// process() call can dispatch.
#[tokio::test]
async fn flush_expired_expires_cooldown_to_idle() {
    // window_ms=0 → immediate dispatch; cooldown_ms=5 → short cooldown
    let collector = ChatCollector::new(0, 0, 5);

    let batch = collector.process(msg("chat:3", "Carol", "hi")).await;
    assert!(batch.is_some(), "should dispatch immediately");

    collector.mark_done("chat:3").await;

    // Messages during cooldown are dropped
    let dropped = collector.process(msg("chat:3", "Carol", "during cooldown")).await;
    assert!(dropped.is_none(), "message during cooldown must be dropped");

    tokio::time::sleep(Duration::from_millis(20)).await;

    // flush_expired expires the cooldown; no batches produced
    let batches = collector.flush_expired().await;
    assert!(batches.is_empty(), "cooldown expiry produces no dispatch");

    // Now Idle: next message dispatches immediately
    let new_batch = collector.process(msg("chat:3", "Carol", "after idle")).await;
    assert!(new_batch.is_some(), "chat should be Idle after cooldown flush");
}

/// flush_expired stops dispatching once the running count hits max_agents.
#[tokio::test]
async fn flush_expired_honours_max_agents_cap() {
    // cap=2, 5ms window
    let collector = ChatCollector::new(5, 2, 0);

    // Open collection windows for 3 chats
    collector.process(msg("chat:A", "u", "m")).await;
    collector.process(msg("chat:B", "u", "m")).await;
    collector.process(msg("chat:C", "u", "m")).await;

    tokio::time::sleep(Duration::from_millis(20)).await;

    let batches = collector.flush_expired().await;
    assert_eq!(batches.len(), 2, "must not dispatch more than max_agents=2 batches");
    assert_eq!(collector.active_count().await, 2);
}

// ── mark_done ─────────────────────────────────────────────────────────────────

/// mark_done decrements the running counter.
#[tokio::test]
async fn mark_done_decrements_running() {
    let collector = ChatCollector::new(0, 0, 0);

    let batch = collector.process(msg("chat:d1", "Dave", "go")).await;
    assert!(batch.is_some());
    assert_eq!(collector.active_count().await, 1);

    collector.mark_done("chat:d1").await;
    assert_eq!(collector.active_count().await, 0);
}

/// mark_done transitions to Cooldown when cooldown_ms > 0.
#[tokio::test]
async fn mark_done_enters_cooldown_when_cooldown_nonzero() {
    let collector = ChatCollector::new(0, 0, 10_000); // 10s cooldown

    let batch = collector.process(msg("chat:e1", "Eve", "msg")).await;
    assert!(batch.is_some());

    collector.mark_done("chat:e1").await;

    // In cooldown: new message must be dropped
    let dropped = collector.process(msg("chat:e1", "Eve", "new")).await;
    assert!(dropped.is_none(), "message must be dropped while in Cooldown");
}

/// mark_done transitions to Idle when cooldown_ms = 0.
#[tokio::test]
async fn mark_done_enters_idle_when_no_cooldown() {
    let collector = ChatCollector::new(0, 0, 0);

    let batch = collector.process(msg("chat:f1", "Frank", "first")).await;
    assert!(batch.is_some());

    collector.mark_done("chat:f1").await;

    // Back to Idle: next message dispatches immediately
    let next = collector.process(msg("chat:f1", "Frank", "second")).await;
    assert!(next.is_some(), "should dispatch immediately when back to Idle");
}

/// mark_done does not underflow when running is already 0.
#[tokio::test]
async fn mark_done_saturates_at_zero() {
    let collector = ChatCollector::new(0, 0, 0);

    assert_eq!(collector.active_count().await, 0);

    // Call mark_done without any running agent
    collector.mark_done("chat:ghost").await;
    assert_eq!(collector.active_count().await, 0, "running must not underflow");
}
