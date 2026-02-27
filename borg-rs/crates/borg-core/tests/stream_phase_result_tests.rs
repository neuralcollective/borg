// Tests for AC7: Rust parity — push_phase_result on TaskStreamManager and
// PipelineEvent::PhaseResult variant.
//
// These tests FAIL initially (fail to compile) because:
//   - `TaskStreamManager::push_phase_result` does not yet exist.
//   - `PipelineEvent::PhaseResult` variant does not yet exist.
//
// Once implemented they cover:
//   AC7: push_phase_result is declared on TaskStreamManager.
//   AC7: push_phase_result injects a line containing "phase_result" into the stream.
//   AC7: the injected line contains the phase name.
//   AC7: the injected line contains the content string.
//   AC7: push_phase_result with a nonexistent task_id is a no-op (no panic).
//   AC7: PipelineEvent::PhaseResult variant is constructible with the right fields.
//   EC9: the phase_result event appears before stream_end in the history.

use borg_core::{stream::TaskStreamManager, types::PipelineEvent};

// =============================================================================
// AC7: PipelineEvent::PhaseResult variant is constructible
// =============================================================================

#[test]
fn test_pipeline_event_phase_result_variant_exists() {
    let event = PipelineEvent::PhaseResult {
        task_id: 1,
        phase: "spec".to_string(),
        content: "Summary text.".to_string(),
        chat_id: "tg:-1234".to_string(),
    };
    // Verify the fields round-trip correctly.
    if let PipelineEvent::PhaseResult {
        task_id,
        phase,
        content,
        chat_id,
    } = event
    {
        assert_eq!(task_id, 1);
        assert_eq!(phase, "spec");
        assert_eq!(content, "Summary text.");
        assert_eq!(chat_id, "tg:-1234");
    } else {
        panic!("pattern match failed");
    }
}

#[test]
fn test_pipeline_event_phase_result_with_empty_chat_id() {
    // Empty chat_id is valid (SSE event only, no chat notification).
    let event = PipelineEvent::PhaseResult {
        task_id: 2,
        phase: "qa".to_string(),
        content: "QA complete.".to_string(),
        chat_id: String::new(),
    };
    if let PipelineEvent::PhaseResult { chat_id, .. } = event {
        assert!(chat_id.is_empty());
    }
}

// =============================================================================
// AC7: push_phase_result injects event into stream history
// =============================================================================

#[tokio::test]
async fn test_push_phase_result_injects_into_history() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 42;
    manager.start(task_id).await;

    manager
        .push_phase_result(task_id, "spec", "Here is the specification.")
        .await;

    let (history, _rx) = manager.subscribe(task_id).await;
    assert!(
        !history.is_empty(),
        "history must contain at least one entry"
    );
}

#[tokio::test]
async fn test_push_phase_result_history_contains_phase_result_type() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 43;
    manager.start(task_id).await;

    manager
        .push_phase_result(task_id, "qa", "QA summary.")
        .await;

    let (history, _rx) = manager.subscribe(task_id).await;
    let joined = history.join("\n");
    assert!(
        joined.contains("phase_result"),
        "history must contain 'phase_result', got: {joined}"
    );
}

#[tokio::test]
async fn test_push_phase_result_history_contains_phase_name() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 44;
    manager.start(task_id).await;

    manager.push_phase_result(task_id, "qa_fix", "Fixed.").await;

    let (history, _rx) = manager.subscribe(task_id).await;
    let joined = history.join("\n");
    assert!(
        joined.contains("qa_fix"),
        "history must contain phase name 'qa_fix', got: {joined}"
    );
}

#[tokio::test]
async fn test_push_phase_result_history_contains_content() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 45;
    manager.start(task_id).await;

    manager
        .push_phase_result(task_id, "spec", "UniqueSentinel_ABC_987")
        .await;

    let (history, _rx) = manager.subscribe(task_id).await;
    let joined = history.join("\n");
    assert!(
        joined.contains("UniqueSentinel_ABC_987"),
        "history must contain the content string, got: {joined}"
    );
}

// =============================================================================
// AC7: push_phase_result with nonexistent task_id is a no-op
// =============================================================================

#[tokio::test]
async fn test_push_phase_result_nonexistent_task_id_is_noop() {
    let manager = TaskStreamManager::new();
    // No stream started for task_id 9999 — must not panic.
    manager.push_phase_result(9999, "spec", "Ghost task.").await;
}

// =============================================================================
// EC9: phase_result appears before stream_end
// =============================================================================

#[tokio::test]
async fn test_phase_result_appears_before_stream_end() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 50;
    manager.start(task_id).await;

    manager
        .push_phase_result(task_id, "spec", "Summary before end.")
        .await;

    // Verify phase_result is in history before end_task is called.
    let (history_before, _rx) = manager.subscribe(task_id).await;
    let joined_before = history_before.join("\n");
    assert!(
        joined_before.contains("phase_result"),
        "phase_result must be in history before end"
    );
    assert!(
        !joined_before.contains("stream_end"),
        "stream_end must not be in history before end_task is called"
    );

    // After end_task, stream_end is injected.
    manager.end_task(task_id).await;
    let (history_after, _rx2) = manager.subscribe(task_id).await;
    let joined_after = history_after.join("\n");
    // phase_result must still precede stream_end in the history.
    let pr_pos = joined_after.find("phase_result");
    let se_pos = joined_after.find("stream_end");
    assert!(pr_pos.is_some(), "phase_result absent after end_task");
    assert!(se_pos.is_some(), "stream_end absent after end_task");
    assert!(
        pr_pos.unwrap() < se_pos.unwrap(),
        "phase_result must precede stream_end in history"
    );
}

// =============================================================================
// Multiple push_phase_result calls both appear in history
// =============================================================================

#[tokio::test]
async fn test_multiple_push_phase_result_both_in_history() {
    let manager = TaskStreamManager::new();
    let task_id: i64 = 60;
    manager.start(task_id).await;

    manager
        .push_phase_result(task_id, "spec", "First_UniqueABC")
        .await;
    manager
        .push_phase_result(task_id, "qa", "Second_UniqueXYZ")
        .await;

    let (history, _rx) = manager.subscribe(task_id).await;
    let joined = history.join("\n");
    assert!(joined.contains("First_UniqueABC"), "first result missing");
    assert!(joined.contains("Second_UniqueXYZ"), "second result missing");
}

// =============================================================================
// Independent streams — push only affects the specified task
// =============================================================================

#[tokio::test]
async fn test_push_phase_result_only_affects_specified_task() {
    let manager = TaskStreamManager::new();
    let task_a: i64 = 70;
    let task_b: i64 = 71;
    manager.start(task_a).await;
    manager.start(task_b).await;

    manager
        .push_phase_result(task_a, "spec", "Only for task A.")
        .await;

    let (history_a, _) = manager.subscribe(task_a).await;
    let (history_b, _) = manager.subscribe(task_b).await;
    let joined_a = history_a.join("\n");
    let joined_b = history_b.join("\n");

    assert!(joined_a.contains("Only for task A."));
    assert!(
        !joined_b.contains("Only for task A."),
        "task B stream must be unaffected, got: {joined_b}"
    );
}
