use borg_agent::event::parse_stream;

// empty input → ("", None)
#[test]
fn test_empty_input() {
    let (output, session_id) = parse_stream("");
    assert_eq!(output, "");
    assert!(session_id.is_none());
}

// blank-only input (only newlines/whitespace lines)
#[test]
fn test_blank_lines_only() {
    let (output, session_id) = parse_stream("\n\n   \n");
    assert_eq!(output, "");
    assert!(session_id.is_none());
}

// malformed JSON lines are silently skipped; valid lines still parsed
#[test]
fn test_malformed_lines_skipped() {
    let data = "not json at all\n\
                {bad: json}\n\
                {\"type\":\"result\",\"result\":\"ok\",\"session_id\":\"s1\"}\n";
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "ok");
    assert_eq!(session_id.as_deref(), Some("s1"));
}

// all lines malformed → ("", None)
#[test]
fn test_all_malformed_returns_empty() {
    let data = "garbage\n{not: valid}\n!!!\n";
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "");
    assert!(session_id.is_none());
}

// Result with non-empty result field → use result, ignore assistant text
#[test]
fn test_result_event_used_over_assistant_text() {
    let data = concat!(
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"assistant says hi\"}]}}\n",
        "{\"type\":\"result\",\"result\":\"final result\",\"session_id\":\"s2\"}\n",
    );
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "final result");
    assert_eq!(session_id.as_deref(), Some("s2"));
}

// Result with empty result falls back to collected assistant text
#[test]
fn test_result_empty_falls_back_to_assistant_text() {
    let data = concat!(
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"assistant output\"}]}}\n",
        "{\"type\":\"result\",\"result\":\"\",\"session_id\":\"s3\"}\n",
    );
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "assistant output");
    assert_eq!(session_id.as_deref(), Some("s3"));
}

// Result with null result also falls back to assistant text
#[test]
fn test_result_null_falls_back_to_assistant_text() {
    let data = concat!(
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"fallback text\"}]}}\n",
        "{\"type\":\"result\",\"session_id\":\"s4\"}\n",
    );
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "fallback text");
    assert_eq!(session_id.as_deref(), Some("s4"));
}

// No Result event → assistant text is returned
#[test]
fn test_no_result_event_returns_assistant_text() {
    let data = concat!(
        "{\"type\":\"system\",\"session_id\":\"sys1\"}\n",
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"the answer\"}]}}\n",
    );
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "the answer");
    assert_eq!(session_id.as_deref(), Some("sys1"));
}

// Multiple assistant text blocks are joined with newlines
#[test]
fn test_multiple_assistant_blocks_joined() {
    let data = concat!(
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"part one\"},{\"type\":\"text\",\"text\":\"part two\"}]}}\n",
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"part three\"}]}}\n",
    );
    let (output, _) = parse_stream(data);
    assert!(output.contains("part one"));
    assert!(output.contains("part two"));
    assert!(output.contains("part three"));
}

// Result session_id overwrites System session_id
#[test]
fn test_result_session_id_overwrites_system() {
    let data = concat!(
        "{\"type\":\"system\",\"session_id\":\"from-system\"}\n",
        "{\"type\":\"result\",\"result\":\"done\",\"session_id\":\"from-result\"}\n",
    );
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "done");
    assert_eq!(session_id.as_deref(), Some("from-result"));
}

// System event provides session_id when Result has none
#[test]
fn test_system_session_id_used_when_result_has_none() {
    let data = concat!(
        "{\"type\":\"system\",\"session_id\":\"sys-only\"}\n",
        "{\"type\":\"result\",\"result\":\"output\"}\n",
    );
    let (_, session_id) = parse_stream(data);
    assert_eq!(session_id.as_deref(), Some("sys-only"));
}

// Unknown event types are silently ignored
#[test]
fn test_unknown_event_type_ignored() {
    let data = concat!(
        "{\"type\":\"init_something_new\",\"data\":\"whatever\"}\n",
        "{\"type\":\"result\",\"result\":\"still works\",\"session_id\":\"s5\"}\n",
    );
    let (output, session_id) = parse_stream(data);
    assert_eq!(output, "still works");
    assert_eq!(session_id.as_deref(), Some("s5"));
}
