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
        Some(MessageBatch { chat_key, messages })
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
                    self.try_dispatch(&mut inner, chat_key, vec![msg.text])
                } else {
                    let deadline = Instant::now() + Duration::from_millis(self.window_ms);
                    inner.chats.insert(
                        chat_key,
                        ChatState::Collecting {
                            window_deadline: deadline,
                            messages: vec![msg.text],
                        },
                    );
                    None
                }
            },

            ChatState::Collecting {
                window_deadline,
                mut messages,
            } => {
                messages.push(msg.text);

                if Instant::now() >= window_deadline {
                    self.try_dispatch(&mut inner, chat_key, messages)
                } else {
                    inner.chats.insert(
                        chat_key,
                        ChatState::Collecting {
                            window_deadline,
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

        let can_dispatch = self.can_dispatch_inner(&inner);

        for (chat_key, chat_state) in inner.chats.iter_mut() {
            match chat_state {
                ChatState::Collecting {
                    window_deadline,
                    messages,
                } => {
                    if now >= *window_deadline && can_dispatch {
                        let batch = MessageBatch {
                            chat_key: chat_key.clone(),
                            messages: std::mem::take(messages),
                        };
                        *chat_state = ChatState::Running;
                        ready.push(batch);
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

        inner.running += ready.len() as u32;

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
