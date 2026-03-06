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

// AC1: process returns None while collecting window has not expired
#[tokio::test]
async fn collecting_window_returns_none() {
    let collector = ChatCollector::new(10_000, 0, 0); // 10s window

    let r1 = collector.process(make_msg("chat1", "hello")).await;
    assert!(
        r1.is_none(),
        "first message should start Collecting and return None"
    );

    let r2 = collector.process(make_msg("chat1", "world")).await;
    assert!(
        r2.is_none(),
        "second message within window should accumulate and return None"
    );
}

// AC2: flush_expired returns a MessageBatch once the window has elapsed
#[tokio::test]
async fn flush_expired_returns_batch_after_window() {
    let collector = ChatCollector::new(1, 0, 0); // 1ms window

    let r = collector.process(make_msg("chat1", "msg1")).await;
    assert!(r.is_none());

    tokio::time::sleep(Duration::from_millis(20)).await;

    let batches = collector.flush_expired().await;
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].chat_key, "chat1");
    assert_eq!(batches[0].messages, vec!["msg1"]);
}

// Multiple messages sent before flush are all included in the batch
#[tokio::test]
async fn messages_accumulate_before_flush() {
    let collector = ChatCollector::new(1, 0, 0); // 1ms window

    assert!(collector.process(make_msg("chat1", "a")).await.is_none());
    assert!(collector.process(make_msg("chat1", "b")).await.is_none());
    assert!(collector.process(make_msg("chat1", "c")).await.is_none());

    tokio::time::sleep(Duration::from_millis(20)).await;

    let batches = collector.flush_expired().await;
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].messages, vec!["a", "b", "c"]);
}

// AC3: mark_done with cooldown_ms > 0 transitions Running → Cooldown,
//       and messages during cooldown are dropped.
#[tokio::test]
async fn mark_done_enters_cooldown() {
    // window_ms=0 → immediate dispatch; cooldown_ms=60_000 → 60s cooldown
    let collector = ChatCollector::new(0, 0, 60_000);

    let batch = collector.process(make_msg("chat1", "go")).await;
    assert!(batch.is_some(), "window=0 should dispatch immediately");
    assert_eq!(collector.active_count().await, 1);

    collector.mark_done("chat1").await;
    assert_eq!(collector.active_count().await, 0);

    // In cooldown: next message must be dropped
    let r = collector.process(make_msg("chat1", "again")).await;
    assert!(r.is_none(), "message during cooldown must be dropped");
}

// mark_done without cooldown returns the chat to Idle
#[tokio::test]
async fn mark_done_without_cooldown_returns_to_idle() {
    let collector = ChatCollector::new(0, 0, 0); // no cooldown

    let batch = collector.process(make_msg("chat1", "first")).await;
    assert!(batch.is_some());

    collector.mark_done("chat1").await;

    // Back to Idle: next message should dispatch again
    let batch2 = collector.process(make_msg("chat1", "second")).await;
    assert!(
        batch2.is_some(),
        "after mark_done (no cooldown) should dispatch again"
    );
}

// AC4: can_dispatch returns false when active_count reaches max_agents
#[tokio::test]
async fn can_dispatch_false_at_max_agents() {
    let collector = ChatCollector::new(0, 1, 0); // max 1 agent

    assert!(
        collector.can_dispatch().await,
        "should be dispatchable initially"
    );

    let batch = collector.process(make_msg("chat1", "go")).await;
    assert!(batch.is_some());

    assert!(
        !collector.can_dispatch().await,
        "at max_agents=1 must return false"
    );
    assert_eq!(collector.active_count().await, 1);
}

// max_agents=0 means unlimited: can_dispatch always returns true regardless of running count
#[tokio::test]
async fn max_agents_zero_means_unlimited() {
    let collector = ChatCollector::new(0, 0, 0); // unlimited

    // Dispatch multiple chats
    assert!(collector.process(make_msg("c1", "a")).await.is_some());
    assert!(collector.process(make_msg("c2", "b")).await.is_some());
    assert!(collector.process(make_msg("c3", "c")).await.is_some());

    assert!(
        collector.can_dispatch().await,
        "unlimited agents: can_dispatch must be true"
    );
    assert_eq!(collector.active_count().await, 3);
}

// AC5: second process call while Running is dropped (not buffered)
#[tokio::test]
async fn process_during_running_is_dropped() {
    let collector = ChatCollector::new(0, 0, 0); // immediate dispatch, no cooldown

    let batch = collector.process(make_msg("chat1", "first")).await;
    assert!(batch.is_some(), "first message should dispatch");

    let r = collector.process(make_msg("chat1", "second")).await;
    assert!(r.is_none(), "message during Running must be dropped");
}

// flush_expired clears expired cooldowns back to Idle
#[tokio::test]
async fn flush_expired_clears_cooldown_to_idle() {
    let collector = ChatCollector::new(0, 0, 1); // 1ms cooldown

    let batch = collector.process(make_msg("chat1", "go")).await;
    assert!(batch.is_some());

    collector.mark_done("chat1").await;

    // Wait for cooldown to expire, then flush
    tokio::time::sleep(Duration::from_millis(20)).await;
    let batches = collector.flush_expired().await;
    assert!(
        batches.is_empty(),
        "flush during cooldown expiry yields no new batches"
    );

    // Now in Idle: next message should dispatch
    let r = collector.process(make_msg("chat1", "after cooldown")).await;
    assert!(
        r.is_some(),
        "after cooldown flush chat should be Idle and dispatch"
    );
}

// flush_expired at max_agents does not dispatch additional batches
#[tokio::test]
async fn flush_expired_respects_max_agents_limit() {
    let collector = ChatCollector::new(1, 1, 0); // 1ms window, max 1 agent

    // Start collecting for two chats
    assert!(collector.process(make_msg("c1", "hello")).await.is_none());
    assert!(collector.process(make_msg("c2", "hello")).await.is_none());

    tokio::time::sleep(Duration::from_millis(20)).await;

    // Only one batch should be dispatched (max_agents=1)
    let batches = collector.flush_expired().await;
    assert_eq!(batches.len(), 1, "max_agents=1 limits flush to one batch");
    assert_eq!(collector.active_count().await, 1);
}

// window_ms=0 dispatches immediately without entering Collecting
#[tokio::test]
async fn window_zero_dispatches_immediately() {
    let collector = ChatCollector::new(0, 0, 0);

    let batch = collector.process(make_msg("chat1", "hi")).await;
    assert!(
        batch.is_some(),
        "window=0 must dispatch on the first process call"
    );
    assert_eq!(batch.unwrap().messages, vec!["hi"]);
}
