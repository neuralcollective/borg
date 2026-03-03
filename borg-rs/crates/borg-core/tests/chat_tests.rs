use std::time::Duration;

use borg_core::chat::{ChatCollector, IncomingMessage};
use tokio::time::sleep;

fn msg(chat_key: &str, text: &str) -> IncomingMessage {
    IncomingMessage {
        chat_key: chat_key.to_string(),
        sender_name: "Alice".to_string(),
        text: text.to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    }
}

// ── Idle → Collecting ─────────────────────────────────────────────────────────

#[tokio::test]
async fn idle_to_collecting_no_dispatch_within_window() {
    let c = ChatCollector::new(5000, 1, 0);
    let result = c.process(msg("chat1", "hello")).await;
    assert!(result.is_none(), "first message within window should not dispatch");
    assert_eq!(c.active_count().await, 0);
}

// ── Idle → Running (window = 0) ───────────────────────────────────────────────

#[tokio::test]
async fn idle_to_running_immediate_when_window_zero() {
    let c = ChatCollector::new(0, 1, 0);
    let batch = c.process(msg("chat1", "hello")).await.expect("should dispatch immediately");
    assert_eq!(batch.chat_key, "chat1");
    assert_eq!(batch.sender_name, "Alice");
    assert_eq!(batch.messages, vec!["hello"]);
    assert_eq!(c.active_count().await, 1);
}

// ── Collecting → Running via process (window expired) ─────────────────────────

#[tokio::test]
async fn collecting_to_running_after_window_expiry_via_process() {
    let c = ChatCollector::new(10, 1, 0);
    let r1 = c.process(msg("chat1", "msg1")).await;
    assert!(r1.is_none(), "first message starts collecting");

    sleep(Duration::from_millis(30)).await;

    let batch = c.process(msg("chat1", "msg2")).await.expect("expired window should dispatch");
    assert_eq!(batch.messages, vec!["msg1", "msg2"]);
    assert_eq!(c.active_count().await, 1);
}

// ── Collecting → Running via flush_expired ────────────────────────────────────

#[tokio::test]
async fn flush_expired_dispatches_expired_collecting_chat() {
    let c = ChatCollector::new(10, 1, 0);
    c.process(msg("chat1", "hello")).await;

    sleep(Duration::from_millis(30)).await;

    let batches = c.flush_expired().await;
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].chat_key, "chat1");
    assert_eq!(c.active_count().await, 1);
}

#[tokio::test]
async fn flush_expired_does_not_dispatch_within_window() {
    let c = ChatCollector::new(5000, 1, 0);
    c.process(msg("chat1", "hello")).await;

    let batches = c.flush_expired().await;
    assert!(batches.is_empty(), "window not expired; nothing should dispatch");
    assert_eq!(c.active_count().await, 0);
}

// ── Collecting accumulates multiple messages ──────────────────────────────────

#[tokio::test]
async fn collecting_accumulates_messages_before_dispatch() {
    let c = ChatCollector::new(10, 1, 0);
    c.process(msg("chat1", "one")).await;
    c.process(msg("chat1", "two")).await;
    c.process(msg("chat1", "three")).await;

    sleep(Duration::from_millis(30)).await;

    let batches = c.flush_expired().await;
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].messages, vec!["one", "two", "three"]);
}

// ── mark_done → Cooldown ──────────────────────────────────────────────────────

#[tokio::test]
async fn mark_done_with_cooldown_enters_cooldown_and_drops_messages() {
    let c = ChatCollector::new(0, 1, 5000);
    c.process(msg("chat1", "hello")).await;
    assert_eq!(c.active_count().await, 1);

    c.mark_done("chat1").await;
    assert_eq!(c.active_count().await, 0);

    let result = c.process(msg("chat1", "during cooldown")).await;
    assert!(result.is_none(), "messages during cooldown should be dropped");
}

// ── mark_done → Idle (no cooldown) ───────────────────────────────────────────

#[tokio::test]
async fn mark_done_without_cooldown_returns_to_idle() {
    let c = ChatCollector::new(0, 1, 0);
    c.process(msg("chat1", "hello")).await;
    assert_eq!(c.active_count().await, 1);

    c.mark_done("chat1").await;
    assert_eq!(c.active_count().await, 0);

    let result = c.process(msg("chat1", "after done")).await;
    assert!(result.is_some(), "should dispatch immediately from Idle again");
}

// ── Cooldown → Idle via flush_expired ─────────────────────────────────────────

#[tokio::test]
async fn flush_expired_transitions_cooldown_to_idle() {
    let c = ChatCollector::new(0, 1, 10);
    c.process(msg("chat1", "hello")).await;
    c.mark_done("chat1").await;

    // Still in cooldown — message dropped
    assert!(c.process(msg("chat1", "blocked")).await.is_none());

    sleep(Duration::from_millis(30)).await;

    let batches = c.flush_expired().await;
    assert!(batches.is_empty(), "cooldown expiry produces no dispatch batches");

    // Now Idle — should dispatch immediately
    let result = c.process(msg("chat1", "after cooldown")).await;
    assert!(result.is_some(), "should be Idle and dispatch again");
}

// ── Concurrency limit: try_dispatch returns None at max agents ────────────────

#[tokio::test]
async fn concurrency_limit_blocks_second_chat_at_max_agents() {
    let c = ChatCollector::new(0, 1, 0);

    let r1 = c.process(msg("chat1", "hello")).await;
    assert!(r1.is_some(), "first chat should dispatch");
    assert_eq!(c.active_count().await, 1);

    let r2 = c.process(msg("chat2", "hello")).await;
    assert!(r2.is_none(), "second chat blocked at concurrency limit");
    assert_eq!(c.active_count().await, 1);
}

#[tokio::test]
async fn flush_expired_respects_concurrency_limit() {
    let c = ChatCollector::new(10, 1, 0);
    c.process(msg("chat1", "m1")).await;
    c.process(msg("chat2", "m2")).await;

    sleep(Duration::from_millis(30)).await;

    let batches = c.flush_expired().await;
    assert_eq!(batches.len(), 1, "only one chat should dispatch at max_agents=1");
    assert_eq!(c.active_count().await, 1);
}

// ── Messages dropped while Running ───────────────────────────────────────────

#[tokio::test]
async fn messages_dropped_while_agent_running() {
    let c = ChatCollector::new(0, 2, 0);
    c.process(msg("chat1", "first")).await;

    let result = c.process(msg("chat1", "second")).await;
    assert!(result.is_none(), "message while Running should be dropped");
    assert_eq!(c.active_count().await, 1, "running count unchanged");
}

// ── Unlimited concurrency (max_agents = 0) ───────────────────────────────────

#[tokio::test]
async fn unlimited_concurrency_dispatches_all_chats() {
    let c = ChatCollector::new(0, 0, 0);

    for i in 0..5 {
        let key = format!("chat{i}");
        let batch = c.process(msg(&key, "hello")).await;
        assert!(batch.is_some(), "chat{i} should dispatch with no limit");
    }
    assert_eq!(c.active_count().await, 5);
}
