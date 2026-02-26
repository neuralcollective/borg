use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A single NDJSON message emitted by Claude Code (`--output-format stream-json`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// First message on stream: session initialisation.
    System(SystemEvent),

    /// An assistant turn (text or tool calls).
    Assistant(AssistantEvent),

    /// A user turn (tool results injected back into the conversation).
    User(UserEvent),

    /// Final result message — emitted once at the very end.
    Result(ResultEvent),

    /// Any message type not explicitly handled above.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SystemEvent {
    pub subtype: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssistantEvent {
    pub message: Option<AssistantMessage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssistantMessage {
    pub role: Option<String>,
    pub content: Option<Vec<ContentBlock>>,
    pub model: Option<String>,
    pub stop_reason: Option<String>,
    pub usage: Option<Usage>,
}

/// A single content block inside an assistant or user message.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text output.
    Text { text: String },

    /// A tool invocation by the agent.
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },

    /// Result returned by a tool (appears in user turn).
    ToolResult {
        tool_use_id: String,
        content: Option<Value>,
        is_error: Option<bool>,
    },

    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserEvent {
    pub message: Option<UserMessage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserMessage {
    pub role: Option<String>,
    pub content: Option<Vec<ContentBlock>>,
}

/// Final result event, emitted once when the agent finishes.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResultEvent {
    pub subtype: Option<String>,
    /// Textual output (may be empty if last turn was a tool call).
    pub result: Option<String>,
    pub session_id: Option<String>,
    pub is_error: Option<bool>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub num_turns: Option<u64>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
}

/// Parse a full NDJSON stream and extract the final output text and session ID.
pub fn parse_stream(data: &str) -> (String, Option<String>) {
    let mut output = String::new();
    let mut assistant_text = String::new();
    let mut session_id: Option<String> = None;

    for line in data.lines() {
        if line.is_empty() {
            continue;
        }
        let event: AgentEvent = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(_) => continue,
        };
        match event {
            AgentEvent::System(e) => {
                if let Some(sid) = e.session_id {
                    session_id = Some(sid);
                }
            }
            AgentEvent::Assistant(e) => {
                if let Some(msg) = e.message {
                    if let Some(blocks) = msg.content {
                        for block in blocks {
                            if let ContentBlock::Text { text } = block {
                                if !assistant_text.is_empty() {
                                    assistant_text.push('\n');
                                }
                                assistant_text.push_str(&text);
                            }
                        }
                    }
                }
            }
            AgentEvent::Result(e) => {
                if let Some(sid) = e.session_id {
                    session_id = Some(sid);
                }
                if let Some(text) = e.result {
                    output = text;
                }
            }
            _ => {}
        }
    }

    // Fall back to collected assistant text if result was empty
    if output.is_empty() && !assistant_text.is_empty() {
        output = assistant_text;
    }

    (output, session_id)
}
