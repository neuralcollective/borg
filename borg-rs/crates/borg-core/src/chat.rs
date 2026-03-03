use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::Mutex;
use tracing::debug;

/// State of a single chat's collection window.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatState {
    Idle,
    /// Collecting messages; window expires at this instant.
    Collecting {
        window_deadline: Instant,
        sender_name: String,
        messages: Vec<String>,
    },
    /// Agent is running for this chat.
    Running,
    /// Post-agent cooldown; no new messages dispatched until deadline.
    Cooldown {
        deadline: Instant,
    },
}

/// An incoming message from any transport.
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    /// Unique key for the chat (e.g. telegram:123456, discord:channel_id).
    pub chat_key: String,
    pub sender_name: String,
    pub text: String,
    pub timestamp: i64,
    pub reply_to_message_id: Option<String>,
}

/// All mutable state behind a single lock to avoid split-state races.
struct CollectorInner {
    chats: HashMap<String, ChatState>,
    running: u32,
}

/// Manages per-chat collection windows.
pub struct ChatCollector {
    state: Arc<Mutex<CollectorInner>>,
    /// Collection window duration. 0 = immediate dispatch.
    window_ms: u64,
    /// Post-agent cooldown duration. 0 = no cooldown.
    cooldown_ms: u64,
    /// Max agents running concurrently.
    max_agents: u32,
}

/// A batch of messages ready to be dispatched to an agent.
#[derive(Debug)]
pub struct MessageBatch {
    pub chat_key: String,
    pub sender_name: String,
    pub messages: Vec<String>,
}

