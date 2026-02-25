# Task #5: Fix subprocess stdout/stderr sequential read deadlock

## 1. Task Summary

`git.zig`, `agent.zig`, `docker.zig`, `pipeline.zig`, and `main.zig` all read stdout to completion before reading stderr (or vice versa) in a sequential loop. If a child process fills the OS pipe buffer (~64 KB on Linux) on the stream being ignored while the parent is blocking on the other stream, both sides deadlock permanently. The fix is to drain both streams concurrently using a dedicated stderr-reader thread, exposed via a new `subprocess.zig` module, and to extract the repeated single-pipe drain loop and exit-code switch into a `process.zig` utility module used by all affected call sites.

## 2. Files to Modify

| File | Location | What changes |
|------|----------|--------------|
| `src/git.zig` | `Git.exec()` lines 23–42 | Replace sequential dual-drain + exit-code switch with `subprocess.collectOutput()` and `process.exitCode()` |
| `src/agent.zig` | `runDirect()` lines 119–135 | Replace stdout-only sequential drain + exit-code switch with `subprocess.collectOutput()` and `process.exitCode()` |
| `src/docker.zig` | `Docker.runWithStdio()` lines 174–189 | Replace stdout-only sequential drain + exit-code switch with `subprocess.collectOutput()` and `process.exitCode()` |
| `src/pipeline.zig` | `runTests()` lines 931–954 | Replace sequential dual-drain + exit-code switch with `subprocess.collectOutput()` and `process.exitCode()` |
| `src/pipeline.zig` | `checkSelfUpdate()` lines 1258–1283 | Replace reversed sequential dual-drain + exit-code switch with `subprocess.collectOutput()` and `process.exitCode()` |
| `src/main.zig` | Agent dispatch function lines 947–962 | Replace stdout-only sequential drain + exit-code switch with `subprocess.collectOutput()` and `process.exitCode()` |

## 3. Files to Create

| File | Purpose |
|------|---------|
| `src/subprocess.zig` | Concurrent stdout+stderr collector; spawns one thread to drain stderr while calling thread drains stdout |
| `src/process.zig` | Low-level utilities: `drainPipe` (single-pipe sequential read into owned slice) and `exitCode` (Child.Term → u8) |

Both new files must be reachable from `src/main.zig` (directly or transitively via an existing module) so that `zig build test` picks up their embedded tests. The existing test files `src/subprocess_test.zig` and `src/process_test.zig` already contain the full test suite for these modules; they must be imported (e.g., via `comptime { _ = @import("subprocess_test.zig"); }`) from within one of the new source files or from `main.zig`.

## 4. Function/Type Signatures

### `src/subprocess.zig`

```zig
const std = @import("std");

/// Holds fully-buffered stdout and stderr captured from a child process.
pub const PipeOutput = struct {
    stdout: []u8,
    stderr: []u8,
    allocator: std.mem.Allocator,

    /// Frees both owned slices.
    pub fn deinit(self: *PipeOutput) void;
};

/// Drain child.stdout on the calling thread and child.stderr on a dedicated
/// thread simultaneously, preventing pipe-buffer deadlocks.
/// Returns after both streams reach EOF and the stderr thread is joined.
/// Each stream is capped independently at max_size bytes; bytes beyond that
/// limit are still read and discarded so the child never blocks.
/// Streams that are null (not piped) are treated as producing zero bytes.
/// The caller must call child.wait() after this function returns.
pub fn collectOutput(
    allocator: std.mem.Allocator,
    child: *std.process.Child,
    max_size: usize,
) !PipeOutput;
```

### `src/process.zig`

```zig
const std = @import("std");

/// Read all bytes from `pipe` into a newly allocated owned slice.
/// Stops on EOF or a read error (accumulated data is returned, not an error).
/// Caller owns the returned slice and must free it.
pub fn drainPipe(allocator: std.mem.Allocator, pipe: std.fs.File) ![]u8;

/// Convert a Child.Term to a u8 exit code.
/// .Exited => the exit code; all other variants (Signal, Stopped, Unknown) => 1.
pub fn exitCode(term: std.process.Child.Term) u8;
```

### Changes to existing signatures

All public types remain **unchanged**:
- `git.ExecResult` — fields `stdout`, `stderr`, `exit_code`, `allocator`; methods `success()`, `deinit()`
- `docker.RunResult` — fields `stdout`, `exit_code`, `allocator`
- `agent.AgentResult` — fields `output`, `raw_stream`, `new_session_id`

