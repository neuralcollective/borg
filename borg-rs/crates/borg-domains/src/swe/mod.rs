use borg_core::types::{IntegrationType, PhaseConfig, PipelineMode, SeedConfig, SeedOutputType};

use crate::{agent_phase, lint_phase, rebase_phase, setup_phase, validate_phase};

pub fn swe_mode() -> PipelineMode {
    PipelineMode {
        name: "sweborg".into(),
        label: "Software Engineering".into(),
        category: "Engineering".into(),
        initial_status: "backlog".into(),
        uses_git_worktrees: true,
        uses_docker: true,
        uses_test_cmd: true,
        integration: IntegrationType::GitPr,
        default_max_attempts: 5,
        phases: vec![
            setup_phase("implement"),
            PhaseConfig {
                include_task_context: true,
                include_file_listing: true,
                error_instruction: SWE_IMPLEMENT_RETRY.into(),
                use_docker: true,
                commits: true,
                commit_message: "feat: implementation from borg agent".into(),
                ..agent_phase(
                    "implement",
                    "Implement",
                    SWE_IMPLEMENT_SYSTEM,
                    SWE_IMPLEMENT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,Bash",
                    "validate",
                )
            },
            validate_phase("implement", "lint_fix"),
            lint_phase("rebase"),
            rebase_phase(),
        ],
        seed_modes: swe_seeds(),
    }
}

pub(crate) fn swe_seeds() -> Vec<SeedConfig> {
    vec![
        SeedConfig {
            name: "refactoring".into(),
            label: "Refactoring".into(),
            output_type: SeedOutputType::Task,
            prompt: SEED_REFACTOR.into(),
            allowed_tools: String::new(),
            target_primary_repo: false,
        },
        SeedConfig {
            name: "security".into(),
            label: "Bug Audit".into(),
            output_type: SeedOutputType::Task,
            prompt: SEED_SECURITY.into(),
            allowed_tools: String::new(),
            target_primary_repo: false,
        },
        SeedConfig {
            name: "tests".into(),
            label: "Test Coverage".into(),
            output_type: SeedOutputType::Task,
            prompt: SEED_TESTS.into(),
            allowed_tools: String::new(),
            target_primary_repo: false,
        },
        SeedConfig {
            name: "features".into(),
            label: "Feature Discovery".into(),
            output_type: SeedOutputType::Proposal,
            prompt: SEED_FEATURES.into(),
            allowed_tools: String::new(),
            target_primary_repo: false,
        },
        SeedConfig {
            name: "architecture".into(),
            label: "Architecture Review".into(),
            output_type: SeedOutputType::Proposal,
            prompt: SEED_ARCHITECTURE.into(),
            allowed_tools: String::new(),
            target_primary_repo: false,
        },
        SeedConfig {
            name: "cross_pollinate".into(),
            label: "Cross-Pollinate".into(),
            output_type: SeedOutputType::Proposal,
            prompt: SEED_CROSS_POLLINATE.into(),
            allowed_tools: String::new(),
            target_primary_repo: true,
        },
    ]
}

// ── Prompt constants ─────────────────────────────────────────────────────

pub(crate) const SWE_IMPLEMENT_SYSTEM: &str = "\
You are an autonomous software engineering agent. You own the full lifecycle: \
understand the task, explore the codebase, write tests, implement, and iterate \
until everything works. You drive your own workflow — there is no separate spec \
or test-writing phase.";

pub(crate) const SWE_IMPLEMENT_INSTRUCTION: &str = "\
Implement the requested change end-to-end:
1. Explore the codebase to understand the relevant code, patterns, and conventions
2. Plan your approach — optionally write spec.md for your own reference
3. Write tests that cover the acceptance criteria and edge cases
4. Write the implementation to make all tests pass
5. Run the test suite yourself and iterate until green
6. Commit your changes with a descriptive message

Work iteratively — if tests fail, read the errors and fix them. If your initial \
approach doesn't work, try a different one. Verify file paths and APIs exist \
before building on them.

