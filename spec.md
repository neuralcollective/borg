# Spec: Consolidate duplicated child-process stdout/stderr drain pattern

## Task Summary

Four source files (`git.zig`, `docker.zig`, `pipeline.zig`, `agent.zig`) each contain copy-pasted logic for draining stdout/stderr from a `std.process.Child` into `ArrayList(u8)` buffers using an 8192-byte read loop, and for extracting a `u8` exit code from the `Child.Term` union via a switch on `.Exited`. This task extracts both patterns into a shared `process.zig` utility module and replaces all five call sites (two in `pipeline.zig`) with calls to the shared functions.

## Files to Modify

1. **`src/git.zig`** — Replace drain loop + exit code switch in `Git.exec` (lines 23–46) with calls to `process.drainPipe` and `process.exitCode`.
2. **`src/docker.zig`** — Replace drain loop + exit code switch in `Docker.runWithStdio` (lines 175–189) with calls to `process.drainPipe` and `process.exitCode`.
3. **`src/agent.zig`** — Replace drain loop + exit code switch in `runDirect` (lines 109–124) with calls to `process.drainPipe` and `process.exitCode`.
4. **`src/pipeline.zig`** — Replace drain loop + exit code switch in `runTestCommandForRepo` (lines 757–780) and in the self-update build section (lines 1039–1064) with calls to `process.drainPipe` and `process.exitCode`.

## Files to Create

1. **`src/process.zig`** — New utility module containing `drainPipe` and `exitCode`.

## Function/Type Signatures

### `src/process.zig`

```zig
const std = @import("std");

/// Read all bytes from a ChildProcess pipe (stdout or stderr) into a
/// caller-owned slice. Uses a stack-local 8192-byte buffer for reads.
/// Returns an allocated slice that the caller must free with `allocator.free()`.
pub fn drainPipe(allocator: std.mem.Allocator, pipe: std.fs.File) ![]u8

/// Extract a u8 exit code from a process termination status.
/// Returns the exit code for normal exits, or 1 for signals/stops/unknown.
pub fn exitCode(term: std.process.Child.Term) u8
```

#### `drainPipe` implementation outline

- Declare `var buf: [8192]u8 = undefined;`
- Declare `var list = std.ArrayList(u8).init(allocator);`
- Loop: `const n = pipe.read(&buf) catch break; if (n == 0) break; try list.appendSlice(buf[0..n]);`
- Return `try list.toOwnedSlice()`

#### `exitCode` implementation outline

- `return switch (term) { .Exited => |code| code, else => 1 };`

### Changes to `src/git.zig`

In `Git.exec`, add `const process = @import("process.zig");` at the top.

Replace lines 23–46:
```zig
// Before (remove):
var stdout_buf = std.ArrayList(u8).init(self.allocator);
var stderr_buf = std.ArrayList(u8).init(self.allocator);
var read_buf: [8192]u8 = undefined;
// ... drain loops ...
const term = try child.wait();
const exit_code: u8 = switch (term) { .Exited => |code| code, else => 1 };

// After:
const stdout_data = if (child.stdout) |pipe| try process.drainPipe(self.allocator, pipe) else try self.allocator.alloc(u8, 0);
const stderr_data = if (child.stderr) |pipe| try process.drainPipe(self.allocator, pipe) else try self.allocator.alloc(u8, 0);
const term = try child.wait();
const exit_code = process.exitCode(term);
```

Return `ExecResult` with `stdout_data` and `stderr_data` directly (already owned slices).

### Changes to `src/docker.zig`

In `Docker.runWithStdio`, add `const process = @import("process.zig");` at the top.

Replace lines 175–189:
```zig
// After:
const stdout_data = if (child.stdout) |pipe| try process.drainPipe(self.allocator, pipe) else try self.allocator.alloc(u8, 0);
const term = try child.wait();
const exit_code = process.exitCode(term);
```

### Changes to `src/agent.zig`

In `runDirect`, add `const process = @import("process.zig");` at the top.

Replace lines 109–124:
```zig
// After:
var stdout_buf = std.ArrayList(u8).init(allocator);
defer stdout_buf.deinit();
if (child.stdout) |pipe| {
    const data = try process.drainPipe(allocator, pipe);
    defer allocator.free(data);
    try stdout_buf.appendSlice(data);
}
const term = try child.wait();
const exit_code = process.exitCode(term);
```

Note: `agent.zig` needs the data in an `ArrayList` because it references `stdout_buf.items` later (line 126–130). An alternative is to drain into an owned slice and use it directly, avoiding the intermediate ArrayList.

