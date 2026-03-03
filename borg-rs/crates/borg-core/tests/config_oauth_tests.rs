use borg_core::config::read_oauth_from_credentials;
use std::io::Write;
use tempfile::NamedTempFile;

fn write_temp(contents: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(contents.as_bytes()).unwrap();
    f
}

#[test]
fn claudeaioauth_access_token_returned() {
    let f = write_temp(r#"{"claudeAiOauth":{"accessToken":"tok-abc"}}"#);
    assert_eq!(
        read_oauth_from_credentials(f.path().to_str().unwrap()),
        Some("tok-abc".to_string())
    );
}

#[test]
fn root_oauth_token_fallback() {
    let f = write_temp(r#"{"oauthToken":"root-tok"}"#);
    assert_eq!(
        read_oauth_from_credentials(f.path().to_str().unwrap()),
        Some("root-tok".to_string())
    );
}

#[test]
fn claudeaioauth_preferred_over_root() {
    let f = write_temp(
        r#"{"claudeAiOauth":{"accessToken":"nested-tok"},"oauthToken":"root-tok"}"#,
    );
    assert_eq!(
        read_oauth_from_credentials(f.path().to_str().unwrap()),
        Some("nested-tok".to_string())
    );
}

#[test]
fn missing_file_returns_none() {
    assert_eq!(
        read_oauth_from_credentials("/tmp/borg-test-nonexistent-credentials.json"),
        None
    );
}

#[test]
fn invalid_json_returns_none() {
    let f = write_temp("not valid json {{{");
    assert_eq!(
        read_oauth_from_credentials(f.path().to_str().unwrap()),
        None
    );
}

#[test]
fn json_without_either_key_returns_none() {
    let f = write_temp(r#"{"someOtherKey":"value"}"#);
    assert_eq!(
        read_oauth_from_credentials(f.path().to_str().unwrap()),
        None
    );
}