If the task is unclear or impossible, write {\"status\":\"blocked\",\"reason\":\"...\"} \
to .borg/signal.json. If you determine the task is already done or nonsensical, \
write {\"status\":\"abandon\",\"reason\":\"...\"} to .borg/signal.json.";

pub(crate) const SWE_IMPLEMENT_RETRY: &str = "\n\n\
Previous attempt failed. Test output:\n```\n{ERROR}\n```\n\
Analyze the failures and fix them. If your previous approach is fundamentally \
wrong, try a different one rather than repeating the same mistake.";

pub const SWE_REBASE_INSTRUCTION: &str = "\
This branch has merge conflicts with main.\n\
Rebase onto origin/main, resolve all conflicts, and ensure tests pass.";

pub const SWE_REBASE_ERROR: &str = "\n\nPrevious error context:\n```\n{ERROR}\n```";

pub const SWE_REBASE_FIX: &str = "\
The git rebase onto origin/main failed with conflicts:\n\n{ERROR}\n\n\
You are in the worktree where the rebase is paused. Resolve all conflicts:\n\
- For 'deleted by us' files (files removed from main): run `git rm <file>` for each one\n\
- For content conflicts (<<<< markers): edit the file to resolve, then `git add <file>`\n\
After resolving all conflicts, run `git rebase --continue`.\n\
Do NOT run `git rebase --abort`.";

// Keep SWE_WORKER_SYSTEM as pub for rebase_phase reference
pub const SWE_WORKER_SYSTEM: &str = SWE_IMPLEMENT_SYSTEM;

// ── Seed prompts ─────────────────────────────────────────────────────────

pub(crate) const SEED_REFACTOR: &str = "Identify 1-3 concrete, small improvements in code quality. Look for:\
\n- Dead code: unused functions, variables, imports, exports, or branches\
\n- Duplication: repeated logic that should be extracted or unified\
\n- Overly complex functions that do too much and should be split\
\n- Inconsistent naming, style, or conventions across the codebase\
\n- Stale, misleading, or redundant comments\
\n- Error handling that silently swallows failures\
\n- Magic numbers or strings that should be named constants\
\n\nEach task should be self-contained and safe to merge independently.\
\nDo not suggest new features. Skip cosmetic-only changes with no real benefit.\
\nOnly target code that actually exists. Verify file paths and function names by reading the source.";

pub(crate) const SEED_SECURITY: &str =
    "Audit for bugs, security vulnerabilities, and reliability issues. Look for:\
\n- Race conditions and unsafe concurrent access\
\n- Resource leaks: memory, file handles, connections not released on all paths\
\n- Silenced errors (empty catch blocks, ignored return values)\
\n- Integer overflows, slice out-of-bounds, or unchecked casts\
\n- Injection vulnerabilities: unsanitised input passed to shell, SQL, or paths\
\n- Logic errors: off-by-one, wrong operator, inverted condition\
\n- Type safety gaps: unsafe casts, missing null checks, wrong assumptions\
\n- Undefined behaviour that passes tests but can corrupt state\
\n\nCreate a task for each real, confirmed issue. Skip false positives and\
\ntheoretical risks that have no realistic exploit path.\
\nOnly target code that actually exists. Verify file paths and function names by reading the source.";

pub(crate) const SEED_TESTS: &str = "Identify gaps in test coverage that matter for correctness. Look for:\
\n- Core logic with no tests at all\
\n- Edge cases not covered: empty input, zero, max values, error paths\
\n- Functions that are tested only via integration, never in isolation\
\n- Recent changes or complex code paths with no regression tests\
\n\nIMPORTANT: Only target functions, types, and fields that ALREADY EXIST in the\
\ncodebase. Verify by reading the actual source. Do not suggest tests for\
\nhypothetical or planned features.\
\n\nEach task should target a specific function or module with a clear\
\ndescription of what cases to cover and why they matter. Skip trivial\
\ngetters, boilerplate, and tests that would only assert mocks.";

pub(crate) const SEED_FEATURES: &str =
    "Suggest 1-3 concrete features that would meaningfully improve this project.\
\nBase your suggestions on actual gaps you found while exploring the code.";

pub(crate) const SEED_ARCHITECTURE: &str =
    "Identify 1-2 significant structural improvements. Think big: module\
\nreorganization, API redesigns, performance overhauls, major refactors\
\nthat span multiple files, or replacing approaches that have outgrown\
\ntheir original design.\
\n\nEach proposal should be a multi-day project, not a quick fix.";

pub(crate) const SEED_CROSS_POLLINATE: &str =
    "Study this codebase to understand its patterns, features, and architecture.\
\nThen suggest 1-3 ideas inspired by what you see here that could be adapted\
\nor ported to a DIFFERENT project (not this one).\
\n\nFocus on: elegant abstractions worth copying, clever approaches to common\
\nproblems, architectural patterns that solve hard problems well.\
\n\nOutput proposals — each one describes a concrete improvement to apply\
\nelsewhere, inspired by what you found here.";
