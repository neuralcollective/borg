use crate::types::{IntegrationType, PhaseConfig, PhaseType, PipelineMode, SeedConfig, SeedOutputType};

pub fn all_modes() -> Vec<PipelineMode> {
    vec![swe_mode(), legal_mode(), web_mode()]
}

pub fn get_mode(name: &str) -> Option<PipelineMode> {
    match name {
        // Backward-compat aliases
        "swe" => get_mode("sweborg"),
        "legal" => get_mode("lawborg"),
        _ => all_modes().into_iter().find(|m| m.name == name),
    }
}

pub fn swe_mode() -> PipelineMode {
    PipelineMode {
        name: "sweborg".into(),
        label: "Software Engineering".into(),
        initial_status: "backlog".into(),
        uses_git_worktrees: true,
        uses_docker: true,
        uses_test_cmd: true,
        integration: IntegrationType::GitPr,
        default_max_attempts: 5,
        phases: vec![
            PhaseConfig {
                name: "backlog".into(),
                label: "Backlog".into(),
                phase_type: PhaseType::Setup,
                next: "spec".into(),
                priority: 60,
                ..default_phase()
            },
            PhaseConfig {
                name: "spec".into(),
                label: "Specification".into(),
                system_prompt: SWE_SPEC_SYSTEM.into(),
                instruction: SWE_SPEC_INSTRUCTION.into(),
                include_task_context: true,
                include_file_listing: true,
                check_artifact: Some("spec.md".into()),
                use_docker: true,
                next: "qa".into(),
                priority: 50,
                ..default_phase()
            },
            PhaseConfig {
                name: "qa".into(),
                label: "Testing".into(),
                system_prompt: SWE_QA_SYSTEM.into(),
                instruction: SWE_QA_INSTRUCTION.into(),
                use_docker: true,
                commits: true,
                commit_message: "test: add tests from QA agent".into(),
                allow_no_changes: true,
                next: "impl".into(),
                priority: 30,
                ..default_phase()
            },
            PhaseConfig {
                name: "qa_fix".into(),
                label: "Test Fix".into(),
                system_prompt: SWE_QA_SYSTEM.into(),
                instruction: SWE_QA_INSTRUCTION.into(),
                error_instruction: SWE_QA_FIX_ERROR.into(),
                use_docker: true,
                commits: true,
                commit_message: "test: fix tests from QA agent".into(),
                allow_no_changes: true,
                fresh_session: true,
                next: "impl".into(),
                priority: 25,
                ..default_phase()
            },
            PhaseConfig {
                name: "impl".into(),
                label: "Implementation".into(),
                system_prompt: SWE_WORKER_SYSTEM.into(),
                instruction: SWE_IMPL_INSTRUCTION.into(),
                error_instruction: SWE_IMPL_RETRY.into(),
                allowed_tools: "Read,Glob,Grep,Write,Edit,Bash".into(),
                use_docker: true,
                commits: true,
                commit_message: "impl: implementation from worker agent".into(),
                runs_tests: true,
                has_qa_fix_routing: true,
                next: "lint_fix".into(),
                priority: 10,
                ..default_phase()
            },
            PhaseConfig {
                name: "retry".into(),
                label: "Retry".into(),
                system_prompt: SWE_WORKER_SYSTEM.into(),
                instruction: SWE_IMPL_INSTRUCTION.into(),
                error_instruction: SWE_IMPL_RETRY.into(),
                allowed_tools: "Read,Glob,Grep,Write,Edit,Bash".into(),
                use_docker: true,
                commits: true,
                commit_message: "impl: implementation from worker agent".into(),
                runs_tests: true,
                has_qa_fix_routing: true,
                next: "lint_fix".into(),
                priority: 8,
                ..default_phase()
            },
            PhaseConfig {
                name: "lint_fix".into(),
                label: "Lint".into(),
                phase_type: PhaseType::LintFix,
                next: "rebase".into(),
                allow_no_changes: true,
                priority: 7,
                ..default_phase()
            },
            PhaseConfig {
                name: "rebase".into(),
                label: "Rebase".into(),
                phase_type: PhaseType::Rebase,
                system_prompt: SWE_WORKER_SYSTEM.into(),
                instruction: SWE_REBASE_INSTRUCTION.into(),
                error_instruction: SWE_REBASE_ERROR.into(),
                allowed_tools: "Read,Glob,Grep,Write,Edit,Bash".into(),
                fix_instruction: SWE_REBASE_FIX.into(),
                fix_error_instruction: SWE_REBASE_FIX_ERROR.into(),
                next: "done".into(),
                priority: 5,
                ..default_phase()
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "refactoring".into(),
                label: "Refactoring".into(),
                output_type: SeedOutputType::Task,
                prompt: SEED_REFACTOR.into(),
            },
            SeedConfig {
                name: "security".into(),
                label: "Bug Audit".into(),
                output_type: SeedOutputType::Task,
                prompt: SEED_SECURITY.into(),
            },
            SeedConfig {
                name: "tests".into(),
                label: "Test Coverage".into(),
                output_type: SeedOutputType::Task,
                prompt: SEED_TESTS.into(),
            },
            SeedConfig {
                name: "features".into(),
                label: "Feature Discovery".into(),
                output_type: SeedOutputType::Proposal,
                prompt: SEED_FEATURES.into(),
            },
            SeedConfig {
                name: "architecture".into(),
                label: "Architecture Review".into(),
                output_type: SeedOutputType::Proposal,
                prompt: SEED_ARCHITECTURE.into(),
            },
        ],
    }
}

