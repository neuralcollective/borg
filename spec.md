# Task #30: Fix failing build on main

## 1. Task Summary

`build.zig` calls `b.run(&.{ "git", "rev-parse", "--short", "HEAD" })` to embed a git commit hash at compile time. `b.run()` panics on any non-zero exit code, so when the build is invoked inside a git worktree (e.g. the pipeline agent working in `.git/worktrees/task-30`), git exits with "fatal: not a git repository" and the entire build fails before any Zig source is compiled. The fix replaces `b.run()` with a graceful `std.process.Child.run()` call that falls back to `"unknown"` on any git error, so all build environments (worktree, bare clone, CI sandbox without git) succeed.

## 2. Files to Modify

| File | What changes |
|------|--------------|
| `build.zig` | Replace `b.run()` git hash call with a fallback-safe `std.process.Child.run()` block |

No files need to be created.

## 3. Implementation

### `build.zig` — git hash with fallback

Replace lines 25–28 (current):

```zig
// BEFORE — panics when git fails:
const git_hash = b.run(&.{ "git", "rev-parse", "--short", "HEAD" });
const build_options = b.addOptions();
build_options.addOption([]const u8, "git_hash", std.mem.trim(u8, git_hash, &std.ascii.whitespace));
```

With:

```zig
// AFTER — falls back to "unknown" on any git error:
const git_hash: []const u8 = blk: {
    const result = std.process.Child.run(.{
        .allocator = b.allocator,
        .argv = &.{ "git", "rev-parse", "--short", "HEAD" },
    }) catch break :blk "unknown";
    defer b.allocator.free(result.stderr);
    if (result.term == .Exited and result.term.Exited == 0) {
        break :blk std.mem.trim(u8, result.stdout, &std.ascii.whitespace);
    }
    b.allocator.free(result.stdout);
    break :blk "unknown";
};
const build_options = b.addOptions();
build_options.addOption([]const u8, "git_hash", git_hash);
```

The `build_options` module wiring (`exe_mod.addOptions("build_options", build_options)`) and all downstream consumers of `@import("build_options").git_hash` are unchanged.

## 4. Acceptance Criteria

**AC1 — Build succeeds in a git worktree.**
Running `zig build` inside a git worktree (where `git rev-parse --short HEAD` exits non-zero) completes with exit code 0. The compiled binary contains `build_options.git_hash == "unknown"`.

**AC2 — Build succeeds in a normal git checkout.**
Running `zig build` in the main repository directory embeds the actual short SHA (7 hex characters) in `build_options.git_hash`.

**AC3 — Build succeeds with no git binary installed.**
When `git` is not on `PATH`, `std.process.Child.run()` returns a `FileNotFound` error; the `catch break :blk "unknown"` arm runs and the build proceeds normally.

**AC4 — Build succeeds in a directory with no `.git` folder.**
Running `zig build` from a plain directory (e.g. a tarball extract) results in `build_options.git_hash == "unknown"` and exit code 0.

**AC5 — All unit tests pass without regression.**
`zig build test` completes with exit code 0. No existing tests are broken by the change.

**AC6 — `build_options` module is unchanged for callers.**
Any source file that does `@import("build_options").git_hash` continues to receive a `[]const u8` value; no call sites require modification.

## 5. Edge Cases

| # | Scenario | Expected behaviour |
|---|----------|--------------------|
| E1 | Detached HEAD / shallow clone | `git rev-parse --short HEAD` still exits 0; real hash is embedded normally |
| E2 | `git` exits 0 but stdout is empty | `std.mem.trim` yields `""`; build proceeds; `git_hash` is `""` (acceptable sentinel) |
| E3 | `git` exits 0 but stdout contains only whitespace | `std.mem.trim` yields `""`; same as E2 |
| E4 | `git` command times out or is killed by signal | `result.term` is `.Signal` or `.Stopped`, not `.Exited`; the non-zero branch frees stdout and breaks with `"unknown"` |
| E5 | OOM when allocating child process buffers | `std.process.Child.run()` returns an error; `catch break :blk "unknown"` handles it |
| E6 | Build run from a subdirectory of the repo | git resolves the repo root via parent traversal; exits 0 and embeds the real hash |
| E7 | CI environment with shallow fetch (`--depth 1`) | `git rev-parse --short HEAD` succeeds on shallow clones; real hash embedded |
| E8 | Multiple simultaneous `zig build` invocations in the same worktree | Each invocation independently runs `std.process.Child.run()`; no shared state; all succeed with `"unknown"` |
