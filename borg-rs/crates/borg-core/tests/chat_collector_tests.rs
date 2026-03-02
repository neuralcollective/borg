use borg_core::chat::{ChatCollector, IncomingMessage};

fn msg(chat_key: &str, text: &str) -> IncomingMessage {
    IncomingMessage {
        chat_key: chat_key.to_string(),
        sender_name: "alice".to_string(),
        text: text.to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    }
}

// ── Idle → Collecting ──────────────────────────────────────────────────────

/// First message with a non-zero window starts collection (returns None).
#[tokio::test]
async fn idle_first_message_starts_collecting() {
    let c = ChatCollector::new(5000, 1, 0);
    let result = c.process(msg("chat:1", "hello")).await;
    assert!(result.is_none());
    assert_eq!(c.active_count().await, 0);
}

// ── Idle → Running (immediate dispatch when window_ms == 0) ───────────────

/// With window_ms = 0, first message dispatches immediately.
#[tokio::test]
async fn idle_zero_window_dispatches_immediately() {
    let c = ChatCollector::new(0, 1, 0);
    let batch = c.process(msg("chat:1", "hello")).await.expect("should dispatch");
    assert_eq!(batch.chat_key, "chat:1");
    assert_eq!(batch.sender_name, "alice");
    assert_eq!(batch.messages, vec!["hello"]);
    assert_eq!(c.active_count().await, 1);
}

// ── Collecting: accumulation within window ─────────────────────────────────

/// Two messages sent before the window expires are both buffered; neither triggers dispatch.
#[tokio::test]
async fn collecting_accumulates_within_window() {
    let c = ChatCollector::new(5000, 1, 0);
    let r1 = c.process(msg("chat:1", "first")).await;
    let r2 = c.process(msg("chat:1", "second")).await;
    assert!(r1.is_none());
    assert!(r2.is_none());
    assert_eq!(c.active_count().await, 0);
}

// ── Collecting → Running on timeout ───────────────────────────────────────

/// A message arriving after the window deadline triggers dispatch with all buffered messages.
#[tokio::test]
async fn collecting_window_expiry_triggers_dispatch() {
    let c = ChatCollector::new(1, 1, 0); // 1 ms window
    let r1 = c.process(msg("chat:1", "first")).await;
    assert!(r1.is_none());

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let batch = c.process(msg("chat:1", "second")).await.expect("should dispatch after deadline");
    assert_eq!(batch.messages, vec!["first", "second"]);
    assert_eq!(c.active_count().await, 1);
}

// ── Running: new messages are dropped ─────────────────────────────────────

/// While an agent is running for a chat, further messages are silently dropped.
#[tokio::test]
async fn running_drops_new_messages() {
    let c = ChatCollector::new(0, 2, 0);
    let r1 = c.process(msg("chat:1", "trigger")).await;
    assert!(r1.is_some()); // chat:1 now Running

    let r2 = c.process(msg("chat:1", "dropped")).await;
    assert!(r2.is_none());
    assert_eq!(c.active_count().await, 1); // still only one agent
}

// ── Concurrency guard ──────────────────────────────────────────────────────

/// When max_agents is reached no new chat can be dispatched even if it is Idle.
#[tokio::test]
async fn max_agents_blocks_new_dispatch() {
    let c = ChatCollector::new(0, 1, 0); // max 1 agent
    let r1 = c.process(msg("chat:1", "msg")).await;
    assert!(r1.is_some());

    // A different chat, also Idle — should be blocked by the concurrency limit.
    let r2 = c.process(msg("chat:2", "msg")).await;
    assert!(r2.is_none());
    assert_eq!(c.active_count().await, 1);
}

/// With unlimited agents (max_agents = 0), multiple chats can dispatch concurrently.
#[tokio::test]
async fn unlimited_agents_allows_concurrent_dispatch() {
    let c = ChatCollector::new(0, 0, 0);
    let r1 = c.process(msg("chat:1", "a")).await;
    let r2 = c.process(msg("chat:2", "b")).await;
    let r3 = c.process(msg("chat:3", "c")).await;
    assert!(r1.is_some());
    assert!(r2.is_some());
    assert!(r3.is_some());
    assert_eq!(c.active_count().await, 3);
}

