const std = @import("std");

// ── Seed prompts ───────────────────────────────────────────────────────

pub const seed_explore_preamble =
    \\First, thoroughly explore the codebase before making any suggestions.
    \\Use Read to examine key source files, Grep to search for patterns,
    \\and Glob to discover the project structure. Understand the architecture,
    \\existing patterns, and current state of the code.
    \\
    \\Then, based on your exploration:
    \\
;

pub const seed_refactor =
    \\Identify 1-3 concrete, small improvements.
    \\Focus on refactoring, code quality, and bug fixes — not new features.
;

pub const seed_security =
    \\Audit for bugs, security vulnerabilities, and reliability issues.
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
    \\Base your suggestions on actual gaps you found while exploring the code.
;

pub const seed_architecture =
    \\Identify 1-2 significant structural improvements. Think big: module
    \\reorganization, API redesigns, performance overhauls, major refactors
    \\that span multiple files, or replacing approaches that have outgrown
    \\their original design.
    \\
    \\Each proposal should be a multi-day project, not a quick fix.
;

pub const seed_cross_pollinate =
    \\Study this codebase to understand its patterns, features, and architecture.
    \\Then suggest 1-3 ideas inspired by what you see here that could be adapted
    \\or ported to a DIFFERENT project (described below). The ideas don't need to
    \\be direct copies — they can be inspired by patterns, approaches, or
    \\capabilities you observe here.
    \\
    \\Target project to generate proposals for:
    \\
;

pub const seed_proposal_suffix =
    \\
    \\For each proposal, output EXACTLY this format:
    \\
    \\PROPOSAL_START
    \\TITLE: <short imperative title, max 80 chars>
    \\DESCRIPTION: <2-4 sentences explaining the feature or change>
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

test "seed prompts are non-empty" {
    try std.testing.expect(seed_explore_preamble.len > 0);
    try std.testing.expect(seed_refactor.len > 0);
    try std.testing.expect(seed_task_suffix.len > 0);
    try std.testing.expect(seed_proposal_suffix.len > 0);
}
