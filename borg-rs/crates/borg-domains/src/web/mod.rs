use borg_core::types::{IntegrationType, PhaseConfig, PipelineMode, SeedConfig, SeedOutputType};

use crate::{agent_phase, lint_phase, rebase_phase, setup_phase, validate_phase};
use crate::swe::swe_seeds;

pub fn web_mode() -> PipelineMode {
    PipelineMode {
        name: "webborg".into(),
        label: "Frontend".into(),
        category: "Engineering".into(),
        initial_status: "backlog".into(),
        uses_git_worktrees: true,
        uses_docker: true,
        uses_test_cmd: true,
        integration: IntegrationType::GitPr,
        default_max_attempts: 3,
        phases: vec![
            setup_phase("implement"),
            PhaseConfig {
                include_task_context: true,
                include_file_listing: true,
                error_instruction: WEB_IMPLEMENT_RETRY.into(),
                use_docker: true,
                commits: true,
                commit_message: "feat: frontend improvements from web agent".into(),
                ..agent_phase(
                    "implement",
                    "Implement",
                    WEB_IMPLEMENT_SYSTEM,
                    WEB_IMPLEMENT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,Bash",
                    "validate",
                )
            },
            validate_phase("implement", "lint_fix"),
            lint_phase("rebase"),
            rebase_phase(),
        ],
        seed_modes: {
            let mut seeds = swe_seeds();
            seeds.splice(0..0, [
                SeedConfig {
                    name: "performance".into(),
                    label: "Performance".into(),
                    output_type: SeedOutputType::Task,
                    prompt: "Analyze the web app for performance issues.".into(),
                    allowed_tools: String::new(),
                    target_primary_repo: false,
                },
                SeedConfig {
                    name: "visual".into(),
                    label: "Visual Polish".into(),
                    output_type: SeedOutputType::Task,
                    prompt: "Review the UI for visual inconsistencies and polish opportunities.".into(),
                    allowed_tools: String::new(),
                    target_primary_repo: false,
                },
                SeedConfig {
                    name: "accessibility".into(),
                    label: "Accessibility".into(),
                    output_type: SeedOutputType::Task,
                    prompt: "Audit the web app for accessibility issues.".into(),
                    allowed_tools: String::new(),
                    target_primary_repo: false,
                },
                SeedConfig {
                    name: "ux".into(),
                    label: "UX Improvements".into(),
                    output_type: SeedOutputType::Proposal,
                    prompt: "Identify 1-3 user experience improvements that would meaningfully reduce friction.".into(),
                    allowed_tools: String::new(),
                    target_primary_repo: false,
                },
            ]);
            seeds
        },
    }
}

const WEB_IMPLEMENT_SYSTEM: &str = "\
You are an autonomous frontend engineering agent. Analyze the web application, \
identify improvements, and implement them end-to-end. Focus on measurable wins: \
faster loads, better visuals, improved accessibility, and better UX.";

const WEB_IMPLEMENT_INSTRUCTION: &str = "\
Implement the requested frontend changes end-to-end:
1. Audit the current state (tech stack, components, pain points)
2. Plan targeted improvements — write audit.md for your own reference if helpful
3. Implement changes with surgical, focused edits
4. Verify changes compile/build correctly
5. Commit your changes with a descriptive message

If the task is unclear or impossible, write {\"status\":\"blocked\",\"reason\":\"...\"} \
to .borg/signal.json.";

const WEB_IMPLEMENT_RETRY: &str =
    "\n\nPrevious attempt failed. Error output:\n```\n{ERROR}\n```\nFix the issue.";