// ── Running → Idle (no cooldown) ──────────────────────────────────────────

/// mark_done with cooldown_ms = 0 transitions back to Idle immediately.
#[tokio::test]
async fn mark_done_no_cooldown_returns_to_idle() {
    let c = ChatCollector::new(0, 1, 0);
    c.process(msg("chat:1", "first")).await;
    assert_eq!(c.active_count().await, 1);

    c.mark_done("chat:1").await;
    assert_eq!(c.active_count().await, 0);

    // Chat is Idle again — next message dispatches.
    let r = c.process(msg("chat:1", "second")).await;
    assert!(r.is_some());
}

// ── Running → Cooldown ────────────────────────────────────────────────────

/// mark_done with cooldown_ms > 0 enters Cooldown; messages sent during cooldown are dropped.
#[tokio::test]
async fn mark_done_with_cooldown_drops_messages() {
    let c = ChatCollector::new(0, 1, 5000); // 5 s cooldown
    c.process(msg("chat:1", "go")).await;
    c.mark_done("chat:1").await;
    assert_eq!(c.active_count().await, 0);

    let r = c.process(msg("chat:1", "during cooldown")).await;
    assert!(r.is_none());
}

// ── Cooldown → Idle via flush_expired ─────────────────────────────────────

/// After the cooldown deadline passes, flush_expired moves the chat to Idle.
#[tokio::test]
async fn cooldown_expires_and_allows_next_dispatch() {
    let c = ChatCollector::new(0, 1, 1); // 1 ms cooldown
    c.process(msg("chat:1", "go")).await;
    c.mark_done("chat:1").await;

    // Still in cooldown — message dropped.
    assert!(c.process(msg("chat:1", "too soon")).await.is_none());

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    c.flush_expired().await; // expires cooldown → Idle

    // Now idle again — dispatch succeeds.
    let r = c.process(msg("chat:1", "after cooldown")).await;
    assert!(r.is_some());
}

// ── flush_expired dispatches stale Collecting windows ─────────────────────

/// flush_expired dispatches a Collecting chat whose window has elapsed even if
/// no new message arrives.
#[tokio::test]
async fn flush_expired_dispatches_stale_collecting() {
    let c = ChatCollector::new(1, 1, 0); // 1 ms window
    c.process(msg("chat:1", "only message")).await;

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let batches = c.flush_expired().await;
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].chat_key, "chat:1");
    assert_eq!(batches[0].messages, vec!["only message"]);
    assert_eq!(c.active_count().await, 1);
}

/// flush_expired respects max_agents: a stale Collecting chat is not dispatched
/// when the concurrency limit is already reached.
#[tokio::test]
async fn flush_expired_respects_max_agents() {
    let _c = ChatCollector::new(1, 1, 0); // max 1 agent, 1 ms window
    // chat:1 dispatches immediately via zero-window? No — window_ms=1 so it collects.
    // Use a separate immediate collector to put us at the limit first.
    // Actually, let's use two separate chats in the same collector but one dispatched via
    // flush and one blocked.
    //
    // Approach: chat:a dispatches first (immediate; different collector), then chat:b is stale.
    // Simpler: use window_ms=0 for chat:a to go Running, then chat:b with window=1 goes Collecting.
    // But this collector has window_ms=1 for all chats.
    //
    // Instead: flush chat:a first (it wins the slot), then check chat:b is blocked.
    let c2 = ChatCollector::new(1, 1, 0);
    c2.process(msg("chat:a", "x")).await; // Collecting (window=1ms)
    c2.process(msg("chat:b", "y")).await; // Collecting (window=1ms)

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let batches = c2.flush_expired().await;
    // Only one batch dispatched (max_agents=1).
    assert_eq!(batches.len(), 1);
    assert_eq!(c2.active_count().await, 1);
}
