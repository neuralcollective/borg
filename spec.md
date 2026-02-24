<<<<<<< HEAD
# Spec: Extract duplicated SQLite parameter binding logic into shared helper

## Task Summary

The `query()` function (lines 106-128) and `execute()` function (lines 165-187) in `src/sqlite.zig` contain identical 22-line `inline for` blocks that bind tuple parameters to SQLite prepared statements. This duplication means any future type-support change (e.g. adding float or blob binding) must be made in two places. Extract the shared logic into a single `inline fn bindParams` that both functions call.

## Files to Modify

1. **`src/sqlite.zig`** — Add `bindParams` helper, replace duplicated inline-for blocks in `query()` and `execute()` with calls to it.
=======
# Spec: Fix subprocess stdout/stderr sequential read deadlock

## Task Summary

In multiple files (`git.zig`, `gt.zig`, `pipeline.zig`), stdout is read to completion before stderr is read (or vice versa). If a child process fills the OS pipe buffer (~64KB on Linux) writing to stderr while the parent blocks reading stdout, both processes deadlock — the child blocks on its stderr write and the parent blocks on its stdout read. Additionally, `agent.zig` and `main.zig` pipe stderr but never drain it, which can deadlock if the child writes enough to stderr. The fix is to drain both streams concurrently using a dedicated thread for one stream while the calling thread reads the other.

## Files to Modify

1. **`src/git.zig`** — Replace sequential stdout-then-stderr reads in `Git.exec()` (lines 27-40) with concurrent draining.
2. **`src/gt.zig`** — Replace sequential stdout-then-stderr reads in `Gt.exec()` (lines 30-43) with concurrent draining.
3. **`src/pipeline.zig`** — Replace sequential reads in `runTestCommandForRepo()` (lines 825-838) and in `checkSelfUpdate()` (lines 1190-1206) with concurrent draining.
4. **`src/agent.zig`** — In `runDirect()` (lines 109-118), stderr is piped but never read; add concurrent stderr draining alongside stdout reads.
5. **`src/main.zig`** — In the agent runner (lines 946-955), stderr is piped but never read; add concurrent stderr draining alongside stdout reads.
>>>>>>> 1923b60 (spec: generate spec.md for task)

## Files to Create

1. **`src/subprocess.zig`** — Shared utility providing concurrent stdout/stderr draining for child processes.

## Function/Type Signatures

<<<<<<< HEAD
### `src/sqlite.zig`

#### `bindParams` — new private inline function

```zig
inline fn bindParams(stmt: *c.sqlite3_stmt, params: anytype) SqliteError!void
```

- Accepts a non-null `*c.sqlite3_stmt` (caller must unwrap the optional before calling) and a tuple of parameters.
- Iterates over `params` with `inline for` and binds each element using the same logic currently duplicated in `query()` and `execute()`:
  - `isStringType` types → `sqlite3_bind_text` with `SQLITE_TRANSIENT`
  - `.int` / `.comptime_int` types → `sqlite3_bind_int64`
  - `.optional` types → unwrap: bind inner value (text or int64), or `sqlite3_bind_null` if `null`
  - Returns `SqliteError.BindFailed` if any bind call returns non-`SQLITE_OK`.
- This is a standalone `inline fn` at module scope (not a method on `Database`), since it only needs the statement handle and params, not `self`.

#### `Database.query` — modify (lines 96-154)

Replace lines 105-128 (the `// Bind parameters` comment and the `inline for` block) with:

```zig
try bindParams(stmt.?, params);
```

No other changes to `query()`.

#### `Database.execute` — modify (lines 156-194)

Replace lines 165-187 (the `inline for` block) with:

```zig
try bindParams(stmt.?, params);
```

No other changes to `execute()`.
=======
### `src/subprocess.zig` (new file)

```zig
const std = @import("std");

pub const PipeOutput = struct {
    stdout: []const u8,
    stderr: []const u8,
    allocator: std.mem.Allocator,

    pub fn deinit(self: *PipeOutput) void {
        self.allocator.free(self.stdout);
        self.allocator.free(self.stderr);
    }
};

/// Concurrently reads both stdout and stderr from a spawned child process.
/// Spawns a thread to drain stderr while the calling thread drains stdout.
/// Both streams are read to EOF. Returns owned slices for both.
/// The child must already be spawned with stdout_behavior = .Pipe and stderr_behavior = .Pipe.
pub fn collectOutput(allocator: std.mem.Allocator, child: *std.process.Child) !PipeOutput
```

