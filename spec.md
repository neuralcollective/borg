# Spec: Fix subprocess pipe deadlock when stderr buffer fills

## Task Summary

Multiple subprocess call sites read stdout to completion before reading stderr (or vice versa). If the child writes more than the OS pipe buffer (~64KB) to the unread stream, the child blocks on write while the parent blocks on read, causing a deadlock. The fix introduces a shared helper that reads both pipes concurrently using a dedicated thread for one stream, and replaces all sequential-read call sites with calls to this helper.

## Files to Modify

1. **`src/git.zig`** — Replace sequential stdout/stderr reads in `exec()` with concurrent helper call.
2. **`src/gt.zig`** — Replace sequential stdout/stderr reads in `exec()` with concurrent helper call.
3. **`src/docker.zig`** — Replace stdout-only read in `runWithStdio()` with concurrent helper call (also drains stderr, which was previously piped but never read).
4. **`src/agent.zig`** — Replace stdout-only read in `runDirect()` with concurrent helper call.
5. **`src/pipeline.zig`** — Replace sequential reads in `runTestCommandForRepo()` and reverse-order reads in `checkSelfUpdate()` with concurrent helper call.
6. **`src/main.zig`** — Replace stdout-only read in `agentThreadInner()` with concurrent helper call.

## Files to Create

1. **`src/subprocess.zig`** — New module containing the shared concurrent pipe-reading helper.

## Function/Type Signatures

### `src/subprocess.zig` (new file)

```zig
pub const PipeOutput = struct {
    stdout: []u8,
    stderr: []u8,
    allocator: std.mem.Allocator,

    pub fn deinit(self: *PipeOutput) void;
};

/// Read both stdout and stderr concurrently from a spawned child process.
/// Spawns a thread to read stderr while the calling thread reads stdout.
/// Returns owned slices for both streams. Caller frees via PipeOutput.deinit().
/// max_size limits each stream independently (e.g. 10 * 1024 * 1024 for 10MB).
pub fn collectOutput(
    allocator: std.mem.Allocator,
    child: *std.process.Child,
    max_size: usize,
) !PipeOutput;
```

Internal implementation detail: a private `readerThread` function serves as the thread entry point:

```zig
const ReaderContext = struct {
    stream: std.fs.File,
    buf: *std.ArrayList(u8),
    max_size: usize,
};

fn readerThread(ctx: ReaderContext) void;
```

### `src/git.zig` — `Git.exec()` (modify)

Replace lines 23–40 (the two sequential `if (child.stdout)` / `if (child.stderr)` read loops) with:

```zig
const subprocess = @import("subprocess.zig");
// ...
var output = try subprocess.collectOutput(self.allocator, &child, 10 * 1024 * 1024);
// Use output.stdout, output.stderr; transfer ownership into ExecResult
```

The returned `ExecResult` is constructed from `output.stdout` and `output.stderr` directly (no copy needed; ownership transfers).

### `src/gt.zig` — `Gt.exec()` (modify)

Same change as `git.zig:exec()`. Replace the two sequential read loops with `subprocess.collectOutput()`.

### `src/docker.zig` — `Docker.runWithStdio()` (modify)

Replace lines 175–183 (stdout-only read loop) with `subprocess.collectOutput()`. The stderr data can be discarded (freed) after `wait()`, or logged on non-zero exit. `RunResult` is unchanged (stdout-only), but stderr is now drained to prevent deadlock.

### `src/agent.zig` — `runDirect()` (modify)

Replace lines 109–118 (stdout-only read loop) with `subprocess.collectOutput()`. stderr data is freed after use. On non-zero exit, stderr content can be logged before returning `error.AgentFailed`.

### `src/pipeline.zig` — `Pipeline.runTestCommandForRepo()` (modify)

Replace lines 821–838 (two sequential read loops) with `subprocess.collectOutput()`. The returned `TestResult` is constructed from the concurrent output.

### `src/pipeline.zig` — `Pipeline.checkSelfUpdate()` (modify)

Replace lines 1192–1208 (stderr-first-then-stdout read loops) with `subprocess.collectOutput()`. Currently reads stderr first which has the reverse deadlock: if stdout fills while stderr is being drained.

### `src/main.zig` — `agentThreadInner()` (modify)

Replace lines 946–955 (stdout-only read loop) with `subprocess.collectOutput()`. stderr data is freed after parsing stdout.

### `build.zig` (modify, if needed)

Add `src/subprocess.zig` as an available module if the build system requires explicit module declarations for `@import` to work. (In standard Zig projects using file-based imports within `src/`, this may not require changes.)

## Acceptance Criteria

1. **No sequential pipe reads**: No call site reads stdout to completion before starting to read stderr (or vice versa). Every call site that sets both `stdout_behavior = .Pipe` and `stderr_behavior = .Pipe` must use `subprocess.collectOutput()` or equivalent concurrent reading.
2. **Deadlock resolved**: A subprocess writing >64KB to stderr while producing minimal stdout output does not hang. Specifically: a test that spawns a child writing 128KB to stderr and 0 bytes to stdout must complete within a reasonable time.
3. **Deadlock resolved (reverse)**: A subprocess writing >64KB to stdout while producing minimal stderr output does not hang.
4. **Build succeeds**: `zig build` compiles without errors.
5. **Tests pass**: `zig build test` passes, including existing tests in `git.zig`, `gt.zig`, `docker.zig`, `agent.zig`, and `pipeline.zig`.
6. **Behavioral equivalence**: All existing `ExecResult`, `RunResult`, `TestResult`, and `AgentResult` return values contain the same data as before. Callers are unaffected.
7. **Thread cleanup**: The stderr reader thread is always joined before `collectOutput` returns, even if stdout reading encounters an error. No thread leaks.
8. **New unit test**: `src/subprocess.zig` contains at least one test that spawns a child process producing >64KB on stderr and verifies both streams are fully captured without hanging.

## Edge Cases

1. **Child closes stdout before stderr (or vice versa)**: The reader thread for stderr must continue reading even after stdout EOF. The calling thread must join the stderr thread regardless of stdout read outcome.
2. **Child produces zero output on one or both streams**: `collectOutput` must return empty slices (not null) for streams with no output.
3. **Child exits before all output is read**: Pipes remain readable after child exit. `collectOutput` must drain both pipes fully before returning, then caller calls `child.wait()`.
4. **Allocation failure during read**: If `appendSlice` fails (OOM), the reader thread must stop cleanly. The calling thread must still join the reader thread and report the error. Already-buffered data for the other stream should be freed.
5. **Very large output (>10MB)**: The `max_size` parameter prevents unbounded memory growth. If a stream exceeds `max_size`, reading stops for that stream (the pipe is left unread and the child may block—this is the existing implicit behavior with a finite buffer, just made explicit). Callers pass a reasonable limit.
6. **stderr set to `.Pipe` but never read (current bug in docker/agent/main)**: After the fix, all piped streams are read. No piped stream is left unread.
7. **Concurrent access to `child` struct**: `collectOutput` receives a mutable pointer to `child`. The two threads access different fields (`child.stdout` vs `child.stderr`), which are independent `?std.fs.File` values. No mutex is needed since each thread reads only its own stream.
8. **`checkSelfUpdate` reverse order**: Currently reads stderr first, then stdout. Has the mirror deadlock: stdout >64KB blocks child while parent drains stderr. Fixed by the same concurrent approach.