Only the internal implementation of the functions listed in §2 changes.

## 5. Acceptance Criteria

**AC1 – Deadlock on large stderr is resolved.**
`collectOutput` with a child writing 128 KB to stderr and 0 bytes to stdout completes without hanging; `output.stderr.len == 128 * 1024` and `output.stdout.len == 0`.

**AC2 – Deadlock on large stdout is resolved.**
`collectOutput` with a child writing 128 KB to stdout and 0 bytes to stderr completes; `output.stdout.len == 128 * 1024` and `output.stderr.len == 0`.

**AC3 – Deadlock on simultaneous large writes is resolved.**
A child alternating 2 KB chunks to stdout and stderr for 100 iterations completes; `output.stdout.len == 204800` and `output.stderr.len == 204800`.

**AC4 – No inline drain buffer remains in modified files.**
After refactoring, `git.zig`, `docker.zig`, `agent.zig`, and `pipeline.zig` do not contain the literal string `"read_buf: [8192]u8"`.

**AC5 – No inline exit-code switch remains in modified files.**
After refactoring, `git.zig`, `docker.zig`, `agent.zig`, and `pipeline.zig` do not contain the literal string `".Exited => |code| code"`.

**AC6 – `process.zig` exports the required symbols with correct signatures.**
- `process.drainPipe` has type `fn(std.mem.Allocator, std.fs.File) anyerror![]u8`.
- `process.exitCode` has type `fn(std.process.Child.Term) u8`.
- `process.exitCode(.{ .Exited = 0 }) == 0`, `process.exitCode(.{ .Exited = 42 }) == 42`, `process.exitCode(.{ .Signal = 9 }) == 1`, `process.exitCode(.{ .Stopped = 19 }) == 1`.

**AC7 – `process.zig` has no project-internal imports.**
The source of `process.zig` contains exactly one `@import` call and it is `@import("std")`.

**AC8 – `collectOutput` captures data correctly.**
- `output.stdout == "hello stdout\n"` for a child running `echo 'hello stdout'`.
- `output.stderr == "hello stderr\n"` for a child running `echo 'hello stderr' >&2`.
- Both streams are correct when child writes to both simultaneously.
- Binary data containing null bytes (`\x00\x01\x02\xff`) is preserved verbatim.

**AC9 – Thread is always joined; no thread leaks.**
Running `collectOutput` 20 times sequentially with 1 KB output on each stream does not exhaust thread resources or leak memory (verified by `std.testing.allocator`).

**AC10 – `PipeOutput.deinit` frees memory without leaks.**
`std.testing.allocator` reports no leaks after `output.deinit()`.

**AC11 – Public API of consuming modules is unchanged.**
`git.ExecResult`, `docker.RunResult`, and `agent.AgentResult` still have the same fields and methods as before; callers require no changes.

**AC12 – Modified files import `process.zig`.**
`git.zig`, `docker.zig`, `agent.zig`, and `pipeline.zig` each contain `@import("process.zig")`.

## 6. Edge Cases to Handle

| # | Scenario | Expected behaviour |
|---|----------|--------------------|
| E1 | Child closes stdout before stderr finishes | Stderr thread continues draining until its own EOF; both slices returned correctly |
| E2 | Child closes stderr before stdout finishes | Main thread continues draining stdout to EOF; both slices returned correctly |
| E3 | Child produces zero bytes on one or both streams | Return a valid zero-length owned slice per stream; `deinit()` must not crash or fault |
| E4 | Child exits before parent reads all pipe data | Pipe buffers remain readable after child exit; all buffered bytes are captured |
| E5 | Output exceeds `max_size` per stream | Excess bytes are drained and discarded so the child never blocks; `output.stdout.len <= max_size` and `output.stderr.len <= max_size` |
| E6 | Read error mid-stream (process crash or kill) | The inner loop breaks via `catch break`; accumulated data is returned without propagating an error to the caller |
| E7 | Child terminated by signal | `process.exitCode` returns `1`; any partial output already in the pipe is still returned |
| E8 | `child.stdout` or `child.stderr` is `null` | `collectOutput` treats the absent stream as zero bytes; no null-pointer dereference occurs |
| E9 | `checkSelfUpdate` reverse-order drain (stderr first, then stdout) | After replacing with `collectOutput`, drain order is irrelevant; both streams drain concurrently |
| E10 | Stderr thread allocation or spawn failure | Error is propagated to caller; if the thread was started it is still joined before returning to prevent resource leaks |
