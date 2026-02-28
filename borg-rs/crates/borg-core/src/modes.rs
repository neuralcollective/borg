use crate::types::{
    IntegrationType, PhaseConfig, PhaseType, PipelineMode, SeedConfig, SeedOutputType,
};

pub fn all_modes() -> Vec<PipelineMode> {
    vec![
        swe_mode(),
        legal_mode(),
        web_mode(),
        crew_mode(),
        sales_mode(),
        data_mode(),
    ]
}

pub fn get_mode(name: &str) -> Option<PipelineMode> {
    match name {
        // Backward-compat aliases
        "swe" => get_mode("sweborg"),
        "legal" => get_mode("lawborg"),
        _ => all_modes().into_iter().find(|m| m.name == name),
    }
}

// ── Phase builders ───────────────────────────────────────────────────────

/// Create a backlog/setup phase that transitions immediately to the first agent phase.
fn setup_phase(next: &str) -> PhaseConfig {
    PhaseConfig {
        name: "backlog".into(),
        label: "Backlog".into(),
        phase_type: PhaseType::Setup,
        next: next.into(),
        ..Default::default()
    }
}

/// Create a standard agent phase with the six most common fields.
/// Callers override additional fields via struct update syntax.
fn agent_phase(
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
fn lint_phase(next: &str) -> PhaseConfig {
    PhaseConfig {
        name: "lint_fix".into(),
        label: "Lint".into(),
        phase_type: PhaseType::LintFix,
        allow_no_changes: true,
        next: next.into(),
        ..Default::default()
    }
}

/// Create a standard rebase phase (shared across sweborg/webborg).
fn rebase_phase() -> PhaseConfig {
    PhaseConfig {
        name: "rebase".into(),
        label: "Rebase".into(),
        phase_type: PhaseType::Rebase,
        system_prompt: SWE_WORKER_SYSTEM.into(),
        instruction: SWE_REBASE_INSTRUCTION.into(),
        error_instruction: SWE_REBASE_ERROR.into(),
        allowed_tools: "Read,Glob,Grep,Write,Edit,Bash".into(),
        fix_instruction: SWE_REBASE_FIX.into(),
        next: "done".into(),
        ..Default::default()
    }
}

/// Shared seed configs used by sweborg/webborg.
fn swe_seeds() -> Vec<SeedConfig> {
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

// ── Mode definitions ─────────────────────────────────────────────────────

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

pub fn legal_mode() -> PipelineMode {
    PipelineMode {
        name: "lawborg".into(),
        label: "Legal".into(),
        category: "Professional Services".into(),
        initial_status: "backlog".into(),
        uses_git_worktrees: true,
        uses_docker: false,
        uses_test_cmd: false,
        integration: IntegrationType::None,
        default_max_attempts: 3,
        phases: vec![
            setup_phase("research"),
            PhaseConfig {
                include_task_context: true,
                include_file_listing: true,
                check_artifact: Some("research.md".into()),
                ..agent_phase("research", "Research", LEGAL_RESEARCH_SYSTEM, LEGAL_RESEARCH_INSTRUCTION, "Read,Glob,Grep,Write,WebSearch,WebFetch", "draft")
            },
            PhaseConfig {
                commits: true,
                commit_message: "draft: legal document from drafting agent".into(),
                ..agent_phase("draft", "Drafting", LEGAL_DRAFT_SYSTEM, LEGAL_DRAFT_INSTRUCTION, "Read,Glob,Grep,Write,Edit", "review")
            },
            PhaseConfig {
                error_instruction: LEGAL_REVIEW_RETRY.into(),
                commits: true,
                commit_message: "review: revisions from review agent".into(),
                ..agent_phase("review", "Review", LEGAL_REVIEW_SYSTEM, LEGAL_REVIEW_INSTRUCTION, "Read,Glob,Grep,Write,Edit", "done")
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "clause_review".into(),
                label: "Clause Review".into(),
                output_type: SeedOutputType::Task,
                prompt: "Review the legal documents in this repository. Identify 1-3 specific\nclauses, provisions, or terms that could be improved, clarified, or\nthat pose legal risk. Focus on practical, actionable improvements.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "compliance".into(),
                label: "Compliance Audit".into(),
                output_type: SeedOutputType::Task,
                prompt: "Audit the documents for compliance gaps against relevant regulations,\nstandards, and best practices. Create a task for each genuine compliance\nissue found. Be specific about the regulation or standard being violated.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "precedent".into(),
                label: "Precedent Research".into(),
                output_type: SeedOutputType::Proposal,
                prompt: "Analyze the legal matters addressed in this repository. Suggest 1-3\nresearch directions for relevant case law, statutory authority, or\nregulatory guidance that could strengthen the legal positions taken.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "risk".into(),
                label: "Risk Assessment".into(),
                output_type: SeedOutputType::Proposal,
                prompt: "Perform a risk assessment of the legal documents and matters. Identify\n1-2 significant legal risks, exposure areas, or positions that could\nbe challenged. Focus on material risks, not theoretical ones.".into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
        ],
    }
}

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
            seeds.splice(0..0, [SeedConfig {
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
            }]);
            seeds
        },
    }
}