Implementation approach:
- Spawn a thread that reads `child.stderr` into a `std.ArrayList(u8)` in a loop until EOF.
- On the calling thread, read `child.stdout` into a `std.ArrayList(u8)` in a loop until EOF.
- Join the stderr thread.
- Return both buffers as owned slices via `PipeOutput`.
- If the stderr thread encounters an allocation error, it stores the error. After join, the calling thread checks and propagates it.

Internal helper (thread entry point):

```zig
const StderrReader = struct {
    allocator: std.mem.Allocator,
    pipe: std.fs.File,
    result: []const u8 = &.{},
    err: ?anyerror = null,
};

fn readStderr(ctx: *StderrReader) void
```
>>>>>>> 1923b60 (spec: generate spec.md for task)

### `src/git.zig` — changes to `Git.exec()`

Replace lines 23-40 (manual sequential reads) with:

```zig
const subprocess = @import("subprocess.zig");

// In exec(), after child.spawn():
var output = try subprocess.collectOutput(self.allocator, &child);
// Then use output.stdout and output.stderr instead of stdout_buf/stderr_buf
```

The returned `ExecResult` is constructed from `output.stdout` and `output.stderr` (transferring ownership, no copy needed).

### `src/gt.zig` — changes to `Gt.exec()`

Same pattern as `git.zig`: replace lines 26-43 with a call to `subprocess.collectOutput()`.

```zig
const subprocess = @import("subprocess.zig");

// In exec(), after child.spawn():
var output = try subprocess.collectOutput(self.allocator, &child);
```

### `src/pipeline.zig` — changes to `runTestCommandForRepo()`

Replace lines 821-838 with:

```zig
const subprocess = @import("subprocess.zig");

// In runTestCommandForRepo(), after child.spawn():
var output = try subprocess.collectOutput(self.allocator, &child);
```

The `TestResult` is constructed from `output.stdout` and `output.stderr`.

### `src/pipeline.zig` — changes to `checkSelfUpdate()`

Replace lines 1190-1206 (which reads stderr first, then drains stdout without storing) with:

```zig
var output = try subprocess.collectOutput(self.allocator, &child);
defer output.deinit();
```

### `src/agent.zig` — changes to `runDirect()`

Replace lines 109-118 with concurrent reading of both streams:

```zig
const subprocess = @import("subprocess.zig");

// In runDirect(), after writing prompt to stdin and closing it:
var output = try subprocess.collectOutput(allocator, &child);
defer allocator.free(output.stderr); // stderr not used but must be drained
// Use output.stdout for parseNdjson (transfer ownership to stdout_buf equivalent)
```

### `src/main.zig` — changes to agent runner

Replace lines 946-955 with concurrent reading:

```zig
const subprocess = @import("subprocess.zig");

// After writing prompt to stdin and closing it:
var output = try subprocess.collectOutput(ctx.allocator, &child);
defer output.deinit();
// Use output.stdout for parseNdjson
```

Note: In `main.zig`, if stderr is currently set to `.Inherit` or `.Close` rather than `.Pipe`, no change is needed. Only locations where `stderr_behavior = .Pipe` and the pipe is not drained are affected.

## Acceptance Criteria

<<<<<<< HEAD
1. **Single definition**: The `inline for` parameter-binding logic exists exactly once in `src/sqlite.zig`, inside the `bindParams` function. Neither `query()` nor `execute()` contain an `inline for` over `params`.
2. **Behavioral equivalence**: `bindParams` handles the same type cases as the original code — string types (`isStringType`), integer types (`.int`, `.comptime_int`), and optional types (unwrapping to text/int64 or binding null). The binding index calculation (`i + 1`) is preserved.
3. **Error propagation**: `bindParams` returns `SqliteError.BindFailed` on any failed bind, and both `query()` and `execute()` propagate this error via `try`.
4. **Build succeeds**: `zig build` compiles without errors or warnings.
5. **Tests pass**: `zig build test` passes. All existing callers of `query()` and `execute()` (in `src/db.zig` and elsewhere) continue to work without modification.
6. **No public API change**: `Database.query` and `Database.execute` retain their existing public signatures. `bindParams` is a private module-level function (not `pub`).

