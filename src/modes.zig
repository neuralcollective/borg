const std = @import("std");
const prompts = @import("prompts.zig");

// ── Types ──────────────────────────────────────────────────────────────

pub const IntegrationType = enum { git_pr, none };
pub const PhaseType = enum { setup, agent, rebase };
pub const SeedOutputType = enum { task, proposal };

pub const PhaseConfig = struct {
    name: []const u8,
    label: []const u8,
    phase_type: PhaseType = .agent,

    // Agent config
    system_prompt: []const u8 = "",
    instruction: []const u8 = "",
    error_instruction: []const u8 = "", // Appended when task.last_error is set; use {ERROR} placeholder
    allowed_tools: []const u8 = "Read,Glob,Grep,Write",
    use_docker: bool = false,

    // Prompt composition
    include_task_context: bool = false, // Prepend "Task #N: title\nDescription:\n..."
    include_file_listing: bool = false, // Append git ls-files output

    // Post-agent actions
    runs_tests: bool = false,
    commits: bool = false,
    commit_message: []const u8 = "",
    check_artifact: ?[]const u8 = null, // File that must exist after phase
    allow_no_changes: bool = false, // Don't fail if agent commits nothing (e.g. tests already exist)

    // Transitions
    next: []const u8 = "done",
    has_qa_fix_routing: bool = false, // On test failure, check if error is in test files → qa_fix
    fresh_session: bool = false, // Start with fresh session (no resume)

    // Rebase-specific (PhaseType.rebase only)
    fix_instruction: []const u8 = "",
    fix_error_instruction: []const u8 = "", // {ERROR} placeholder

    // Queue priority (lower = processed first)
    priority: u8 = 100,
};

pub const SeedConfig = struct {
    name: []const u8,
    label: []const u8,
    prompt: []const u8,
    output_type: SeedOutputType,
};

pub const PipelineMode = struct {
    name: []const u8,
    label: []const u8,
    phases: []const PhaseConfig,
    seed_modes: []const SeedConfig,
    initial_status: []const u8,
    uses_git_worktrees: bool,
    uses_docker: bool,
    uses_test_cmd: bool,
    integration: IntegrationType,
    default_max_attempts: u8 = 5,

    pub fn getPhase(self: PipelineMode, phase_name: []const u8) ?*const PhaseConfig {
        for (self.phases) |*p| {
            if (std.mem.eql(u8, p.name, phase_name)) return p;
        }
        return null;
    }

    pub fn getPhaseIndex(self: PipelineMode, phase_name: []const u8) ?usize {
        for (self.phases, 0..) |p, i| {
            if (std.mem.eql(u8, p.name, phase_name)) return i;
        }
        return null;
    }

    pub fn isTerminal(_: PipelineMode, status: []const u8) bool {
        return std.mem.eql(u8, status, "done") or
            std.mem.eql(u8, status, "merged") or
            std.mem.eql(u8, status, "failed");
    }
};

// ── Utility ────────────────────────────────────────────────────────────

/// Substitute {ERROR} placeholder in a template with the given error text.
/// If no placeholder is found, appends error text after the template.
pub fn substituteError(writer: anytype, template: []const u8, error_text: []const u8) !void {
    const marker = "{ERROR}";
    if (std.mem.indexOf(u8, template, marker)) |pos| {
        try writer.writeAll(template[0..pos]);
        try writer.writeAll(error_text);
        try writer.writeAll(template[pos + marker.len ..]);
    } else {
        try writer.writeAll(template);
        try writer.writeAll("\n");
        try writer.writeAll(error_text);
    }
}

// ── SWE Prompts ────────────────────────────────────────────────────────

const swe_spec_system =
    \\You are the spec-writing agent in an autonomous engineering pipeline.
    \\Read the task and codebase, then write spec.md at the repository root.
    \\Do not modify source files.
;

const swe_qa_system =
    \\You are the test-writing agent in an autonomous engineering pipeline.
    \\Read spec.md and write test files only.
    \\Do not write implementation code or modify non-test files.
;

const swe_worker_system =
    \\You are the implementation agent in an autonomous engineering pipeline.
    \\Read spec.md and tests, write code to make all tests pass.
    \\Do not modify test files.
;

const swe_spec_instruction =
    \\Write spec.md containing:
    \\1. Task summary (2-3 sentences)
    \\2. Files to modify and create (exact paths)
    \\3. Function/type signatures for new or changed code
    \\4. Acceptance criteria (testable assertions)
    \\5. Edge cases