pub fn legal_mode() -> PipelineMode {
    PipelineMode {
        name: "lawborg".into(),
        label: "Legal".into(),
        initial_status: "backlog".into(),
        uses_git_worktrees: true,
        uses_docker: false,
        uses_test_cmd: false,
        integration: IntegrationType::None,
        default_max_attempts: 3,
        phases: vec![
            PhaseConfig {
                name: "backlog".into(),
                label: "Backlog".into(),
                phase_type: PhaseType::Setup,
                next: "research".into(),
                priority: 60,
                ..default_phase()
            },
            PhaseConfig {
                name: "research".into(),
                label: "Research".into(),
                system_prompt: LEGAL_RESEARCH_SYSTEM.into(),
                instruction: LEGAL_RESEARCH_INSTRUCTION.into(),
                allowed_tools: "Read,Glob,Grep,Write,WebSearch,WebFetch".into(),
                include_task_context: true,
                include_file_listing: true,
                check_artifact: Some("research.md".into()),
                next: "draft".into(),
                priority: 40,
                ..default_phase()
            },
            PhaseConfig {
                name: "draft".into(),
                label: "Drafting".into(),
                system_prompt: LEGAL_DRAFT_SYSTEM.into(),
                instruction: LEGAL_DRAFT_INSTRUCTION.into(),
                allowed_tools: "Read,Glob,Grep,Write,Edit".into(),
                commits: true,
                commit_message: "draft: legal document from drafting agent".into(),
                next: "review".into(),
                priority: 30,
                ..default_phase()
            },
            PhaseConfig {
                name: "review".into(),
                label: "Review".into(),
                system_prompt: LEGAL_REVIEW_SYSTEM.into(),
                instruction: LEGAL_REVIEW_INSTRUCTION.into(),
                error_instruction: LEGAL_REVIEW_RETRY.into(),
                allowed_tools: "Read,Glob,Grep,Write,Edit".into(),
                commits: true,
                commit_message: "review: revisions from review agent".into(),
                next: "done".into(),
                priority: 10,
                ..default_phase()
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "clause_review".into(),
                label: "Clause Review".into(),
                output_type: SeedOutputType::Task,
                prompt: "Review the legal documents in this repository. Identify 1-3 specific\nclauses, provisions, or terms that could be improved, clarified, or\nthat pose legal risk. Focus on practical, actionable improvements.".into(),
            },
            SeedConfig {
                name: "compliance".into(),
                label: "Compliance Audit".into(),
                output_type: SeedOutputType::Task,
                prompt: "Audit the documents for compliance gaps against relevant regulations,\nstandards, and best practices. Create a task for each genuine compliance\nissue found. Be specific about the regulation or standard being violated.".into(),
            },
            SeedConfig {
                name: "precedent".into(),
                label: "Precedent Research".into(),
                output_type: SeedOutputType::Proposal,
                prompt: "Analyze the legal matters addressed in this repository. Suggest 1-3\nresearch directions for relevant case law, statutory authority, or\nregulatory guidance that could strengthen the legal positions taken.".into(),
            },
            SeedConfig {
                name: "risk".into(),
                label: "Risk Assessment".into(),
                output_type: SeedOutputType::Proposal,
                prompt: "Perform a risk assessment of the legal documents and matters. Identify\n1-2 significant legal risks, exposure areas, or positions that could\nbe challenged. Focus on material risks, not theoretical ones.".into(),
            },
        ],
    }
}

