use borg_agent::event::parse_stream;

fn system_line(session_id: &str) -> String {
    format!(r#"{{"type":"system","subtype":"init","session_id":"{session_id}"}}"#)
}

fn assistant_text_line(text: &str) -> String {
    let escaped = text.replace('"', r#"\""#).replace('\n', r#"\n"#);
    format!(
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"{escaped}"}}]}}}}"#
    )
}

fn result_line(result: Option<&str>, session_id: Option<&str>) -> String {
    let result_field = match result {
        Some(r) => {
            let escaped = r.replace('"', r#"\""#);
            format!(r#""result":"{escaped}""#)
        }
        None => r#""result":null"#.to_string(),
    };
    let session_field = match session_id {
        Some(s) => format!(r#","session_id":"{s}""#),
        None => String::new(),
    };
    format!(r#"{{"type":"result",{result_field}{session_field}}}"#)
}

// =============================================================================
// AC: empty input
// =============================================================================

#[test]
fn empty_input_returns_empty_string_and_none() {
    let (output, sid) = parse_stream("");
    assert_eq!(output, "");
    assert!(sid.is_none());
}

// =============================================================================
// AC: Result event with non-empty result field is used directly
// =============================================================================

#[test]
fn result_event_present_uses_result_field() {
    let ndjson = result_line(Some("Final answer."), None);
    let (output, _) = parse_stream(&ndjson);
    assert_eq!(output, "Final answer.");
}

// =============================================================================
// AC: Result with null result falls back to accumulated assistant text
// =============================================================================

#[test]
fn result_null_falls_back_to_assistant_text() {
    let lines = [
        assistant_text_line("Assistant said this."),
        result_line(None, None),
    ]
    .join("\n");
    let (output, _) = parse_stream(&lines);
    assert_eq!(output, "Assistant said this.");
}

#[test]
fn result_empty_string_falls_back_to_assistant_text() {
    let lines = [
        assistant_text_line("Fallback text."),
        result_line(Some(""), None),
    ]
    .join("\n");
    let (output, _) = parse_stream(&lines);
    assert_eq!(output, "Fallback text.");
}

// =============================================================================
// AC: multiple assistant text blocks are joined with newlines
// =============================================================================

#[test]
fn multiple_assistant_blocks_joined_with_newlines() {
    let lines = [
        assistant_text_line("Block one."),
        assistant_text_line("Block two."),
        assistant_text_line("Block three."),
    ]
    .join("\n");
    let (output, _) = parse_stream(&lines);
    assert_eq!(output, "Block one.\nBlock two.\nBlock three.");
}

// =============================================================================
// AC: session_id captured from System event
// =============================================================================

#[test]
fn session_id_captured_from_system_event() {
    let ndjson = system_line("sess-abc-123");
    let (_, sid) = parse_stream(&ndjson);
    assert_eq!(sid.as_deref(), Some("sess-abc-123"));
}

#[test]
fn session_id_none_when_no_system_or_result_event() {
    let ndjson = assistant_text_line("hello");
    let (_, sid) = parse_stream(&ndjson);
    assert!(sid.is_none());
}

// =============================================================================
// AC: session_id overridden by later Result event
// =============================================================================

#[test]
fn result_session_id_overrides_system_session_id() {
    let lines = [
        system_line("from-system"),
        result_line(Some("done"), Some("from-result")),
    ]
    .join("\n");
    let (_, sid) = parse_stream(&lines);
    assert_eq!(sid.as_deref(), Some("from-result"));
}

#[test]
fn system_session_id_kept_when_result_has_no_session_id() {
    let lines = [
        system_line("from-system"),
        result_line(Some("done"), None),
    ]
    .join("\n");
    let (_, sid) = parse_stream(&lines);
    assert_eq!(sid.as_deref(), Some("from-system"));
}

// =============================================================================
// AC: malformed NDJSON lines are silently skipped
// =============================================================================

#[test]
fn malformed_lines_are_skipped() {
    let lines = [
        "not json at all".to_string(),
        r#"{"unclosed":"#.to_string(),
        assistant_text_line("Valid block."),
        "{ bad json }".to_string(),
        result_line(None, None),
    ]
    .join("\n");
    let (output, _) = parse_stream(&lines);
    assert_eq!(output, "Valid block.");
}

#[test]
fn blank_lines_are_skipped() {
    let lines = format!(
        "\n\n{}\n\n{}\n",
        assistant_text_line("Content."),
        result_line(None, None)
    );
    let (output, _) = parse_stream(&lines);
    assert_eq!(output, "Content.");
}
