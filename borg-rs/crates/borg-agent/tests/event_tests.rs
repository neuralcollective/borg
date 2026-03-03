use borg_agent::event::parse_stream;

// =============================================================================
// Empty input
// =============================================================================

#[test]
fn test_empty_input() {
    let (output, session_id) = parse_stream("");
    assert!(output.is_empty());
    assert!(session_id.is_none());
}

#[test]
fn test_whitespace_only_input() {
    let (output, session_id) = parse_stream("   \n\n  \n");
    assert!(output.is_empty());
    assert!(session_id.is_none());
}

// =============================================================================
// Result-only stream
// =============================================================================

#[test]
fn test_result_only_stream() {
    let data = r#"{"type":"result","subtype":"success","result":"Task completed.","session_id":"sess-abc","is_error":false}"#;
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "Task completed.");
    assert_eq!(session_id.as_deref(), Some("sess-abc"));
}

#[test]
fn test_result_with_empty_result_field() {
    let data = r#"{"type":"result","subtype":"success","result":"","session_id":"sess-xyz"}"#;
    let (output, session_id) = parse_stream(data);
    assert!(output.is_empty());
    assert_eq!(session_id.as_deref(), Some("sess-xyz"));
}

// =============================================================================
// Fallback: assistant-text-only stream (no result or empty result)
// =============================================================================

#[test]
fn test_assistant_text_fallback() {
    let data = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Here is the answer."}]}}
{"type":"result","subtype":"success","result":""}"#;
    let (output, _) = parse_stream(data);
    assert_eq!(output, "Here is the answer.");
}

#[test]
fn test_assistant_text_fallback_no_result_event() {
    let data = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Fallback text."}]}}"#;
    let (output, _) = parse_stream(data);
    assert_eq!(output, "Fallback text.");
}

#[test]
fn test_result_takes_priority_over_assistant_text() {
    let data = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Assistant spoke."}]}}
{"type":"result","subtype":"success","result":"Final result."}"#;
    let (output, _) = parse_stream(data);
    assert_eq!(output, "Final result.");
}

// =============================================================================
// session_id from System vs Result event
// =============================================================================

#[test]
fn test_session_id_from_system_event() {
    let data = r#"{"type":"system","subtype":"init","session_id":"sys-session-1"}
{"type":"result","subtype":"success","result":"done"}"#;
    let (_, session_id) = parse_stream(data);
    assert_eq!(session_id.as_deref(), Some("sys-session-1"));
}

#[test]
fn test_session_id_from_result_event() {
    let data = r#"{"type":"result","subtype":"success","result":"done","session_id":"result-session-2"}"#;
    let (_, session_id) = parse_stream(data);
    assert_eq!(session_id.as_deref(), Some("result-session-2"));
}

#[test]
fn test_result_session_id_overrides_system() {
    // Result event appears after System, so its session_id should win.
    let data = r#"{"type":"system","subtype":"init","session_id":"sys-1"}
{"type":"result","subtype":"success","result":"done","session_id":"result-1"}"#;
    let (_, session_id) = parse_stream(data);
    assert_eq!(session_id.as_deref(), Some("result-1"));
}

#[test]
fn test_system_session_id_used_when_result_has_none() {
    let data = r#"{"type":"system","subtype":"init","session_id":"sys-only"}
{"type":"result","subtype":"success","result":"done"}"#;
    let (_, session_id) = parse_stream(data);
    assert_eq!(session_id.as_deref(), Some("sys-only"));
}

// =============================================================================
// Multiple assistant text blocks concatenated with newline
// =============================================================================

#[test]
fn test_multiple_assistant_text_blocks_joined() {
    let data = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"First block."},{"type":"text","text":"Second block."}]}}
{"type":"result","subtype":"success","result":""}"#;
    let (output, _) = parse_stream(data);
    assert!(output.contains("First block."), "got: {output}");
    assert!(output.contains("Second block."), "got: {output}");
    // Blocks are joined with a newline
    assert!(output.contains('\n'), "expected newline between blocks, got: {output}");
}

#[test]
fn test_multiple_assistant_events_concatenated() {
    let data = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Turn one."}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Turn two."}]}}
{"type":"result","subtype":"success","result":""}"#;
    let (output, _) = parse_stream(data);
    assert!(output.contains("Turn one."), "got: {output}");
    assert!(output.contains("Turn two."), "got: {output}");
    // Separated by newline
    assert!(output.contains('\n'), "expected newline separator, got: {output}");
}

// =============================================================================
// Malformed / non-JSON lines are skipped
// =============================================================================

#[test]
fn test_malformed_lines_skipped() {
    let data = r#"not json at all
{"type":"result","subtype":"success","result":"Valid result.","session_id":"s1"}
also not json"#;
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "Valid result.");
    assert_eq!(session_id.as_deref(), Some("s1"));
}

#[test]
fn test_partial_json_skipped() {
    let data = r#"{"type":"result"
{"type":"result","subtype":"success","result":"After partial.","session_id":"s2"}"#;
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "After partial.");
    assert_eq!(session_id.as_deref(), Some("s2"));
}

#[test]
fn test_all_malformed_returns_empty() {
    let data = "garbage\n{broken json}\n[not an object]";
    let (output, session_id) = parse_stream(data);
    assert!(output.is_empty());
    assert!(session_id.is_none());
}

#[test]
fn test_unknown_event_types_skipped() {
    let data = r#"{"type":"tool_call","data":"something"}
{"type":"result","subtype":"success","result":"OK","session_id":"s3"}"#;
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "OK");
    assert_eq!(session_id.as_deref(), Some("s3"));
}