pub fn web_mode() -> PipelineMode {
    PipelineMode {
        name: "webborg".into(),
        label: "Frontend".into(),
        initial_status: "backlog".into(),
        uses_git_worktrees: true,
        uses_docker: true,
        uses_test_cmd: true,
        integration: IntegrationType::GitPr,
        default_max_attempts: 3,
        phases: vec![
            PhaseConfig {
                name: "backlog".into(),
                label: "Backlog".into(),
                phase_type: PhaseType::Setup,
                next: "audit".into(),
                priority: 60,
                ..default_phase()
            },
            PhaseConfig {
                name: "audit".into(),
                label: "Audit".into(),
                system_prompt: WEB_AUDIT_SYSTEM.into(),
                instruction: WEB_AUDIT_INSTRUCTION.into(),
                include_task_context: true,
                include_file_listing: true,
                check_artifact: Some("audit.md".into()),
                use_docker: true,
                next: "improve".into(),
                priority: 50,
                ..default_phase()
            },
            PhaseConfig {
                name: "improve".into(),
                label: "Improve".into(),
                system_prompt: WEB_IMPROVE_SYSTEM.into(),
                instruction: WEB_IMPROVE_INSTRUCTION.into(),
                error_instruction: WEB_IMPROVE_RETRY.into(),
                allowed_tools: "Read,Glob,Grep,Write,Edit,Bash".into(),
                use_docker: true,
                commits: true,
                commit_message: "improve: frontend improvements from web agent".into(),
                runs_tests: true,
                next: "lint_fix".into(),
                priority: 10,
                ..default_phase()
            },
            PhaseConfig {
                name: "lint_fix".into(),
                label: "Lint".into(),
                phase_type: PhaseType::LintFix,
                next: "rebase".into(),
                allow_no_changes: true,
                priority: 7,
                ..default_phase()
            },
            PhaseConfig {
                name: "rebase".into(),
                label: "Rebase".into(),
                phase_type: PhaseType::Rebase,
                system_prompt: SWE_WORKER_SYSTEM.into(),
                instruction: SWE_REBASE_INSTRUCTION.into(),
                error_instruction: SWE_REBASE_ERROR.into(),
                allowed_tools: "Read,Glob,Grep,Write,Edit,Bash".into(),
                fix_instruction: SWE_REBASE_FIX.into(),
                fix_error_instruction: SWE_REBASE_FIX_ERROR.into(),
                next: "done".into(),
                priority: 5,
                ..default_phase()
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "performance".into(),
                label: "Performance".into(),
                output_type: SeedOutputType::Task,
                prompt: "Analyze the web app for performance issues.".into(),
            },
            SeedConfig {
                name: "visual".into(),
                label: "Visual Polish".into(),
                output_type: SeedOutputType::Task,
                prompt: "Review the UI for visual inconsistencies and polish opportunities.".into(),
            },
            SeedConfig {
                name: "accessibility".into(),
                label: "Accessibility".into(),
                output_type: SeedOutputType::Task,
                prompt: "Audit the web app for accessibility issues.".into(),
            },
            SeedConfig {
                name: "ux".into(),
                label: "UX Improvements".into(),
                output_type: SeedOutputType::Proposal,
                prompt: "Identify 1-3 user experience improvements that would meaningfully reduce friction.".into(),
            },
        ],
    }
}

fn default_phase() -> PhaseConfig {
    PhaseConfig::default()
}

// ── Prompt constants ─────────────────────────────────────────────────────

