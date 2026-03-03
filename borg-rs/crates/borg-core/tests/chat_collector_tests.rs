use borg_core::chat::{ChatCollector, IncomingMessage};
use std::time::Duration;

fn msg(chat_key: &str, sender: &str, text: &str) -> IncomingMessage {
    IncomingMessage {
        chat_key: chat_key.to_string(),
        sender_name: sender.to_string(),
        text: text.to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    }
}

// AC: expired Collecting window returns a MessageBatch on flush_expired
#[tokio::test]
async fn test_flush_expired_returns_batch_for_expired_window() {
    let collector = ChatCollector::new(1 /* window_ms */, 0 /* unlimited */, 0);

    // Opens a Collecting window
    let result = collector.process(msg("chat:1", "Alice", "hello")).await;
    assert!(result.is_none(), "should buffer into window, not dispatch yet");

    tokio::time::sleep(Duration::from_millis(10)).await;

    let batches = collector.flush_expired().await;
    assert_eq!(batches.len(), 1, "one expired window should be flushed");
    assert_eq!(batches[0].chat_key, "chat:1");
    assert_eq!(batches[0].sender_name, "Alice");
    assert_eq!(batches[0].messages, vec!["hello"]);
}

// AC: unexpired Collecting window is left intact by flush_expired
#[tokio::test]
async fn test_flush_expired_leaves_unexpired_window_intact() {
    let collector = ChatCollector::new(100_000, 0, 0);

    collector.process(msg("chat:2", "Bob", "hi")).await;

    let batches = collector.flush_expired().await;
    assert!(batches.is_empty(), "window not expired; must not flush");

    // Still not running — the window is still collecting
    assert_eq!(collector.active_count().await, 0);
}

// AC: Cooldown past its deadline is cleared; next process() accepts messages
#[tokio::test]
async fn test_flush_expired_clears_expired_cooldown() {
    // window_ms=0 → immediate dispatch; cooldown_ms=1
    let collector = ChatCollector::new(0, 0, 1 /* cooldown_ms */);

    // Immediate dispatch
    let first = collector.process(msg("chat:3", "Carol", "first")).await;
    assert!(first.is_some(), "window_ms=0 should dispatch immediately");

    // Agent done → enters Cooldown
    collector.mark_done("chat:3").await;

    tokio::time::sleep(Duration::from_millis(10)).await;

    // flush_expired must promote Cooldown → Idle
    let batches = collector.flush_expired().await;
    assert!(batches.is_empty(), "cooldown clears to Idle, no batch produced");

    // Now process() must accept the message (Idle → dispatch with window_ms=0)
    let second = collector.process(msg("chat:3", "Carol", "second")).await;
    assert!(second.is_some(), "after cooldown cleared, message should be dispatched");
}

// AC: concurrent-agent limit is respected during flush_expired promotion
#[tokio::test]
async fn test_flush_expired_respects_max_agents_limit() {
    let collector = ChatCollector::new(1 /* window_ms */, 1 /* max_agents */, 0);

    collector.process(msg("chat:a", "Alice", "msg-a")).await;
    collector.process(msg("chat:b", "Bob", "msg-b")).await;

    tokio::time::sleep(Duration::from_millis(10)).await;

    let batches = collector.flush_expired().await;
    assert_eq!(batches.len(), 1, "only one agent slot; only one batch promoted");
    assert_eq!(collector.active_count().await, 1);
}
