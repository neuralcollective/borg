use borg_agent::event::parse_stream;

// =============================================================================
// Session ID: System event only
// =============================================================================

#[test]
fn test_session_id_from_system_event() {
    let ndjson = r#"{"type":"system","session_id":"sys-abc","subtype":"init"}
{"type":"result","result":"done","cost_usd":0.01}"#;
    let (_, session_id) = parse_stream(ndjson);
    assert_eq!(session_id.as_deref(), Some("sys-abc"));
}

#[test]
fn test_session_id_none_when_absent() {
    let ndjson = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}
{"type":"result","result":"done"}"#;
    let (_, session_id) = parse_stream(ndjson);
    assert!(session_id.is_none());
}

// =============================================================================
// Session ID: Result event overrides System event
// =============================================================================

#[test]
fn test_result_session_id_overrides_system() {
    let ndjson = r#"{"type":"system","session_id":"sys-1","subtype":"init"}
{"type":"assistant","message":{"content":[{"type":"text","text":"thinking"}]}}
{"type":"result","result":"output","session_id":"res-2"}"#;
    let (_, session_id) = parse_stream(ndjson);
    assert_eq!(session_id.as_deref(), Some("res-2"));
}

#[test]
fn test_system_session_id_kept_when_result_has_none() {
    let ndjson = r#"{"type":"system","session_id":"sys-only","subtype":"init"}
{"type":"result","result":"output"}"#;
    let (_, session_id) = parse_stream(ndjson);
    assert_eq!(session_id.as_deref(), Some("sys-only"));
}

// =============================================================================
// Output: taken from Result event
// =============================================================================

#[test]
fn test_output_from_result_event() {
    let ndjson = r#"{"type":"system","session_id":"s1","subtype":"init"}
{"type":"assistant","message":{"content":[{"type":"text","text":"intermediate"}]}}
{"type":"result","result":"Final answer here.","session_id":"s1"}"#;
    let (output, _) = parse_stream(ndjson);
    assert_eq!(output, "Final answer here.");
}

// =============================================================================
// Output: fallback to assistant text when Result has no text
// =============================================================================

#[test]
fn test_fallback_to_assistant_text_when_result_empty() {
    let ndjson = r#"{"type":"system","session_id":"s1","subtype":"init"}
{"type":"assistant","message":{"content":[{"type":"text","text":"Assistant said this."}]}}
{"type":"result","result":"","session_id":"s1"}"#;
    let (output, _) = parse_stream(ndjson);
    assert_eq!(output, "Assistant said this.");
}

#[test]
fn test_fallback_to_assistant_text_when_result_absent() {
    let ndjson = r#"{"type":"system","session_id":"s1","subtype":"init"}
{"type":"assistant","message":{"content":[{"type":"text","text":"Assistant said this."}]}}
{"type":"result","session_id":"s1"}"#;
    let (output, _) = parse_stream(ndjson);
    assert_eq!(output, "Assistant said this.");
}

#[test]
fn test_fallback_concatenates_multiple_assistant_blocks() {
    let ndjson = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Part one."}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Part two."}]}}
{"type":"result"}"#;
    let (output, _) = parse_stream(ndjson);
    assert!(output.contains("Part one."), "got: {output}");
    assert!(output.contains("Part two."), "got: {output}");
}

// =============================================================================
// Unknown event types silently skipped
// =============================================================================

#[test]
fn test_unknown_event_type_skipped() {
    let ndjson = r#"{"type":"system","session_id":"s1","subtype":"init"}
{"type":"debug","payload":"some debug info"}
{"type":"result","result":"ok","session_id":"s1"}"#;
    let (output, session_id) = parse_stream(ndjson);
    assert_eq!(output, "ok");
    assert_eq!(session_id.as_deref(), Some("s1"));
}

// =============================================================================
// Empty and invalid NDJSON lines ignored
// =============================================================================

#[test]
fn test_empty_lines_ignored() {
    let ndjson = "\n\n{\"type\":\"result\",\"result\":\"clean\",\"session_id\":\"s1\"}\n\n";
    let (output, session_id) = parse_stream(ndjson);
    assert_eq!(output, "clean");
    assert_eq!(session_id.as_deref(), Some("s1"));
}

#[test]
fn test_invalid_json_lines_ignored() {
    let ndjson = r#"{"type":"system","session_id":"s1","subtype":"init"}
not-valid-json
{broken
{"type":"result","result":"still works","session_id":"s1"}"#;
    let (output, session_id) = parse_stream(ndjson);
    assert_eq!(output, "still works");
    assert_eq!(session_id.as_deref(), Some("s1"));
}

#[test]
fn test_entirely_empty_input() {
    let (output, session_id) = parse_stream("");
    assert!(output.is_empty());
    assert!(session_id.is_none());
}

#[test]
fn test_only_blank_lines() {
    let (output, session_id) = parse_stream("\n\n\n");
    assert!(output.is_empty());
    assert!(session_id.is_none());
}
