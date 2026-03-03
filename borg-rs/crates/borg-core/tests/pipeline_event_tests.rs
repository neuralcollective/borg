use borg_core::types::PipelineEvent;

// =============================================================================
// kind()
// =============================================================================

#[test]
fn test_phase_kind() {
    let event = PipelineEvent::Phase {
        task_id: Some(1),
        message: "running spec".into(),
    };
    assert_eq!(event.kind(), "task_phase");
}

#[test]
fn test_output_kind() {
    let event = PipelineEvent::Output {
        task_id: Some(2),
        message: "line output".into(),
    };
    assert_eq!(event.kind(), "task_output");
}

#[test]
fn test_notify_kind() {
    let event = PipelineEvent::Notify {
        chat_id: "tg:123".into(),
        message: "task done".into(),
    };
    assert_eq!(event.kind(), "notify");
}

#[test]
fn test_phase_result_kind() {
    let event = PipelineEvent::PhaseResult {
        task_id: 3,
        phase: "spec".into(),
        content: "Summary.".into(),
        chat_id: "tg:456".into(),
    };
    assert_eq!(event.kind(), "phase_result");
}

// =============================================================================
// task_id()
// =============================================================================

#[test]
fn test_phase_task_id_some() {
    let event = PipelineEvent::Phase {
        task_id: Some(42),
        message: "msg".into(),
    };
    assert_eq!(event.task_id(), Some(42));
}

#[test]
fn test_phase_task_id_none() {
    let event = PipelineEvent::Phase {
        task_id: None,
        message: "msg".into(),
    };
    assert_eq!(event.task_id(), None);
}

#[test]
fn test_output_task_id_some() {
    let event = PipelineEvent::Output {
        task_id: Some(7),
        message: "out".into(),
    };
    assert_eq!(event.task_id(), Some(7));
}

#[test]
fn test_output_task_id_none() {
    let event = PipelineEvent::Output {
        task_id: None,
        message: "out".into(),
    };
    assert_eq!(event.task_id(), None);
}

#[test]
fn test_notify_task_id_is_none() {
    let event = PipelineEvent::Notify {
        chat_id: "tg:999".into(),
        message: "hello".into(),
    };
    assert_eq!(event.task_id(), None);
}

#[test]
fn test_phase_result_task_id() {
    let event = PipelineEvent::PhaseResult {
        task_id: 99,
        phase: "qa".into(),
        content: "done".into(),
        chat_id: String::new(),
    };
    assert_eq!(event.task_id(), Some(99));
}

// =============================================================================
// message()
// =============================================================================

#[test]
fn test_phase_message() {
    let event = PipelineEvent::Phase {
        task_id: None,
        message: "phase message text".into(),
    };
    assert_eq!(event.message(), "phase message text");
}

#[test]
fn test_output_message() {
    let event = PipelineEvent::Output {
        task_id: Some(1),
        message: "output line".into(),
    };
    assert_eq!(event.message(), "output line");
}

#[test]
fn test_notify_message() {
    let event = PipelineEvent::Notify {
        chat_id: "tg:1".into(),
        message: "notify text".into(),
    };
    assert_eq!(event.message(), "notify text");
}

#[test]
fn test_phase_result_message_is_content() {
    let event = PipelineEvent::PhaseResult {
        task_id: 5,
        phase: "impl".into(),
        content: "implementation complete".into(),
        chat_id: "tg:200".into(),
    };
    assert_eq!(event.message(), "implementation complete");
}