;

const swe_qa_instruction =
    \\Read spec.md and write test files covering every acceptance criterion.
    \\Only create/modify test files (*_test.* or tests/ directory).
    \\Tests should FAIL initially since features are not yet implemented.
;

const swe_qa_fix_error =
    \\
    \\
    \\Your tests from the previous QA pass have bugs that prevent them from passing.
    \\The implementation agent tried multiple times but the test code itself is broken.
    \\
    \\Test output showing the failures:
    \\```
    \\{ERROR}
    \\```
    \\
    \\Fix the test files. Common issues: use-after-free in test setup, wrong allocator
    \\usage, compile errors, missing defer/errdefer, incorrect test assertions.
    \\Do NOT weaken tests or remove test cases — fix the test code so it correctly
    \\validates the behavior described in spec.md.
;

const swe_impl_instruction =
    \\Read spec.md and the test files.
    \\Write implementation code that makes all tests pass.
    \\Only modify files listed in spec.md. Do not modify test files.
;

const swe_impl_retry =
    \\
    \\
    \\Previous attempt failed. Test output:
    \\```
    \\{ERROR}
    \\```
    \\Fix the failures.
;

const swe_rebase_instruction =
    \\This branch has merge conflicts with main.
    \\Rebase onto origin/main, resolve all conflicts, and ensure tests pass.
    \\Read spec.md for context on what this branch does.
;

const swe_rebase_error =
    \\
    \\
    \\Previous error context:
    \\```
    \\{ERROR}
    \\```
;

const swe_rebase_fix =
    \\The branch was rebased onto origin/main successfully, but tests now fail.
    \\Fix the code so tests pass. Read spec.md for context on what this branch does.
    \\Run the test command to verify your fix before finishing.
;

const swe_rebase_fix_error =
    \\
    \\
    \\Test output:
    \\```
    \\{ERROR}
    \\```
;

// ── SWE Mode ───────────────────────────────────────────────────────────

pub const swe_mode = PipelineMode{
    .name = "sweborg",
    .label = "Software Engineering",
    .initial_status = "backlog",
    .uses_git_worktrees = true,
    .uses_docker = true,
    .uses_test_cmd = true,
    .integration = .git_pr,
    .default_max_attempts = 5,
    .phases = &.{
        .{
            .name = "backlog",
            .label = "Backlog",
            .phase_type = .setup,
            .next = "spec",
            .priority = 60,
        },
        .{
            .name = "spec",
            .label = "Specification",
            .system_prompt = swe_spec_system,
            .instruction = swe_spec_instruction,
            .include_task_context = true,
            .include_file_listing = true,
            .check_artifact = "spec.md",
            .use_docker = true,
            .next = "qa",
            .priority = 50,
        },
        .{
            .name = "qa",
            .label = "Testing",
            .system_prompt = swe_qa_system,
            .instruction = swe_qa_instruction,
            .use_docker = true,
            .commits = true,
            .commit_message = "test: add tests from QA agent",
            .allow_no_changes = true,
            .next = "impl",
            .priority = 30,
        },
        .{
            .name = "qa_fix",
            .label = "Test Fix",
            .system_prompt = swe_qa_system,
            .instruction = swe_qa_instruction,
            .error_instruction = swe_qa_fix_error,
            .use_docker = true,
            .commits = true,
            .commit_message = "test: fix tests from QA agent",
            .allow_no_changes = true,
            .next = "impl",
            .fresh_session = true,
            .priority = 25,
        },
        .{
            .name = "impl",
            .label = "Implementation",
            .system_prompt = swe_worker_system,
            .instruction = swe_impl_instruction,
            .error_instruction = swe_impl_retry,
            .allowed_tools = "Read,Glob,Grep,Write,Edit,Bash",
            .use_docker = true,
            .commits = true,
            .commit_message = "impl: implementation from worker agent",
            .runs_tests = true,
            .next = "done",
            .has_qa_fix_routing = true,
            .priority = 10,
        },
        .{
            .name = "retry",
            .label = "Retry",
            .system_prompt = swe_worker_system,
            .instruction = swe_impl_instruction,
            .error_instruction = swe_impl_retry,
            .allowed_tools = "Read,Glob,Grep,Write,Edit,Bash",
            .use_docker = true,
            .commits = true,
            .commit_message = "impl: implementation from worker agent",
            .runs_tests = true,
            .next = "done",
            .has_qa_fix_routing = true,
            .priority = 8,
        },
        .{
            .name = "rebase",
            .label = "Rebase",
            .phase_type = .rebase,
            .system_prompt = swe_worker_system,
            .instruction = swe_rebase_instruction,
            .error_instruction = swe_rebase_error,
            .allowed_tools = "Read,Glob,Grep,Write,Edit,Bash",
            .fix_instruction = swe_rebase_fix,
            .fix_error_instruction = swe_rebase_fix_error,
            .next = "done",
            .priority = 5,
        },
    },
    .seed_modes = &.{
        .{ .name = "refactoring", .label = "Refactoring", .output_type = .task, .prompt = prompts.seed_refactor },
        .{ .name = "security", .label = "Bug Audit", .output_type = .task, .prompt = prompts.seed_security },
        .{ .name = "tests", .label = "Test Coverage", .output_type = .task, .prompt = prompts.seed_tests },
        .{ .name = "features", .label = "Feature Discovery", .output_type = .proposal, .prompt =
        \\Suggest 1-3 concrete features that would meaningfully improve this project.
        \\Base your suggestions on actual gaps you found while exploring the code.
        },
        .{ .name = "architecture", .label = "Architecture Review", .output_type = .proposal, .prompt =
        \\Identify 1-2 significant structural improvements. Think big: module
        \\reorganization, API redesigns, performance overhauls, major refactors
        \\that span multiple files, or replacing approaches that have outgrown
        \\their original design.
        \\
        \\Each proposal should be a multi-day project, not a quick fix.
        },
    },
};