const SWE_SPEC_SYSTEM: &str = "You are the spec-writing agent in an autonomous engineering pipeline.\nRead the task and codebase, then write spec.md at the repository root.\nDo not modify source files.";

const SWE_QA_SYSTEM: &str = "You are the test-writing agent in an autonomous engineering pipeline.\nRead spec.md and write test files only.\nDo not write implementation code or modify non-test files.";

const SWE_WORKER_SYSTEM: &str = "You are the implementation agent in an autonomous engineering pipeline.\nRead spec.md and tests, write code to make all tests pass.\nDo not modify test files.";

const SWE_SPEC_INSTRUCTION: &str = "Write spec.md containing:\n1. Task summary (2-3 sentences)\n2. Files to modify and create (exact paths)\n3. Function/type signatures for new or changed code\n4. Acceptance criteria (testable assertions)\n5. Edge cases";

const SWE_QA_INSTRUCTION: &str = "Read spec.md and write test files covering every acceptance criterion.\nOnly create/modify test files (*_test.* or tests/ directory).\nTests should FAIL initially since features are not yet implemented.";

const SWE_QA_FIX_ERROR: &str = "\n\nYour tests from the previous QA pass have bugs that prevent them from passing.\nThe implementation agent tried multiple times but the test code itself is broken.\n\nTest output showing the failures:\n```\n{ERROR}\n```\n\nFix the test files. Common issues: use-after-free in test setup, wrong allocator\nusage, compile errors, missing defer/errdefer, incorrect test assertions.\nDo NOT weaken tests or remove test cases — fix the test code so it correctly\nvalidates the behavior described in spec.md.";

const SWE_IMPL_INSTRUCTION: &str = "Read spec.md and the test files.\nWrite implementation code that makes all tests pass.\nOnly modify files listed in spec.md. Do not modify test files.";

const SWE_IMPL_RETRY: &str = "\n\nPrevious attempt failed. Test output:\n```\n{ERROR}\n```\nFix the failures.";

const SWE_REBASE_INSTRUCTION: &str = "This branch has merge conflicts with main.\nRebase onto origin/main, resolve all conflicts, and ensure tests pass.\nRead spec.md for context on what this branch does.";

const SWE_REBASE_ERROR: &str = "\n\nPrevious error context:\n```\n{ERROR}\n```";

const SWE_REBASE_FIX: &str = "The branch was rebased onto origin/main successfully, but tests now fail.\nFix the code so tests pass. Read spec.md for context on what this branch does.\nRun the test command to verify your fix before finishing.";

const SWE_REBASE_FIX_ERROR: &str = "\n\nTest output:\n```\n{ERROR}\n```";

const LEGAL_RESEARCH_SYSTEM: &str = "You are the research agent in an autonomous legal pipeline.\nAnalyze the legal issue, research relevant law, precedent, and context,\nthen produce a research memo (research.md) at the workspace root.\nDo not draft legal documents yet — focus on thorough analysis.";

const LEGAL_RESEARCH_INSTRUCTION: &str = "Write research.md containing:\n1. Issue summary (2-3 sentences)\n2. Relevant statutes, regulations, and rules\n3. Key case law and precedent (with citations)\n4. Analysis of how the law applies to this matter\n5. Open questions and areas requiring further research";

const LEGAL_DRAFT_SYSTEM: &str = "You are the drafting agent in an autonomous legal pipeline.\nRead research.md and draft the requested legal document.\nFocus on accuracy, completeness, and proper legal formatting.\nCite sources from the research memo where applicable.";

const LEGAL_DRAFT_INSTRUCTION: &str = "Read research.md and draft the legal document described in the task.\nFollow standard legal formatting conventions.\nInclude all necessary sections, clauses, and provisions.\nCite relevant authority from the research memo.";

const LEGAL_REVIEW_SYSTEM: &str = "You are the review agent in an autonomous legal pipeline.\nReview the drafted document for legal accuracy, completeness, and quality.\nFix any issues you find directly in the document.";

