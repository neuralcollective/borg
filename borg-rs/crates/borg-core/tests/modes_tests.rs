use std::sync::Once;

use borg_core::modes::{get_mode, register_modes};
use borg_core::types::{IntegrationType, PipelineMode};

static INIT: Once = Once::new();

fn setup() {
    INIT.call_once(|| {
        register_modes(vec![
            make_mode("sweborg"),
            make_mode("lawborg"),
            make_mode("healthborg"),
            make_mode("webborg"),
            make_mode("chefborg"),
            make_mode("buildborg"),
            make_mode("medborg"),
        ]);
    });
}

fn make_mode(name: &str) -> PipelineMode {
    PipelineMode {
        name: name.into(),
        label: name.into(),
        category: String::new(),
        phases: vec![],
        seed_modes: vec![],
        initial_status: "backlog".into(),
        uses_docker: false,
        uses_test_cmd: false,
        integration: IntegrationType::None,
        default_max_attempts: 3,
    }
}

#[test]
fn test_alias_swe_resolves_to_sweborg() {
    setup();
    let mode = get_mode("swe").unwrap();
    assert_eq!(mode.name, "sweborg");
}

#[test]
fn test_alias_legal_resolves_to_lawborg() {
    setup();
    let mode = get_mode("legal").unwrap();
    assert_eq!(mode.name, "lawborg");
}

#[test]
fn test_alias_health_resolves_to_healthborg() {
    setup();
    let mode = get_mode("health").unwrap();
    assert_eq!(mode.name, "healthborg");
}

#[test]
fn test_alias_chef_resolves_to_chefborg() {
    setup();
    let mode = get_mode("chef").unwrap();
    assert_eq!(mode.name, "chefborg");
}

#[test]
fn test_alias_construction_resolves_to_buildborg() {
    setup();
    let mode = get_mode("construction").unwrap();
    assert_eq!(mode.name, "buildborg");
}

#[test]
fn test_alias_medwrite_resolves_to_medborg() {
    setup();
    let mode = get_mode("medwrite").unwrap();
    assert_eq!(mode.name, "medborg");
}

#[test]
fn test_canonical_name_passes_through() {
    setup();
    let mode = get_mode("sweborg").unwrap();
    assert_eq!(mode.name, "sweborg");
}

#[test]
fn test_unknown_name_returns_none() {
    setup();
    assert!(get_mode("unknown_mode_xyz").is_none());
}
