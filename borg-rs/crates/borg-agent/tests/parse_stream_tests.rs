use borg_agent::event::parse_stream;

// Result event with non-empty result and session_id fields.
#[test]
fn test_well_formed_result_event() {
    let ndjson = r#"{"type":"system","subtype":"init","session_id":"sess-abc"}
{"type":"result","subtype":"success","result":"Final answer.","session_id":"sess-xyz","is_error":false,"cost_usd":0.001,"duration_ms":1200,"num_turns":1}"#;

    let (text, session_id) = parse_stream(ndjson);
    assert_eq!(text, "Final answer.");
    assert_eq!(session_id.as_deref(), Some("sess-xyz"));
}

// When the Result event has a null result, fall back to accumulated assistant text.
#[test]
fn test_fallback_when_result_is_null() {
    let ndjson = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Computed via tool."}]}}
{"type":"result","subtype":"success","result":null,"session_id":"sess-fallback"}"#;

    let (text, session_id) = parse_stream(ndjson);
    assert_eq!(text, "Computed via tool.");
    assert_eq!(session_id.as_deref(), Some("sess-fallback"));
}

// When the Result event has an empty string result, fall back to accumulated assistant text.
#[test]
fn test_fallback_when_result_is_empty_string() {
    let ndjson = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Assistant output."}]}}
{"type":"result","subtype":"success","result":"","session_id":"sess-empty"}"#;

    let (text, session_id) = parse_stream(ndjson);
    assert_eq!(text, "Assistant output.");
    assert_eq!(session_id.as_deref(), Some("sess-empty"));
}

// Multiple assistant content blocks are concatenated in order, separated by newlines.
#[test]
fn test_multiple_assistant_blocks_concatenated() {
    let ndjson = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"First block."},{"type":"text","text":"Second block."}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Third block."}]}}
{"type":"result","subtype":"success","result":null,"session_id":"sess-multi"}"#;

    let (text, _) = parse_stream(ndjson);
    // Blocks within one event are joined with newline; so are separate assistant events.
    assert!(text.contains("First block."), "got: {text}");
    assert!(text.contains("Second block."), "got: {text}");
    assert!(text.contains("Third block."), "got: {text}");
    // Order is preserved.
    let first = text.find("First block.").unwrap();
    let second = text.find("Second block.").unwrap();
    let third = text.find("Third block.").unwrap();
    assert!(first < second && second < third);
}

// Unknown event types must not panic and are simply skipped.
#[test]
fn test_unknown_event_types_skipped() {
    let ndjson = r#"{"type":"totally_unknown_event","foo":"bar"}
{"type":"result","subtype":"success","result":"Done.","session_id":"sess-unk"}"#;

    let (text, session_id) = parse_stream(ndjson);
    assert_eq!(text, "Done.");
    assert_eq!(session_id.as_deref(), Some("sess-unk"));
}

// Malformed / non-JSON lines are ignored without panicking.
#[test]
fn test_malformed_lines_ignored() {
    let ndjson = "not json at all\n{\"type\":\"result\",\"result\":\"OK.\",\"session_id\":\"sess-bad\"}";

    let (text, session_id) = parse_stream(ndjson);
    assert_eq!(text, "OK.");
    assert_eq!(session_id.as_deref(), Some("sess-bad"));
}

// Empty input returns empty text and no session_id.
#[test]
fn test_empty_input() {
    let (text, session_id) = parse_stream("");
    assert!(text.is_empty());
    assert!(session_id.is_none());
}

// Blank lines between events are skipped without panicking.
#[test]
fn test_blank_lines_skipped() {
    let ndjson = "\n\n{\"type\":\"result\",\"result\":\"Hello.\",\"session_id\":\"sess-blank\"}\n\n";

    let (text, session_id) = parse_stream(ndjson);
    assert_eq!(text, "Hello.");
    assert_eq!(session_id.as_deref(), Some("sess-blank"));
}

// ToolUse blocks inside assistant events do not contribute to assistant text.
#[test]
fn test_tool_use_blocks_not_included() {
    let ndjson = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"bash","input":{"command":"ls"}}]}}
{"type":"result","subtype":"success","result":null,"session_id":"sess-tool"}"#;

    let (text, _) = parse_stream(ndjson);
    // No text blocks, so output should be empty (fallback also empty).
    assert!(text.is_empty(), "got: {text}");
}

// Session ID is taken from the Result event if present, overriding the System event value.
#[test]
fn test_result_session_id_overrides_system() {
    let ndjson = r#"{"type":"system","subtype":"init","session_id":"sys-sid"}
{"type":"result","subtype":"success","result":"Output.","session_id":"res-sid"}"#;

    let (_, session_id) = parse_stream(ndjson);
    assert_eq!(session_id.as_deref(), Some("res-sid"));
}
