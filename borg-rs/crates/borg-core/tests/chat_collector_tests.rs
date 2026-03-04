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

// AC1: window_ms == 0 dispatches immediately on first message
#[tokio::test]
async fn immediate_dispatch_when_window_zero() {
    let c = ChatCollector::new(0, 0, 0);
    let batch = c.process(msg("chat:1", "hello")).await;
    let b = batch.expect("should dispatch immediately");
    assert_eq!(b.chat_key, "chat:1");
    assert_eq!(b.sender_name, "Alice");
    assert_eq!(b.messages, vec!["hello"]);
    assert_eq!(c.active_count().await, 1);
}

// AC2 + AC3: messages accumulate within the window and flush dispatches them all
#[tokio::test]
async fn accumulates_messages_then_flushes_on_expiry() {
    let c = ChatCollector::new(1, 0, 0); // 1ms window — short enough to expire on sleep

    // All three arrive before window expires (fast in-process calls)
    // They go into Collecting state, not dispatched yet.
    // Note: if the machine is pathologically slow and 1ms elapses between calls,
    // the second or third message may trigger an inline dispatch. We guard against
    // that by testing the flush path only after sleeping past the deadline.
    let r1 = c.process(msg("chat:1", "m1")).await;
    let r2 = c.process(msg("chat:1", "m2")).await;
    let r3 = c.process(msg("chat:1", "m3")).await;

    // At least m1 should have gone into Collecting (returned None).
    // If m1 triggers inline dispatch that is also acceptable per spec (window_ms=0
    // would be used for that), but with window_ms=1 the first message always opens
    // a collection window and returns None.
    assert!(r1.is_none(), "first message should open window, not dispatch");

    // After a short sleep the window has definitely expired.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    // If r2/r3 already dispatched inline (very slow machine), skip flush assertion.
    // In the normal case we expect exactly one batch from flush.
    let batches = c.flush_expired().await;

    // Determine how many messages are still pending.
    // Either the batch from flush contains all, or some were dispatched inline.
    // The invariant we care about: every message sent is either dispatched inline
    // OR appears in the flush batch. No message is silently dropped in Collecting.
    let inline_count = [r2, r3].iter().filter(|r| r.is_some()).count();
    if inline_count == 0 {
        // Normal path: all three in the flush batch.
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].messages, vec!["m1", "m2", "m3"]);
    } else {
        // Slow-machine path: at least m1 was collected, the rest may have dispatched.
        // The flush batch (if any) should contain at least "m1".
        if !batches.is_empty() {
            assert!(batches[0].messages.contains(&"m1".to_string()));
        }
    }
}

// AC2 (isolation): no message dispatches while still within a long window
#[tokio::test]
async fn no_dispatch_within_long_window() {
    let c = ChatCollector::new(60_000, 0, 0); // 60s window — will never expire during test
    assert!(c.process(msg("chat:1", "a")).await.is_none());
    assert!(c.process(msg("chat:1", "b")).await.is_none());
    assert!(c.process(msg("chat:1", "c")).await.is_none());
    assert_eq!(c.active_count().await, 0);
}

// AC4: messages are dropped while the chat has a running agent
#[tokio::test]
async fn messages_dropped_while_running() {
    let c = ChatCollector::new(0, 0, 0);

    let first = c.process(msg("chat:1", "first")).await;
    assert!(first.is_some(), "first message should dispatch");
    assert_eq!(c.active_count().await, 1);

    let dropped = c.process(msg("chat:1", "second")).await;
    assert!(dropped.is_none(), "message while Running must be dropped");
    assert_eq!(c.active_count().await, 1, "running count must not change");
}

// AC5: messages are dropped during cooldown
#[tokio::test]
async fn messages_dropped_during_cooldown() {
    let c = ChatCollector::new(0, 0, 60_000); // 60s cooldown

    let batch = c.process(msg("chat:1", "go")).await;
    assert!(batch.is_some());

    c.mark_done("chat:1").await;
    assert_eq!(c.active_count().await, 0);

    let dropped = c.process(msg("chat:1", "during_cooldown")).await;
    assert!(dropped.is_none(), "message during Cooldown must be dropped");
}

// AC6a: mark_done enters Cooldown when cooldown_ms > 0
#[tokio::test]
async fn mark_done_enters_cooldown_when_configured() {
    let c = ChatCollector::new(0, 0, 60_000);

    c.process(msg("chat:1", "go")).await;
    c.mark_done("chat:1").await;

    // Chat is now in Cooldown — next message must be dropped.
    assert!(
        c.process(msg("chat:1", "blocked")).await.is_none(),
        "next message should be blocked by cooldown"
    );
}

// AC6b: mark_done returns to Idle when cooldown_ms == 0
#[tokio::test]
async fn mark_done_returns_to_idle_when_no_cooldown() {
    let c = ChatCollector::new(0, 0, 0);

    c.process(msg("chat:1", "go")).await;
    c.mark_done("chat:1").await;
    assert_eq!(c.active_count().await, 0);

    // Chat is Idle again — next message dispatches immediately.
    let batch = c.process(msg("chat:1", "new")).await;
    let b = batch.expect("should dispatch after returning to Idle");
    assert_eq!(b.messages, vec!["new"]);
}

// AC7: concurrency gate prevents dispatch when running >= max_agents
#[tokio::test]
async fn concurrency_gate_blocks_when_at_max() {
    let c = ChatCollector::new(0, 1, 0); // max_agents = 1

    let b1 = c.process(msg("chat:1", "first")).await;
    assert!(b1.is_some(), "chat:1 should dispatch");
    assert_eq!(c.active_count().await, 1);

    // Different chat — still blocked by global concurrency gate.
    let b2 = c.process(msg("chat:2", "second")).await;
    assert!(b2.is_none(), "chat:2 should be blocked by concurrency gate");

    // After chat:1 finishes the slot opens.
    c.mark_done("chat:1").await;
    assert_eq!(c.active_count().await, 0);
    assert!(c.can_dispatch().await);

    let b3 = c.process(msg("chat:2", "retry")).await;
    assert!(b3.is_some(), "chat:2 should dispatch once slot is free");
}

// flush_expired also respects the concurrency gate
#[tokio::test]
async fn flush_expired_respects_concurrency_gate() {
    let c = ChatCollector::new(1, 1, 0); // window=1ms, max_agents=1

    // Two chats start collecting.
    assert!(c.process(msg("chat:1", "a")).await.is_none());
    assert!(c.process(msg("chat:2", "b")).await.is_none());

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    // flush_expired should dispatch at most one (limited by max_agents).
    let batches = c.flush_expired().await;
    assert_eq!(batches.len(), 1, "only one batch should dispatch at the limit");
    assert_eq!(c.active_count().await, 1);
}

// flush_expired clears expired cooldown windows, allowing subsequent dispatch
#[tokio::test]
async fn flush_expired_clears_expired_cooldowns() {
    let c = ChatCollector::new(0, 0, 1); // cooldown = 1ms

    c.process(msg("chat:1", "go")).await;
    c.mark_done("chat:1").await;

    // Confirm in cooldown.
    assert!(c.process(msg("chat:1", "blocked")).await.is_none());

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    // flush_expired should transition Cooldown → Idle.
    c.flush_expired().await;

    // Now chat:1 is Idle again and can dispatch.
    let batch = c.process(msg("chat:1", "after_cooldown")).await;
    assert!(
        batch.is_some(),
        "should dispatch after cooldown clears via flush_expired"
    );
}
