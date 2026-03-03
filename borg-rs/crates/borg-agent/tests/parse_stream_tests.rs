use borg_agent::event::parse_stream;

#[test]
fn empty_string_returns_empty_output() {
    let (output, session_id) = parse_stream("");
    assert!(output.is_empty());
    assert!(session_id.is_none());
}

#[test]
fn invalid_json_lines_are_skipped_without_panic() {
    let data = "not json at all\n{bad json\n{\"type\":\"result\",\"result\":\"ok\",\"session_id\":null}";
    let (output, _) = parse_stream(data);
    assert_eq!(output, "ok");
}

#[test]
fn result_field_is_preferred_over_assistant_text() {
    let data = concat!(
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"assistant says\"}]}}\n",
        "{\"type\":\"result\",\"result\":\"result wins\",\"session_id\":null}",
    );
    let (output, _) = parse_stream(data);
    assert_eq!(output, "result wins");
}

#[test]
fn falls_back_to_assistant_text_when_result_absent() {
    let data = concat!(
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hello\"}]}}\n",
        "{\"type\":\"result\",\"session_id\":null}",
    );
    let (output, _) = parse_stream(data);
    assert_eq!(output, "hello");
}

#[test]
fn falls_back_to_assistant_text_when_result_empty() {
    let data = concat!(
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"fallback\"}]}}\n",
        "{\"type\":\"result\",\"result\":\"\",\"session_id\":null}",
    );
    let (output, _) = parse_stream(data);
    assert_eq!(output, "fallback");
}

#[test]
fn multiple_text_blocks_joined_with_newline() {
    let data = concat!(
        "{\"type\":\"assistant\",\"message\":{\"content\":[",
        "{\"type\":\"text\",\"text\":\"first\"},",
        "{\"type\":\"text\",\"text\":\"second\"}",
        "]}}\n",
        "{\"type\":\"result\",\"session_id\":null}",
    );
    let (output, _) = parse_stream(data);
    assert_eq!(output, "first\nsecond");
}

#[test]
fn session_id_from_result_overwrites_system_session_id() {
    let data = concat!(
        "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"sys-id\"}\n",
        "{\"type\":\"result\",\"result\":\"done\",\"session_id\":\"result-id\"}",
    );
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "done");
    assert_eq!(session_id.as_deref(), Some("result-id"));
}

#[test]
fn session_id_from_system_used_when_result_has_none() {
    let data = concat!(
        "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"sys-id\"}\n",
        "{\"type\":\"result\",\"result\":\"done\"}",
    );
    let (_, session_id) = parse_stream(data);
    assert_eq!(session_id.as_deref(), Some("sys-id"));
}

#[test]
fn blank_lines_are_skipped() {
    let data = "\n\n{\"type\":\"result\",\"result\":\"hi\",\"session_id\":null}\n\n";
    let (output, _) = parse_stream(data);
    assert_eq!(output, "hi");
}