// ── Legal Prompts ──────────────────────────────────────────────────────

const legal_research_system =
    \\You are the research agent in an autonomous legal pipeline.
    \\Analyze the legal issue, research relevant law, precedent, and context,
    \\then produce a research memo (research.md) at the workspace root.
    \\Do not draft legal documents yet — focus on thorough analysis.
;

const legal_research_instruction =
    \\Write research.md containing:
    \\1. Issue summary (2-3 sentences)
    \\2. Relevant statutes, regulations, and rules
    \\3. Key case law and precedent (with citations)
    \\4. Analysis of how the law applies to this matter
    \\5. Open questions and areas requiring further research
;

const legal_draft_system =
    \\You are the drafting agent in an autonomous legal pipeline.
    \\Read research.md and draft the requested legal document.
    \\Focus on accuracy, completeness, and proper legal formatting.
    \\Cite sources from the research memo where applicable.
;

const legal_draft_instruction =
    \\Read research.md and draft the legal document described in the task.
    \\Follow standard legal formatting conventions.
    \\Include all necessary sections, clauses, and provisions.
    \\Cite relevant authority from the research memo.
;

const legal_review_system =
    \\You are the review agent in an autonomous legal pipeline.
    \\Review the drafted document for legal accuracy, completeness, and quality.
    \\Fix any issues you find directly in the document.
;

const legal_review_instruction =
    \\Review all documents in the workspace for:
    \\1. Legal accuracy and correctness
    \\2. Completeness — all required sections present
    \\3. Internal consistency
    \\4. Proper citations and references
    \\5. Potential risks or weaknesses
    \\6. Formatting and style
    \\
    \\Fix any issues directly. Do not just list problems — resolve them.
;

const legal_review_retry =
    \\
    \\
    \\Previous review found unresolved issues:
    \\{ERROR}
    \\
    \\Address these issues in the document.
;

// ── Legal Mode ─────────────────────────────────────────────────────────

