use std::time::Duration;

use borg_core::chat::{ChatCollector, IncomingMessage};

fn msg(chat_key: &str, text: &str) -> IncomingMessage {
    IncomingMessage {
        chat_key: chat_key.to_string(),
        sender_name: "Alice".to_string(),
        text: text.to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    }
}

// AC1: Idle → Collecting when window_ms > 0
#[tokio::test]
async fn idle_to_collecting_with_window() {
    let collector = ChatCollector::new(10_000, 4, 0);
    let result = collector.process(msg("chat1", "hello")).await;
    assert!(result.is_none(), "should not dispatch immediately with window > 0");
    assert_eq!(collector.active_count().await, 0);
}

// AC1 extended: multiple messages accumulate in Collecting before window expires
#[tokio::test]
async fn collecting_accumulates_messages() {
    let collector = ChatCollector::new(10_000, 4, 0);
    assert!(collector.process(msg("chat1", "first")).await.is_none());
    assert!(collector.process(msg("chat1", "second")).await.is_none());
    assert_eq!(collector.active_count().await, 0);
}

// AC2: Idle → Running dispatch when window_ms = 0
#[tokio::test]
async fn idle_to_running_immediate_dispatch() {
    let collector = ChatCollector::new(0, 4, 0);
    let result = collector.process(msg("chat1", "hello")).await;
    let batch = result.expect("should dispatch immediately when window_ms = 0");
    assert_eq!(batch.chat_key, "chat1");
    assert_eq!(batch.messages, vec!["hello"]);
    assert_eq!(collector.active_count().await, 1);
}

// AC3: message dropped while Running
#[tokio::test]
async fn message_dropped_while_running() {
    let collector = ChatCollector::new(0, 4, 0);
    let _ = collector.process(msg("chat1", "first")).await;
    let result = collector.process(msg("chat1", "second")).await;
    assert!(result.is_none(), "message while Running should be dropped");
    assert_eq!(collector.active_count().await, 1);
}

// AC4: message dropped while Cooldown
#[tokio::test]
async fn message_dropped_while_cooldown() {
    let collector = ChatCollector::new(0, 4, 10_000);
    let _ = collector.process(msg("chat1", "first")).await;
    collector.mark_done("chat1").await;
    let result = collector.process(msg("chat1", "second")).await;
    assert!(result.is_none(), "message while Cooldown should be dropped");
}

// AC5: window expiry triggers dispatch in flush_expired
#[tokio::test]
async fn window_expiry_triggers_dispatch_in_flush_expired() {
    let collector = ChatCollector::new(1, 4, 0);
    assert!(collector.process(msg("chat1", "hello")).await.is_none());
    tokio::time::sleep(Duration::from_millis(5)).await;
    let batches = collector.flush_expired().await;
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].chat_key, "chat1");
    assert_eq!(batches[0].messages, vec!["hello"]);
    assert_eq!(collector.active_count().await, 1);
}

// AC5 extended: flush before window expires returns nothing
#[tokio::test]
async fn flush_before_window_expires_returns_nothing() {
    let collector = ChatCollector::new(10_000, 4, 0);
    assert!(collector.process(msg("chat1", "hello")).await.is_none());
    let batches = collector.flush_expired().await;
    assert!(batches.is_empty(), "flush before window expires must return empty");
}

// AC6: cooldown expiry returns to Idle in flush_expired
#[tokio::test]
async fn cooldown_expiry_returns_to_idle_in_flush_expired() {
    let collector = ChatCollector::new(0, 4, 1);
    let _ = collector.process(msg("chat1", "first")).await;
    collector.mark_done("chat1").await;

    // Still in cooldown — message dropped
    assert!(collector.process(msg("chat1", "second")).await.is_none());

    tokio::time::sleep(Duration::from_millis(5)).await;
    let batches = collector.flush_expired().await;
    assert!(batches.is_empty(), "cooldown flush produces no dispatch batches");

    // Now Idle — next message dispatches immediately
    let result = collector.process(msg("chat1", "third")).await;
    assert!(result.is_some(), "after cooldown expiry should dispatch immediately");
    assert_eq!(result.unwrap().messages, vec!["third"]);
}

// AC7: mark_done enters Cooldown when cooldown_ms > 0
#[tokio::test]
async fn mark_done_enters_cooldown() {
    let collector = ChatCollector::new(0, 4, 10_000);
    let _ = collector.process(msg("chat1", "first")).await;
    assert_eq!(collector.active_count().await, 1);
    collector.mark_done("chat1").await;
    assert_eq!(collector.active_count().await, 0);
    // In cooldown — next message dropped
    assert!(collector.process(msg("chat1", "second")).await.is_none());
}

// AC8: mark_done returns to Idle when cooldown_ms = 0
#[tokio::test]
async fn mark_done_returns_to_idle_no_cooldown() {
    let collector = ChatCollector::new(0, 4, 0);
    let _ = collector.process(msg("chat1", "first")).await;
    assert_eq!(collector.active_count().await, 1);
    collector.mark_done("chat1").await;
    assert_eq!(collector.active_count().await, 0);
    // Back to Idle — next message dispatches
    let result = collector.process(msg("chat1", "second")).await;
    let batch = result.expect("after mark_done with no cooldown, should dispatch");
    assert_eq!(batch.messages, vec!["second"]);
}

// AC9: concurrency cap prevents dispatch when running >= max_agents (process path)
#[tokio::test]
async fn concurrency_cap_prevents_dispatch_in_process() {
    let collector = ChatCollector::new(0, 1, 0);
    let result1 = collector.process(msg("chat1", "hello")).await;
    assert!(result1.is_some(), "first dispatch should succeed");
    assert_eq!(collector.active_count().await, 1);

    // Different chat, at limit — must not dispatch
    let result2 = collector.process(msg("chat2", "world")).await;
    assert!(result2.is_none(), "second chat blocked by concurrency cap");
    assert_eq!(collector.active_count().await, 1);
}

// AC9 extended: flush_expired also respects concurrency cap
#[tokio::test]
async fn concurrency_cap_prevents_dispatch_in_flush_expired() {
    let collector = ChatCollector::new(1, 1, 0);
    // Both chats start collecting
    assert!(collector.process(msg("chat1", "first")).await.is_none());
    assert!(collector.process(msg("chat2", "second")).await.is_none());

    tokio::time::sleep(Duration::from_millis(5)).await;
    let batches = collector.flush_expired().await;
    // Only one should dispatch — cap is 1
    assert_eq!(batches.len(), 1, "flush should only dispatch one batch when capped at 1");
    assert_eq!(collector.active_count().await, 1);
}
