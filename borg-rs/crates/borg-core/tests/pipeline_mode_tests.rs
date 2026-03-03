use borg_core::types::{PhaseConfig, PipelineMode, IntegrationType};

fn make_mode(phase_names: &[&str]) -> PipelineMode {
    PipelineMode {
        name: "testborg".into(),
        label: "Test".into(),
        category: String::new(),
        phases: phase_names
            .iter()
            .map(|n| PhaseConfig { name: n.to_string(), ..Default::default() })
            .collect(),
        seed_modes: vec![],
        initial_status: "backlog".into(),
        uses_docker: false,
        uses_test_cmd: false,
        integration: IntegrationType::None,
        default_max_attempts: 3,
    }
}

// ── get_phase ─────────────────────────────────────────────────────────────

#[test]
fn test_get_phase_returns_some_for_existing_phase() {
    let mode = make_mode(&["backlog", "implement", "done"]);
    let phase = mode.get_phase("implement");
    assert!(phase.is_some());
    assert_eq!(phase.unwrap().name, "implement");
}

#[test]
fn test_get_phase_returns_none_for_unknown_name() {
    let mode = make_mode(&["backlog", "implement"]);
    assert!(mode.get_phase("nonexistent").is_none());
}

#[test]
fn test_get_phase_matches_first_phase() {
    let mode = make_mode(&["backlog", "implement"]);
    let phase = mode.get_phase("backlog");
    assert!(phase.is_some());
    assert_eq!(phase.unwrap().name, "backlog");
}

#[test]
fn test_get_phase_matches_last_phase() {
    let mode = make_mode(&["backlog", "implement", "review"]);
    let phase = mode.get_phase("review");
    assert!(phase.is_some());
    assert_eq!(phase.unwrap().name, "review");
}

// ── get_phase_index ───────────────────────────────────────────────────────

#[test]
fn test_get_phase_index_first_phase_is_zero() {
    let mode = make_mode(&["backlog", "implement", "validate"]);
    assert_eq!(mode.get_phase_index("backlog"), Some(0));
}

#[test]
fn test_get_phase_index_second_phase_is_one() {
    let mode = make_mode(&["backlog", "implement", "validate"]);
    assert_eq!(mode.get_phase_index("implement"), Some(1));
}

#[test]
fn test_get_phase_index_last_phase_correct() {
    let mode = make_mode(&["backlog", "implement", "validate"]);
    assert_eq!(mode.get_phase_index("validate"), Some(2));
}

#[test]
fn test_get_phase_index_unknown_returns_none() {
    let mode = make_mode(&["backlog", "implement"]);
    assert_eq!(mode.get_phase_index("missing"), None);
}

// ── is_terminal ───────────────────────────────────────────────────────────

#[test]
fn test_is_terminal_done() {
    let mode = make_mode(&["backlog"]);
    assert!(mode.is_terminal("done"));
}

#[test]
fn test_is_terminal_merged() {
    let mode = make_mode(&["backlog"]);
    assert!(mode.is_terminal("merged"));
}

#[test]
fn test_is_terminal_failed() {
    let mode = make_mode(&["backlog"]);
    assert!(mode.is_terminal("failed"));
}

#[test]
fn test_is_terminal_backlog_is_false() {
    let mode = make_mode(&["backlog"]);
    assert!(!mode.is_terminal("backlog"));
}

#[test]
fn test_is_terminal_implement_is_false() {
    let mode = make_mode(&["backlog", "implement"]);
    assert!(!mode.is_terminal("implement"));
}

#[test]
fn test_is_terminal_validate_is_false() {
    let mode = make_mode(&["backlog", "implement", "validate"]);
    assert!(!mode.is_terminal("validate"));
}

#[test]
fn test_is_terminal_blocked_is_false() {
    let mode = make_mode(&["backlog"]);
    assert!(!mode.is_terminal("blocked"));
}