const LEGAL_REVIEW_INSTRUCTION: &str = "Review all documents in the workspace for:\n1. Legal accuracy and correctness\n2. Completeness — all required sections present\n3. Internal consistency\n4. Proper citations and references\n5. Potential risks or weaknesses\n6. Formatting and style\n\nFix any issues directly. Do not just list problems — resolve them.";

const LEGAL_REVIEW_RETRY: &str = "\n\nPrevious review found unresolved issues:\n{ERROR}\n\nAddress these issues in the document.";

const WEB_AUDIT_SYSTEM: &str = "You are a frontend performance and UX expert in an autonomous web improvement pipeline.\nAnalyze the web application codebase and identify concrete opportunities to improve\nperformance, visual design, accessibility, and user experience.\nWrite your findings to audit.md at the repository root. Do not modify source files.";

const WEB_AUDIT_INSTRUCTION: &str = "Write audit.md containing:\n1. Current state summary (tech stack, key components)\n2. Performance issues (bundle size, render bottlenecks, missing lazy loading)\n3. Visual and UX improvements (layout, spacing, typography, responsiveness)\n4. Accessibility gaps (contrast, keyboard navigation, ARIA)\n5. Prioritized action items — concrete, targeted changes for this iteration";

const WEB_IMPROVE_SYSTEM: &str = "You are a frontend performance and UX expert in an autonomous web improvement pipeline.\nRead audit.md and implement the prioritized improvements.\nFocus on measurable wins: faster loads, better visuals, improved UX.\nDo not modify audit.md.";

const WEB_IMPROVE_INSTRUCTION: &str = "Read audit.md and implement the action items listed under \"Prioritized action items\".\nMake targeted, surgical edits. Verify changes compile/build correctly.";

const WEB_IMPROVE_RETRY: &str = "\n\nPrevious attempt failed. Error output:\n```\n{ERROR}\n```\nFix the issue.";

const SEED_REFACTOR: &str = "Identify 1-3 concrete, small improvements in code quality. Look for:\
\n- Dead code: unused functions, variables, imports, exports, or branches\
\n- Duplication: repeated logic that should be extracted or unified\
\n- Overly complex functions that do too much and should be split\
\n- Inconsistent naming, style, or conventions across the codebase\
\n- Stale, misleading, or redundant comments\
\n- Error handling that silently swallows failures\
\n- Magic numbers or strings that should be named constants\
\n\nEach task should be self-contained and safe to merge independently.\
\nDo not suggest new features. Skip cosmetic-only changes with no real benefit.";

const SEED_SECURITY: &str = "Audit for bugs, security vulnerabilities, and reliability issues. Look for:\
\n- Race conditions and unsafe concurrent access\
\n- Resource leaks: memory, file handles, connections not released on all paths\
\n- Silenced errors (empty catch blocks, ignored return values)\
\n- Integer overflows, slice out-of-bounds, or unchecked casts\
\n- Injection vulnerabilities: unsanitised input passed to shell, SQL, or paths\
\n- Logic errors: off-by-one, wrong operator, inverted condition\
\n- Type safety gaps: unsafe casts, missing null checks, wrong assumptions\
\n- Undefined behaviour that passes tests but can corrupt state\
\n\nCreate a task for each real, confirmed issue. Skip false positives and\
\ntheoretical risks that have no realistic exploit path.";

const SEED_TESTS: &str = "Identify gaps in test coverage that matter for correctness. Look for:\
\n- Core logic with no tests at all\
\n- Edge cases not covered: empty input, zero, max values, error paths\
\n- Functions that are tested only via integration, never in isolation\
\n- Recent changes or complex code paths with no regression tests\
\n\nEach task should target a specific function or module with a clear\
\ndescription of what cases to cover and why they matter. Skip trivial\
\ngetters, boilerplate, and tests that would only assert mocks.";

const SEED_FEATURES: &str = "Suggest 1-3 concrete features that would meaningfully improve this project.\
\nBase your suggestions on actual gaps you found while exploring the code.";

const SEED_ARCHITECTURE: &str = "Identify 1-2 significant structural improvements. Think big: module\
\nreorganization, API redesigns, performance overhauls, major refactors\
\nthat span multiple files, or replacing approaches that have outgrown\
\ntheir original design.\
\n\nEach proposal should be a multi-day project, not a quick fix.";
