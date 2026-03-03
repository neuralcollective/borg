use borg_core::config::read_oauth_from_credentials;
use std::io::Write;
use tempfile::NamedTempFile;

fn write_temp(contents: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(contents.as_bytes()).unwrap();
    f
}

#[test]
fn test_primary_path_returns_token() {
    let f = write_temp(r#"{"claudeAiOauth":{"accessToken":"primary-tok"}}"#);
    assert_eq!(
        read_oauth_from_credentials(f.path().to_str().unwrap()),
        Some("primary-tok".to_string())
    );
}

#[test]
fn test_fallback_path_returns_token() {
    let f = write_temp(r#"{"oauthToken":"fallback-tok"}"#);
    assert_eq!(
        read_oauth_from_credentials(f.path().to_str().unwrap()),
        Some("fallback-tok".to_string())
    );
}

#[test]
fn test_primary_wins_when_both_present() {
    let f = write_temp(r#"{"claudeAiOauth":{"accessToken":"primary-tok"},"oauthToken":"fallback-tok"}"#);
    assert_eq!(
        read_oauth_from_credentials(f.path().to_str().unwrap()),
        Some("primary-tok".to_string())
    );
}

#[test]
fn test_neither_present_returns_none() {
    let f = write_temp(r#"{"someOtherKey":"value"}"#);
    assert_eq!(read_oauth_from_credentials(f.path().to_str().unwrap()), None);
}

#[test]
fn test_malformed_json_returns_none() {
    let f = write_temp("not valid json {{{");
    assert_eq!(read_oauth_from_credentials(f.path().to_str().unwrap()), None);
}

#[test]
fn test_missing_file_returns_none() {
    assert_eq!(read_oauth_from_credentials("/nonexistent/path/credentials.json"), None);
}
