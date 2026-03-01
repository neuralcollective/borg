use borg_core::types::{IntegrationType, PhaseConfig, PipelineMode, SeedConfig, SeedOutputType};

use crate::{agent_phase, lint_phase, rebase_phase, setup_phase};
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
            setup_phase("audit"),
            PhaseConfig {
                include_task_context: true,
                include_file_listing: true,
                check_artifact: Some("audit.md".into()),
                use_docker: true,
                ..agent_phase(
                    "audit",
                    "Audit",
                    WEB_AUDIT_SYSTEM,
                    WEB_AUDIT_INSTRUCTION,
                    "Read,Glob,Grep,Write",
                    "improve",
                )
            },
            PhaseConfig {
                error_instruction: WEB_IMPROVE_RETRY.into(),
                use_docker: true,
                commits: true,
                commit_message: "improve: frontend improvements from web agent".into(),
                runs_tests: true,
                ..agent_phase(
                    "improve",
                    "Improve",
                    WEB_IMPROVE_SYSTEM,
                    WEB_IMPROVE_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,Bash",
                    "lint_fix",
                )
            },
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

const WEB_AUDIT_SYSTEM: &str = "You are a frontend performance and UX expert in an autonomous web improvement pipeline.\nAnalyze the web application codebase and identify concrete opportunities to improve\nperformance, visual design, accessibility, and user experience.\nWrite your findings to audit.md at the repository root. Do not modify source files.";

const WEB_AUDIT_INSTRUCTION: &str = "Write audit.md containing:\n1. Current state summary (tech stack, key components)\n2. Performance issues (bundle size, render bottlenecks, missing lazy loading)\n3. Visual and UX improvements (layout, spacing, typography, responsiveness)\n4. Accessibility gaps (contrast, keyboard navigation, ARIA)\n5. Prioritized action items — concrete, targeted changes for this iteration";

const WEB_IMPROVE_SYSTEM: &str = "You are a frontend performance and UX expert in an autonomous web improvement pipeline.\nRead audit.md and implement the prioritized improvements.\nFocus on measurable wins: faster loads, better visuals, improved UX.\nDo not modify audit.md.";

const WEB_IMPROVE_INSTRUCTION: &str = "Read audit.md and implement the action items listed under \"Prioritized action items\".\nMake targeted, surgical edits. Verify changes compile/build correctly.";

const WEB_IMPROVE_RETRY: &str =
    "\n\nPrevious attempt failed. Error output:\n```\n{ERROR}\n```\nFix the issue.";