pub const legal_mode = PipelineMode{
    .name = "lawborg",
    .label = "Legal",
    .initial_status = "backlog",
    .uses_git_worktrees = true,
    .uses_docker = false,
    .uses_test_cmd = false,
    .integration = .none,
    .default_max_attempts = 3,
    .phases = &.{
        .{
            .name = "backlog",
            .label = "Backlog",
            .phase_type = .setup,
            .next = "research",
            .priority = 60,
        },
        .{
            .name = "research",
            .label = "Research",
            .system_prompt = legal_research_system,
            .instruction = legal_research_instruction,
            .allowed_tools = "Read,Glob,Grep,Write,WebSearch,WebFetch",
            .include_task_context = true,
            .include_file_listing = true,
            .check_artifact = "research.md",
            .next = "draft",
            .priority = 40,
        },
        .{
            .name = "draft",
            .label = "Drafting",
            .system_prompt = legal_draft_system,
            .instruction = legal_draft_instruction,
            .allowed_tools = "Read,Glob,Grep,Write,Edit",
            .commits = true,
            .commit_message = "draft: legal document from drafting agent",
            .next = "review",
            .priority = 30,
        },
        .{
            .name = "review",
            .label = "Review",
            .system_prompt = legal_review_system,
            .instruction = legal_review_instruction,
            .error_instruction = legal_review_retry,
            .allowed_tools = "Read,Glob,Grep,Write,Edit",
            .commits = true,
            .commit_message = "review: revisions from review agent",
            .next = "done",
            .priority = 10,
        },
    },
    .seed_modes = &.{
        .{ .name = "clause_review", .label = "Clause Review", .output_type = .task, .prompt =
        \\Review the legal documents in this repository. Identify 1-3 specific
        \\clauses, provisions, or terms that could be improved, clarified, or
        \\that pose legal risk. Focus on practical, actionable improvements.
        },
        .{ .name = "compliance", .label = "Compliance Audit", .output_type = .task, .prompt =
        \\Audit the documents for compliance gaps against relevant regulations,
        \\standards, and best practices. Create a task for each genuine compliance
        \\issue found. Be specific about the regulation or standard being violated.
        },
        .{ .name = "precedent", .label = "Precedent Research", .output_type = .proposal, .prompt =
        \\Analyze the legal matters addressed in this repository. Suggest 1-3
        \\research directions for relevant case law, statutory authority, or
        \\regulatory guidance that could strengthen the legal positions taken.
        },
        .{ .name = "risk", .label = "Risk Assessment", .output_type = .proposal, .prompt =
        \\Perform a risk assessment of the legal documents and matters. Identify
        \\1-2 significant legal risks, exposure areas, or positions that could
        \\be challenged. Focus on material risks, not theoretical ones.
        },
    },
};

// ── Web Prompts ─────────────────────────────────────────────────────────

const web_audit_system =
    \\You are a frontend performance and UX expert in an autonomous web improvement pipeline.
    \\Analyze the web application codebase and identify concrete opportunities to improve
    \\performance, visual design, accessibility, and user experience.
    \\Write your findings to audit.md at the repository root. Do not modify source files.
;

const web_audit_instruction =
    \\Write audit.md containing:
    \\1. Current state summary (tech stack, key components)
    \\2. Performance issues (bundle size, render bottlenecks, missing lazy loading)
    \\3. Visual and UX improvements (layout, spacing, typography, responsiveness)
    \\4. Accessibility gaps (contrast, keyboard navigation, ARIA)
    \\5. Prioritized action items — concrete, targeted changes for this iteration
;

const web_improve_system =
    \\You are a frontend performance and UX expert in an autonomous web improvement pipeline.
    \\Read audit.md and implement the prioritized improvements.
    \\Focus on measurable wins: faster loads, better visuals, improved UX.
    \\Do not modify audit.md.
;

const web_improve_instruction =
    \\Read audit.md and implement the action items listed under "Prioritized action items".
    \\Make targeted, surgical edits. Verify changes compile/build correctly.
;

const web_improve_retry =
    \\
    \\
    \\Previous attempt failed. Error output:
    \\```
    \\{ERROR}
    \\```
    \\Fix the issue.
;

// ── Web Mode ────────────────────────────────────────────────────────────

