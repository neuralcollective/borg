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

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // AgentEvent deserialization — known variants
    // -------------------------------------------------------------------------

    #[test]
    fn system_event_full() {
        let json = r#"{"type":"system","subtype":"init","session_id":"sess-1"}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        let AgentEvent::System(e) = event else { panic!("expected System") };
        assert_eq!(e.subtype.as_deref(), Some("init"));
        assert_eq!(e.session_id.as_deref(), Some("sess-1"));
    }

    #[test]
    fn system_event_optional_fields_absent() {
        let json = r#"{"type":"system"}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        let AgentEvent::System(e) = event else { panic!("expected System") };
        assert!(e.subtype.is_none());
        assert!(e.session_id.is_none());
    }

    #[test]
    fn assistant_event_with_text_block() {
        let json = r#"{
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "Hello world"}],
                "model": "claude-3",
                "stop_reason": "end_turn"
            }
        }"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        let AgentEvent::Assistant(e) = event else { panic!("expected Assistant") };
        let msg = e.message.unwrap();
        assert_eq!(msg.role.as_deref(), Some("assistant"));
        assert_eq!(msg.model.as_deref(), Some("claude-3"));
        let blocks = msg.content.unwrap();
        assert_eq!(blocks.len(), 1);
        let ContentBlock::Text { text } = &blocks[0] else { panic!("expected Text block") };
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn assistant_event_with_tool_use_block() {
        let json = r#"{
            "type": "assistant",
            "message": {
                "content": [{"type": "tool_use", "id": "call-1", "name": "bash", "input": {"cmd": "ls"}}]
            }
        }"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        let AgentEvent::Assistant(e) = event else { panic!("expected Assistant") };
        let blocks = e.message.unwrap().content.unwrap();
        let ContentBlock::ToolUse { id, name, input } = &blocks[0] else {
            panic!("expected ToolUse block")
        };
        assert_eq!(id, "call-1");
        assert_eq!(name, "bash");
        assert_eq!(input["cmd"], "ls");
    }

    #[test]
    fn assistant_event_no_message_field() {
        let json = r#"{"type":"assistant"}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        let AgentEvent::Assistant(e) = event else { panic!("expected Assistant") };
        assert!(e.message.is_none());
    }

    #[test]
    fn user_event_with_tool_result_block() {
        let json = r#"{
            "type": "user",
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "call-1",
                    "content": "output text",
                    "is_error": false
                }]
            }
        }"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        let AgentEvent::User(e) = event else { panic!("expected User") };
        let msg = e.message.unwrap();
        assert_eq!(msg.role.as_deref(), Some("user"));
        let blocks = msg.content.unwrap();
        let ContentBlock::ToolResult { tool_use_id, content, is_error } = &blocks[0] else {
            panic!("expected ToolResult block")
        };
        assert_eq!(tool_use_id, "call-1");
        assert!(content.is_some());
        assert_eq!(*is_error, Some(false));
    }

    #[test]
    fn user_event_no_message_field() {
        let json = r#"{"type":"user"}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        let AgentEvent::User(e) = event else { panic!("expected User") };
        assert!(e.message.is_none());
    }

    #[test]
    fn result_event_full() {
        let json = r#"{
            "type": "result",
            "subtype": "success",
            "result": "Done.",
            "session_id": "sess-2",
            "is_error": false,
            "cost_usd": 0.002,
            "duration_ms": 1500,
            "num_turns": 3,
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_read_input_tokens": 10,
                "cache_creation_input_tokens": 5
            }
        }"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        let AgentEvent::Result(e) = event else { panic!("expected Result") };
        assert_eq!(e.subtype.as_deref(), Some("success"));
        assert_eq!(e.result.as_deref(), Some("Done."));
        assert_eq!(e.session_id.as_deref(), Some("sess-2"));
        assert_eq!(e.is_error, Some(false));
        assert!((e.cost_usd.unwrap() - 0.002).abs() < 1e-9);
        assert_eq!(e.duration_ms, Some(1500));
        assert_eq!(e.num_turns, Some(3));
        let usage = e.usage.unwrap();
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(50));
        assert_eq!(usage.cache_read_input_tokens, Some(10));
        assert_eq!(usage.cache_creation_input_tokens, Some(5));
    }

    #[test]
    fn result_event_optional_fields_absent() {
        let json = r#"{"type":"result"}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        let AgentEvent::Result(e) = event else { panic!("expected Result") };
        assert!(e.result.is_none());
        assert!(e.session_id.is_none());
        assert!(e.is_error.is_none());
        assert!(e.cost_usd.is_none());
        assert!(e.usage.is_none());
    }

    // -------------------------------------------------------------------------
    // Unknown variant fallback
    // -------------------------------------------------------------------------

    #[test]
    fn unknown_variant_for_unrecognized_type() {
        let json = r#"{"type":"debug","payload":"ignored"}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, AgentEvent::Unknown));
    }

    #[test]
    fn unknown_variant_for_empty_type() {
        let json = r#"{"type":"","data":42}"#;
        let event: AgentEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, AgentEvent::Unknown));
    }

    // -------------------------------------------------------------------------
    // Malformed JSON
    // -------------------------------------------------------------------------

    #[test]
    fn malformed_json_returns_error() {
        assert!(serde_json::from_str::<AgentEvent>("not json").is_err());
    }

    #[test]
    fn truncated_json_returns_error() {
        assert!(serde_json::from_str::<AgentEvent>(r#"{"type":"system""#).is_err());
    }

    #[test]
    fn missing_type_field_returns_error() {
        // Tagged enums require the tag field to be present
        assert!(serde_json::from_str::<AgentEvent>(r#"{"session_id":"x"}"#).is_err());
    }

    // -------------------------------------------------------------------------
    // ContentBlock — unknown block type fallback
    // -------------------------------------------------------------------------

    #[test]
    fn content_block_unknown_fallback() {
        let json = r#"{"type":"image","source":{"url":"http://example.com"}}"#;
        let block: ContentBlock = serde_json::from_str(json).unwrap();
        assert!(matches!(block, ContentBlock::Unknown));
    }

    // -------------------------------------------------------------------------
    // parse_stream end-to-end
    // -------------------------------------------------------------------------

    #[test]
    fn parse_stream_extracts_result_and_session() {
        let ndjson = concat!(
            "{\"type\":\"system\",\"session_id\":\"s1\"}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"thinking\"}]}}\n",
            "{\"type\":\"result\",\"result\":\"Final answer.\",\"session_id\":\"s2\"}\n",
        );
        let (output, session_id) = parse_stream(ndjson);
        assert_eq!(output, "Final answer.");
        assert_eq!(session_id.as_deref(), Some("s2"));
    }

    #[test]
    fn parse_stream_falls_back_to_assistant_text_when_result_empty() {
        let ndjson = concat!(
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"line one\"}]}}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"line two\"}]}}\n",
            "{\"type\":\"result\"}\n",
        );
        let (output, _) = parse_stream(ndjson);
        assert!(output.contains("line one"), "got: {output}");
        assert!(output.contains("line two"), "got: {output}");
    }

    #[test]
    fn parse_stream_skips_malformed_lines() {
        let ndjson = concat!(
            "not valid json\n",
            "{\"type\":\"result\",\"result\":\"ok\",\"session_id\":\"s3\"}\n",
        );
        let (output, session_id) = parse_stream(ndjson);
        assert_eq!(output, "ok");
        assert_eq!(session_id.as_deref(), Some("s3"));
    }

    #[test]
    fn parse_stream_empty_input() {
        let (output, session_id) = parse_stream("");
        assert!(output.is_empty());
        assert!(session_id.is_none());
    }

    #[test]
    fn parse_stream_session_id_prefers_result_over_system() {
        let ndjson = concat!(
            "{\"type\":\"system\",\"session_id\":\"from-system\"}\n",
            "{\"type\":\"result\",\"result\":\"done\",\"session_id\":\"from-result\"}\n",
        );
        let (_, session_id) = parse_stream(ndjson);
        assert_eq!(session_id.as_deref(), Some("from-result"));
    }
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
            },
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
            },
            AgentEvent::Result(e) => {
                if let Some(sid) = e.session_id {
                    session_id = Some(sid);
                }
                if let Some(text) = e.result {
                    output = text;
                }
            },
            _ => {},
        }
    }

    // Fall back to collected assistant text if result was empty
    if output.is_empty() && !assistant_text.is_empty() {
        output = assistant_text;
    }

    (output, session_id)
}
