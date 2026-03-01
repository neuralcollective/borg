use borg_core::types::{IntegrationType, PhaseConfig, PipelineMode, SeedConfig, SeedOutputType};

use crate::{agent_phase, lint_phase, rebase_phase, setup_phase};

pub fn swe_mode() -> PipelineMode {
    const IMPL_TOOLS: &str = "Read,Glob,Grep,Write,Edit,Bash";
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
            setup_phase("spec"),
            PhaseConfig {
                include_task_context: true,
                include_file_listing: true,
                check_artifact: Some("spec.md".into()),
                use_docker: true,
                ..agent_phase(
                    "spec",
                    "Specification",
                    SWE_SPEC_SYSTEM,
                    SWE_SPEC_INSTRUCTION,
                    "Read,Glob,Grep,Write",
                    "qa",
                )
            },
            PhaseConfig {
                use_docker: true,
                commits: true,
                commit_message: "test: add tests from QA agent".into(),
                allow_no_changes: true,
                compile_check: true,
                ..agent_phase(
                    "qa",
                    "Testing",
                    SWE_QA_SYSTEM,
                    SWE_QA_INSTRUCTION,
                    "Read,Glob,Grep,Write",
                    "impl",
                )
            },
            PhaseConfig {
                error_instruction: SWE_QA_FIX_ERROR.into(),
                use_docker: true,
                commits: true,
                commit_message: "test: fix tests from QA agent".into(),
                allow_no_changes: true,
                fresh_session: true,
                ..agent_phase(
                    "qa_fix",
                    "Test Fix",
                    SWE_QA_SYSTEM,
                    SWE_QA_INSTRUCTION,
                    "Read,Glob,Grep,Write",
                    "impl",
                )
            },
            PhaseConfig {
                error_instruction: SWE_IMPL_RETRY.into(),
                use_docker: true,
                commits: true,
                commit_message: "impl: implementation from worker agent".into(),
                runs_tests: true,
                has_qa_fix_routing: true,
                ..agent_phase(
                    "impl",
                    "Implementation",
                    SWE_WORKER_SYSTEM,
                    SWE_IMPL_INSTRUCTION,
                    IMPL_TOOLS,
                    "lint_fix",
                )
            },
            PhaseConfig {
                error_instruction: SWE_IMPL_RETRY.into(),
                use_docker: true,
                commits: true,
                commit_message: "impl: implementation from worker agent".into(),
                runs_tests: true,
                has_qa_fix_routing: true,
                ..agent_phase(
                    "retry",
                    "Retry",
                    SWE_WORKER_SYSTEM,
                    SWE_IMPL_INSTRUCTION,
                    IMPL_TOOLS,
                    "lint_fix",
                )
            },
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

pub(crate) const SWE_SPEC_SYSTEM: &str = "You are the spec-writing agent in an autonomous engineering pipeline.\nRead the task and codebase, then write spec.md at the repository root.\nDo not modify source files.";

pub(crate) const SWE_QA_SYSTEM: &str = "You are the test-writing agent in an autonomous engineering pipeline.\nRead spec.md and write test files only.\nDo not write implementation code or modify non-test files.";

pub const SWE_WORKER_SYSTEM: &str = "You are the implementation agent in an autonomous engineering pipeline.\nRead spec.md and tests, write code to make all tests pass.\nPrefer not to modify test files, but if tests reference APIs or types that don't exist in the codebase, fix the tests to match reality before implementing.";

pub(crate) const SWE_SPEC_INSTRUCTION: &str = "Write spec.md containing:\n1. Task summary (2-3 sentences)\n2. Files to modify and create (exact paths — verify each exists with Glob)\n3. Function/type signatures for new or changed code (verify existing ones with Grep)\n4. Acceptance criteria (testable assertions)\n5. Edge cases\n\nBefore finalizing: verify every file path you reference actually exists (unless it's a new file to create). Verify every function or type you reference is real. Remove any references to code that doesn't exist.";

pub(crate) const SWE_QA_INSTRUCTION: &str = "Read spec.md and write test files covering every acceptance criterion.\nOnly create/modify test files (*_test.* or tests/ directory).\nTests should FAIL initially since features are not yet implemented.\n\nBefore writing tests, verify that the APIs and types referenced in spec.md actually exist in the codebase. If spec.md references something that doesn't exist, write tests against the real API instead.";

pub(crate) const SWE_QA_FIX_ERROR: &str = "\n\nYour tests from the previous QA pass have issues that prevent them from passing.\nThe implementation agent tried multiple times but the test code itself is broken.\n\nTest output showing the failures:\n```\n{ERROR}\n```\n\nFix the test files. Common issues:\n- Tests reference functions, types, or fields that don't exist in the codebase\n- Compile errors from wrong API assumptions\n- use-after-free in test setup, wrong allocator usage\n- Missing defer/errdefer, incorrect test assertions\nDo NOT weaken tests or remove test cases — fix the test code so it correctly\nvalidates the behavior described in spec.md, using only APIs that actually exist.";

pub(crate) const SWE_IMPL_INSTRUCTION: &str = "Read spec.md and the test files.\nWrite implementation code that makes all tests pass.\nPrefer to only modify files listed in spec.md.\nIf tests reference APIs, types, or fields that don't exist in the codebase, fix them to match reality — keep the test intent but correct wrong API assumptions.";

pub(crate) const SWE_IMPL_RETRY: &str =
    "\n\nPrevious attempt failed. Test output:\n```\n{ERROR}\n```\nFix the failures.";

pub const SWE_REBASE_INSTRUCTION: &str = "This branch has merge conflicts with main.\nRebase onto origin/main, resolve all conflicts, and ensure tests pass.\nRead spec.md for context on what this branch does.";

pub const SWE_REBASE_ERROR: &str = "\n\nPrevious error context:\n```\n{ERROR}\n```";

pub const SWE_REBASE_FIX: &str = "The git rebase onto origin/main failed with conflicts:\n\n{ERROR}\n\nYou are in the worktree where the rebase is paused. Resolve all conflicts:\n- For 'deleted by us' files (files removed from main): run `git rm <file>` for each one\n- For content conflicts (<<<< markers): edit the file to resolve, then `git add <file>`\nAfter resolving all conflicts, run `git rebase --continue`.\nDo NOT run `git rebase --abort`.";

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
