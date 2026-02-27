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

/// Manages per-chat collection windows.
pub struct ChatCollector {
    /// Per-chat state. Key = chat_key.
    state: Arc<Mutex<HashMap<String, ChatState>>>,
    /// Collection window duration. 0 = immediate dispatch.
    window_ms: u64,
    /// Post-agent cooldown duration. 0 = no cooldown.
    cooldown_ms: u64,
    /// Max agents running concurrently.
    max_agents: u32,
    /// Current running agent count.
    running: Arc<std::sync::atomic::AtomicU32>,
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
            state: Arc::new(Mutex::new(HashMap::new())),
            window_ms,
            cooldown_ms,
            max_agents,
            running: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        }
    }

    /// Process an incoming message. Returns Some(batch) if ready to dispatch.
    pub async fn process(&self, msg: IncomingMessage) -> Option<MessageBatch> {
        let mut state = self.state.lock().await;
        let chat_key = msg.chat_key.clone();

        let current = state.get(&chat_key).cloned().unwrap_or(ChatState::Idle);

        match current {
            ChatState::Running => {
                // Agent already running — drop message
                debug!("Chat {} has running agent, dropping message", chat_key);
                None
            },

            ChatState::Cooldown { .. } => {
                debug!("Chat {} in cooldown, dropping message", chat_key);
                None
            },

            ChatState::Idle => {
                if self.window_ms == 0 {
                    // Immediate dispatch
                    state.insert(chat_key.clone(), ChatState::Running);
                    Some(MessageBatch {
                        chat_key,
                        messages: vec![msg.text],
                    })
                } else {
                    let deadline = Instant::now() + Duration::from_millis(self.window_ms);
                    state.insert(
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
                    state.insert(chat_key.clone(), ChatState::Running);
                    Some(MessageBatch { chat_key, messages })
                } else {
                    state.insert(
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
        let mut state = self.state.lock().await;
        let now = Instant::now();
        let mut ready = Vec::new();

        for (chat_key, chat_state) in state.iter_mut() {
            match chat_state {
                ChatState::Collecting {
                    window_deadline,
                    messages,
                } => {
                    if now >= *window_deadline {
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

        ready
    }

    /// Mark a chat as done (agent finished). Enters Cooldown if configured, else Idle.
    pub async fn mark_done(&self, chat_key: &str) {
        let mut state = self.state.lock().await;
        if self.cooldown_ms > 0 {
            let deadline = Instant::now() + Duration::from_millis(self.cooldown_ms);
            state.insert(chat_key.to_string(), ChatState::Cooldown { deadline });
            debug!(
                "Chat {} entering cooldown ({}ms)",
                chat_key, self.cooldown_ms
            );
        } else {
            state.insert(chat_key.to_string(), ChatState::Idle);
            debug!("Chat {} returned to IDLE", chat_key);
        }
        self.running
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Check if we can dispatch more agents.
    pub fn can_dispatch(&self) -> bool {
        self.running.load(std::sync::atomic::Ordering::Relaxed) < self.max_agents
    }

    /// Mark dispatch started.
    pub fn mark_dispatched(&self) {
        self.running
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn active_count(&self) -> u32 {
        self.running.load(std::sync::atomic::Ordering::Relaxed)
    }
}
