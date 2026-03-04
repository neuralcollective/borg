use borg_agent::parse_test_result;

const MARKER: &str = "---BORG_TEST_RESULT---";

#[test]
fn no_marker_prefix_returns_none() {
    assert!(parse_test_result(r#"{"phase":"test","passed":true,"exitCode":0,"output":""}"#).is_none());
}

#[test]
fn plain_text_returns_none() {
    assert!(parse_test_result("some random log line").is_none());
}

#[test]
fn empty_string_returns_none() {
    assert!(parse_test_result("").is_none());
}

#[test]
fn valid_all_fields_populated() {
    let line = format!(
        r#"{MARKER}{{"phase":"compile","passed":true,"exitCode":0,"output":"ok"}}"#
    );
    let r = parse_test_result(&line).expect("should parse");
    assert_eq!(r.phase, "compile");
    assert!(r.passed);
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.output, "ok");
}

#[test]
fn missing_passed_defaults_to_false() {
    let line = format!(r#"{MARKER}{{"phase":"test","exitCode":0,"output":""}}"#);
    let r = parse_test_result(&line).expect("should parse");
    assert!(!r.passed);
}

#[test]
fn missing_exit_code_defaults_to_1() {
    let line = format!(r#"{MARKER}{{"phase":"test","passed":false,"output":""}}"#);
    let r = parse_test_result(&line).expect("should parse");
    assert_eq!(r.exit_code, 1);
}

#[test]
fn malformed_json_returns_none() {
    let line = format!("{MARKER}not-valid-json");
    assert!(parse_test_result(&line).is_none());
}

#[test]
fn malformed_json_truncated_returns_none() {
    let line = format!(r#"{MARKER}{{"phase":"test""#);
    assert!(parse_test_result(&line).is_none());
}
