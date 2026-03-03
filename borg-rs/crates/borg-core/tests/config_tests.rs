use borg_core::config::read_oauth_from_credentials;

fn write_temp(contents: &str) -> tempfile::NamedTempFile {
    let f = tempfile::NamedTempFile::new().expect("tempfile");
    std::fs::write(f.path(), contents).expect("write");
    f
}

#[test]
fn test_primary_path_claudeaioauth_access_token_returned() {
    let f = write_temp(r#"{"claudeAiOauth":{"accessToken":"tok-primary"}}"#);
    let result = read_oauth_from_credentials(f.path().to_str().unwrap());
    assert_eq!(result, Some("tok-primary".to_string()));
}

#[test]
fn test_fallback_oauth_token_returned_when_primary_absent() {
    let f = write_temp(r#"{"oauthToken":"tok-fallback"}"#);
    let result = read_oauth_from_credentials(f.path().to_str().unwrap());
    assert_eq!(result, Some("tok-fallback".to_string()));
}

#[test]
fn test_both_absent_returns_none() {
    let f = write_temp(r#"{"someOtherField":"value"}"#);
    let result = read_oauth_from_credentials(f.path().to_str().unwrap());
    assert_eq!(result, None);
}

#[test]
fn test_non_json_file_returns_none() {
    let f = write_temp("this is not json at all");
    let result = read_oauth_from_credentials(f.path().to_str().unwrap());
    assert_eq!(result, None);
}

#[test]
fn test_empty_access_token_not_filtered() {
    let f = write_temp(r#"{"claudeAiOauth":{"accessToken":""}}"#);
    let result = read_oauth_from_credentials(f.path().to_str().unwrap());
    assert_eq!(result, Some("".to_string()));
}

#[test]
fn test_missing_file_returns_none() {
    let result = read_oauth_from_credentials("/tmp/borg-nonexistent-credentials-file.json");
    assert_eq!(result, None);
}

#[test]
fn test_primary_takes_precedence_over_fallback() {
    let f = write_temp(r#"{"claudeAiOauth":{"accessToken":"tok-primary"},"oauthToken":"tok-fallback"}"#);
    let result = read_oauth_from_credentials(f.path().to_str().unwrap());
    assert_eq!(result, Some("tok-primary".to_string()));
}
