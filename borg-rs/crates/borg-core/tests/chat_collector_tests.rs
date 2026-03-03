use std::time::Duration;

use borg_core::chat::{ChatCollector, IncomingMessage};

fn make_msg(chat_key: &str, text: &str) -> IncomingMessage {
    IncomingMessage {
        chat_key: chat_key.to_string(),
        sender_name: "Alice".to_string(),
        text: text.to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    }
}

// window_ms=0 dispatches immediately from Idle
#[tokio::test]
async fn window_zero_dispatches_immediately_from_idle() {
    let c = ChatCollector::new(0, 10, 0);
    let batch = c.process(make_msg("chat:1", "hello")).await;
    assert!(batch.is_some());
    let b = batch.unwrap();
    assert_eq!(b.chat_key, "chat:1");
    assert_eq!(b.messages, vec!["hello"]);
    assert_eq!(b.sender_name, "Alice");
}

// Non-expired Collecting window accumulates without dispatching
#[tokio::test]
async fn collecting_window_accumulates_without_dispatching() {
    let c = ChatCollector::new(60_000, 10, 0);
    let r1 = c.process(make_msg("chat:1", "first")).await;
    assert!(r1.is_none(), "first message should start collection, not dispatch");
    let r2 = c.process(make_msg("chat:1", "second")).await;
    assert!(r2.is_none(), "second message within live window should accumulate");
}

// Expired Collecting window dispatches via process()
#[tokio::test]
async fn expired_collecting_window_dispatches_via_process() {
    let c = ChatCollector::new(1, 10, 0);
    let r1 = c.process(make_msg("chat:1", "first")).await;
    assert!(r1.is_none());

    tokio::time::sleep(Duration::from_millis(10)).await;

    let r2 = c.process(make_msg("chat:1", "second")).await;
    assert!(r2.is_some(), "process() after window expiry should dispatch");
    let b = r2.unwrap();
    assert_eq!(b.messages, vec!["first", "second"]);
}

// flush_expired() dispatches an expired window
#[tokio::test]
async fn flush_expired_dispatches_expired_window() {
    let c = ChatCollector::new(1, 10, 0);
    assert!(c.process(make_msg("chat:1", "hi")).await.is_none());

    tokio::time::sleep(Duration::from_millis(10)).await;

    let batches = c.flush_expired().await;
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].chat_key, "chat:1");
    assert_eq!(batches[0].messages, vec!["hi"]);
}

// flush_expired() respects max_agents
#[tokio::test]
async fn flush_expired_respects_max_agents() {
    let c = ChatCollector::new(1, 1, 0); // max_agents = 1
    assert!(c.process(make_msg("chat:A", "a")).await.is_none());
    assert!(c.process(make_msg("chat:B", "b")).await.is_none());

    tokio::time::sleep(Duration::from_millis(10)).await;

    let batches = c.flush_expired().await;
    assert_eq!(batches.len(), 1, "only 1 batch when max_agents=1");
    assert_eq!(c.active_count().await, 1);
}

// mark_done() enters Cooldown when cooldown_ms > 0 — next message is dropped
#[tokio::test]
async fn mark_done_enters_cooldown_when_configured() {
    let c = ChatCollector::new(0, 10, 60_000);
    assert!(c.process(make_msg("chat:1", "go")).await.is_some());
    c.mark_done("chat:1").await;
    let r = c.process(make_msg("chat:1", "during cooldown")).await;
    assert!(r.is_none(), "message during cooldown must be dropped");
}

// mark_done() enters Idle when cooldown_ms = 0 — next message dispatches
#[tokio::test]
async fn mark_done_enters_idle_when_no_cooldown() {
    let c = ChatCollector::new(0, 10, 0);
    assert!(c.process(make_msg("chat:1", "go")).await.is_some());
    c.mark_done("chat:1").await;
    assert_eq!(c.active_count().await, 0);
    let r = c.process(make_msg("chat:1", "after done")).await;
    assert!(r.is_some(), "message after mark_done with no cooldown should dispatch");
}

// Running chat drops incoming messages
#[tokio::test]
async fn running_chat_drops_messages() {
    let c = ChatCollector::new(0, 10, 0);
    assert!(c.process(make_msg("chat:1", "start")).await.is_some());
    let r = c.process(make_msg("chat:1", "dropped")).await;
    assert!(r.is_none(), "message to Running chat must be dropped");
}

// Cooldown chat drops incoming messages
#[tokio::test]
async fn cooldown_chat_drops_messages() {
    let c = ChatCollector::new(0, 10, 60_000);
    assert!(c.process(make_msg("chat:1", "start")).await.is_some());
    c.mark_done("chat:1").await;
    let r = c.process(make_msg("chat:1", "dropped")).await;
    assert!(r.is_none(), "message to Cooldown chat must be dropped");
}
