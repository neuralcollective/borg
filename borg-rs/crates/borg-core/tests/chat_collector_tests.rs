use std::time::Duration;

use borg_core::chat::{ChatCollector, IncomingMessage};

fn msg(chat_key: &str, text: &str) -> IncomingMessage {
    IncomingMessage {
        chat_key: chat_key.to_string(),
        sender_name: "tester".to_string(),
        text: text.to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    }
}

/// With max_agents=1 and one Running chat, flush_expired must not dispatch an expired Collecting chat.
#[tokio::test]
async fn flush_expired_respects_max_agents_limit() {
    // window=1ms, max=1, cooldown=0
    let c = ChatCollector::new(1, 1, 0);

    // Chat A goes into Collecting (window > 0)
    let r = c.process(msg("A", "hello")).await;
    assert!(r.is_none(), "window not expired yet, should be Collecting");

    // Wait for A's window to expire, then flush -> dispatches A (running=1)
    tokio::time::sleep(Duration::from_millis(10)).await;
    let batches = c.flush_expired().await;
    assert_eq!(batches.len(), 1, "A should be dispatched after window expires");
    assert_eq!(batches[0].chat_key, "A");
    assert_eq!(c.active_count().await, 1);

    // Put chat B into Collecting
    let r = c.process(msg("B", "world")).await;
    assert!(r.is_none(), "B should enter Collecting state");

    // Wait for B's window to expire
    tokio::time::sleep(Duration::from_millis(10)).await;

    // flush_expired: running=1 == max_agents=1, so B must not be dispatched
    let batches = c.flush_expired().await;
    assert!(batches.is_empty(), "B must not dispatch while at max_agents limit");
    assert_eq!(c.active_count().await, 1, "running count must stay at 1");
}

/// flush_expired transitions a Cooldown chat to Idle after the cooldown deadline passes.
#[tokio::test]
async fn flush_expired_cooldown_expiry_resets_to_idle() {
    // window=0 (immediate dispatch), max=0 (unlimited), cooldown=10ms
    let c = ChatCollector::new(0, 0, 10);

    // Dispatch A immediately (Idle + window=0)
    let batch = c.process(msg("A", "first")).await;
    assert!(batch.is_some(), "A should dispatch immediately");

    // Agent done: A enters Cooldown(10ms), running goes to 0
    c.mark_done("A").await;
    assert_eq!(c.active_count().await, 0);

    // Messages during cooldown are dropped
    let r = c.process(msg("A", "during cooldown")).await;
    assert!(r.is_none(), "message during cooldown must be dropped");

    // Cooldown not yet expired: flush_expired does nothing
    let batches = c.flush_expired().await;
    assert!(batches.is_empty(), "no dispatch from Cooldown flush before expiry");

    // Wait for cooldown to expire
    tokio::time::sleep(Duration::from_millis(25)).await;

    // flush_expired now transitions A: Cooldown -> Idle (no batch produced)
    let batches = c.flush_expired().await;
    assert!(batches.is_empty(), "cooldown flush produces no MessageBatch");

    // A is now Idle: next message dispatches immediately (window=0)
    let batch = c.process(msg("A", "after cooldown")).await;
    assert!(batch.is_some(), "A must accept new messages after cooldown resets to Idle");
}

/// mark_done with cooldown_ms=0 returns chat to Idle and correctly decrements running.
#[tokio::test]
async fn mark_done_no_cooldown_returns_idle_and_decrements_running() {
    // window=0, max=1, cooldown=0
    let c = ChatCollector::new(0, 1, 0);

    // Dispatch A: running becomes 1
    let batch = c.process(msg("A", "first")).await;
    assert!(batch.is_some());
    assert_eq!(c.active_count().await, 1);
    assert!(!c.can_dispatch().await, "at max_agents limit, can_dispatch must be false");

    // mark_done with cooldown=0: A -> Idle, running decremented to 0
    c.mark_done("A").await;
    assert_eq!(c.active_count().await, 0);
    assert!(c.can_dispatch().await, "after mark_done, can_dispatch must be true");

    // A is Idle: a new message dispatches immediately
    let batch = c.process(msg("A", "second")).await;
    assert!(batch.is_some(), "A must be dispatchable after returning to Idle");
    assert_eq!(c.active_count().await, 1);
}
