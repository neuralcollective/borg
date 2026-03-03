use borg_core::types::{IntegrationType, PhaseConfig, PipelineMode};

fn make_mode(phase_names: &[&str]) -> PipelineMode {
    PipelineMode {
        name: "testmode".into(),
        label: "Test Mode".into(),
        category: String::new(),
        phases: phase_names
            .iter()
            .map(|n| PhaseConfig {
                name: n.to_string(),
                label: n.to_string(),
                ..Default::default()
            })
            .collect(),
        seed_modes: vec![],
        initial_status: "spec".into(),
        uses_docker: false,
        uses_test_cmd: false,
        integration: IntegrationType::None,
        default_max_attempts: 3,
    }
}

#[test]
fn test_get_phase_exact_name_returns_config() {
    let mode = make_mode(&["spec", "impl", "validate"]);
    let phase = mode.get_phase("impl").expect("phase must be found");
    assert_eq!(phase.name, "impl");
}

#[test]
fn test_get_phase_first_phase() {
    let mode = make_mode(&["spec", "impl", "validate"]);
    let phase = mode.get_phase("spec").expect("first phase found");
    assert_eq!(phase.name, "spec");
}

#[test]
fn test_get_phase_last_phase() {
    let mode = make_mode(&["spec", "impl", "validate"]);
    let phase = mode.get_phase("validate").expect("last phase found");
    assert_eq!(phase.name, "validate");
}

#[test]
fn test_get_phase_nonexistent_returns_none() {
    let mode = make_mode(&["spec", "impl", "validate"]);
    assert!(mode.get_phase("missing").is_none());
}

#[test]
fn test_get_phase_empty_phases_returns_none() {
    let mode = make_mode(&[]);
    assert!(mode.get_phase("spec").is_none());
}

#[test]
fn test_get_phase_index_start() {
    let mode = make_mode(&["spec", "impl", "validate"]);
    assert_eq!(mode.get_phase_index("spec"), Some(0));
}

#[test]
fn test_get_phase_index_middle() {
    let mode = make_mode(&["spec", "impl", "validate"]);
    assert_eq!(mode.get_phase_index("impl"), Some(1));
}

#[test]
fn test_get_phase_index_end() {
    let mode = make_mode(&["spec", "impl", "validate"]);
    assert_eq!(mode.get_phase_index("validate"), Some(2));
}

#[test]
fn test_get_phase_index_nonexistent_returns_none() {
    let mode = make_mode(&["spec", "impl", "validate"]);
    assert!(mode.get_phase_index("missing").is_none());
}

#[test]
fn test_get_phase_index_empty_phases_returns_none() {
    let mode = make_mode(&[]);
    assert!(mode.get_phase_index("spec").is_none());
}