Simpler alternative for `agent.zig`:
```zig
const stdout_data = if (child.stdout) |pipe| try process.drainPipe(allocator, pipe) else try allocator.alloc(u8, 0);
defer allocator.free(stdout_data);
const term = try child.wait();
const exit_code = process.exitCode(term);
if (exit_code != 0 and stdout_data.len == 0) return error.AgentFailed;
return try parseNdjson(allocator, stdout_data);
```

### Changes to `src/pipeline.zig`

Add `const process = @import("process.zig");` at the top.

**`runTestCommandForRepo`** (lines 757–780): Replace with:
```zig
const stdout_data = if (child.stdout) |pipe| try process.drainPipe(self.allocator, pipe) else try self.allocator.alloc(u8, 0);
const stderr_data = if (child.stderr) |pipe| try process.drainPipe(self.allocator, pipe) else try self.allocator.alloc(u8, 0);
const term = try child.wait();
const exit_code = process.exitCode(term);
return TestResult{ .stdout = stdout_data, .stderr = stderr_data, .exit_code = exit_code };
```

**Self-update build section** (lines 1039–1064): Replace with:
```zig
var stderr_data = if (child.stderr) |pipe| process.drainPipe(self.allocator, pipe) catch return else "";
defer if (stderr_data.len > 0) self.allocator.free(stderr_data);
// Drain stdout (discard)
if (child.stdout) |pipe| {
    const discard = process.drainPipe(self.allocator, pipe) catch &.{};
    if (discard.len > 0) self.allocator.free(discard);
}
const term = child.wait() catch |err| { std.log.err("Self-update: wait failed: {}", .{err}); return; };
const exit_code = process.exitCode(term);
```

Note: The self-update section uses `catch return` / `catch break` patterns instead of `try` because it is a `void`-returning function that logs and returns on error rather than propagating. The refactored version preserves this error-handling style.

## Acceptance Criteria

1. **`zig build` succeeds** with no compilation errors after all changes.
2. **`zig build test` passes** — all existing unit tests in `git.zig`, `docker.zig`, `agent.zig`, and `pipeline.zig` continue to pass.
3. **No 8192-byte buffer loop remains** in `git.zig`, `docker.zig`, `agent.zig`, or `pipeline.zig`. All instances are replaced by calls to `process.drainPipe`.
4. **No `switch (term) { .Exited => |code| code, else => 1 }` remains** in `git.zig`, `docker.zig`, `agent.zig`, or `pipeline.zig`. All instances are replaced by calls to `process.exitCode`.
5. **`process.zig` contains unit tests** for both `drainPipe` and `exitCode`.
6. **Behavioral equivalence**: The refactored code produces identical `ExecResult`, `RunResult`, `TestResult`, and `AgentResult` values for the same child process outputs as the original code.
7. **No new public API changes** to `Git`, `Docker`, `Pipeline`, or `agent_mod` — all changes are internal implementation details.
8. **`process.zig` is imported only by the four modified files** — it does not pull in any dependencies beyond `std`.

## Edge Cases

1. **Pipe is null**: When `child.stdout` or `child.stderr` is `null` (e.g., if behavior was set to `.Close` or `.Inherit`), callers must handle the `null` case before calling `drainPipe`. The `if (child.stdout) |pipe|` pattern is preserved at each call site.
2. **Empty output**: `drainPipe` must return a valid zero-length owned slice (not undefined) when the pipe produces no bytes, so callers can safely `free()` the result.
3. **Read error mid-stream**: The existing pattern uses `catch break` on `pipe.read()`, discarding partial read errors. `drainPipe` must preserve this behavior — a read error terminates the loop and returns whatever was accumulated so far rather than propagating the error.
4. **Allocation failure during drain**: `appendSlice` can fail with `OutOfMemory`. `drainPipe` propagates this via `try` (matching the behavior in `git.zig`, `docker.zig`, `agent.zig`, and `pipeline.zig:runTestCommandForRepo`). The self-update call site in `pipeline.zig` catches this at the call site level.
5. **Signal/stop termination**: `exitCode` returns 1 for any `Term` variant other than `.Exited` (matching the existing `else => 1` in all call sites). This covers `.Signal`, `.Stopped`, and `.Unknown`.
6. **Large output**: The 8192-byte buffer size is preserved. `ArrayList` grows dynamically, so arbitrarily large outputs are handled as before.
