use borg_agent::event::parse_stream;

// Empty input → empty output and no session_id
#[test]
fn test_empty_input() {
    let (output, session_id) = parse_stream("");
    assert_eq!(output, "");
    assert!(session_id.is_none());
}

// Whitespace-only input is also empty
#[test]
fn test_whitespace_only_input() {
    let (output, session_id) = parse_stream("   \n\n  ");
    assert_eq!(output, "");
    assert!(session_id.is_none());
}

// Result event with non-empty result field → used as output
#[test]
fn test_result_event_non_empty() {
    let data = r#"{"type":"result","result":"Done successfully.","session_id":null}"#;
    let (output, _) = parse_stream(data);
    assert_eq!(output, "Done successfully.");
}

// Result event with empty result field → falls back to assistant text
#[test]
fn test_empty_result_falls_back_to_assistant_text() {
    let data = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Assistant says hello."}]}}"#,
        "\n",
        r#"{"type":"result","result":"","session_id":null}"#,
    );
    let (output, _) = parse_stream(data);
    assert_eq!(output, "Assistant says hello.");
}

// Result event with null result field → falls back to assistant text
#[test]
fn test_null_result_falls_back_to_assistant_text() {
    let data = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Fallback content."}]}}"#,
        "\n",
        r#"{"type":"result","result":null}"#,
    );
    let (output, _) = parse_stream(data);
    assert_eq!(output, "Fallback content.");
}

// No result event at all → falls back to assistant text
#[test]
fn test_no_result_event_uses_assistant_text() {
    let data = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Only assistant."}]}}"#;
    let (output, _) = parse_stream(data);
    assert_eq!(output, "Only assistant.");
}

// Multiple assistant text blocks are joined with newlines
#[test]
fn test_multiple_assistant_text_blocks_joined_with_newlines() {
    let data = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"First block."},{"type":"text","text":"Second block."}]}}"#,
        "\n",
        r#"{"type":"result","result":""}"#,
    );
    let (output, _) = parse_stream(data);
    assert_eq!(output, "First block.\nSecond block.");
}

// Multiple assistant events — text blocks accumulated across events
#[test]
fn test_multiple_assistant_events_accumulated() {
    let data = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Part one."}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Part two."}]}}"#,
        "\n",
        r#"{"type":"result","result":""}"#,
    );
    let (output, _) = parse_stream(data);
    assert_eq!(output, "Part one.\nPart two.");
}

// session_id captured from System event
#[test]
fn test_session_id_from_system_event() {
    let data = r#"{"type":"system","session_id":"abc-123","subtype":"init"}"#;
    let (_, session_id) = parse_stream(data);
    assert_eq!(session_id.as_deref(), Some("abc-123"));
}

// session_id from Result event is also captured
#[test]
fn test_session_id_from_result_event() {
    let data = r#"{"type":"result","result":"ok","session_id":"sess-999"}"#;
    let (_, session_id) = parse_stream(data);
    assert_eq!(session_id.as_deref(), Some("sess-999"));
}

// Result session_id overrides System session_id (last write wins)
#[test]
fn test_result_session_id_overrides_system() {
    let data = concat!(
        r#"{"type":"system","session_id":"first-id"}"#,
        "\n",
        r#"{"type":"result","result":"done","session_id":"second-id"}"#,
    );
    let (_, session_id) = parse_stream(data);
    assert_eq!(session_id.as_deref(), Some("second-id"));
}

// Malformed NDJSON lines are silently skipped
#[test]
fn test_malformed_lines_skipped() {
    let data = concat!(
        "not json at all\n",
        r#"{"type":"system","session_id":"good-id"}"#,
        "\n",
        "{broken\n",
        r#"{"type":"result","result":"output"}"#,
    );
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "output");
    assert_eq!(session_id.as_deref(), Some("good-id"));
}

// Empty lines in the stream are silently skipped
#[test]
fn test_empty_lines_skipped() {
    let data = concat!(
        "\n",
        "\n",
        r#"{"type":"result","result":"clean"}"#,
        "\n",
        "\n",
    );
    let (output, _) = parse_stream(data);
    assert_eq!(output, "clean");
}

// Non-text content blocks (tool_use) are ignored
#[test]
fn test_tool_use_blocks_ignored_for_assistant_text() {
    let data = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}"#,
        "\n",
        r#"{"type":"result","result":""}"#,
    );
    let (output, _) = parse_stream(data);
    assert_eq!(output, "");
}
