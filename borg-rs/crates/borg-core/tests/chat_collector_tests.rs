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

// Idle → Running immediately when window_ms = 0.
#[tokio::test]
async fn test_window_zero_dispatches_immediately() {
    let collector = ChatCollector::new(0, 10, 0);
    let batch = collector.process(make_msg("chat:1", "Alice", "hello")).await;
    let batch = batch.expect("should dispatch immediately");
    assert_eq!(batch.chat_key, "chat:1");
    assert_eq!(batch.sender_name, "Alice");
    assert_eq!(batch.messages, vec!["hello"]);
    assert_eq!(collector.active_count().await, 1);
}

// Idle → Collecting; messages accumulate while window is open.
#[tokio::test]
async fn test_collecting_accumulates_messages() {
    let collector = ChatCollector::new(10_000, 10, 0); // 10-second window
    assert!(collector.process(make_msg("chat:1", "Alice", "a")).await.is_none());
    assert!(collector.process(make_msg("chat:1", "Alice", "b")).await.is_none());
    assert!(collector.process(make_msg("chat:1", "Alice", "c")).await.is_none());
    assert_eq!(collector.active_count().await, 0);
}

// Collecting → Running when window expires (detected in process()).
#[tokio::test]
async fn test_collecting_dispatches_after_window_expires() {
    let collector = ChatCollector::new(1, 10, 0); // 1 ms window
    assert!(collector.process(make_msg("chat:1", "Alice", "first")).await.is_none());
    tokio::time::sleep(Duration::from_millis(5)).await;
    // second message arrives after deadline → dispatch with both messages
    let batch = collector
        .process(make_msg("chat:1", "Alice", "second"))
        .await
        .expect("should dispatch after window expires");
    assert_eq!(batch.messages, vec!["first", "second"]);
    assert_eq!(collector.active_count().await, 1);
}

// Running → drops new messages for the same chat.
#[tokio::test]
async fn test_running_state_drops_messages() {
    let collector = ChatCollector::new(0, 10, 0);
    assert!(collector.process(make_msg("chat:1", "Alice", "first")).await.is_some());
    assert!(collector.process(make_msg("chat:1", "Alice", "second")).await.is_none());
    assert_eq!(collector.active_count().await, 1);
}

// Cooldown → drops messages until mark_done is not called (cooldown still active).
#[tokio::test]
async fn test_cooldown_state_drops_messages() {
    let collector = ChatCollector::new(0, 10, 60_000); // 60-second cooldown
    assert!(collector.process(make_msg("chat:1", "Alice", "trigger")).await.is_some());
    collector.mark_done("chat:1").await;
    assert_eq!(collector.active_count().await, 0);
    // still in cooldown → message dropped
    assert!(collector.process(make_msg("chat:1", "Alice", "during cooldown")).await.is_none());
}

// max_agents = 1 prevents a second Idle chat from being dispatched.
#[tokio::test]
async fn test_max_agents_cap_prevents_idle_dispatch() {
    let collector = ChatCollector::new(0, 1, 0);
    assert!(collector.process(make_msg("chat:a", "Alice", "x")).await.is_some());
    assert!(!collector.can_dispatch().await);
    assert!(collector.process(make_msg("chat:b", "Bob", "y")).await.is_none());
}

// max_agents = 1 prevents Collecting → Running when a slot is already occupied.
#[tokio::test]
async fn test_max_agents_cap_prevents_collecting_to_running() {
    let collector = ChatCollector::new(1, 1, 0); // 1 ms window, max 1
    // chat:a starts collecting
    assert!(collector.process(make_msg("chat:a", "Alice", "msg")).await.is_none());
    tokio::time::sleep(Duration::from_millis(5)).await;
    // chat:a window expired → dispatch, running = 1
    assert!(collector
        .process(make_msg("chat:a", "Alice", "dispatch"))
        .await
        .is_some());
    assert_eq!(collector.active_count().await, 1);

    // chat:b starts collecting
    assert!(collector.process(make_msg("chat:b", "Bob", "msg")).await.is_none());
    tokio::time::sleep(Duration::from_millis(5)).await;
    // chat:b window expired but at max_agents → blocked
    assert!(collector
        .process(make_msg("chat:b", "Bob", "dispatch"))
        .await
        .is_none());
    assert_eq!(collector.active_count().await, 1); // still only 1 running
}

// mark_done with cooldown=0 returns chat to Idle (next message dispatches).
#[tokio::test]
async fn test_mark_done_no_cooldown_returns_to_idle() {
    let collector = ChatCollector::new(0, 10, 0);
    assert!(collector.process(make_msg("chat:1", "Alice", "first")).await.is_some());
    collector.mark_done("chat:1").await;
    assert_eq!(collector.active_count().await, 0);
    assert!(collector.process(make_msg("chat:1", "Alice", "second")).await.is_some());
}

// Two independent chats can run concurrently when max_agents allows it.
#[tokio::test]
async fn test_two_chats_run_concurrently_within_limit() {
    let collector = ChatCollector::new(0, 2, 0);
    assert!(collector.process(make_msg("chat:a", "Alice", "x")).await.is_some());
    assert!(collector.process(make_msg("chat:b", "Bob", "y")).await.is_some());
    assert_eq!(collector.active_count().await, 2);
    assert!(!collector.can_dispatch().await);
}