pub const web_mode = PipelineMode{
    .name = "webborg",
    .label = "Frontend",
    .initial_status = "backlog",
    .uses_git_worktrees = true,
    .uses_docker = true,
    .uses_test_cmd = true,
    .integration = .git_pr,
    .default_max_attempts = 3,
    .phases = &.{
        .{
            .name = "backlog",
            .label = "Backlog",
            .phase_type = .setup,
            .next = "audit",
            .priority = 60,
        },
        .{
            .name = "audit",
            .label = "Audit",
            .system_prompt = web_audit_system,
            .instruction = web_audit_instruction,
            .include_task_context = true,
            .include_file_listing = true,
            .check_artifact = "audit.md",
            .use_docker = true,
            .next = "improve",
            .priority = 50,
        },
        .{
            .name = "improve",
            .label = "Improve",
            .system_prompt = web_improve_system,
            .instruction = web_improve_instruction,
            .error_instruction = web_improve_retry,
            .allowed_tools = "Read,Glob,Grep,Write,Edit,Bash",
            .use_docker = true,
            .commits = true,
            .commit_message = "improve: frontend improvements from web agent",
            .runs_tests = true,
            .next = "done",
            .priority = 10,
        },
        .{
            .name = "rebase",
            .label = "Rebase",
            .phase_type = .rebase,
            .system_prompt = swe_worker_system,
            .instruction = swe_rebase_instruction,
            .error_instruction = swe_rebase_error,
            .allowed_tools = "Read,Glob,Grep,Write,Edit,Bash",
            .fix_instruction = swe_rebase_fix,
            .fix_error_instruction = swe_rebase_fix_error,
            .next = "done",
            .priority = 5,
        },
    },
    .seed_modes = &.{
        .{ .name = "performance", .label = "Performance", .output_type = .task, .prompt =
        \\Analyze the web app for performance issues. Look for: large bundle sizes,
        \\missing code splitting or lazy loading, expensive re-renders, unoptimized
        \\assets, blocking resources. Create a task for each concrete improvement.
        },
        .{ .name = "visual", .label = "Visual Polish", .output_type = .task, .prompt =
        \\Review the UI for visual inconsistencies and polish opportunities.
        \\Look for: spacing/padding inconsistencies, typography mismatches,
        \\color inconsistencies, rough responsive breakpoints, missing hover/focus states.
        \\Create a task for each concrete visual improvement.
        },
        .{ .name = "accessibility", .label = "Accessibility", .output_type = .task, .prompt =
        \\Audit the web app for accessibility issues. Look for: missing alt text,
        \\poor color contrast, missing ARIA labels, broken keyboard navigation,
        \\missing focus indicators, improper heading hierarchy.
        \\Create a task for each real a11y issue found.
        },
        .{ .name = "ux", .label = "UX Improvements", .output_type = .proposal, .prompt =
        \\Identify 1-3 user experience improvements that would meaningfully reduce
        \\friction or improve usability. Base suggestions on actual UX patterns
        \\found while exploring the UI code and component structure.
        },
    },
};

// ── Registry ───────────────────────────────────────────────────────────

pub const all_modes: []const PipelineMode = &.{ swe_mode, legal_mode, web_mode };

/// SQL CASE expression for ordering tasks by phase priority (lower = first).
/// Generated at comptime from all modes' phase configs.
pub const sql_priority_case: []const u8 = &ComptimeSql.priority_case;

/// SQL IN list of all active (non-terminal) phase names: ('backlog','spec',...)
pub const sql_active_statuses: []const u8 = &ComptimeSql.active_statuses;

