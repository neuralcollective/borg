use borg_core::chat::{ChatCollector, IncomingMessage};

fn make_msg(chat_key: &str) -> IncomingMessage {
    IncomingMessage {
        chat_key: chat_key.to_string(),
        sender_name: "alice".to_string(),
        text: "hello".to_string(),
        timestamp: 0,
        reply_to_message_id: None,
    }
}

/// Drive a chat to Running state: window_ms=0 dispatches immediately.
async fn chat_in_running(cooldown_ms: u64) -> ChatCollector {
    let collector = ChatCollector::new(0, 10, cooldown_ms);
    let batch = collector.process(make_msg("chat1")).await;
    assert!(batch.is_some(), "expected immediate dispatch to Running");
    collector
}

#[tokio::test]
async fn mark_done_with_nonzero_cooldown_inserts_cooldown_state() {
    let collector = chat_in_running(5_000).await;
    assert_eq!(collector.active_count().await, 1);

    collector.mark_done("chat1").await;

    assert_eq!(collector.active_count().await, 0, "running count must decrement");

    // Cooldown state drops new messages
    let result = collector.process(make_msg("chat1")).await;
    assert!(result.is_none(), "message to chat in Cooldown must be dropped");
}

#[tokio::test]
async fn mark_done_with_zero_cooldown_inserts_idle_state() {
    let collector = chat_in_running(0).await;
    assert_eq!(collector.active_count().await, 1);

    collector.mark_done("chat1").await;

    assert_eq!(collector.active_count().await, 0, "running count must decrement");

    // Idle state dispatches immediately (window_ms=0)
    let result = collector.process(make_msg("chat1")).await;
    assert!(result.is_some(), "message to chat in Idle must dispatch");
}

#[tokio::test]
async fn mark_done_running_counter_does_not_underflow_when_chat_was_never_running() {
    let collector = ChatCollector::new(0, 10, 0);

    // running starts at 0; saturating_sub must keep it at 0
    collector.mark_done("phantom").await;

    assert_eq!(collector.active_count().await, 0, "running must not underflow below 0");
}