## Edge Cases

1. **Empty params tuple**: Calling `query(alloc, sql, .{})` or `execute(sql, .{})` with an empty tuple must still work — `bindParams` with an empty tuple is a no-op (the `inline for` iterates zero times).
2. **Optional null values**: `bindParams` must correctly call `sqlite3_bind_null` when an optional parameter is `null`, same as the current code.
3. **Mixed parameter types**: A call like `execute(sql, .{ "text", 42, @as(?[]const u8, null) })` with mixed string, integer, and null-optional params must bind all three correctly in order.
4. **Comptime int literals**: Parameters like `.{ 1, 2 }` (comptime_int) must continue to work — the `.comptime_int` check in `@typeInfo(T)` must be preserved.
5. **String-coercible types**: Pointer-to-array types (e.g. `*const [5]u8` from string literals) must still be handled via `isStringType` and coerced to `[]const u8`.
6. **`rc` variable scoping**: The current code reuses the outer `var rc` from `prepare_v2` for binding results. The new `bindParams` function must use its own local `rc` variable (or check return codes inline), since it won't have access to the caller's `rc`. The caller's `rc` variable remains available for post-bind use (e.g. `sqlite3_step`).
=======
1. **No sequential reads remain**: Every location where a child process has both `stdout_behavior = .Pipe` and `stderr_behavior = .Pipe` drains both streams concurrently, not sequentially.
2. **No unread pipes remain**: Every location where stderr is piped (`.Pipe`) has the stream drained — no piped stream is left unread.
3. **Deadlock is eliminated**: A child process that writes >64KB to stderr while also writing to stdout does not cause a deadlock. Specifically, a test where a subprocess writes 128KB to stderr and 128KB to stdout completes without hanging.
4. **Existing behavior preserved**: `ExecResult` and `TestResult` contain the same stdout/stderr content as before. All callers that inspect `.stdout`, `.stderr`, and `.exit_code` continue to work identically.
5. **Build succeeds**: `zig build` and `zig build test` pass without errors.
6. **Existing tests pass**: All existing unit tests in `git.zig`, `agent.zig`, and `pipeline_shutdown_test.zig` continue to pass.
7. **`collectOutput` is reusable**: The new `subprocess.zig` module is imported by all affected files, avoiding code duplication.
8. **Thread cleanup**: The stderr reader thread is always joined before `collectOutput` returns, even if the stdout read encounters an error (use `errdefer` to join).

## Edge Cases

1. **Child produces no output**: If stdout and stderr are both empty, `collectOutput` returns two empty slices. No thread synchronization issue — the stderr thread reads EOF immediately.
2. **Child closes stderr before stdout**: The stderr reader thread finishes and returns. The calling thread continues reading stdout normally. Join succeeds immediately.
3. **Child closes stdout before stderr**: The calling thread finishes stdout read and blocks on joining the stderr thread. The stderr thread continues until the child closes stderr (which happens when the child exits or explicitly closes fd 2). No deadlock because both pipes are being drained.
4. **Child writes only to stderr (stdout empty)**: The calling thread reads EOF on stdout immediately, then joins the stderr thread which reads all stderr data. No deadlock.
5. **Child writes >64KB to one stream, 0 to the other**: Both streams are drained independently, so the pipe buffer never fills while the other side is blocked.
6. **Allocation failure in stderr reader thread**: The thread stores the error and returns. After join, `collectOutput` checks for the stored error and propagates it to the caller. Any partial stderr data is freed.
7. **Child process crashes mid-output**: Broken pipe / EOF is detected on the read side. Both reader loops terminate. The thread is joined. `child.wait()` returns the appropriate exit status.
8. **`child.stdout` or `child.stderr` is null despite `.Pipe` behavior**: Defensive check — if either is null, fall back to returning an empty slice for that stream. This matches the existing `if (child.stdout) |stdout|` guard pattern.
9. **Self-update build in `pipeline.zig`**: Currently reads stderr first then drains stdout (reverse order). The fix handles both directions equivalently since both are drained concurrently.
10. **Very large output (hundreds of MB)**: The `ArrayList` will grow dynamically. This is unchanged from current behavior — no new memory bound is introduced. Callers that truncate output (e.g., `pipeline.zig` using `@min(len, 2000)`) continue to do so.
>>>>>>> 1923b60 (spec: generate spec.md for task)