const ComptimeSql = struct {
    // Collect unique phases across all modes
    const unique = collectUniquePhases();

    const priority_case = buildPriorityCase();
    const active_statuses = buildActiveStatuses();

    fn collectUniquePhases() struct { names: [64][]const u8, prios: [64]u8, count: usize } {
        var names: [64][]const u8 = undefined;
        var prios: [64]u8 = undefined;
        var count: usize = 0;

        for (all_modes) |mode| {
            for (mode.phases) |phase| {
                var found = false;
                for (0..count) |i| {
                    if (eql(names[i], phase.name)) {
                        if (phase.priority < prios[i]) prios[i] = phase.priority;
                        found = true;
                        break;
                    }
                }
                if (!found) {
                    names[count] = phase.name;
                    prios[count] = phase.priority;
                    count += 1;
                }
            }
        }
        return .{ .names = names, .prios = prios, .count = count };
    }

    fn buildPriorityCase() [priorityCaseLen()]u8 {
        var buf: [priorityCaseLen()]u8 = undefined;
        var pos: usize = 0;
        for ("CASE status ") |c| {
            buf[pos] = c;
            pos += 1;
        }
        for (0..unique.count) |i| {
            for ("WHEN '") |c| {
                buf[pos] = c;
                pos += 1;
            }
            for (unique.names[i]) |c| {
                buf[pos] = c;
                pos += 1;
            }
            for ("' THEN ") |c| {
                buf[pos] = c;
                pos += 1;
            }
            if (unique.prios[i] >= 100) {
                buf[pos] = '0' + (unique.prios[i] / 100);
                pos += 1;
                buf[pos] = '0' + ((unique.prios[i] / 10) % 10);
                pos += 1;
            } else if (unique.prios[i] >= 10) {
                buf[pos] = '0' + (unique.prios[i] / 10);
                pos += 1;
            }
            buf[pos] = '0' + (unique.prios[i] % 10);
            pos += 1;
            buf[pos] = ' ';
            pos += 1;
        }
        for ("ELSE 100 END") |c| {
            buf[pos] = c;
            pos += 1;
        }
        return buf;
    }

    fn priorityCaseLen() usize {
        var len: usize = "CASE status ".len + "ELSE 100 END".len;
        for (0..unique.count) |i| {
            len += "WHEN '".len + unique.names[i].len + "' THEN ".len;
            len += if (unique.prios[i] >= 100) @as(usize, 3) else if (unique.prios[i] >= 10) @as(usize, 2) else 1;
            len += 1; // space
        }
        return len;
    }

    fn buildActiveStatuses() [activeStatusesLen()]u8 {
        var buf: [activeStatusesLen()]u8 = undefined;
        var pos: usize = 0;
        buf[pos] = '(';
        pos += 1;
        for (0..unique.count) |i| {
            if (i > 0) {
                buf[pos] = ',';
                pos += 1;
            }
            buf[pos] = '\'';
            pos += 1;
            for (unique.names[i]) |c| {
                buf[pos] = c;
                pos += 1;
            }
            buf[pos] = '\'';
            pos += 1;
        }
        buf[pos] = ')';
        pos += 1;
        return buf;
    }

    fn activeStatusesLen() usize {
        var len: usize = 2; // parens
        for (0..unique.count) |i| {
            if (i > 0) len += 1; // comma
            len += 2 + unique.names[i].len; // quotes + name
        }
        return len;
    }

    fn eql(a: []const u8, b: []const u8) bool {
        if (a.len != b.len) return false;
        for (a, b) |ca, cb| {
            if (ca != cb) return false;
        }
        return true;
    }
};

pub fn getMode(name: []const u8) ?*const PipelineMode {
    for (all_modes) |*m| {
        if (std.mem.eql(u8, m.name, name)) return m;
    }
    // Backward-compat aliases for old mode names
    if (std.mem.eql(u8, name, "swe")) return getMode("sweborg");
    if (std.mem.eql(u8, name, "legal")) return getMode("lawborg");
    return null;
}

// ── Tests ──────────────────────────────────────────────────────────────

test "sweborg_mode has all expected phases" {
    try std.testing.expect(swe_mode.getPhase("backlog") != null);
    try std.testing.expect(swe_mode.getPhase("spec") != null);
    try std.testing.expect(swe_mode.getPhase("qa") != null);
    try std.testing.expect(swe_mode.getPhase("qa_fix") != null);
    try std.testing.expect(swe_mode.getPhase("impl") != null);
    try std.testing.expect(swe_mode.getPhase("retry") != null);
    try std.testing.expect(swe_mode.getPhase("rebase") != null);
    try std.testing.expect(swe_mode.getPhase("nonexistent") == null);
}

test "lawborg_mode has all expected phases" {
    try std.testing.expect(legal_mode.getPhase("backlog") != null);
    try std.testing.expect(legal_mode.getPhase("research") != null);
    try std.testing.expect(legal_mode.getPhase("draft") != null);
    try std.testing.expect(legal_mode.getPhase("review") != null);
    try std.testing.expect(legal_mode.getPhase("nonexistent") == null);
}

test "webborg_mode has all expected phases" {
    try std.testing.expect(web_mode.getPhase("backlog") != null);
    try std.testing.expect(web_mode.getPhase("audit") != null);
    try std.testing.expect(web_mode.getPhase("improve") != null);
    try std.testing.expect(web_mode.getPhase("rebase") != null);
    try std.testing.expect(web_mode.getPhase("nonexistent") == null);
}

test "getMode returns correct modes" {
    try std.testing.expect(getMode("sweborg") != null);
    try std.testing.expect(getMode("lawborg") != null);
    try std.testing.expect(getMode("webborg") != null);
    try std.testing.expect(getMode("nonexistent") == null);
    try std.testing.expectEqualStrings("Software Engineering", getMode("sweborg").?.label);
    try std.testing.expectEqualStrings("Legal", getMode("lawborg").?.label);
    try std.testing.expectEqualStrings("Frontend", getMode("webborg").?.label);
    // Backward-compat aliases
    try std.testing.expect(getMode("swe") != null);
    try std.testing.expect(getMode("legal") != null);
}

