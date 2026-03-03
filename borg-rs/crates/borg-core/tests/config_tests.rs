use borg_core::config::{codex_has_credentials, read_oauth_from_credentials};

fn write_tmp(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("creds.json");
    std::fs::write(&path, content).unwrap();
    (dir, path)
}

// ── read_oauth_from_credentials ───────────────────────────────────────────────

#[test]
fn test_oauth_returns_claude_ai_oauth_access_token() {
    let (_dir, path) = write_tmp(r#"{"claudeAiOauth":{"accessToken":"tok-abc"}}"#);
    assert_eq!(
        read_oauth_from_credentials(path.to_str().unwrap()),
        Some("tok-abc".into())
    );
}

#[test]
fn test_oauth_falls_back_to_root_oauth_token() {
    let (_dir, path) = write_tmp(r#"{"oauthToken":"root-tok"}"#);
    assert_eq!(
        read_oauth_from_credentials(path.to_str().unwrap()),
        Some("root-tok".into())
    );
}

#[test]
fn test_oauth_prefers_claude_ai_oauth_when_both_present() {
    let (_dir, path) = write_tmp(
        r#"{"claudeAiOauth":{"accessToken":"preferred"},"oauthToken":"fallback"}"#,
    );
    assert_eq!(
        read_oauth_from_credentials(path.to_str().unwrap()),
        Some("preferred".into())
    );
}

#[test]
fn test_oauth_nonexistent_file_returns_none() {
    assert_eq!(
        read_oauth_from_credentials("/tmp/borg-test-nonexistent-credentials-xyz.json"),
        None
    );
}

#[test]
fn test_oauth_invalid_json_returns_none() {
    let (_dir, path) = write_tmp("not valid json {{{");
    assert_eq!(read_oauth_from_credentials(path.to_str().unwrap()), None);
}

#[test]
fn test_oauth_missing_both_fields_returns_none() {
    let (_dir, path) = write_tmp(r#"{"someOtherKey":"value"}"#);
    assert_eq!(read_oauth_from_credentials(path.to_str().unwrap()), None);
}

#[test]
fn test_oauth_claude_ai_oauth_without_access_token_falls_back_to_root() {
    let (_dir, path) = write_tmp(r#"{"claudeAiOauth":{"expiresAt":9999},"oauthToken":"fallback"}"#);
    assert_eq!(
        read_oauth_from_credentials(path.to_str().unwrap()),
        Some("fallback".into())
    );
}

// ── codex_has_credentials ─────────────────────────────────────────────────────

#[test]
fn test_codex_has_credentials_true_when_access_token_present() {
    let (_dir, path) = write_tmp(r#"{"tokens":{"access_token":"codex-tok-xyz"}}"#);
    assert!(codex_has_credentials(path.to_str().unwrap()));
}

#[test]
fn test_codex_has_credentials_false_when_access_token_empty() {
    let (_dir, path) = write_tmp(r#"{"tokens":{"access_token":""}}"#);
    assert!(!codex_has_credentials(path.to_str().unwrap()));
}

#[test]
fn test_codex_has_credentials_false_when_tokens_missing() {
    let (_dir, path) = write_tmp(r#"{"other":"stuff"}"#);
    assert!(!codex_has_credentials(path.to_str().unwrap()));
}

#[test]
fn test_codex_has_credentials_false_for_nonexistent_file() {
    assert!(!codex_has_credentials(
        "/tmp/borg-test-nonexistent-codex-auth-xyz.json"
    ));
}

#[test]
fn test_codex_has_credentials_false_for_invalid_json() {
    let (_dir, path) = write_tmp("not json");
    assert!(!codex_has_credentials(path.to_str().unwrap()));
}
