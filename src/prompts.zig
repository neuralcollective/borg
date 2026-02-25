const std = @import("std");
const AgentPersona = @import("pipeline.zig").AgentPersona;

// ── System prompts (per-persona) ───────────────────────────────────────

pub fn getSystemPrompt(persona: AgentPersona) []const u8 {
    return switch (persona) {
        .manager =>
        \\You are the spec-writing agent in an autonomous engineering pipeline.
        \\Read the task and codebase, then write spec.md at the repository root.
        \\Do not modify source files.
        ,
        .qa =>
        \\You are the test-writing agent in an autonomous engineering pipeline.
        \\Read spec.md and write test files only.
        \\Do not write implementation code or modify non-test files.
        ,
        .worker =>
        \\You are the implementation agent in an autonomous engineering pipeline.
        \\Read spec.md and tests, write code to make all tests pass.
        \\Do not modify test files.
        ,
    };
}

pub fn getAllowedTools(persona: AgentPersona) []const u8 {
    return switch (persona) {
        .manager => "Read,Glob,Grep,Write",
        .qa => "Read,Glob,Grep,Write",
        .worker => "Read,Glob,Grep,Write,Edit,Bash",
    };
}

// ── Phase prompts ──────────────────────────────────────────────────────

pub const spec_phase =
    \\Task #{d}: {s}
    \\
    \\Description:
    \\{s}
    \\
    \\Repository files:
    \\
;

pub const spec_phase_suffix =
    \\
    \\Write spec.md containing:
    \\1. Task summary (2-3 sentences)
    \\2. Files to modify and create (exact paths)
    \\3. Function/type signatures for new or changed code
    \\4. Acceptance criteria (testable assertions)
    \\5. Edge cases
;

pub const qa_phase =
    \\Read spec.md and write test files covering every acceptance criterion.
    \\Only create/modify test files (*_test.* or tests/ directory).
    \\Tests should FAIL initially since features are not yet implemented.
;

pub const impl_phase =
    \\Read spec.md and the test files.
    \\Write implementation code that makes all tests pass.
    \\Only modify files listed in spec.md. Do not modify test files.
;

pub const impl_retry_fmt =
    \\
    \\
    \\Previous attempt failed. Test output:
    \\```
    \\{s}
    \\```
    \\Fix the failures.
;

pub const rebase_phase =
    \\This branch has merge conflicts with main.
    \\Rebase onto origin/main, resolve all conflicts, and ensure tests pass.
    \\Read spec.md for context on what this branch does.
;

pub const rebase_error_fmt =
    \\
    \\
    \\Previous error context:
    \\```
    \\{s}
    \\```
;

// ── Seed prompts ───────────────────────────────────────────────────────

pub const seed_refactor =
    \\Analyze this codebase and identify 1-3 concrete, small improvements.
    \\Focus on refactoring, code quality, and bug fixes — not new features.
;

pub const seed_security =
    \\Audit this codebase for bugs, security vulnerabilities, and reliability issues.
    \\Look for: race conditions, resource leaks, error handling gaps,
    \\integer overflows, injection vulnerabilities, undefined behavior.
    \\Create a task for each real issue. Skip false positives.
;

pub const seed_tests =
    \\Identify gaps in test coverage that matter for correctness.
    \\Create tasks to add specific test cases targeting individual functions or modules.
;

pub const seed_features =
    \\Suggest 1-3 concrete features that would meaningfully improve this project.
    \\
    \\For each proposal, output EXACTLY this format:
    \\
    \\PROPOSAL_START
    \\TITLE: <short imperative title, max 80 chars>
    \\DESCRIPTION: <2-4 sentences explaining the feature>
    \\RATIONALE: <1-2 sentences on why this would be valuable>
    \\PROPOSAL_END
    \\
    \\Output ONLY the proposal blocks above. No other text.
;

pub const seed_task_suffix =
    \\
    \\
    \\For each improvement, output EXACTLY this format (one per task):
    \\
    \\TASK_START
    \\TITLE: <short imperative title, max 80 chars>
    \\DESCRIPTION: <2-4 sentences explaining what to change and why>
    \\TASK_END
    \\
    \\Output ONLY the task blocks above. No other text.
;

// ── Director prompt ────────────────────────────────────────────────────

pub const director_system =
    \\You are {s}, a director-level AI agent controlling the borg system.
    \\You speak using plural pronouns (we/us/our). You are a collective.
    \\
    \\Manage the engineering pipeline via the REST API at http://127.0.0.1:{d}.
    \\Use curl from Bash.
    \\
    \\### API
    \\```
    \\GET    /api/tasks                     List tasks
    \\GET    /api/tasks/<id>                Task detail + agent output
    \\POST   /api/tasks                     Create task: {{"title":"...","description":"...","repo":"..."}}
    \\DELETE /api/tasks/<id>                Cancel/delete task
    \\POST   /api/release                   Trigger integration
    \\GET    /api/queue                      Integration queue
    \\GET    /api/status                     System status
    \\```
    \\
    \\You have full Bash, Read, Write, Edit, Glob, Grep access to the filesystem.
    \\
;

// ── Tests ──────────────────────────────────────────────────────────────

test "getSystemPrompt returns non-empty for all personas" {
    try std.testing.expect(getSystemPrompt(.manager).len > 0);
    try std.testing.expect(getSystemPrompt(.qa).len > 0);
    try std.testing.expect(getSystemPrompt(.worker).len > 0);
}
