# Spec: Fix subprocess pipe deadlock when stderr buffer fills

## Task Summary

In five subprocess call sites (`git.zig:exec()`, `docker.zig:runWithStdio()`, `agent.zig:runDirect()`, `pipeline.zig:runTestCommandForRepo()`, `main.zig:agentThreadInner()`), stdout is read to completion before stderr is drained. If a child process writes more than the OS pipe buffer (~64KB on Linux) to stderr before stdout EOF, the child blocks on its stderr write while the parent blocks on its stdout read, causing a deadlock. Fix by draining both streams concurrently using a shared helper that spawns a thread for the secondary stream.

## Files to Modify

1. `src/git.zig` — `exec()` at lines 11-54: replace sequential stdout-then-stderr reads with concurrent drain
2. `src/docker.zig` — `runWithStdio()` at lines 124-196: stderr is piped but never read; drain it concurrently with stdout
3. `src/agent.zig` — `runDirect()` at lines 56-131: stderr is piped but never read; drain it concurrently with stdout
4. `src/pipeline.zig` — `runTestCommandForRepo()` at lines 736-779: replace sequential stdout-then-stderr reads with concurrent drain
5. `src/main.zig` — `agentThreadInner()` at lines 878-947: stderr is piped but never read; drain it concurrently with stdout

## Files to Create

None. The helper function should be added to an existing file. Since `git.zig` already defines the most general-purpose `ExecResult` and all five call sites share the same pattern, add the concurrent-drain helper as a standalone public function in a new section of `src/git.zig` (or alternatively as a new `src/subprocess.zig` if the implementer prefers — either is acceptable as long as all five sites use it).

## Function/Type Signatures

### New helper function (in `src/git.zig` or `src/subprocess.zig`)

```zig
/// Reads both stdout and stderr from a spawned child process concurrently.
/// Spawns a thread to drain stderr while the calling thread drains stdout.
/// Returns owned slices for both streams. Caller must free with `allocator`.
pub fn collectPipeOutput(
    allocator: std.mem.Allocator,
    stdout_pipe: std.fs.File,
    stderr_pipe: std.fs.File,
    max_size: usize,
) struct { stdout: []u8, stderr: []u8 }
```

The function should:
- Spawn a `std.Thread` that reads `stderr_pipe` into a buffer up to `max_size`
- On the calling thread, read `stdout_pipe` into a buffer up to `max_size`
- Join the stderr thread
- Return both buffers as owned slices

### Changes to existing functions

**`git.zig:Git.exec()`** — Replace lines 23-40 (the sequential read loops) with a call to `collectPipeOutput(self.allocator, child.stdout.?, child.stderr.?, max_size)`. The returned stdout/stderr slices feed directly into `ExecResult`.

**`docker.zig:Docker.runWithStdio()`** — Replace lines 175-183 (stdout-only read loop) with a call to `collectPipeOutput(self.allocator, child.stdout.?, child.stderr.?, max_size)`. The stderr output can be logged on non-zero exit or discarded. Update `RunResult` to optionally include stderr:

```zig
pub const RunResult = struct {
    stdout: []const u8,
    stderr: []const u8,  // new field
    exit_code: u8,
    allocator: std.mem.Allocator,

    pub fn deinit(self: *RunResult) void {
        self.allocator.free(self.stdout);
        self.allocator.free(self.stderr);
    }
};
```

**`agent.zig:runDirect()`** — Replace lines 109-118 (stdout-only read loop) with a call to `collectPipeOutput`. Log stderr at `err` level when exit_code != 0, to aid debugging of `AgentFailed` errors.

**`pipeline.zig:Pipeline.runTestCommandForRepo()`** — Replace lines 749-766 (sequential read loops) with a call to `collectPipeOutput`. `TestResult` already has both `stdout` and `stderr` fields, so wire them through directly.

**`main.zig:agentThreadInner()`** — Replace lines 919-928 (stdout-only read loop) with a call to `collectPipeOutput`. Log stderr at `err` level on failure. The `AgentOutcome` struct does not need to change since stderr is for diagnostics only.

### Thread function signature (internal to helper)

```zig
fn stderrReaderThread(stderr_pipe: std.fs.File, buf: *std.ArrayList(u8), max_size: usize) void
```

## Acceptance Criteria

1. **No deadlock on large stderr**: A subprocess that writes >64KB to stderr before writing to stdout must complete without hanging. Test by running a command like `/bin/sh -c "dd if=/dev/urandom bs=1024 count=128 status=none >&2; echo done"` through `git.zig:exec()` and verifying it returns within a reasonable time.

2. **No deadlock on large stdout**: A subprocess that writes >64KB to stdout must still complete. Verify no regression from the existing behavior.

3. **Both streams captured**: `git.zig:exec()` and `pipeline.zig:runTestCommandForRepo()` must return both stdout and stderr content accurately in their result structs.

4. **Stderr available for diagnostics**: `agent.zig:runDirect()` and `main.zig:agentThreadInner()` must log stderr content (at minimum at `err` level) when the subprocess exits with a non-zero exit code.

5. **docker.zig stderr captured**: `docker.zig:runWithStdio()` `RunResult` includes the new `stderr` field. Callers of `runWithStdio()` updated to handle the new field (check all call sites in `pipeline.zig`).

6. **Thread cleanup**: The stderr reader thread is always joined before `child.wait()` is called, ensuring no thread leak on any code path (including error paths via `errdefer`).

7. **Existing tests pass**: `zig build test` passes with no regressions. The existing tests in `git.zig`, `docker.zig`, `agent.zig`, and `pipeline.zig` continue to pass.

8. **Interleaved output correctness**: When a process writes interleaved stdout and stderr, both streams are fully captured (no truncation or data loss up to the max buffer size).

## Edge Cases

1. **Child closes stderr before stdout (or vice versa)**: The thread reading the early-closed stream must exit cleanly while the other thread continues reading. The join must not hang.

2. **Child produces no stderr**: The stderr thread should return an empty slice without error. This is the common case for git commands.

3. **Child produces no stdout**: The stdout read should return an empty slice. The stderr thread should still drain stderr fully.

4. **Child exits before all output is read**: After the child exits, the pipes remain readable until drained. The readers must continue reading until EOF, not stop at child exit.

5. **Thread spawn failure**: If `std.Thread.spawn` fails (resource exhaustion), fall back to sequential reads (accepting the deadlock risk) or propagate the error. At minimum, do not crash.

6. **Allocator failure during stderr read**: If the stderr thread hits `OutOfMemory` while appending to its buffer, it should stop reading and return what it has. The main thread must still join it and proceed.

7. **errdefer on main thread**: If the main thread's stdout read hits an error after the stderr thread is spawned, the stderr thread must be joined (via `errdefer`) before the error propagates, to avoid a detached thread holding a dangling pipe fd.

8. **Max buffer enforcement**: If either stream exceeds `max_size`, stop reading that stream (close the pipe or discard further reads). The child may receive SIGPIPE on the truncated stream; this is acceptable.

9. **Signal interruption (EINTR)**: The read loops should handle `EINTR` by retrying. Zig's `std.fs.File.read()` handles this internally, but verify.

10. **`runWithStdio` callers**: All callers of `Docker.runWithStdio()` in `pipeline.zig` must be updated to account for the new `stderr` field in `RunResult.deinit()`, otherwise memory leaks.
