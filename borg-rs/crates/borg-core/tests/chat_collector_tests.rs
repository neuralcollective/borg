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
async fn window_zero_dispatches_immediately() {
    let collector = ChatCollector::new(0, 1, 0);
    let batch = collector.process(msg("chat1", "hello")).await;
    assert!(batch.is_some(), "window_ms=0 should dispatch immediately");
    let batch = batch.unwrap();
    assert_eq!(batch.chat_key, "chat1");
    assert_eq!(batch.messages, vec!["hello"]);
}

#[tokio::test]
async fn window_zero_transitions_to_running_not_collecting() {
    let collector = ChatCollector::new(0, 1, 0);
    collector.process(msg("chat1", "hello")).await;
    // active_count should be 1 (Running)
    assert_eq!(collector.active_count().await, 1);
    // can_dispatch should be false (at limit of 1)
    assert!(!collector.can_dispatch().await);
}

#[tokio::test]
async fn running_drops_messages() {
    let collector = ChatCollector::new(0, 1, 0);
    // First message starts the agent
    let first = collector.process(msg("chat1", "first")).await;
    assert!(first.is_some());
    // While Running, second message is dropped
    let second = collector.process(msg("chat1", "second")).await;
    assert!(second.is_none(), "messages during Running should be dropped");
}

#[tokio::test]
async fn cooldown_drops_messages_unlimited_agents() {
    // max_agents=0 means unlimited, so concurrency limit is not the reason for dropping
    let collector = ChatCollector::new(0, 0, 500);
    // Dispatch and finish
    let batch = collector.process(msg("chat1", "hello")).await;
    assert!(batch.is_some());
    collector.mark_done("chat1").await;
    // Now in Cooldown; next message should be dropped regardless of max_agents
    let dropped = collector.process(msg("chat1", "during cooldown")).await;
    assert!(dropped.is_none(), "messages during Cooldown should be dropped");
}

#[tokio::test]
async fn cooldown_drops_messages_with_capacity() {
    // max_agents=10 gives plenty of capacity; drop must be due to Cooldown state
    let collector = ChatCollector::new(0, 10, 500);
    let batch = collector.process(msg("chat1", "hello")).await;
    assert!(batch.is_some());
    collector.mark_done("chat1").await;
    let dropped = collector.process(msg("chat1", "during cooldown")).await;
    assert!(dropped.is_none(), "Cooldown drop is independent of max_agents");
}

#[tokio::test]
async fn window_nonzero_enters_collecting_state() {
    let collector = ChatCollector::new(500, 1, 0);
    let batch = collector.process(msg("chat1", "hello")).await;
    assert!(batch.is_none(), "with window_ms>0 the first message should enter Collecting");
}

#[tokio::test]
async fn mark_done_without_cooldown_returns_to_idle() {
    let collector = ChatCollector::new(0, 1, 0);
    let batch = collector.process(msg("chat1", "hello")).await;
    assert!(batch.is_some());
    collector.mark_done("chat1").await;
    assert_eq!(collector.active_count().await, 0);
    // Can dispatch again
    let second = collector.process(msg("chat1", "again")).await;
    assert!(second.is_some(), "after mark_done with no cooldown, should dispatch again");
}
