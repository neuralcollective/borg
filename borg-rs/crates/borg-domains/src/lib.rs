pub mod crew;
pub mod data;
pub mod legal;
pub mod sales;
pub mod swe;
pub mod web;

use borg_core::types::{PhaseConfig, PhaseType, PipelineMode};

/// Return all built-in pipeline modes from every domain.
pub fn all_modes() -> Vec<PipelineMode> {
    vec![
        swe::swe_mode(),
        legal::legal_mode(),
        web::web_mode(),
        crew::crew_mode(),
        sales::sales_mode(),
        data::data_mode(),
    ]
}

/// Look up a built-in mode by name (with backward-compat aliases).
pub fn get_mode(name: &str) -> Option<PipelineMode> {
    match name {
        "swe" => get_mode("sweborg"),
        "legal" => get_mode("lawborg"),
        _ => all_modes().into_iter().find(|m| m.name == name),
    }
}

// ── Shared phase builders ────────────────────────────────────────────────

/// Create a backlog/setup phase that transitions immediately to the first agent phase.
pub(crate) fn setup_phase(next: &str) -> PhaseConfig {
    PhaseConfig {
        name: "backlog".into(),
        label: "Backlog".into(),
        phase_type: PhaseType::Setup,
        next: next.into(),
        ..Default::default()
    }
}

/// Create a standard agent phase with the six most common fields.
pub(crate) fn agent_phase(
    name: &str,
    label: &str,
    system: &str,
    instruction: &str,
    tools: &str,
    next: &str,
) -> PhaseConfig {
    PhaseConfig {
        name: name.into(),
        label: label.into(),
        system_prompt: system.into(),
        instruction: instruction.into(),
        allowed_tools: tools.into(),
        next: next.into(),
        ..Default::default()
    }
}

/// Create a lint_fix phase.
pub(crate) fn lint_phase(next: &str) -> PhaseConfig {
    PhaseConfig {
        name: "lint_fix".into(),
        label: "Lint".into(),
        phase_type: PhaseType::LintFix,
        allow_no_changes: true,
        next: next.into(),
        ..Default::default()
    }
}

/// Create a validate phase that runs tests/compilation and loops back on failure.
pub(crate) fn validate_phase(retry_phase: &str, next: &str) -> PhaseConfig {
    PhaseConfig {
        name: "validate".into(),
        label: "Validate".into(),
        phase_type: PhaseType::Validate,
        retry_phase: retry_phase.into(),
        next: next.into(),
        ..Default::default()
    }
}

/// Create a standard rebase phase (shared across sweborg/webborg).
pub(crate) fn rebase_phase() -> PhaseConfig {
    PhaseConfig {
        name: "rebase".into(),
        label: "Rebase".into(),
        phase_type: PhaseType::Rebase,
        system_prompt: swe::SWE_WORKER_SYSTEM.into(),
        instruction: swe::SWE_REBASE_INSTRUCTION.into(),
        error_instruction: swe::SWE_REBASE_ERROR.into(),
        allowed_tools: "Read,Glob,Grep,Write,Edit,Bash".into(),
        fix_instruction: swe::SWE_REBASE_FIX.into(),
        next: "done".into(),
        ..Default::default()
    }
}