impl ChatCollector {
    pub fn new(window_ms: u64, max_agents: u32, cooldown_ms: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(CollectorInner {
                chats: HashMap::new(),
                running: 0,
            })),
            window_ms,
            cooldown_ms,
            max_agents,
        }
    }

    fn can_dispatch_inner(&self, inner: &CollectorInner) -> bool {
        self.max_agents == 0 || inner.running < self.max_agents
    }

    /// Transition a chat to Running and bump the running count atomically.
    /// Returns None if we're at the concurrency limit.
    fn try_dispatch(
        &self,
        inner: &mut CollectorInner,
        chat_key: String,
        sender_name: String,
        messages: Vec<String>,
    ) -> Option<MessageBatch> {
        if !self.can_dispatch_inner(inner) {
            debug!(
                "At max agents ({}/{}), deferring chat {}",
                inner.running, self.max_agents, chat_key
            );
            return None;
        }
        inner.chats.insert(chat_key.clone(), ChatState::Running);
        inner.running += 1;
        Some(MessageBatch { chat_key, sender_name, messages })
    }

    /// Process an incoming message. Returns Some(batch) if ready to dispatch.
    pub async fn process(&self, msg: IncomingMessage) -> Option<MessageBatch> {
        let mut inner = self.state.lock().await;
        let chat_key = msg.chat_key.clone();

        let current = inner.chats.get(&chat_key).cloned().unwrap_or(ChatState::Idle);

        match current {
            ChatState::Running => {
                debug!("Chat {} has running agent, dropping message", chat_key);
                None
            },

            ChatState::Cooldown { .. } => {
                debug!("Chat {} in cooldown, dropping message", chat_key);
                None
            },

            ChatState::Idle => {
                if self.window_ms == 0 {
                    self.try_dispatch(&mut inner, chat_key, msg.sender_name, vec![msg.text])
                } else {
                    let deadline = Instant::now() + Duration::from_millis(self.window_ms);
                    inner.chats.insert(
                        chat_key,
                        ChatState::Collecting {
                            window_deadline: deadline,
                            sender_name: msg.sender_name,
                            messages: vec![msg.text],
                        },
                    );
                    None
                }
            },

            ChatState::Collecting {
                window_deadline,
                sender_name,
                mut messages,
            } => {
                messages.push(msg.text);

                if Instant::now() >= window_deadline {
                    self.try_dispatch(&mut inner, chat_key, sender_name, messages)
                } else {
                    inner.chats.insert(
                        chat_key,
                        ChatState::Collecting {
                            window_deadline,
                            sender_name,
                            messages,
                        },
                    );
                    None
                }
            },
        }
    }

    /// Call periodically to flush expired collection windows and cooldowns.
    /// Returns all batches ready to dispatch.
    pub async fn flush_expired(&self) -> Vec<MessageBatch> {
        let mut inner = self.state.lock().await;
        let now = Instant::now();
        let mut ready = Vec::new();
        let mut running = inner.running;

        for (chat_key, chat_state) in inner.chats.iter_mut() {
            match chat_state {
                ChatState::Collecting {
                    window_deadline,
                    sender_name,
                    messages,
                } => {
                    let at_limit = self.max_agents > 0 && running >= self.max_agents;
                    if now >= *window_deadline && !at_limit {
                        let batch = MessageBatch {
                            chat_key: chat_key.clone(),
                            sender_name: std::mem::take(sender_name),
                            messages: std::mem::take(messages),
                        };
                        *chat_state = ChatState::Running;
                        ready.push(batch);
                        running += 1;
                    }
                },
                ChatState::Cooldown { deadline } => {
                    if now >= *deadline {
                        *chat_state = ChatState::Idle;
                        debug!("Chat {} cooldown expired", chat_key);
                    }
                },
                _ => {},
            }
        }

        inner.running = running;

        ready
    }

    /// Mark a chat as done (agent finished). Enters Cooldown if configured, else Idle.
    pub async fn mark_done(&self, chat_key: &str) {
        let mut inner = self.state.lock().await;
        if self.cooldown_ms > 0 {
            let deadline = Instant::now() + Duration::from_millis(self.cooldown_ms);
            inner
                .chats
                .insert(chat_key.to_string(), ChatState::Cooldown { deadline });
            debug!(
                "Chat {} entering cooldown ({}ms)",
                chat_key, self.cooldown_ms
            );
        } else {
            inner
                .chats
                .insert(chat_key.to_string(), ChatState::Idle);
            debug!("Chat {} returned to IDLE", chat_key);
        }
        inner.running = inner.running.saturating_sub(1);
    }

    /// Check if we can dispatch more agents. 0 = unlimited.
    pub async fn can_dispatch(&self) -> bool {
        let inner = self.state.lock().await;
        self.can_dispatch_inner(&inner)
    }

    pub async fn active_count(&self) -> u32 {
        let inner = self.state.lock().await;
        inner.running
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    // flush_expired: skip chats whose window has not expired
    #[tokio::test]
    async fn flush_expired_skips_non_expired_window() {
        let c = ChatCollector::new(10_000, 0, 0);
        assert!(c.process(msg("chat1", "hello")).await.is_none());

        let batches = c.flush_expired().await;
        assert!(batches.is_empty());
        assert_eq!(c.active_count().await, 0);
    }

    // flush_expired: transition expired Collecting chat to Running
    #[tokio::test]
    async fn flush_expired_transitions_expired_chat_to_running() {
        let c = ChatCollector::new(1, 0, 0);
        assert!(c.process(msg("chat1", "hello")).await.is_none());

        tokio::time::sleep(Duration::from_millis(5)).await;
        let batches = c.flush_expired().await;

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].chat_key, "chat1");
        assert_eq!(batches[0].sender_name, "alice");
        assert_eq!(batches[0].messages, vec!["hello"]);
        assert_eq!(c.active_count().await, 1);

        // Chat is now Running: new messages are dropped
        assert!(c.process(msg("chat1", "dropped")).await.is_none());
    }

    // flush_expired: all messages collected in the window are included in the batch
    #[tokio::test]
    async fn flush_expired_includes_all_collected_messages() {
        let c = ChatCollector::new(1, 0, 0);
        c.process(msg("chat1", "msg1")).await;
        c.process(msg("chat1", "msg2")).await;

        tokio::time::sleep(Duration::from_millis(5)).await;
        let batches = c.flush_expired().await;

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].messages, vec!["msg1", "msg2"]);
    }

    // flush_expired: respects max_agents concurrency limit
    #[tokio::test]
    async fn flush_expired_respects_max_agents_limit() {
        let c = ChatCollector::new(1, 1, 0); // max 1 concurrent agent
        c.process(msg("chat1", "hi")).await;
        c.process(msg("chat2", "yo")).await;

        tokio::time::sleep(Duration::from_millis(5)).await;
        let batches = c.flush_expired().await;

        assert_eq!(batches.len(), 1, "only one chat dispatched at max_agents=1");
        assert_eq!(c.active_count().await, 1);
    }

    // flush_expired: expires Cooldown state → Idle
    #[tokio::test]
    async fn flush_expired_transitions_expired_cooldown_to_idle() {
        let c = ChatCollector::new(0, 0, 1); // 1ms cooldown
        let batch = c.process(msg("chat1", "hi")).await;
        assert!(batch.is_some());

        c.mark_done("chat1").await;
        // Immediately in Cooldown: messages dropped
        assert!(c.process(msg("chat1", "dropped")).await.is_none());

        tokio::time::sleep(Duration::from_millis(5)).await;
        c.flush_expired().await;

        // Now Idle: message dispatches immediately (window_ms=0)
        assert!(c.process(msg("chat1", "new")).await.is_some());
    }

    // mark_done: enters Cooldown when cooldown_ms > 0
    #[tokio::test]
    async fn mark_done_enters_cooldown_when_configured() {
        let c = ChatCollector::new(0, 0, 60_000);
        assert!(c.process(msg("chat1", "hi")).await.is_some());
        assert_eq!(c.active_count().await, 1);

        c.mark_done("chat1").await;
        assert_eq!(c.active_count().await, 0);

        // Cooldown: new messages are dropped
        assert!(c.process(msg("chat1", "new")).await.is_none());
    }

    // mark_done: returns to Idle when cooldown_ms == 0
    #[tokio::test]
    async fn mark_done_returns_to_idle_when_no_cooldown() {
        let c = ChatCollector::new(0, 0, 0);
        assert!(c.process(msg("chat1", "hi")).await.is_some());
        assert_eq!(c.active_count().await, 1);

        c.mark_done("chat1").await;
        assert_eq!(c.active_count().await, 0);

        // Idle: message dispatches immediately
        assert!(c.process(msg("chat1", "again")).await.is_some());
    }

    // mark_done: saturating_sub does not underflow when running == 0
    #[tokio::test]
    async fn mark_done_saturating_sub_prevents_underflow() {
        let c = ChatCollector::new(0, 0, 0);
        c.mark_done("nonexistent").await;
        assert_eq!(c.active_count().await, 0);
    }
}
