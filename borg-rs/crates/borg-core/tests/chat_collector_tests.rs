#![allow(clippy::unwrap_used, clippy::expect_used)]

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

#[tokio::test]
async fn immediate_dispatch_when_window_ms_zero() {
    let collector = ChatCollector::new(0, 0, 0);
    let batch = collector.process(msg("chat:1", "hello")).await.unwrap();
    assert_eq!(batch.chat_key, "chat:1");
    assert_eq!(batch.messages, vec!["hello"]);
    assert_eq!(batch.sender_name, "Alice");
}

#[tokio::test]
async fn collecting_accumulates_messages_within_window() {
    let collector = ChatCollector::new(60_000, 0, 0);

    let r1 = collector.process(msg("chat:1", "first")).await;
    assert!(r1.is_none(), "first message should start Collecting, not dispatch");

    let r2 = collector.process(msg("chat:1", "second")).await;
    assert!(r2.is_none(), "second message should accumulate, not dispatch");
}

#[tokio::test]
async fn collecting_dispatches_all_messages_when_window_expires() {
    let collector = ChatCollector::new(1, 0, 0); // 1ms window

    let r1 = collector.process(msg("chat:1", "first")).await;
    assert!(r1.is_none(), "first message opens Collecting window");

    tokio::time::sleep(Duration::from_millis(5)).await;

    // Second message arrives after deadline — triggers dispatch with both messages
    let batch = collector.process(msg("chat:1", "second")).await.unwrap();
    assert_eq!(batch.chat_key, "chat:1");
    assert_eq!(batch.messages, vec!["first", "second"]);
}

#[tokio::test]
async fn message_dropped_when_running() {
    let collector = ChatCollector::new(0, 0, 0);

    // First message dispatches and puts chat into Running
    let first = collector.process(msg("chat:1", "first")).await;
    assert!(first.is_some(), "first message should dispatch");

    // Subsequent messages on same chat are dropped
    let dropped = collector.process(msg("chat:1", "second")).await;
    assert!(dropped.is_none(), "message dropped when agent is Running");
}

#[tokio::test]
async fn message_dropped_when_cooldown() {
    let collector = ChatCollector::new(0, 0, 60_000); // 60s cooldown

    let first = collector.process(msg("chat:1", "first")).await;
    assert!(first.is_some(), "first message should dispatch");

    collector.mark_done("chat:1").await;

    let dropped = collector.process(msg("chat:1", "during_cooldown")).await;
    assert!(dropped.is_none(), "message dropped during Cooldown");
}

#[tokio::test]
async fn max_agents_blocks_dispatch_from_idle() {
    let collector = ChatCollector::new(0, 1, 0); // max 1 concurrent agent

    // chat:1 dispatches successfully
    let first = collector.process(msg("chat:1", "hello")).await;
    assert!(first.is_some(), "first dispatch succeeds");

    // chat:2 is at Idle but blocked by max_agents limit
    let blocked = collector.process(msg("chat:2", "hello")).await;
    assert!(blocked.is_none(), "dispatch from Idle blocked when at max_agents");
}

#[tokio::test]
async fn max_agents_unblocks_after_done() {
    let collector = ChatCollector::new(0, 1, 0);

    let first = collector.process(msg("chat:1", "run")).await;
    assert!(first.is_some());

    // At limit — chat:2 is blocked
    assert!(collector.process(msg("chat:2", "wait")).await.is_none());

    // chat:1 finishes, slot freed
    collector.mark_done("chat:1").await;

    let second = collector.process(msg("chat:2", "now")).await;
    assert!(second.is_some(), "dispatch allowed once a slot is freed");
}

#[tokio::test]
async fn independent_chats_dispatch_independently() {
    let collector = ChatCollector::new(0, 0, 0);

    let a = collector.process(msg("chat:a", "hi")).await;
    let b = collector.process(msg("chat:b", "hi")).await;

    assert!(a.is_some(), "chat:a dispatches");
    assert!(b.is_some(), "chat:b dispatches independently");
}