test "phase transitions are valid" {
    // Every phase's .next should be either a valid phase or a terminal status
    for (all_modes) |mode| {
        for (mode.phases) |phase| {
            if (mode.isTerminal(phase.next)) continue;
            if (mode.getPhase(phase.next) == null) {
                std.debug.print("Invalid transition: {s}.{s} -> {s}\n", .{ mode.name, phase.name, phase.next });
                return error.InvalidTransition;
            }
        }
    }
}

test "phase priorities are unique within each mode" {
    for (all_modes) |mode| {
        for (mode.phases, 0..) |a, i| {
            for (mode.phases[i + 1 ..]) |b| {
                if (a.priority == b.priority) {
                    std.debug.print("Duplicate priority {d} in {s}: {s} and {s}\n", .{ a.priority, mode.name, a.name, b.name });
                    return error.DuplicatePriority;
                }
            }
        }
    }
}

test "all phases have non-empty names and labels" {
    for (all_modes) |mode| {
        try std.testing.expect(mode.name.len > 0);
        try std.testing.expect(mode.label.len > 0);
        for (mode.phases) |phase| {
            try std.testing.expect(phase.name.len > 0);
            try std.testing.expect(phase.label.len > 0);
        }
    }
}

test "agent phases have system prompts" {
    for (all_modes) |mode| {
        for (mode.phases) |phase| {
            if (phase.phase_type == .agent) {
                try std.testing.expect(phase.system_prompt.len > 0);
                try std.testing.expect(phase.instruction.len > 0);
            }
        }
    }
}

test "substituteError with placeholder" {
    var buf: [256]u8 = undefined;
    var fbs = std.io.fixedBufferStream(&buf);
    try substituteError(fbs.writer(), "before {ERROR} after", "ERR");
    try std.testing.expectEqualStrings("before ERR after", fbs.getWritten());
}

test "substituteError without placeholder" {
    var buf: [256]u8 = undefined;
    var fbs = std.io.fixedBufferStream(&buf);
    try substituteError(fbs.writer(), "no placeholder here", "ERR");
    try std.testing.expectEqualStrings("no placeholder here\nERR", fbs.getWritten());
}

test "sql_priority_case covers all phases" {
    try std.testing.expect(std.mem.indexOf(u8, sql_priority_case, "CASE status") != null);
    try std.testing.expect(std.mem.indexOf(u8, sql_priority_case, "'backlog'") != null);
    try std.testing.expect(std.mem.indexOf(u8, sql_priority_case, "'impl'") != null);
    try std.testing.expect(std.mem.indexOf(u8, sql_priority_case, "'research'") != null);
    try std.testing.expect(std.mem.indexOf(u8, sql_priority_case, "'draft'") != null);
    try std.testing.expect(std.mem.indexOf(u8, sql_priority_case, "'audit'") != null);
    try std.testing.expect(std.mem.indexOf(u8, sql_priority_case, "'improve'") != null);
    try std.testing.expect(std.mem.indexOf(u8, sql_priority_case, "ELSE 100 END") != null);
}

test "sql_active_statuses covers all phases" {
    try std.testing.expect(std.mem.indexOf(u8, sql_active_statuses, "'backlog'") != null);
    try std.testing.expect(std.mem.indexOf(u8, sql_active_statuses, "'impl'") != null);
    try std.testing.expect(std.mem.indexOf(u8, sql_active_statuses, "'research'") != null);
    try std.testing.expect(std.mem.indexOf(u8, sql_active_statuses, "'audit'") != null);
    try std.testing.expect(std.mem.indexOf(u8, sql_active_statuses, "'improve'") != null);
    try std.testing.expect(sql_active_statuses[0] == '(');
    try std.testing.expect(sql_active_statuses[sql_active_statuses.len - 1] == ')');
}

test "getPhaseIndex returns correct indices" {
    try std.testing.expectEqual(@as(?usize, 0), swe_mode.getPhaseIndex("backlog"));
    try std.testing.expectEqual(@as(?usize, 1), swe_mode.getPhaseIndex("spec"));
    try std.testing.expectEqual(@as(?usize, null), swe_mode.getPhaseIndex("nonexistent"));
}
