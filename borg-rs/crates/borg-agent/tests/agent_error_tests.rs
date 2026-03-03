// Tests for agent_error container event JSON encoding.
//
// entrypoint.sh emits an `agent_error` event with `stderr_tail` embedded in
// the JSON payload.  The old implementation used `sed 's/"/\\"/g'` which
// escaped double-quotes only, leaving backslashes, tabs, newlines, and carriage
// returns unescaped — all of which produce invalid JSON.
//
// The fix uses `bun -e "JSON.stringify(...)"` which handles the full escape set.
// These tests verify that the Rust event parser can round-trip agent_error JSON
// that contains each of those problematic characters.

use serde_json::Value;

/// Parse the JSON string that the (fixed) entrypoint produces for agent_error,
/// then retrieve the stderr_tail value.
fn parse_agent_error(json: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json).ok()?;
    v["stderr_tail"].as_str().map(|s| s.to_string())
}

// =============================================================================
// Valid JSON produced by the fixed entrypoint must parse successfully
// =============================================================================

#[test]
fn test_plain_stderr_parses() {
    let json = r#"{"type":"container_event","event":"agent_error","exit_code":1,"stderr_tail":"some plain error message"}"#;
    let tail = parse_agent_error(json);
    assert_eq!(tail.as_deref(), Some("some plain error message"));
}

#[test]
fn test_stderr_with_backslash_parses() {
    // JSON.stringify encodes \ as \\
    let json = r#"{"type":"container_event","event":"agent_error","exit_code":1,"stderr_tail":"path\\to\\file error"}"#;
    let tail = parse_agent_error(json);
    assert_eq!(tail.as_deref(), Some("path\\to\\file error"));
}

#[test]
fn test_stderr_with_embedded_newline_parses() {
    // JSON.stringify encodes newlines as \n
    let json = "{\"type\":\"container_event\",\"event\":\"agent_error\",\"exit_code\":1,\"stderr_tail\":\"line one\\nline two\"}";
    let tail = parse_agent_error(json);
    assert_eq!(tail.as_deref(), Some("line one\nline two"));
}

#[test]
fn test_stderr_with_tab_parses() {
    // JSON.stringify encodes tabs as \t
    let json = "{\"type\":\"container_event\",\"event\":\"agent_error\",\"exit_code\":1,\"stderr_tail\":\"col1\\tcol2\"}";
    let tail = parse_agent_error(json);
    assert_eq!(tail.as_deref(), Some("col1\tcol2"));
}

#[test]
fn test_stderr_with_carriage_return_parses() {
    // JSON.stringify encodes \r as \r
    let json = "{\"type\":\"container_event\",\"event\":\"agent_error\",\"exit_code\":1,\"stderr_tail\":\"line\\r\\nwindows\"}";
    let tail = parse_agent_error(json);
    assert_eq!(tail.as_deref(), Some("line\r\nwindows"));
}

#[test]
fn test_stderr_with_double_quotes_parses() {
    // JSON.stringify encodes " as \"
    let json = r#"{"type":"container_event","event":"agent_error","exit_code":1,"stderr_tail":"error: \"file\" not found"}"#;
    let tail = parse_agent_error(json);
    assert_eq!(tail.as_deref(), Some("error: \"file\" not found"));
}

#[test]
fn test_stderr_with_all_special_chars_parses() {
    // Combined: backslash, newline, tab, double-quote — all properly escaped
    let json = "{\"type\":\"container_event\",\"event\":\"agent_error\",\"exit_code\":2,\"stderr_tail\":\"err: \\\"path\\\\to\\\\file\\\"\\n\\terror detail\"}";
    let tail = parse_agent_error(json);
    assert!(tail.is_some(), "combined special chars should parse");
    let t = tail.unwrap();
    assert!(t.contains("path\\to\\file"));
    assert!(t.contains('\n'));
    assert!(t.contains('\t'));
    assert!(t.contains('"'));
}

#[test]
fn test_exit_code_preserved() {
    let json = r#"{"type":"container_event","event":"agent_error","exit_code":137,"stderr_tail":"OOM killed"}"#;
    let v: Value = serde_json::from_str(json).unwrap();
    assert_eq!(v["exit_code"].as_i64(), Some(137));
    assert_eq!(v["event"].as_str(), Some("agent_error"));
}

// =============================================================================
// The old format (unescaped backslash) must fail to parse — confirming the bug
// =============================================================================

#[test]
fn test_old_unescaped_backslash_is_invalid_json() {
    // Simulates old: sed 's/"/\\"/g' — backslash left raw in JSON string value.
    // \q is not a valid JSON escape sequence, so parsing must fail.
    let bad_json = r#"{"type":"container_event","event":"agent_error","exit_code":1,"stderr_tail":"C:\users\name"}"#;
    let result: Result<Value, _> = serde_json::from_str(bad_json);
    assert!(result.is_err(), "unescaped backslash must produce invalid JSON");
}

#[test]
fn test_old_unescaped_newline_is_invalid_json() {
    // Raw newline inside a JSON string literal is invalid
    let bad_json = "{\
        \"type\":\"container_event\",\
        \"event\":\"agent_error\",\
        \"exit_code\":1,\
        \"stderr_tail\":\"line one\nline two\"\
    }";
    let result: Result<Value, _> = serde_json::from_str(bad_json);
    assert!(result.is_err(), "raw newline in JSON string must be invalid");
}

#[test]
fn test_old_unescaped_tab_is_invalid_json() {
    // Raw tab inside a JSON string literal is invalid
    let bad_json = "{\
        \"type\":\"container_event\",\
        \"event\":\"agent_error\",\
        \"exit_code\":1,\
        \"stderr_tail\":\"col1\tcol2\"\
    }";
    let result: Result<Value, _> = serde_json::from_str(bad_json);
    assert!(result.is_err(), "raw tab in JSON string must be invalid");
}
