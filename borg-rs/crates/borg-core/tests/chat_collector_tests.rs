use std::time::Duration;

use borg_core::chat::ChatCollector;

// Helper: build a collector with an already-expired Collecting window.
// Sets window_ms=1 and sleeps long enough for the window to expire.
async fn collector_with_expired_window(
    chat_key: &str,
    max_agents: u32,
    cooldown_ms: u64,
) -> ChatCollector {
    let collector = ChatCollector::new(1, max_agents, cooldown_ms);
    let msg = borg_core::chat::IncomingMessage {
        chat_key: chat_key.to_string(),
        sender_name: "alice".to_string(),
        text: "hello".to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    };
    collector.process(msg).await;
    // Wait for the 1ms window to expire.
    tokio::time::sleep(Duration::from_millis(5)).await;
    collector
}

// ── flush_expired ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn flush_expired_converts_expired_collecting_to_running_and_increments_running() {
    let collector = collector_with_expired_window("chat:1", 0, 0).await;

    let batches = collector.flush_expired().await;

    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].chat_key, "chat:1");
    assert_eq!(batches[0].sender_name, "alice");
    assert_eq!(batches[0].messages, vec!["hello"]);
    assert_eq!(collector.active_count().await, 1);
}

#[tokio::test]
async fn flush_expired_does_not_flush_unexpired_collecting_window() {
    // Use a long window (60 s) so it definitely has not expired.
    let collector = ChatCollector::new(60_000, 0, 0);
    let msg = borg_core::chat::IncomingMessage {
        chat_key: "chat:2".to_string(),
        sender_name: "bob".to_string(),
        text: "hi".to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    };
    collector.process(msg).await;

    let batches = collector.flush_expired().await;

    assert!(batches.is_empty());
    assert_eq!(collector.active_count().await, 0);
}

#[tokio::test]
async fn flush_expired_transitions_expired_cooldown_to_idle() {
    // Put the chat directly into a Cooldown that expires immediately (1ms).
    let collector = ChatCollector::new(0, 0, 1);

    // Dispatch immediately (window_ms=0) so it enters Running.
    let msg = borg_core::chat::IncomingMessage {
        chat_key: "chat:3".to_string(),
        sender_name: "alice".to_string(),
        text: "go".to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    };
    let batch = collector.process(msg).await;
    assert!(batch.is_some(), "should dispatch immediately");

    // Mark done → enters Cooldown(1ms).
    collector.mark_done("chat:3").await;
    assert_eq!(collector.active_count().await, 0);

    // Wait for cooldown to expire.
    tokio::time::sleep(Duration::from_millis(5)).await;

    // flush_expired should transition Cooldown → Idle.
    let batches = collector.flush_expired().await;
    assert!(batches.is_empty(), "cooldown → idle produces no batch");

    // After cooldown, a new message must be accepted.
    let msg2 = borg_core::chat::IncomingMessage {
        chat_key: "chat:3".to_string(),
        sender_name: "alice".to_string(),
        text: "again".to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    };
    let batch2 = collector.process(msg2).await;
    assert!(batch2.is_some(), "new message after cooldown should dispatch");
}

#[tokio::test]
async fn flush_expired_respects_max_agents_limit() {
    // Use window_ms=1, max_agents=1.
    // Both chats enter Collecting; first flush occupies the only slot,
    // second flush must be blocked.
    let c = ChatCollector::new(1, 1, 0);

    let msg_a = borg_core::chat::IncomingMessage {
        chat_key: "chat:A".to_string(),
        sender_name: "alice".to_string(),
        text: "first".to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    };
    let msg_b = borg_core::chat::IncomingMessage {
        chat_key: "chat:B".to_string(),
        sender_name: "bob".to_string(),
        text: "second".to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    };

    // Both go into Collecting windows.
    let r1 = c.process(msg_a).await;
    let r2 = c.process(msg_b).await;
    assert!(r1.is_none(), "window_ms=1, goes into Collecting");
    assert!(r2.is_none(), "window_ms=1, goes into Collecting");

    // Wait for both windows to expire.
    tokio::time::sleep(Duration::from_millis(5)).await;

    // First flush_expired call: exactly one batch dispatched, running → 1.
    let batches1 = c.flush_expired().await;
    assert_eq!(batches1.len(), 1, "first flush dispatches one chat");
    assert_eq!(c.active_count().await, 1);

    // Second flush_expired call: running==max_agents, nothing dispatched.
    let batches2 = c.flush_expired().await;
    assert!(batches2.is_empty(), "at max_agents, second chat must not be flushed");
    assert_eq!(c.active_count().await, 1, "running count stays at 1");
}

// ── mark_done ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn mark_done_with_cooldown_enters_cooldown_state() {
    let collector = ChatCollector::new(0, 0, 5_000);

    let msg = borg_core::chat::IncomingMessage {
        chat_key: "chat:D".to_string(),
        sender_name: "alice".to_string(),
        text: "work".to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    };
    collector.process(msg).await;
    assert_eq!(collector.active_count().await, 1);

    collector.mark_done("chat:D").await;

    // Running counter must have decremented.
    assert_eq!(collector.active_count().await, 0);

    // A new message during cooldown must be dropped (no batch returned).
    let msg2 = borg_core::chat::IncomingMessage {
        chat_key: "chat:D".to_string(),
        sender_name: "alice".to_string(),
        text: "too soon".to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    };
    let batch = collector.process(msg2).await;
    assert!(batch.is_none(), "messages during cooldown must be dropped");
}

#[tokio::test]
async fn mark_done_without_cooldown_enters_idle_state() {
    let collector = ChatCollector::new(0, 0, 0);

    let msg = borg_core::chat::IncomingMessage {
        chat_key: "chat:E".to_string(),
        sender_name: "alice".to_string(),
        text: "work".to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    };
    collector.process(msg).await;
    assert_eq!(collector.active_count().await, 1);

    collector.mark_done("chat:E").await;

    assert_eq!(collector.active_count().await, 0);

    // A new message after Idle should dispatch immediately (window_ms=0).
    let msg2 = borg_core::chat::IncomingMessage {
        chat_key: "chat:E".to_string(),
        sender_name: "alice".to_string(),
        text: "next".to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    };
    let batch = collector.process(msg2).await;
    assert!(batch.is_some(), "message after idle mark_done should dispatch");
}

#[tokio::test]
async fn mark_done_running_counter_never_underflows_below_zero() {
    let collector = ChatCollector::new(0, 0, 0);

    // Call mark_done with no running agents — counter must stay at 0.
    collector.mark_done("chat:ghost").await;
    assert_eq!(collector.active_count().await, 0);

    // Call again multiple times.
    collector.mark_done("chat:ghost").await;
    collector.mark_done("chat:ghost").await;
    assert_eq!(collector.active_count().await, 0);
}

#[tokio::test]
async fn mark_done_decrements_running_counter() {
    let collector = ChatCollector::new(0, 0, 0);

    // Dispatch two chats.
    for key in &["chat:F1", "chat:F2"] {
        let msg = borg_core::chat::IncomingMessage {
            chat_key: key.to_string(),
            sender_name: "alice".to_string(),
            text: "work".to_string(),
            timestamp: 0,
            reply_to_message_id: None,
        };
        collector.process(msg).await;
    }
    assert_eq!(collector.active_count().await, 2);

    collector.mark_done("chat:F1").await;
    assert_eq!(collector.active_count().await, 1);

    collector.mark_done("chat:F2").await;
    assert_eq!(collector.active_count().await, 0);
}
