use borg_core::types::AgentSignal;

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
