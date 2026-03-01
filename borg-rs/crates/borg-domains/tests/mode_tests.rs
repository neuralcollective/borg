use borg_core::types::PhaseType;

#[test]
fn test_swe_mode_has_implement_validate_flow() {
    let mode = borg_domains::swe::swe_mode();
    assert_eq!(mode.name, "sweborg");
    let names: Vec<&str> = mode.phases.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, &["backlog", "implement", "validate", "lint_fix", "rebase"]);
}

#[test]
fn test_swe_implement_has_full_tools() {
    let mode = borg_domains::swe::swe_mode();
    let implement = mode.get_phase("implement").unwrap();
    assert!(implement.allowed_tools.contains("Bash"));
    assert!(implement.allowed_tools.contains("Edit"));
    assert!(implement.include_task_context);
    assert!(implement.commits);
    assert!(implement.use_docker);
}

#[test]
fn test_swe_validate_loops_back_to_implement() {
    let mode = borg_domains::swe::swe_mode();
    let validate = mode.get_phase("validate").unwrap();
    assert_eq!(validate.phase_type, PhaseType::Validate);
    assert_eq!(validate.retry_phase, "implement");
    assert_eq!(validate.next, "lint_fix");
}

#[test]
fn test_web_mode_has_implement_validate_flow() {
    let mode = borg_domains::web::web_mode();
    assert_eq!(mode.name, "webborg");
    let names: Vec<&str> = mode.phases.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, &["backlog", "implement", "validate", "lint_fix", "rebase"]);
}

#[test]
fn test_legal_mode_has_implement_review() {
    let mode = borg_domains::legal::legal_mode();
    assert_eq!(mode.name, "lawborg");
    let names: Vec<&str> = mode.phases.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, &["backlog", "implement", "review"]);
}

#[test]
fn test_legal_review_is_fresh_session() {
    let mode = borg_domains::legal::legal_mode();
    let review = mode.get_phase("review").unwrap();
    assert!(review.fresh_session);
}

#[test]
fn test_sales_mode_has_implement_review() {
    let mode = borg_domains::sales::sales_mode();
    assert_eq!(mode.name, "salesborg");
    let names: Vec<&str> = mode.phases.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, &["backlog", "implement", "review"]);
}

#[test]
fn test_crew_mode_has_single_implement() {
    let mode = borg_domains::crew::crew_mode();
    assert_eq!(mode.name, "crewborg");
    let names: Vec<&str> = mode.phases.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, &["backlog", "implement"]);
}

#[test]
fn test_data_mode_has_single_implement() {
    let mode = borg_domains::data::data_mode();
    assert_eq!(mode.name, "databorg");
    let names: Vec<&str> = mode.phases.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, &["backlog", "implement"]);
}

#[test]
fn test_all_modes_have_backlog_first() {
    for mode in borg_domains::all_modes() {
        let first = &mode.phases[0];
        assert_eq!(first.name, "backlog", "mode {} must start with backlog", mode.name);
        assert_eq!(first.phase_type, PhaseType::Setup);
    }
}

#[test]
fn test_all_modes_first_agent_phase_has_task_context() {
    for mode in borg_domains::all_modes() {
        let first_agent = mode.phases.iter()
            .find(|p| p.phase_type == PhaseType::Agent)
            .unwrap_or_else(|| panic!("mode {} has no agent phase", mode.name));
        assert!(first_agent.include_task_context, "mode {} first agent phase must include task context", mode.name);
    }
}

#[test]
fn test_no_mode_uses_old_spec_qa_impl_phases() {
    for mode in borg_domains::all_modes() {
        for phase in &mode.phases {
            assert_ne!(phase.name, "spec", "mode {} still has spec phase", mode.name);
            assert_ne!(phase.name, "qa", "mode {} still has qa phase", mode.name);
            assert_ne!(phase.name, "qa_fix", "mode {} still has qa_fix phase", mode.name);
            assert_ne!(phase.name, "impl", "mode {} still has impl phase", mode.name);
        }
    }
}

#[test]
fn test_swe_signal_instructions_in_prompt() {
    let mode = borg_domains::swe::swe_mode();
    let implement = mode.get_phase("implement").unwrap();
    assert!(implement.instruction.contains("signal.json"));
    assert!(implement.instruction.contains("blocked"));
    assert!(implement.instruction.contains("abandon"));
}
