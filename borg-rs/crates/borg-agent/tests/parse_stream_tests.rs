use borg_agent::event::parse_stream;

#[test]
fn test_assistant_event_with_text_returns_text_no_session_id() {
    let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hello, world!"}]}}"#;
    let (text, session_id) = parse_stream(line);
    assert_eq!(text, "Hello, world!");
    assert!(session_id.is_none());
}

#[test]
fn test_system_event_with_session_id_returns_empty_text_and_session_id() {
    let line = r#"{"type":"system","subtype":"init","session_id":"abc-123"}"#;
    let (text, session_id) = parse_stream(line);
    assert_eq!(text, "");
    assert_eq!(session_id.as_deref(), Some("abc-123"));
}

#[test]
fn test_unknown_event_type_returns_empty_and_no_session_id() {
    let line = r#"{"type":"totally_unknown_event","data":"something"}"#;
    let (text, session_id) = parse_stream(line);
    assert_eq!(text, "");
    assert!(session_id.is_none());
}

#[test]
fn test_malformed_json_returns_empty_without_panic() {
    let (text, session_id) = parse_stream("{not valid json at all!!!");
    assert_eq!(text, "");
    assert!(session_id.is_none());
}

#[test]
fn test_empty_string_returns_empty_and_no_session_id() {
    let (text, session_id) = parse_stream("");
    assert_eq!(text, "");
    assert!(session_id.is_none());
}
