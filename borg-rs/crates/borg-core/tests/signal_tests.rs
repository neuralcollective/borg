use std::fs;
#[cfg(unix)]
use std::os::unix::fs as unix_fs;

use borg_core::{pipeline::Pipeline, types::AgentSignal};
use tempfile::TempDir;

#[test]
fn test_signal_default_is_done() {
    let signal = AgentSignal::default();
    assert_eq!(signal.status, "done");
    assert!(!signal.is_blocked());
    assert!(!signal.is_abandon());
}

#[test]
fn test_signal_done_constructor() {
    let signal = AgentSignal::done();
    assert_eq!(signal.status, "done");
    assert!(signal.reason.is_empty());
    assert!(signal.question.is_empty());
}

#[test]
fn test_signal_parse_blocked() {
    let json = r#"{"status":"blocked","reason":"task is ambiguous","question":"which API?"}"#;
    let signal: AgentSignal = serde_json::from_str(json).unwrap();
    assert!(signal.is_blocked());
    assert!(!signal.is_abandon());
    assert_eq!(signal.reason, "task is ambiguous");
    assert_eq!(signal.question, "which API?");
}

#[test]
fn test_signal_parse_abandon() {
    let json = r#"{"status":"abandon","reason":"task already done"}"#;
    let signal: AgentSignal = serde_json::from_str(json).unwrap();
    assert!(signal.is_abandon());
    assert!(!signal.is_blocked());
    assert_eq!(signal.reason, "task already done");
}

#[test]
fn test_signal_parse_done_explicit() {
    let json = r#"{"status":"done"}"#;
    let signal: AgentSignal = serde_json::from_str(json).unwrap();
    assert_eq!(signal.status, "done");
    assert!(!signal.is_blocked());
    assert!(!signal.is_abandon());
}

#[test]
fn test_signal_parse_missing_optional_fields() {
    let json = r#"{"status":"blocked","reason":"stuck"}"#;
    let signal: AgentSignal = serde_json::from_str(json).unwrap();
    assert!(signal.is_blocked());
    assert!(signal.question.is_empty());
}

#[test]
fn test_signal_parse_empty_object_defaults_to_done() {
    let json = r#"{}"#;
    let signal: AgentSignal = serde_json::from_str(json).unwrap();
    assert_eq!(signal.status, "done");
}

#[test]
fn test_signal_malformed_json_returns_default() {
    let result: Result<AgentSignal, _> = serde_json::from_str("not json at all");
    // Malformed JSON should fail to parse — the pipeline handles this by falling back to default
    assert!(result.is_err());
}

#[test]
fn test_signal_roundtrip_serialize_deserialize() {
    let original = AgentSignal {
        status: "blocked".into(),
        reason: "need clarification".into(),
        question: "what scope?".into(),
    };
    let json = serde_json::to_string(&original).unwrap();
    let parsed: AgentSignal = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.status, "blocked");
    assert_eq!(parsed.reason, "need clarification");
    assert_eq!(parsed.question, "what scope?");
}

// ── read_agent_signal: O_NOFOLLOW protection ──────────────────────────────────

fn make_worktree_with_signal(dir: &TempDir, json: &str) {
    fs::create_dir_all(dir.path().join(".borg")).unwrap();
    fs::write(dir.path().join(".borg/signal.json"), json).unwrap();
}

#[test]
fn read_agent_signal_parses_valid_file() {
    let dir = TempDir::new().unwrap();
    make_worktree_with_signal(&dir, r#"{"status":"blocked","reason":"ambiguous"}"#);

    let signal = Pipeline::read_agent_signal(dir.path().to_str().unwrap());
    assert!(signal.is_blocked());
    assert_eq!(signal.reason, "ambiguous");
}

#[test]
fn read_agent_signal_removes_file_after_read() {
    let dir = TempDir::new().unwrap();
    make_worktree_with_signal(&dir, r#"{"status":"done"}"#);

    Pipeline::read_agent_signal(dir.path().to_str().unwrap());

    assert!(
        !dir.path().join(".borg/signal.json").exists(),
        "signal.json should be removed after a successful read"
    );
}

#[test]
fn read_agent_signal_missing_returns_default() {
    let dir = TempDir::new().unwrap();
    let signal = Pipeline::read_agent_signal(dir.path().to_str().unwrap());
    assert_eq!(signal.status, "done");
}

#[test]
fn read_agent_signal_malformed_json_returns_default() {
    let dir = TempDir::new().unwrap();
    make_worktree_with_signal(&dir, "not valid json {{");
    let signal = Pipeline::read_agent_signal(dir.path().to_str().unwrap());
    assert_eq!(signal.status, "done");
}

#[cfg(unix)]
#[test]
fn read_agent_signal_symlink_returns_default_and_quarantines() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".borg")).unwrap();

    // Create a "sensitive" file outside the worktree
    let secret = dir.path().join("secret.txt");
    fs::write(&secret, r#"{"status":"abandon","reason":"injected"}"#).unwrap();

    // Replace signal.json with a symlink pointing at it
    unix_fs::symlink(&secret, dir.path().join(".borg/signal.json")).unwrap();

    let signal = Pipeline::read_agent_signal(dir.path().to_str().unwrap());

    // Must not follow the symlink — returns safe default
    assert_eq!(signal.status, "done", "symlink must not be followed");

    // The symlink must have been quarantined (no longer exists at original path)
    assert!(
        !dir.path().join(".borg/signal.json").exists()
            && dir.path().join(".borg/signal.json").symlink_metadata().is_err(),
        "symlink should have been quarantined"
    );
}
