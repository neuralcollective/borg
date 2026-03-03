use std::fs;
use tempfile::TempDir;

use borg_core::config::{codex_has_credentials, read_oauth_from_credentials};

fn write_json(dir: &TempDir, name: &str, content: &str) -> String {
    let path = dir.path().join(name);
    fs::write(&path, content).unwrap();
    path.to_str().unwrap().to_string()
}

// ── codex_has_credentials ─────────────────────────────────────────────────────

#[test]
fn codex_creds_missing_file_returns_false() {
    assert!(!codex_has_credentials("/tmp/nonexistent_borg_config_test_99999.json"));
}

#[test]
fn codex_creds_invalid_json_returns_false() {
    let dir = TempDir::new().unwrap();
    let path = write_json(&dir, "auth.json", "not valid json {{");
    assert!(!codex_has_credentials(&path));
}

#[test]
fn codex_creds_missing_tokens_key_returns_false() {
    let dir = TempDir::new().unwrap();
    let path = write_json(&dir, "auth.json", r#"{"other": "data"}"#);
    assert!(!codex_has_credentials(&path));
}

#[test]
fn codex_creds_missing_access_token_key_returns_false() {
    let dir = TempDir::new().unwrap();
    let path = write_json(&dir, "auth.json", r#"{"tokens": {"refresh_token": "r"}}"#);
    assert!(!codex_has_credentials(&path));
}

#[test]
fn codex_creds_empty_access_token_returns_false() {
    let dir = TempDir::new().unwrap();
    let path = write_json(&dir, "auth.json", r#"{"tokens": {"access_token": ""}}"#);
    assert!(!codex_has_credentials(&path));
}

#[test]
fn codex_creds_non_empty_access_token_returns_true() {
    let dir = TempDir::new().unwrap();
    let path = write_json(&dir, "auth.json", r#"{"tokens": {"access_token": "tok_abc123"}}"#);
    assert!(codex_has_credentials(&path));
}

// ── read_oauth_from_credentials ───────────────────────────────────────────────

#[test]
fn oauth_missing_file_returns_none() {
    assert!(read_oauth_from_credentials("/tmp/nonexistent_borg_oauth_test_99999.json").is_none());
}

#[test]
fn oauth_claude_ai_oauth_key_preferred() {
    let dir = TempDir::new().unwrap();
    let path = write_json(
        &dir,
        "creds.json",
        r#"{"claudeAiOauth": {"accessToken": "claude_tok"}, "oauthToken": "root_tok"}"#,
    );
    assert_eq!(
        read_oauth_from_credentials(&path).as_deref(),
        Some("claude_tok")
    );
}

#[test]
fn oauth_falls_back_to_root_oauth_token() {
    let dir = TempDir::new().unwrap();
    let path = write_json(&dir, "creds.json", r#"{"oauthToken": "root_tok"}"#);
    assert_eq!(
        read_oauth_from_credentials(&path).as_deref(),
        Some("root_tok")
    );
}

#[test]
fn oauth_returns_none_when_neither_key_exists() {
    let dir = TempDir::new().unwrap();
    let path = write_json(&dir, "creds.json", r#"{"unrelated": "data"}"#);
    assert!(read_oauth_from_credentials(&path).is_none());
}
