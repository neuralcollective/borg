// Tests for TaskStreamManager post-end_task() contract.
//
// Covers:
//   - subscribe() after end_task() returns non-empty history and None receiver.
//   - push_line() after end_task() still appends to history.
//   - push_phase_result() produces well-formed JSON with properly escaped strings.

use borg_core::stream::TaskStreamManager;

// =============================================================================
// subscribe after end_task: history is non-empty, receiver is None
// =============================================================================

#[tokio::test]
async fn test_subscribe_after_end_task_has_history() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 100;
    manager.start(task_id).await;
    manager.push_line(task_id, "line one".to_string()).await;
    manager.end_task(task_id).await;

    let (history, rx) = manager.subscribe(task_id).await;
    assert!(!history.is_empty(), "history must be non-empty after end_task");
    assert!(rx.is_none(), "receiver must be None after end_task");
}

#[tokio::test]
async fn test_subscribe_after_end_task_history_contains_stream_end() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 101;
    manager.start(task_id).await;
    manager.end_task(task_id).await;

    let (history, _rx) = manager.subscribe(task_id).await;
    let joined = history.join("\n");
    assert!(
        joined.contains("stream_end"),
        "history must contain stream_end event, got: {joined}"
    );
}

#[tokio::test]
async fn test_subscribe_after_end_task_receiver_is_none() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 102;
    manager.start(task_id).await;
    manager.push_line(task_id, "before end".to_string()).await;
    manager.end_task(task_id).await;

    let (_history, rx) = manager.subscribe(task_id).await;
    assert!(rx.is_none(), "live receiver must be None once stream has ended");
}

#[tokio::test]
async fn test_subscribe_active_stream_has_live_receiver() {
    // Sanity check: a stream that has NOT ended returns Some(rx).
    let manager = TaskStreamManager::new();
    let task_id: i64 = 103;
    manager.start(task_id).await;

    let (_history, rx) = manager.subscribe(task_id).await;
    assert!(rx.is_some(), "live receiver must be Some while stream is active");
}

// =============================================================================
// push_line after end_task: appends to history
// =============================================================================

#[tokio::test]
async fn test_push_line_after_end_task_appends_to_history() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 110;
    manager.start(task_id).await;
    manager.end_task(task_id).await;

    // Push after end — must still land in history.
    manager
        .push_line(task_id, "PostEndSentinel_XYZ".to_string())
        .await;

    let (history, _rx) = manager.subscribe(task_id).await;
    let joined = history.join("\n");
    assert!(
        joined.contains("PostEndSentinel_XYZ"),
        "line pushed after end_task must appear in history, got: {joined}"
    );
}

#[tokio::test]
async fn test_push_line_after_end_task_history_ordering() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 111;
    manager.start(task_id).await;
    manager.push_line(task_id, "BEFORE_END".to_string()).await;
    manager.end_task(task_id).await;
    manager.push_line(task_id, "AFTER_END".to_string()).await;

    let (history, _rx) = manager.subscribe(task_id).await;
    let joined = history.join("\n");
    let before_pos = joined.find("BEFORE_END").expect("BEFORE_END missing");
    let end_pos = joined.find("stream_end").expect("stream_end missing");
    let after_pos = joined.find("AFTER_END").expect("AFTER_END missing");

    assert!(before_pos < end_pos, "BEFORE_END must precede stream_end");
    assert!(end_pos < after_pos, "stream_end must precede AFTER_END");
}

// =============================================================================
// push_phase_result: well-formed JSON with proper escaping
// =============================================================================

#[tokio::test]
async fn test_push_phase_result_produces_valid_json() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 120;
    manager.start(task_id).await;
    manager.push_phase_result(task_id, "spec", "Some content.").await;

    let (history, _rx) = manager.subscribe(task_id).await;
    let phase_result_line = history
        .iter()
        .find(|l| l.contains("phase_result"))
        .expect("phase_result line not found");

    let parsed: serde_json::Value =
        serde_json::from_str(phase_result_line).expect("phase_result line must be valid JSON");
    assert_eq!(parsed["type"], "phase_result");
    assert_eq!(parsed["phase"], "spec");
    assert_eq!(parsed["content"], "Some content.");
}

#[tokio::test]
async fn test_push_phase_result_escapes_quotes_in_phase() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 121;
    manager.start(task_id).await;
    // Phase name with embedded double-quote (pathological but must not break JSON).
    manager
        .push_phase_result(task_id, r#"ph"ase"#, "content")
        .await;

    let (history, _rx) = manager.subscribe(task_id).await;
    let line = history
        .iter()
        .find(|l| l.contains("phase_result"))
        .expect("phase_result line not found");

    let parsed: serde_json::Value =
        serde_json::from_str(line).expect("JSON must be valid even with quoted phase");
    assert_eq!(parsed["type"], "phase_result");
}

#[tokio::test]
async fn test_push_phase_result_escapes_quotes_in_content() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 122;
    manager.start(task_id).await;
    manager
        .push_phase_result(task_id, "spec", r#"He said "hello"."#)
        .await;

    let (history, _rx) = manager.subscribe(task_id).await;
    let line = history
        .iter()
        .find(|l| l.contains("phase_result"))
        .expect("phase_result line not found");

    let parsed: serde_json::Value =
        serde_json::from_str(line).expect("JSON must be valid with quoted content");
    assert_eq!(parsed["content"], r#"He said "hello"."#);
}

#[tokio::test]
async fn test_push_phase_result_escapes_backslashes_in_content() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 123;
    manager.start(task_id).await;
    manager
        .push_phase_result(task_id, "impl", r"path\to\file")
        .await;

    let (history, _rx) = manager.subscribe(task_id).await;
    let line = history
        .iter()
        .find(|l| l.contains("phase_result"))
        .expect("phase_result line not found");

    let parsed: serde_json::Value =
        serde_json::from_str(line).expect("JSON must be valid with backslashes in content");
    assert_eq!(parsed["content"], r"path\to\file");
}

#[tokio::test]
async fn test_push_phase_result_escapes_newlines_in_content() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 124;
    manager.start(task_id).await;
    manager
        .push_phase_result(task_id, "qa", "line1\nline2")
        .await;

    let (history, _rx) = manager.subscribe(task_id).await;
    let line = history
        .iter()
        .find(|l| l.contains("phase_result"))
        .expect("phase_result line not found");

    // The NDJSON line itself must not contain a literal newline (would break NDJSON).
    assert!(
        !line.contains('\n'),
        "phase_result NDJSON line must not contain a literal newline"
    );
    let parsed: serde_json::Value = serde_json::from_str(line).expect("must be valid JSON");
    assert_eq!(parsed["content"], "line1\nline2");
}
