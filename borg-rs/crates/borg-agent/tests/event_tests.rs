use borg_agent::event::parse_stream;

#[test]
fn test_empty_string_returns_empty_output_and_no_session_id() {
    let (output, session_id) = parse_stream("");
    assert!(output.is_empty());
    assert!(session_id.is_none());
}

#[test]
fn test_malformed_json_lines_are_silently_skipped() {
    let data = "not json at all\n{bad json\n{\"type\":\"result\",\"result\":\"ok\",\"session_id\":\"s1\"}";
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "ok");
    assert_eq!(session_id.as_deref(), Some("s1"));
}

#[test]
fn test_result_event_sets_output_and_session_id() {
    let data = r#"{"type":"result","result":"final answer","session_id":"abc123"}"#;
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "final answer");
    assert_eq!(session_id.as_deref(), Some("abc123"));
}

#[test]
fn test_empty_result_falls_back_to_assistant_text() {
    let data = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hello from assistant"}]}}"#,
        "\n",
        r#"{"type":"result","result":"","session_id":"s2"}"#,
    );
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "hello from assistant");
    assert_eq!(session_id.as_deref(), Some("s2"));
}

#[test]
fn test_multiple_assistant_events_joined_with_newline() {
    let data = concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"first block"}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"second block"}]}}"#,
        "\n",
        r#"{"type":"result","result":"","session_id":"s3"}"#,
    );
    let (output, _) = parse_stream(data);
    assert_eq!(output, "first block\nsecond block");
}

#[test]
fn test_system_session_id_overwritten_by_result() {
    let data = concat!(
        r#"{"type":"system","session_id":"system-sid"}"#,
        "\n",
        r#"{"type":"result","result":"done","session_id":"result-sid"}"#,
    );
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "done");
    assert_eq!(session_id.as_deref(), Some("result-sid"));
}

#[test]
fn test_system_session_id_captured_when_no_result_sid() {
    let data = concat!(
        r#"{"type":"system","session_id":"system-only"}"#,
        "\n",
        r#"{"type":"result","result":"text"}"#,
    );
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "text");
    assert_eq!(session_id.as_deref(), Some("system-only"));
}

#[test]
fn test_blank_lines_are_skipped() {
    let data = "\n\n{\"type\":\"result\",\"result\":\"val\",\"session_id\":\"s\"}\n\n";
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "val");
    assert_eq!(session_id.as_deref(), Some("s"));
}

#[test]
fn test_no_result_event_no_assistant_text_is_empty() {
    let data = r#"{"type":"system","session_id":"s4"}"#;
    let (output, session_id) = parse_stream(data);
    assert!(output.is_empty());
    assert_eq!(session_id.as_deref(), Some("s4"));
}
