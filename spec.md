# Task #4: Fix WhatsApp stdout blocking the main event loop

## 1. Task Summary

`sidecar.zig` (and the legacy `whatsapp.zig`) call `stdout.read()` in a `while (true)` loop
inside their `poll()` methods. Because the underlying pipe fd is blocking, `read()` hangs
indefinitely when the bridge process has no output, freezing the entire main event loop at
`main.zig:624` and preventing Telegram polling, agent dispatch, and cooldown expiry from
running. Setting `O_NONBLOCK` on the stdout pipe fd immediately after `child.spawn()` makes
`read()` return `WouldBlock` (EAGAIN) when no data is available; the existing `catch break`
in both poll loops already handles that error correctly, so the minimal change is confined to
the two `start()` functions.

## 2. Files to Modify

| File | Location | What changes |
|------|----------|--------------|
| `src/sidecar.zig` | `Sidecar.start()` lines 78–80 | After `try child.spawn()`, set `O_NONBLOCK` on `child.stdout.?.handle` |
| `src/whatsapp.zig` | `WhatsApp.start()` lines 53–55 | After `try child.spawn()`, set `O_NONBLOCK` on `child.stdout.?.handle` |

No new files are required.

## 3. Function/Type Signatures

Neither `start()` signature changes. The internal addition inside each `start()` is:

```zig
// In Sidecar.start() and WhatsApp.start(), immediately after `try child.spawn();`:
if (child.stdout) |f| {
    const flags = try std.posix.fcntl(f.handle, std.posix.F.GETFL, 0);
    _ = try std.posix.fcntl(f.handle, std.posix.F.SETFL, flags | @as(u32, std.posix.O.NONBLOCK));
}
```

Existing signatures that must remain unchanged:

```zig
// sidecar.zig
pub fn start(self: *Sidecar, discord_token: []const u8, wa_auth_dir: []const u8, wa_disabled: bool) !void;
pub fn poll(self: *Sidecar, allocator: std.mem.Allocator) ![]SidecarMessage;

// whatsapp.zig
pub fn start(self: *WhatsApp) !void;
pub fn poll(self: *WhatsApp, allocator: std.mem.Allocator) ![]WaMessage;
```

`Sidecar.start()` and `WhatsApp.start()` already return `!void`, so propagating the
`fcntl` error union requires no signature change.

## 4. Acceptance Criteria

**AC1 – poll() returns promptly when the bridge has no output.**
A call to `Sidecar.poll()` (or `WhatsApp.poll()`) on a running bridge that has produced no
NDJSON lines returns within 5 ms with an empty slice.

**AC2 – Main loop is not blocked.**
After the fix, the `POLL_INTERVAL_MS = 500` sleep at `main.zig:726` drives each iteration;
Telegram `getUpdates` is called at least once per second even when the sidecar bridge emits
no data.

**AC3 – Messages already buffered in the pipe are not lost.**
If the bridge writes N complete NDJSON lines before `poll()` is called, `poll()` returns
all N parsed messages in the same call.

**AC4 – Large bursts are read in full.**
If the bridge writes more than 4 096 bytes of NDJSON before `poll()` is called, `poll()`
iterates the read loop until all available data is consumed (loop continues while
`n == read_buf.len`; stops on `WouldBlock` or `n == 0`).

**AC5 – `O_NONBLOCK` is set on both files.**
`src/sidecar.zig` and `src/whatsapp.zig` each contain a call to `std.posix.fcntl` with
`std.posix.F.SETFL` after `child.spawn()`.

**AC6 – `WouldBlock` is handled silently.**
Neither `Sidecar.poll()` nor `WhatsApp.poll()` propagates `error.WouldBlock` to callers;
the existing `catch break` already absorbs it. No new error paths are introduced in `poll()`.

**AC7 – poll() error set is unchanged.**
`Sidecar.poll()` and `WhatsApp.poll()` return the same error union as before the fix;
callers in `main.zig` (`catch &[_]...{}`) require no changes.

**AC8 – start() failure when fcntl fails is surfaced.**
If `std.posix.fcntl` returns an error (e.g., invalid fd), `start()` propagates it to the
caller rather than silently continuing with a blocking fd.

**AC9 – Existing unit tests continue to pass.**
`just t` passes with no new test failures. The `WhatsApp init/deinit` and
`Sidecar init/deinit` tests pass unchanged.

**AC10 – No change to stdin or stderr fds.**
Only `child.stdout` receives `O_NONBLOCK`; `child.stdin` and `child.stderr` are left in
their default blocking mode.

## 5. Edge Cases

| # | Scenario | Expected behaviour |
|---|----------|--------------------|
| E1 | `child.stdout` is `null` (stdout not piped) | The `if (child.stdout) \|f\|` guard skips `fcntl`; `poll()` returns an empty slice as before |
| E2 | `fcntl(GETFL)` fails | `start()` returns the error to its caller; the process is not used |
| E3 | `fcntl(SETFL)` fails | Same as E2 |
| E4 | Bridge writes data between two consecutive `poll()` calls | Data accumulates in the OS pipe buffer; next `poll()` reads it in full |
| E5 | Bridge writes a partial NDJSON line (no trailing `\n`) | `poll()` buffers the incomplete line in `stdout_buf` and returns zero messages; the line is completed on a future poll |
| E6 | Bridge writes more than 4 096 bytes in one burst (fills `read_buf`) | Loop re-enters because `n == read_buf.len`; continues until `WouldBlock` or `n == 0`; all data is captured |
| E7 | Bridge process exits unexpectedly | `read()` returns `n == 0` (EOF); loop breaks; already-buffered complete lines are parsed and returned |
| E8 | Bridge produces data faster than `poll()` is called (sustained high throughput) | OS pipe buffer absorbs the difference (up to ~64 KB); `poll()` drains all available bytes each call; no data is lost as long as the buffer does not overflow |
| E9 | `O_NONBLOCK` is set but the bridge immediately sends a connected/QR event | The flag does not suppress data that is already in the pipe; first `poll()` reads and parses it normally |
| E10 | `start()` is called a second time after a previous crash (child restarted) | A new `child` is created; `O_NONBLOCK` must be set on the new `child.stdout` fd; the old fd is closed in `deinit()` |
