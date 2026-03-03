use borg_core::config::{codex_has_credentials, read_oauth_from_credentials};
use std::io::Write;
use tempfile::NamedTempFile;

fn write_temp(contents: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(contents.as_bytes()).unwrap();
    f
}

// codex_has_credentials

#[test]
fn codex_has_credentials_returns_true_for_valid_token() {
    let f = write_temp(r#"{"tokens":{"access_token":"tok_abc123"}}"#);
    assert!(codex_has_credentials(f.path().to_str().unwrap()));
}

#[test]
fn codex_has_credentials_returns_false_for_empty_token() {
    let f = write_temp(r#"{"tokens":{"access_token":""}}"#);
    assert!(!codex_has_credentials(f.path().to_str().unwrap()));
}

#[test]
fn codex_has_credentials_returns_false_for_missing_access_token() {
    let f = write_temp(r#"{"tokens":{}}"#);
    assert!(!codex_has_credentials(f.path().to_str().unwrap()));
}

#[test]
fn codex_has_credentials_returns_false_for_missing_tokens_key() {
    let f = write_temp(r#"{"other":"value"}"#);
    assert!(!codex_has_credentials(f.path().to_str().unwrap()));
}

#[test]
fn codex_has_credentials_returns_false_for_malformed_json() {
    let f = write_temp("not json at all");
    assert!(!codex_has_credentials(f.path().to_str().unwrap()));
}

#[test]
fn codex_has_credentials_returns_false_for_nonexistent_file() {
    assert!(!codex_has_credentials("/tmp/borg_test_nonexistent_file_xyz.json"));
}

// read_oauth_from_credentials

#[test]
fn read_oauth_prefers_claude_ai_oauth_access_token() {
    let f = write_temp(
        r#"{"claudeAiOauth":{"accessToken":"claude_tok"},"oauthToken":"fallback_tok"}"#,
    );
    let result = read_oauth_from_credentials(f.path().to_str().unwrap());
    assert_eq!(result, Some("claude_tok".to_string()));
}

#[test]
fn read_oauth_falls_back_to_oauth_token() {
    let f = write_temp(r#"{"oauthToken":"fallback_tok"}"#);
    let result = read_oauth_from_credentials(f.path().to_str().unwrap());
    assert_eq!(result, Some("fallback_tok".to_string()));
}

#[test]
fn read_oauth_returns_none_when_neither_key_present() {
    let f = write_temp(r#"{"other":"value"}"#);
    let result = read_oauth_from_credentials(f.path().to_str().unwrap());
    assert_eq!(result, None);
}

#[test]
fn read_oauth_returns_none_for_missing_file() {
    let result = read_oauth_from_credentials("/tmp/borg_test_nonexistent_file_xyz.json");
    assert_eq!(result, None);
}

#[test]
fn read_oauth_returns_none_for_malformed_json() {
    let f = write_temp("not json");
    let result = read_oauth_from_credentials(f.path().to_str().unwrap());
    assert_eq!(result, None);
}

#[test]
fn read_oauth_uses_claude_ai_oauth_even_without_fallback() {
    let f = write_temp(r#"{"claudeAiOauth":{"accessToken":"only_primary"}}"#);
    let result = read_oauth_from_credentials(f.path().to_str().unwrap());
    assert_eq!(result, Some("only_primary".to_string()));
}
