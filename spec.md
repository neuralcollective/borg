# Spec: Fix WhatsApp/Sidecar stdout blocking the main event loop

## Task Summary

The `poll()` methods in `src/whatsapp.zig` (line 77) and `src/sidecar.zig` (line 99) call `stdout.read()` on a blocking pipe fd. When the bridge child process has no data to send, this blocks indefinitely, freezing the entire single-threaded main event loop in `src/main.zig:578` — halting Telegram polling, agent dispatch, cooldown expiry, and web dashboard message draining. The fix is to set `O_NONBLOCK` on the child process stdout pipe fd immediately after spawning, so that `read()` returns `error.WouldBlock` (caught by the existing `catch break`) instead of blocking.

## Files to Modify

1. **`src/sidecar.zig`** — This is the actively-used unified bridge (Discord + WhatsApp). Set `O_NONBLOCK` on `child.stdout` after `child.spawn()` in `start()`. This is the primary fix since `main.zig` uses `Sidecar.poll()` at line 617.
2. **`src/whatsapp.zig`** — The standalone WhatsApp module has the same blocking pattern. Set `O_NONBLOCK` on `child.stdout` after `child.spawn()` in `start()` for consistency and correctness if this module is used independently.

## Files to Create

None.

## Function/Type Signatures

No new functions or types are needed. The changes are within existing function bodies.

### `src/sidecar.zig` — `Sidecar.start()`

After `try child.spawn();` (line 78), add a call to set `O_NONBLOCK` on the stdout fd:

```zig
// Inside start(), after child.spawn():
if (child.stdout) |stdout| {
    const fd = stdout.handle;
    const current_flags = std.posix.fcntl(fd, .GET_FL) catch 0;
    _ = std.posix.fcntl(fd, .SET_FL, .{ .flags = @as(u32, @intCast(current_flags)) | @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true })) }) catch {};
}
```

### `src/whatsapp.zig` — `WhatsApp.start()`

After `try child.spawn();` (line 53), add the same `O_NONBLOCK` pattern:

```zig
// Inside start(), after child.spawn():
if (child.stdout) |stdout| {
    const fd = stdout.handle;
    const current_flags = std.posix.fcntl(fd, .GET_FL) catch 0;
    _ = std.posix.fcntl(fd, .SET_FL, .{ .flags = @as(u32, @intCast(current_flags)) | @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true })) }) catch {};
}
```

### `poll()` methods — No signature changes

The existing `catch break` in both `sidecar.zig:100` and `whatsapp.zig:78` already handles the `error.WouldBlock` that `read()` returns on a non-blocking fd with no data available. No changes to the `poll()` methods are required.

## Acceptance Criteria

1. **Non-blocking read**: After `Sidecar.start()` is called, the stdout pipe fd has `O_NONBLOCK` set (verifiable via `fcntl(fd, F_GETFL)` in a test).
2. **Non-blocking read (whatsapp)**: After `WhatsApp.start()` is called, the stdout pipe fd has `O_NONBLOCK` set.
3. **poll() returns immediately when no data**: `Sidecar.poll()` returns an empty slice within bounded time (< 10ms) when the child process has produced no output, instead of blocking indefinitely.
4. **poll() still reads data**: When the child process writes NDJSON lines to stdout, `poll()` correctly reads and parses them into `SidecarMessage` / `WaMessage` slices (existing parsing logic unchanged).
5. **Main loop continues**: The main loop in `main.zig:578` completes a full cycle (Telegram poll → sidecar poll → web drain → group state checks → sleep) without blocking on sidecar stdout.
6. **Existing tests pass**: `zig build test` passes with no regressions. The existing `Sidecar init/deinit`, `WhatsApp init/deinit`, and `parseSource` tests remain green.
7. **No new threads introduced**: The fix uses `O_NONBLOCK` on the existing pipe fd, not a reader thread, keeping the architecture simple.

## Edge Cases to Handle

1. **`fcntl` failure**: If `fcntl(GET_FL)` or `fcntl(SET_FL)` fails (unlikely on a valid pipe fd), the code should log a warning but not prevent the process from starting. The worst case is the old blocking behavior.
2. **Child process exits before poll**: If the child process exits, `stdout.read()` returns 0 (EOF) regardless of blocking mode. The existing `if (n == 0) break;` handles this correctly in both files.
3. **Partial NDJSON lines**: A non-blocking read may return a partial line (data available but no trailing `\n` yet). The existing `stdout_buf` accumulation logic and `indexOf("\n")` line-splitting already handles this — partial data is buffered until the next `poll()` call completes the line.
4. **Rapid successive polls with no data**: When the main loop polls at 500ms intervals and there's no data, each `poll()` call will immediately get `WouldBlock` and return an empty slice. This is the desired behavior and introduces no CPU overhead beyond the syscall.
5. **Large burst of data**: If the bridge writes many lines between polls, the non-blocking read loop (`while (true) { read ... if (n < buf.len) break; }`) will drain all available data in a single poll call, same as before. The `n < read_buf.len` heuristic correctly detects when the kernel buffer is drained.
6. **stdout is null**: Both `poll()` methods already guard `child.stdout orelse return &[_]...{}` at the top, so a null stdout (impossible with `.Pipe` behavior, but defensive) is handled.
