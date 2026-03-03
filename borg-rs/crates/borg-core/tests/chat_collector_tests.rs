use borg_core::chat::{ChatCollector, IncomingMessage};
use std::time::Duration;

fn msg(chat_key: &str, text: &str) -> IncomingMessage {
    IncomingMessage {
        chat_key: chat_key.to_string(),
        sender_name: "alice".to_string(),
        text: text.to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    }
}

// Idle + window_ms=0 → immediate dispatch
#[tokio::test]
async fn test_idle_zero_window_dispatches_immediately() {
    let c = ChatCollector::new(0, 0, 0);
    let batch = c.process(msg("chat:1", "hello")).await.unwrap();
    assert_eq!(batch.chat_key, "chat:1");
    assert_eq!(batch.sender_name, "alice");
    assert_eq!(batch.messages, vec!["hello"]);
    assert_eq!(c.active_count().await, 1);
}

// Idle + window_ms>0 → moves to Collecting, returns None
#[tokio::test]
async fn test_idle_nonzero_window_returns_none() {
    let c = ChatCollector::new(5_000, 0, 0);
    let result = c.process(msg("chat:1", "hello")).await;
    assert!(result.is_none());
    assert_eq!(c.active_count().await, 0);
}

// Collecting + expired deadline → dispatches via try_dispatch
#[tokio::test]
async fn test_collecting_expired_deadline_dispatches() {
    let c = ChatCollector::new(1, 0, 0); // 1 ms window
    let r1 = c.process(msg("chat:1", "msg1")).await;
    assert!(r1.is_none(), "first message should enter Collecting");

    tokio::time::sleep(Duration::from_millis(10)).await;

    let batch = c.process(msg("chat:1", "msg2")).await.unwrap();
    assert_eq!(batch.messages, vec!["msg1", "msg2"]);
    assert_eq!(c.active_count().await, 1);
}

// Collecting + window still open → stays Collecting, returns None
#[tokio::test]
async fn test_collecting_within_window_returns_none() {
    let c = ChatCollector::new(60_000, 0, 0); // very long window
    let r1 = c.process(msg("chat:1", "first")).await;
    assert!(r1.is_none());
    let r2 = c.process(msg("chat:1", "second")).await;
    assert!(r2.is_none(), "window not expired, should remain Collecting");
    assert_eq!(c.active_count().await, 0);
}

// Running state drops messages silently
#[tokio::test]
async fn test_running_drops_messages() {
    let c = ChatCollector::new(0, 0, 0);
    // Dispatch first → Running
    let r1 = c.process(msg("chat:1", "first")).await;
    assert!(r1.is_some());
    // Further messages dropped
    let r2 = c.process(msg("chat:1", "second")).await;
    assert!(r2.is_none());
    let r3 = c.process(msg("chat:1", "third")).await;
    assert!(r3.is_none());
    // Running count unchanged
    assert_eq!(c.active_count().await, 1);
}

// Cooldown drops messages silently
#[tokio::test]
async fn test_cooldown_drops_messages() {
    let c = ChatCollector::new(0, 0, 60_000); // 60 s cooldown
    let r1 = c.process(msg("chat:1", "first")).await;
    assert!(r1.is_some());

    c.mark_done("chat:1").await;
    assert_eq!(c.active_count().await, 0);

    let r2 = c.process(msg("chat:1", "during cooldown")).await;
    assert!(r2.is_none());
}

// max_agents enforcement: blocks dispatch when limit reached
#[tokio::test]
async fn test_max_agents_blocks_dispatch() {
    let c = ChatCollector::new(0, 1, 0); // max 1 agent, no window

    // First chat dispatches (running=1)
    let r1 = c.process(msg("chat:1", "first")).await;
    assert!(r1.is_some());
    assert_eq!(c.active_count().await, 1);

    // Second chat blocked at the limit
    let r2 = c.process(msg("chat:2", "second")).await;
    assert!(r2.is_none());
    assert!(!c.can_dispatch().await);

    // After chat:1 finishes the limit is lifted
    c.mark_done("chat:1").await;
    assert!(c.can_dispatch().await);

    let r3 = c.process(msg("chat:2", "retry")).await;
    assert!(r3.is_some());
}

// max_agents=0 means unlimited
#[tokio::test]
async fn test_max_agents_zero_is_unlimited() {
    let c = ChatCollector::new(0, 0, 0);
    for i in 0..10u32 {
        let key = format!("chat:{i}");
        let r = c.process(msg(&key, "hi")).await;
        assert!(r.is_some(), "chat {i} should dispatch");
    }
    assert_eq!(c.active_count().await, 10);
}

// mark_done with no cooldown returns chat to Idle (can dispatch again)
#[tokio::test]
async fn test_mark_done_no_cooldown_returns_to_idle() {
    let c = ChatCollector::new(0, 0, 0);
    let r1 = c.process(msg("chat:1", "first")).await;
    assert!(r1.is_some());

    c.mark_done("chat:1").await;
    assert_eq!(c.active_count().await, 0);

    // Now Idle again — should dispatch immediately
    let r2 = c.process(msg("chat:1", "second")).await;
    assert!(r2.is_some());
}
