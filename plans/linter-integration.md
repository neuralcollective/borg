# Linter Integration Plan

## Goal

Run language-appropriate linters as a pipeline phase so agents produce
clean, lint-passing code without relying on prompt text alone.

---

## Architecture: New `lint_fix` Phase Type

Add a `lint_fix` phase type alongside the existing `agent`, `rebase`, and
`test_check` types. It runs a lint command, and if it produces output,
spawns an agent to fix the issues, then re-runs lint to verify.

```
impl → lint_fix → done → release
```

The lint_fix phase:
1. Runs `repo.lint_cmd` in the worktree
2. If exit 0 and no output → advance to `done`
3. If non-zero or non-empty output → spawn agent with lint output as context
4. Re-run lint to verify fix (up to `lint_fix_max_attempts`, default 2)
5. If lint still fails after max attempts → fail task (same path as test failure)

---

## Configuration

### Per-repo lint command

Extend `RepoConfig` with an optional `lint_cmd` field. Two ways to configure:

**Option 1 — WATCHED_REPOS fourth field** (consistent with existing format):
```
WATCHED_REPOS=path:test_cmd:mode:lint_cmd|...
```
Example:
```
WATCHED_REPOS=/home/shulgin/borg:zig build test:sweborg:zig build check
```
The primary repo uses a new `PIPELINE_LINT_CMD` env var.

**Option 2 — `.borg/lint.sh` convention** (zero-config):
If a file `.borg/lint.sh` exists in the repo, use it as the lint command.
Checked after explicit config, so explicit config takes priority.

Recommendation: implement both; explicit config wins.

### Auto-detection (future)

Could auto-detect linters by file presence:
- `eslint.config.*` or `.eslintrc*` → `npx eslint .`
- `biome.json` → `npx biome check .`
- `build.zig` → `zig build check` (if target exists)
- `Cargo.toml` → `cargo clippy -- -D warnings`
- `pyproject.toml` with ruff → `ruff check .`

Auto-detection is low priority; explicit config is clearer.

---

## Phase Config Changes

```zig
pub const PhaseType = enum {
    agent,
    rebase,
    test_check,
    lint_fix,     // NEW
};

pub const PhaseConfig = struct {
    // existing fields...
    lint_max_attempts: u32 = 2,
};
```

The `lint_fix` phase needs no `system_prompt` of its own — it uses a
hardcoded lint-fix system prompt (see Prompts section below).

---

## Pipeline Logic (`pipeline.zig`)

New `runLintFixPhase` function:

```zig
fn runLintFixPhase(self: *Pipeline, task: PipelineTask, phase: *const PhaseConfig) !void {
    const lint_cmd = self.repoLintCmd(task.repo_path) orelse {
        // No lint cmd configured — skip phase, advance to next
        try self.advanceTask(task, phase.next);
        return;
    };

    var attempts: u32 = 0;
    while (attempts < phase.lint_max_attempts + 1) : (attempts += 1) {
        const lint_result = self.runLintCmd(task.repo_path_worktree, lint_cmd);
        if (lint_result.clean) {
            try self.advanceTask(task, phase.next);
            return;
        }
        if (attempts >= phase.lint_max_attempts) break;

        // Spawn agent to fix lint errors
        const prompt = try std.fmt.allocPrint(...,
            "Fix all lint errors. Lint output:\n\n{s}", .{lint_result.output});
        const result = try self.spawnAgent(lint_fix_system, tools, prompt, worktree, ...);
        // commit if any changes
    }

    // Still failing — fail the task
    try self.failTask(task, lint_result.output);
}
```

`repoLintCmd` checks `RepoConfig.lint_cmd`, then falls back to `.borg/lint.sh`.

---

## Prompts

New `lint_fix_system` prompt:

```
You are a lint-fix agent. Your only job is to make the codebase pass
the project's linter with zero warnings or errors. Do not refactor,
rename, or change logic — only fix what the linter reports.
Read the lint output carefully and make the minimal changes needed.
After editing, do not run the linter yourself — the pipeline will verify.
```

---

## Mode Changes (`modes.zig`)

Add `lint_fix` phase to `sweborg` between `impl` and `done`:

```zig
// After impl phase
.{
    .name = "lint_fix",
    .phase_type = .lint_fix,
    .next = "done",
    .priority = 4,
    .allow_no_changes = true,  // lint may already pass
},
```

The phase is a no-op if no lint command is configured for the repo, so
it's safe to add to all modes.

---

## Dashboard

- Show lint output in task detail alongside agent output (new output key `"lint_fix"`)
- Status badge: new `lint_fix` status (amber, similar to `qa`)
- No new panel needed — fits into existing task detail layout

---

## Rollout Order

1. `RepoConfig.lint_cmd` + `PIPELINE_LINT_CMD` env parsing
2. `lint_fix` phase type + `runLintFixPhase` in `pipeline.zig`
3. `repoLintCmd` helper (config → `.borg/lint.sh` fallback)
4. Add `lint_fix` phase to `sweborg` and `webborg` modes
5. Lint status in dashboard (types.ts, status-badge, task-detail output key)
6. Auto-detection (optional, later)

---

## What This Does NOT Do

- Does not replace the test command — linting and testing are separate steps
- Does not block proposals/seed scans on lint (that's a pipeline concern only)
- Does not enforce a specific linter — user supplies the command
