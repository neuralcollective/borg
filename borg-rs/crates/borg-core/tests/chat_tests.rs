use std::time::Duration;

use borg_core::chat::{ChatCollector, IncomingMessage};

fn make_msg(chat_key: &str, sender_name: &str, text: &str) -> IncomingMessage {
    IncomingMessage {
        chat_key: chat_key.to_string(),
        sender_name: sender_name.to_string(),
        text: text.to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    }
}

// Idle + window_ms=0 → immediate dispatch
#[tokio::test]
async fn idle_window_zero_dispatches_immediately() {
    let collector = ChatCollector::new(0, 10, 0);
    let batch = collector.process(make_msg("chat1", "alice", "hello")).await;
    let batch = batch.expect("should dispatch immediately");
    assert_eq!(batch.chat_key, "chat1");
    assert_eq!(batch.sender_name, "alice");
    assert_eq!(batch.messages, vec!["hello"]);
}

// Idle + window_ms > 0 → starts Collecting, returns None
#[tokio::test]
async fn idle_with_window_starts_collecting() {
    let collector = ChatCollector::new(5_000, 10, 0);
    let result = collector.process(make_msg("chat1", "alice", "hello")).await;
    assert!(result.is_none(), "should not dispatch while window is open");
}

// Second message within the window accumulates, does not dispatch
#[tokio::test]
async fn second_message_within_window_accumulates() {
    let collector = ChatCollector::new(5_000, 10, 0);
    let r1 = collector.process(make_msg("chat1", "alice", "msg1")).await;
    let r2 = collector.process(make_msg("chat1", "alice", "msg2")).await;
    assert!(r1.is_none());
    assert!(r2.is_none(), "second message within window should not dispatch");
}

// flush_expired includes both accumulated messages once the window expires
#[tokio::test]
async fn accumulated_messages_flushed_together() {
    let collector = ChatCollector::new(5, 10, 0); // 5ms window
    collector.process(make_msg("chat1", "alice", "msg1")).await;
    collector.process(make_msg("chat1", "alice", "msg2")).await;

    tokio::time::sleep(Duration::from_millis(20)).await;

    let mut batches = collector.flush_expired().await;
    assert_eq!(batches.len(), 1);
    let batch = batches.remove(0);
    assert_eq!(batch.messages, vec!["msg1", "msg2"]);
}

// A message arriving after the window deadline triggers dispatch
#[tokio::test]
async fn message_after_deadline_triggers_dispatch() {
    let collector = ChatCollector::new(1, 10, 0); // 1ms window
    collector.process(make_msg("chat1", "alice", "msg1")).await;

    tokio::time::sleep(Duration::from_millis(10)).await;

    let result = collector.process(make_msg("chat1", "alice", "msg2")).await;
    let batch = result.expect("should dispatch after window deadline");
    assert_eq!(batch.messages, vec!["msg1", "msg2"]);
}

// Running state silently drops messages
#[tokio::test]
async fn running_state_drops_messages() {
    let collector = ChatCollector::new(0, 10, 0);
    let first = collector.process(make_msg("chat1", "alice", "first")).await;
    assert!(first.is_some(), "first message should dispatch");

    let dropped = collector.process(make_msg("chat1", "alice", "dropped")).await;
    assert!(dropped.is_none(), "messages during Running should be dropped");
}

// Cooldown state silently drops messages
#[tokio::test]
async fn cooldown_state_drops_messages() {
    let collector = ChatCollector::new(0, 10, 5_000); // 5s cooldown
    collector.process(make_msg("chat1", "alice", "first")).await;
    collector.mark_done("chat1").await;

    let dropped = collector.process(make_msg("chat1", "alice", "during_cooldown")).await;
    assert!(dropped.is_none(), "messages during Cooldown should be dropped");
}

// active_count tracks Running agents
#[tokio::test]
async fn active_count_increments_on_dispatch_and_decrements_on_done() {
    let collector = ChatCollector::new(0, 10, 0);
    assert_eq!(collector.active_count().await, 0);

    collector.process(make_msg("chat1", "alice", "msg")).await;
    assert_eq!(collector.active_count().await, 1);

    collector.mark_done("chat1").await;
    assert_eq!(collector.active_count().await, 0);
}

// max_agents limit prevents dispatch when at capacity
#[tokio::test]
async fn max_agents_blocks_dispatch_at_limit() {
    let collector = ChatCollector::new(0, 1, 0); // max 1 agent
    let first = collector.process(make_msg("chat1", "alice", "msg")).await;
    assert!(first.is_some());

    // Second chat at the concurrency limit — should not dispatch
    let second = collector.process(make_msg("chat2", "bob", "msg")).await;
    assert!(second.is_none(), "should not dispatch when at max_agents limit");
}

// Cooldown expires and chat returns to Idle via flush_expired
#[tokio::test]
async fn cooldown_expires_returns_to_idle() {
    let collector = ChatCollector::new(0, 10, 5); // 5ms cooldown
    collector.process(make_msg("chat1", "alice", "first")).await;
    collector.mark_done("chat1").await;

    // Still in cooldown — message dropped
    let during = collector.process(make_msg("chat1", "alice", "during")).await;
    assert!(during.is_none());

    tokio::time::sleep(Duration::from_millis(20)).await;
    collector.flush_expired().await; // transitions Cooldown → Idle

    // Now Idle with window=0 — should dispatch immediately
    let after = collector.process(make_msg("chat1", "alice", "after")).await;
    assert!(after.is_some(), "should dispatch after cooldown expires");
}
