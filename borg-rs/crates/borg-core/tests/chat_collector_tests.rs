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

// ── Idle → immediate dispatch (zero window) ───────────────────────────────────

#[tokio::test]
async fn test_zero_window_idle_dispatches_immediately() {
    let collector = ChatCollector::new(0, 1, 0);
    let result = collector.process(msg("chat:1", "hello")).await;
    assert!(result.is_some(), "zero-window Idle should dispatch immediately");
    let batch = result.unwrap();
    assert_eq!(batch.chat_key, "chat:1");
    assert_eq!(batch.sender_name, "Alice");
    assert_eq!(batch.messages, vec!["hello"]);
}

// ── Idle → Collecting (windowed, window not yet expired) ──────────────────────

#[tokio::test]
async fn test_windowed_idle_enters_collecting() {
    let collector = ChatCollector::new(500, 1, 0);
    let r = collector.process(msg("chat:1", "first")).await;
    assert!(r.is_none(), "first windowed message should start collecting, not dispatch");
}

// ── Collecting accumulates without dispatching while window is open ───────────

#[tokio::test]
async fn test_collecting_accumulates_messages_within_window() {
    let collector = ChatCollector::new(500, 1, 0);
    let r1 = collector.process(msg("chat:1", "first")).await;
    assert!(r1.is_none());
    let r2 = collector.process(msg("chat:1", "second")).await;
    assert!(r2.is_none(), "second message inside window should also accumulate");
    let r3 = collector.process(msg("chat:1", "third")).await;
    assert!(r3.is_none(), "third message inside window should also accumulate");
}

// ── Collecting → dispatch once window expires ─────────────────────────────────

#[tokio::test]
async fn test_collecting_dispatches_on_window_expiry() {
    // 1ms window → expire by sleeping 20ms before the second message
    let collector = ChatCollector::new(1, 1, 0);

    let r1 = collector.process(msg("chat:1", "first")).await;
    assert!(r1.is_none(), "first message starts collecting");

    tokio::time::sleep(Duration::from_millis(20)).await;

    let r2 = collector.process(msg("chat:1", "second")).await;
    assert!(r2.is_some(), "message after window expiry should trigger dispatch");
    let batch = r2.unwrap();
    assert_eq!(batch.chat_key, "chat:1");
    assert_eq!(batch.messages, vec!["first", "second"]);
}

// ── Running → drops messages ──────────────────────────────────────────────────

#[tokio::test]
async fn test_running_drops_messages() {
    let collector = ChatCollector::new(0, 1, 0);

    let r1 = collector.process(msg("chat:1", "first")).await;
    assert!(r1.is_some(), "first message dispatches, chat enters Running");

    let r2 = collector.process(msg("chat:1", "second")).await;
    assert!(r2.is_none(), "Running state must drop incoming messages");

    let r3 = collector.process(msg("chat:1", "third")).await;
    assert!(r3.is_none(), "Running state must drop all subsequent messages");
}

// ── Cooldown → drops messages ─────────────────────────────────────────────────

#[tokio::test]
async fn test_cooldown_drops_messages() {
    let collector = ChatCollector::new(0, 1, 500);

    let r1 = collector.process(msg("chat:1", "first")).await;
    assert!(r1.is_some(), "initial dispatch succeeds");

    collector.mark_done("chat:1").await;

    let r2 = collector.process(msg("chat:1", "during-cooldown")).await;
    assert!(r2.is_none(), "Cooldown state must drop messages");
}

// ── Concurrency limit defers via try_dispatch ─────────────────────────────────

#[tokio::test]
async fn test_concurrency_limit_defers_new_chat() {
    // max_agents=1: once one chat is Running, a second chat cannot dispatch
    let collector = ChatCollector::new(0, 1, 0);

    let r1 = collector.process(msg("chat:1", "hello")).await;
    assert!(r1.is_some(), "first chat dispatches");
    assert_eq!(collector.active_count().await, 1);

    let r2 = collector.process(msg("chat:2", "world")).await;
    assert!(r2.is_none(), "second chat deferred at concurrency limit");
}

#[tokio::test]
async fn test_flush_expired_defers_when_at_concurrency_limit() {
    // max_agents=1, window=1ms:
    // Step 1: chat:a collects, window expires, flush dispatches it (Running).
    // Step 2: chat:b collects, window expires, flush sees slot full → deferred.
    let collector = ChatCollector::new(1, 1, 0);

    // chat:a enters Collecting
    let r_a = collector.process(msg("chat:a", "ping")).await;
    assert!(r_a.is_none(), "chat:a starts collecting");

    tokio::time::sleep(Duration::from_millis(20)).await;

    // flush: chat:a window expired, slot free → dispatches
    let flushed = collector.flush_expired().await;
    assert_eq!(flushed.len(), 1, "chat:a should flush");
    assert_eq!(collector.active_count().await, 1);

    // chat:b enters Collecting
    let r_b = collector.process(msg("chat:b", "msg")).await;
    assert!(r_b.is_none(), "chat:b starts collecting");

    tokio::time::sleep(Duration::from_millis(20)).await;

    // flush: chat:b window expired but slot is full → not dispatched
    let flushed2 = collector.flush_expired().await;
    assert!(flushed2.is_empty(), "flush must defer when at concurrency limit");
}

// ── After mark_done (no cooldown) chat returns to Idle ────────────────────────

#[tokio::test]
async fn test_mark_done_no_cooldown_returns_to_idle() {
    let collector = ChatCollector::new(0, 1, 0);

    let r1 = collector.process(msg("chat:1", "first")).await;
    assert!(r1.is_some());
    assert_eq!(collector.active_count().await, 1);

    collector.mark_done("chat:1").await;
    assert_eq!(collector.active_count().await, 0);

    let r2 = collector.process(msg("chat:1", "second")).await;
    assert!(r2.is_some(), "after mark_done without cooldown, chat is Idle and dispatches");
}

// ── Independent chats are isolated ───────────────────────────────────────────

#[tokio::test]
async fn test_independent_chats_do_not_interfere() {
    // max_agents=0 (unlimited): each chat has its own state
    let collector = ChatCollector::new(0, 0, 0);

    let r1 = collector.process(msg("chat:1", "a")).await;
    let r2 = collector.process(msg("chat:2", "b")).await;
    let r3 = collector.process(msg("chat:3", "c")).await;

    assert!(r1.is_some(), "chat:1 dispatches");
    assert!(r2.is_some(), "chat:2 dispatches independently");
    assert!(r3.is_some(), "chat:3 dispatches independently");

    // Running messages in chat:1 do not affect chat:2
    let r4 = collector.process(msg("chat:1", "dropped")).await;
    let r5 = collector.process(msg("chat:2", "also dropped")).await;
    assert!(r4.is_none(), "chat:1 Running drops message");
    assert!(r5.is_none(), "chat:2 Running drops message");
}