pub fn crew_mode() -> PipelineMode {
    PipelineMode {
        name: "crewborg".into(),
        label: "Talent Search".into(),
        category: "People & Ops".into(),
        initial_status: "backlog".into(),
        uses_git_worktrees: true,
        uses_docker: false,
        uses_test_cmd: false,
        integration: IntegrationType::None,
        default_max_attempts: 3,
        phases: vec![
            setup_phase("source"),
            PhaseConfig {
                include_task_context: true,
                include_file_listing: true,
                check_artifact: Some("candidates.md".into()),
                ..agent_phase(
                    "source",
                    "Sourcing",
                    CREW_SOURCE_SYSTEM,
                    CREW_SOURCE_INSTRUCTION,
                    "Read,Glob,Grep,Write,WebSearch,WebFetch",
                    "evaluate",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "eval: candidate evaluations from crew agent".into(),
                ..agent_phase(
                    "evaluate",
                    "Evaluation",
                    CREW_EVALUATE_SYSTEM,
                    CREW_EVALUATE_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,WebSearch,WebFetch",
                    "rank",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "rank: prioritized shortlist from crew agent".into(),
                ..agent_phase(
                    "rank",
                    "Ranking",
                    CREW_RANK_SYSTEM,
                    CREW_RANK_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit",
                    "done",
                )
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "discovery".into(),
                label: "Candidate Discovery".into(),
                output_type: SeedOutputType::Task,
                prompt: CREW_SEED_DISCOVERY.into(),
                allowed_tools: "Read,Glob,Grep,Bash,WebSearch,WebFetch".into(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "refresh".into(),
                label: "Re-evaluate Pool".into(),
                output_type: SeedOutputType::Task,
                prompt: CREW_SEED_REFRESH.into(),
                allowed_tools: "Read,Glob,Grep,Bash,WebSearch,WebFetch".into(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "criteria".into(),
                label: "Refine Criteria".into(),
                output_type: SeedOutputType::Proposal,
                prompt: CREW_SEED_CRITERIA.into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
        ],
    }
}

pub fn sales_mode() -> PipelineMode {
    PipelineMode {
        name: "salesborg".into(),
        label: "Sales Outreach".into(),
        category: "Professional Services".into(),
        initial_status: "backlog".into(),
        uses_git_worktrees: true,
        uses_docker: false,
        uses_test_cmd: false,
        integration: IntegrationType::None,
        default_max_attempts: 3,
        phases: vec![
            setup_phase("research"),
            PhaseConfig {
                include_task_context: true,
                check_artifact: Some("prospect.md".into()),
                ..agent_phase(
                    "research",
                    "Prospect Research",
                    SALES_RESEARCH_SYSTEM,
                    SALES_RESEARCH_INSTRUCTION,
                    "Read,Glob,Grep,Write,WebSearch,WebFetch",
                    "draft",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "draft: outreach from sales agent".into(),
                ..agent_phase(
                    "draft",
                    "Outreach Draft",
                    SALES_DRAFT_SYSTEM,
                    SALES_DRAFT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit",
                    "review",
                )
            },
            PhaseConfig {
                error_instruction: SALES_REVIEW_RETRY.into(),
                commits: true,
                commit_message: "review: revisions from sales review agent".into(),
                ..agent_phase(
                    "review",
                    "Review",
                    SALES_REVIEW_SYSTEM,
                    SALES_REVIEW_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit",
                    "done",
                )
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "lead_discovery".into(),
                label: "Lead Discovery".into(),
                output_type: SeedOutputType::Task,
                prompt: SALES_SEED_DISCOVERY.into(),
                allowed_tools: "Read,Glob,Grep,Bash,WebSearch,WebFetch".into(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "follow_up".into(),
                label: "Follow-up Drafts".into(),
                output_type: SeedOutputType::Task,
                prompt: SALES_SEED_FOLLOWUP.into(),
                allowed_tools: "Read,Glob,Grep,Bash,WebSearch,WebFetch".into(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "icp".into(),
                label: "ICP Refinement".into(),
                output_type: SeedOutputType::Proposal,
                prompt: SALES_SEED_ICP.into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
        ],
    }
}

pub fn data_mode() -> PipelineMode {
    PipelineMode {
        name: "databorg".into(),
        label: "Data Analysis".into(),
        category: "Data & Analytics".into(),
        initial_status: "backlog".into(),
        uses_git_worktrees: true,
        uses_docker: false,
        uses_test_cmd: false,
        integration: IntegrationType::None,
        default_max_attempts: 3,
        phases: vec![
            setup_phase("ingest"),
            PhaseConfig {
                include_task_context: true,
                include_file_listing: true,
                check_artifact: Some("data.md".into()),
                ..agent_phase(
                    "ingest",
                    "Data Ingestion",
                    DATA_INGEST_SYSTEM,
                    DATA_INGEST_INSTRUCTION,
                    "Read,Glob,Grep,Write,Bash,WebSearch,WebFetch",
                    "analyze",
                )
            },
            PhaseConfig {
                error_instruction: DATA_ANALYZE_RETRY.into(),
                commits: true,
                commit_message: "analyze: data analysis from databorg agent".into(),
                ..agent_phase(
                    "analyze",
                    "Analysis",
                    DATA_ANALYZE_SYSTEM,
                    DATA_ANALYZE_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit,Bash",
                    "report",
                )
            },
            PhaseConfig {
                commits: true,
                commit_message: "report: findings from databorg agent".into(),
                ..agent_phase(
                    "report",
                    "Report",
                    DATA_REPORT_SYSTEM,
                    DATA_REPORT_INSTRUCTION,
                    "Read,Glob,Grep,Write,Edit",
                    "done",
                )
            },
        ],
        seed_modes: vec![
            SeedConfig {
                name: "explore".into(),
                label: "Exploratory Analysis".into(),
                output_type: SeedOutputType::Task,
                prompt: DATA_SEED_EXPLORE.into(),
                allowed_tools: "Read,Glob,Grep,Bash".into(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "quality".into(),
                label: "Data Quality Audit".into(),
                output_type: SeedOutputType::Task,
                prompt: DATA_SEED_QUALITY.into(),
                allowed_tools: "Read,Glob,Grep,Bash".into(),
                target_primary_repo: false,
            },
            SeedConfig {
                name: "refresh".into(),
                label: "Refresh Reports".into(),
                output_type: SeedOutputType::Task,
                prompt: DATA_SEED_REFRESH.into(),
                allowed_tools: String::new(),
                target_primary_repo: false,
            },
        ],
    }
}

// ── Prompt constants ─────────────────────────────────────────────────────

const SWE_SPEC_SYSTEM: &str = "You are the spec-writing agent in an autonomous engineering pipeline.\nRead the task and codebase, then write spec.md at the repository root.\nDo not modify source files.";

const SWE_QA_SYSTEM: &str = "You are the test-writing agent in an autonomous engineering pipeline.\nRead spec.md and write test files only.\nDo not write implementation code or modify non-test files.";

const SWE_WORKER_SYSTEM: &str = "You are the implementation agent in an autonomous engineering pipeline.\nRead spec.md and tests, write code to make all tests pass.\nPrefer not to modify test files, but if tests reference APIs or types that don't exist in the codebase, fix the tests to match reality before implementing.";

const SWE_SPEC_INSTRUCTION: &str = "Write spec.md containing:\n1. Task summary (2-3 sentences)\n2. Files to modify and create (exact paths — verify each exists with Glob)\n3. Function/type signatures for new or changed code (verify existing ones with Grep)\n4. Acceptance criteria (testable assertions)\n5. Edge cases\n\nBefore finalizing: verify every file path you reference actually exists (unless it's a new file to create). Verify every function or type you reference is real. Remove any references to code that doesn't exist.";

const SWE_QA_INSTRUCTION: &str = "Read spec.md and write test files covering every acceptance criterion.\nOnly create/modify test files (*_test.* or tests/ directory).\nTests should FAIL initially since features are not yet implemented.\n\nBefore writing tests, verify that the APIs and types referenced in spec.md actually exist in the codebase. If spec.md references something that doesn't exist, write tests against the real API instead.";

const SWE_QA_FIX_ERROR: &str = "\n\nYour tests from the previous QA pass have issues that prevent them from passing.\nThe implementation agent tried multiple times but the test code itself is broken.\n\nTest output showing the failures:\n```\n{ERROR}\n```\n\nFix the test files. Common issues:\n- Tests reference functions, types, or fields that don't exist in the codebase\n- Compile errors from wrong API assumptions\n- use-after-free in test setup, wrong allocator usage\n- Missing defer/errdefer, incorrect test assertions\nDo NOT weaken tests or remove test cases — fix the test code so it correctly\nvalidates the behavior described in spec.md, using only APIs that actually exist.";

const SWE_IMPL_INSTRUCTION: &str = "Read spec.md and the test files.\nWrite implementation code that makes all tests pass.\nPrefer to only modify files listed in spec.md.\nIf tests reference APIs, types, or fields that don't exist in the codebase, fix them to match reality — keep the test intent but correct wrong API assumptions.";

const SWE_IMPL_RETRY: &str =
    "\n\nPrevious attempt failed. Test output:\n```\n{ERROR}\n```\nFix the failures.";

const SWE_REBASE_INSTRUCTION: &str = "This branch has merge conflicts with main.\nRebase onto origin/main, resolve all conflicts, and ensure tests pass.\nRead spec.md for context on what this branch does.";

const SWE_REBASE_ERROR: &str = "\n\nPrevious error context:\n```\n{ERROR}\n```";

const SWE_REBASE_FIX: &str = "The git rebase onto origin/main failed with conflicts:\n\n{ERROR}\n\nYou are in the worktree where the rebase is paused. Resolve all conflicts:\n- For 'deleted by us' files (files removed from main): run `git rm <file>` for each one\n- For content conflicts (<<<< markers): edit the file to resolve, then `git add <file>`\nAfter resolving all conflicts, run `git rebase --continue`.\nDo NOT run `git rebase --abort`.";

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

const WEB_IMPROVE_RETRY: &str =
    "\n\nPrevious attempt failed. Error output:\n```\n{ERROR}\n```\nFix the issue.";

const SEED_REFACTOR: &str = "Identify 1-3 concrete, small improvements in code quality. Look for:\
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

const SEED_SECURITY: &str =
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

const SEED_TESTS: &str = "Identify gaps in test coverage that matter for correctness. Look for:\
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

const SEED_FEATURES: &str =
    "Suggest 1-3 concrete features that would meaningfully improve this project.\
\nBase your suggestions on actual gaps you found while exploring the code.";

const SEED_ARCHITECTURE: &str =
    "Identify 1-2 significant structural improvements. Think big: module\
\nreorganization, API redesigns, performance overhauls, major refactors\
\nthat span multiple files, or replacing approaches that have outgrown\
\ntheir original design.\
\n\nEach proposal should be a multi-day project, not a quick fix.";

const SEED_CROSS_POLLINATE: &str =
    "Study this codebase to understand its patterns, features, and architecture.\
\nThen suggest 1-3 ideas inspired by what you see here that could be adapted\
\nor ported to a DIFFERENT project (not this one).\
\n\nFocus on: elegant abstractions worth copying, clever approaches to common\
\nproblems, architectural patterns that solve hard problems well.\
\n\nOutput proposals — each one describes a concrete improvement to apply\
\nelsewhere, inspired by what you found here.";

// ── CrewBorg prompts ──────────────────────────────────────────────────────

const CREW_SOURCE_SYSTEM: &str =
    "You are a talent sourcing agent. Your job is to find real, verifiable\
\ncandidates that match the brief. Use web search to locate profiles,\
\nportfolios, GitHub accounts, LinkedIn, personal sites, and relevant\
\ncommunities. Do not invent candidates — only record those you can verify.";

const CREW_SOURCE_INSTRUCTION: &str = "Read the task brief and search for matching candidates.\
\nWrite candidates.md containing:\
\n1. Brief (role, requirements, key criteria)\
\n2. Sourcing channels searched\
\n3. Candidate list — for each: name, profile URL(s), location, and why they match\
\nAim for 10-20 candidates. Prefer quality over quantity.";

const CREW_EVALUATE_SYSTEM: &str =
    "You are a talent evaluation agent. Read candidates.md and do deeper\
\nresearch on each candidate. Assess fit against the brief criteria.\
\nBe honest about gaps and uncertainties. Do not inflate scores.";

const CREW_EVALUATE_INSTRUCTION: &str = "Read candidates.md and evaluate each candidate.\
\nFor each, research their background, work quality, and relevance.\
\nAppend an evaluation section to candidates.md with:\
\n- Score (1-10) per key criterion\
\n- Overall fit score\
\n- Strengths and gaps\
\n- Availability signals (if findable)\
\n- Red flags if any";

const CREW_RANK_SYSTEM: &str = "You are a talent ranking agent. Synthesise the evaluations from\
\ncandidates.md into a final prioritised shortlist. Be concise and decisive.\
\nThe shortlist is the deliverable — make it actionable.";

const CREW_RANK_INSTRUCTION: &str = "Read candidates.md with evaluations and produce shortlist.md:\
\n1. Top picks (ranked 1-5) — name, score, 2-sentence summary, contact/profile link\
\n2. Reserves (next 5) — brief note on each\
\n3. Not recommended — list names and one-line reason\
\n4. Recommended next steps (outreach order, questions to ask)";

const CREW_SEED_DISCOVERY: &str = "Review the existing candidate pool in this repository.\
\nIdentify gaps: roles not yet sourced, underrepresented skill sets,\
\nor geographies not yet searched. Create a task to source candidates\
\nfor the most critical gap.";

const CREW_SEED_REFRESH: &str = "Check candidates in the existing shortlist for staleness.\
\nSearch for any whose profiles or availability may have changed\
\nsince last evaluated. Create a task to re-evaluate those candidates.";

const CREW_SEED_CRITERIA: &str =
    "Review the search briefs and evaluation criteria in this repository.\
\nSuggest 1-2 improvements: criteria that are too vague, missing signals\
\nthat would better predict fit, or sourcing channels not yet tried.";

// ── SalesBorg prompts ─────────────────────────────────────────────────────

const SALES_RESEARCH_SYSTEM: &str =
    "You are a sales research agent. Research the prospect thoroughly\
\nbefore any outreach is drafted. Find recent news, product focus, team\
\nsize, funding, pain points, and relevant context. Do not fabricate facts.";

const SALES_RESEARCH_INSTRUCTION: &str =
    "Research the prospect described in the task and write prospect.md:\
\n1. Company/person overview (what they do, size, stage)\
\n2. Recent news and signals (funding, launches, hires, press)\
\n3. Likely pain points relevant to our offering\
\n4. Key decision-makers and their roles\
\n5. Recommended angle for outreach (what to lead with and why)";

const SALES_DRAFT_SYSTEM: &str = "You are a sales outreach agent. Read prospect.md and draft\
\npersonalised, concise outreach. Lead with insight, not a pitch.\
\nAvoid generic templates — every word should be specific to this prospect.";

const SALES_DRAFT_INSTRUCTION: &str = "Read prospect.md and draft outreach.md containing:\
\n1. Email subject line (sharp, specific, not clickbait)\
\n2. Email body (3-5 short paragraphs max)\
\n   - Hook: something specific and relevant to them\
\n   - Bridge: connect their situation to what we offer\
\n   - Ask: one clear, low-friction call to action\
\n3. LinkedIn message variant (under 300 chars)\
\n4. Notes on timing or personalisation to add before sending";

const SALES_REVIEW_SYSTEM: &str =
    "You are a senior sales reviewer. Read prospect.md and outreach.md.\
\nAssess the outreach for relevance, tone, personalisation, and clarity.\
\nFix weak spots directly in outreach.md. Do not just list issues.";

const SALES_REVIEW_INSTRUCTION: &str = "Review outreach.md against prospect.md. Check:\
\n1. Does the hook reference something genuinely specific to the prospect?\
\n2. Is the value prop clear and relevant to their likely pain points?\
\n3. Is the ask concrete and easy to say yes to?\
\n4. Tone: confident but not pushy, peer-to-peer\
\n5. Length: email under 200 words, LinkedIn under 300 chars\
\nFix any issues directly. Leave a brief review note at the top of outreach.md.";

const SALES_REVIEW_RETRY: &str =
    "\n\nPrevious review flagged unresolved issues:\n{ERROR}\n\nAddress them.";

const SALES_SEED_DISCOVERY: &str = "Review the prospect list in this repository.\
\nIdentify 1-3 new leads that fit the ideal customer profile based on\
\nwhat's already here. Each lead should be a real, findable company or person.\
\nCreate a task for each with enough context to kick off research.";

const SALES_SEED_FOLLOWUP: &str = "Review prospects where outreach was sent but no reply recorded.\
\nDraft a follow-up task for the highest-priority ones.\
\nFollow-ups should add new value (a resource, insight, or hook)\
\nrather than just bumping the thread.";

const SALES_SEED_ICP: &str = "Analyse the prospect list and outreach results in this repository.\
\nSuggest 1-2 refinements to the ideal customer profile: segments that\
\nare converting better, new verticals worth targeting, or personas\
\nbeing missed. Base suggestions on what's already in the data.";

// ── DataBorg prompts ──────────────────────────────────────────────────────

const DATA_INGEST_SYSTEM: &str =
    "You are a data ingestion agent. Your job is to understand the raw data\
\nin this repository — schemas, formats, quality, and coverage — and produce\
\na clear summary so downstream agents can work with it effectively.\
\nDo not draw conclusions yet. Focus on accurate characterisation.";

const DATA_INGEST_INSTRUCTION: &str =
    "Explore the data files in this repository and write data.md:\
\n1. Data inventory — files/tables, formats, row counts, date ranges\
\n2. Schema summary — key fields, types, nullability, relationships\
\n3. Quality issues — missing values, outliers, encoding problems, duplicates\
\n4. Coverage gaps — what's present vs what the task requires\
\n5. Recommended approach for the analysis requested in the task";

const DATA_ANALYZE_SYSTEM: &str = "You are a data analysis agent. Read data.md and the raw data,\
\nthen perform the analysis described in the task. Write clean, reproducible\
\ncode (Python preferred). Prefer simple, correct analysis over complex models.";

const DATA_ANALYZE_INSTRUCTION: &str = "Read data.md and the task description, then:\
\n1. Write analysis code in analysis.py (or analysis.sql for pure SQL tasks)\
\n2. Run the code and capture output — include key numbers and results inline\
\n3. Note any assumptions made or data limitations that affect conclusions";

const DATA_ANALYZE_RETRY: &str =
    "\n\nPrevious attempt failed. Error:\n```\n{ERROR}\n```\nFix the issue.";

const DATA_REPORT_SYSTEM: &str =
    "You are a data reporting agent. Read the analysis outputs and produce\
\na clear, concise report. Lead with findings, not methodology.\
\nWrite for a non-technical reader unless the task specifies otherwise.";

const DATA_REPORT_INSTRUCTION: &str =
    "Read data.md and the analysis outputs, then write report.md:\
\n1. Key findings (3-5 bullet points, concrete numbers)\
\n2. Methodology summary (1 short paragraph — what was done and why)\
\n3. Detailed results — tables, trends, comparisons as appropriate\
\n4. Caveats and limitations\
\n5. Recommended next steps or actions based on findings";

const DATA_SEED_EXPLORE: &str = "Survey the data in this repository. Identify 1-3 analyses that\
\nwould yield actionable insight: trends worth quantifying, anomalies\
\nworth investigating, or comparisons not yet made. Create a task for each.";

const DATA_SEED_QUALITY: &str = "Audit data quality in this repository. Look for:\
\n- Fields with high null rates that should be populated\
\n- Duplicate records or inconsistent identifiers\
\n- Value distributions that suggest encoding errors or pipeline bugs\
\n- Date/time coverage gaps\
\nCreate a task for each genuine quality issue worth fixing.";

const DATA_SEED_REFRESH: &str = "Review the existing reports and analyses in this repository.\
\nIdentify which are stale and would benefit from a refresh with current data.\
\nCreate a task to re-run the most valuable one.";
