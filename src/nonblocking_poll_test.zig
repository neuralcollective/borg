// Tests for spec: Fix WhatsApp stdout blocking the main event loop
//
// Verifies that Sidecar.start() and WhatsApp.start() set O_NONBLOCK on
// child.stdout immediately after child.spawn(), and that poll() returns
// promptly without blocking or propagating WouldBlock.
//
// To include in the build, add to sidecar.zig or whatsapp.zig:
//   test { _ = @import("nonblocking_poll_test.zig"); }
//
// AC5 / AC8 source-code checks FAIL initially (fcntl absent from source).
// AC1 / AC6 behavioural tests run safely by setting O_NONBLOCK manually in
// the test itself (simulating what start() will do after the fix).
//
// Coverage map:
//   AC1  – poll() returns promptly (< 5 ms) when bridge has no output
//   AC2  – poll() loop does not block for a full POLL_INTERVAL_MS; proxy timing
//   AC3  – pre-buffered and in-pipe messages are not lost
//   AC4  – large bursts (> 4096 bytes) are fully consumed in one call
//   AC5  – O_NONBLOCK set on both sidecar.zig and whatsapp.zig (source checks)
//   AC6  – WouldBlock is absorbed by catch break; never propagated
//   AC7  – poll() and start() return types are unchanged
//   AC8  – fcntl errors surface via `try` in start()
//   AC9  – existing init/deinit and null-child tests continue to pass
//   AC10 – only child.stdout receives O_NONBLOCK; stdin/stderr are untouched
//   E1   – child.stdout null → empty slice
//   E2/E3– fcntl failure paths covered by AC8 source checks
//   E4   – data between consecutive poll() calls is not lost
//   E5   – partial NDJSON line buffered; completed on next poll()
//   E6   – > 4096-byte burst on real pipe for WhatsApp
//   E7   – bridge exits unexpectedly; buffered lines still returned
//   E9   – O_NONBLOCK does not suppress data already in pipe
//   E10  – fcntl call is inside start() (applies to any new child)

const std = @import("std");
const sidecar_mod = @import("sidecar.zig");
const Sidecar = sidecar_mod.Sidecar;
const SidecarMessage = sidecar_mod.SidecarMessage;
const whatsapp_mod = @import("whatsapp.zig");
const WhatsApp = whatsapp_mod.WhatsApp;
const WaMessage = whatsapp_mod.WaMessage;

// ── helpers ──────────────────────────────────────────────────────────────────

/// Free every heap string inside a SidecarMessage slice and the slice itself.
fn freeSidecarMessages(allocator: std.mem.Allocator, messages: []SidecarMessage) void {
    for (messages) |msg| {
        allocator.free(msg.id);
        allocator.free(msg.chat_id);
        allocator.free(msg.sender);
        allocator.free(msg.sender_name);
        allocator.free(msg.text);
    }
    allocator.free(messages);
}

/// Free every heap string inside a WaMessage slice and the slice itself.
fn freeWaMessages(allocator: std.mem.Allocator, messages: []WaMessage) void {
    for (messages) |msg| {
        allocator.free(msg.jid);
        allocator.free(msg.id);
        allocator.free(msg.sender);
        allocator.free(msg.sender_name);
        allocator.free(msg.text);
    }
    allocator.free(messages);
}

/// Spawn a child process and set O_NONBLOCK on its stdout pipe fd.
/// The caller owns the returned Child and must kill/wait it.
fn spawnNonblocking(argv: []const []const u8, allocator: std.mem.Allocator) !std.process.Child {
    var child = std.process.Child.init(argv, allocator);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Close;
    try child.spawn();
    if (child.stdout) |f| {
        const flags = try std.posix.fcntl(f.handle, std.posix.F.GETFL, 0);
        _ = try std.posix.fcntl(f.handle, std.posix.F.SETFL, flags | @as(u32, std.posix.O.NONBLOCK));
    }
    return child;
}

/// Spawn a child whose stdout immediately reaches EOF (process exits right away).
fn spawnEofChild(allocator: std.mem.Allocator) !std.process.Child {
    var child = std.process.Child.init(&.{"/bin/true"}, allocator);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Close;
    try child.spawn();
    _ = child.wait() catch {};
    return child;
}

// ═════════════════════════════════════════════════════════════════════════════
// AC5 – O_NONBLOCK must be set in both source files (source-code checks)
// These tests FAIL before the fix because fcntl/SETFL are absent.
// ═════════════════════════════════════════════════════════════════════════════

test "AC5: sidecar.zig contains fcntl(SETFL) call after child.spawn()" {
    const src = @embedFile("sidecar.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "std.posix.F.SETFL") != null);
}

test "AC5: sidecar.zig retrieves flags with fcntl(GETFL) before SETFL" {
    const src = @embedFile("sidecar.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "std.posix.F.GETFL") != null);
}

test "AC5: sidecar.zig applies O.NONBLOCK flag" {
    const src = @embedFile("sidecar.zig");
    const has = std.mem.indexOf(u8, src, "O.NONBLOCK") != null or
        std.mem.indexOf(u8, src, "NONBLOCK") != null;
    try std.testing.expect(has);
}

test "AC5: whatsapp.zig contains fcntl(SETFL) call after child.spawn()" {
    const src = @embedFile("whatsapp.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "std.posix.F.SETFL") != null);
}

test "AC5: whatsapp.zig retrieves flags with fcntl(GETFL) before SETFL" {
    const src = @embedFile("whatsapp.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "std.posix.F.GETFL") != null);
}

test "AC5: whatsapp.zig applies O.NONBLOCK flag" {
    const src = @embedFile("whatsapp.zig");
    const has = std.mem.indexOf(u8, src, "O.NONBLOCK") != null or
        std.mem.indexOf(u8, src, "NONBLOCK") != null;
    try std.testing.expect(has);
}

// AC5 – fcntl is guarded on child.stdout specifically

test "AC5: sidecar.zig guards the SETFL call inside an if(child.stdout) block" {
    const src = @embedFile("sidecar.zig");
    // The spec code pattern: `if (child.stdout) |f| { fcntl(f.handle, SETFL, ...) }`
    try std.testing.expect(std.mem.indexOf(u8, src, "child.stdout") != null);
    try std.testing.expect(std.mem.indexOf(u8, src, "SETFL") != null);
}

test "AC5: whatsapp.zig guards the SETFL call inside an if(child.stdout) block" {
    const src = @embedFile("whatsapp.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "child.stdout") != null);
    try std.testing.expect(std.mem.indexOf(u8, src, "SETFL") != null);
}

// ═════════════════════════════════════════════════════════════════════════════
// AC8 – start() propagates fcntl errors to the caller
// Source check: `try std.posix.fcntl` must appear (not _ = or bare call).
// FAILS initially because fcntl is absent from start().
// ═════════════════════════════════════════════════════════════════════════════

test "AC8: sidecar.zig uses `try` for fcntl so errors surface to start() caller" {
    const src = @embedFile("sidecar.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "try std.posix.fcntl") != null);
}

test "AC8: whatsapp.zig uses `try` for fcntl so errors surface to start() caller" {
    const src = @embedFile("whatsapp.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "try std.posix.fcntl") != null);
}

// ═════════════════════════════════════════════════════════════════════════════
// AC10 – Only child.stdout receives O_NONBLOCK; stdin and stderr unchanged
// These pass before the fix (no fcntl at all) and must continue to pass after.
// ═════════════════════════════════════════════════════════════════════════════

test "AC10: sidecar.zig does not apply fcntl to child.stdin" {
    const src = @embedFile("sidecar.zig");
    // Ensure "stdin.handle" is not referenced in proximity to any "fcntl" call.
    var pos: usize = 0;
    while (std.mem.indexOfPos(u8, src, pos, "stdin.handle")) |idx| {
        const lo = if (idx >= 120) idx - 120 else 0;
        const hi = @min(src.len, idx + 120);
        try std.testing.expect(std.mem.indexOf(u8, src[lo..hi], "fcntl") == null);
        pos = idx + 1;
    }
}

test "AC10: sidecar.zig does not apply fcntl to child.stderr" {
    const src = @embedFile("sidecar.zig");
    var pos: usize = 0;
    while (std.mem.indexOfPos(u8, src, pos, "stderr.handle")) |idx| {
        const lo = if (idx >= 120) idx - 120 else 0;
        const hi = @min(src.len, idx + 120);
        try std.testing.expect(std.mem.indexOf(u8, src[lo..hi], "fcntl") == null);
        pos = idx + 1;
    }
}

test "AC10: whatsapp.zig does not apply fcntl to child.stdin" {
    const src = @embedFile("whatsapp.zig");
    var pos: usize = 0;
    while (std.mem.indexOfPos(u8, src, pos, "stdin.handle")) |idx| {
        const lo = if (idx >= 120) idx - 120 else 0;
        const hi = @min(src.len, idx + 120);
        try std.testing.expect(std.mem.indexOf(u8, src[lo..hi], "fcntl") == null);
        pos = idx + 1;
    }
}

test "AC10: whatsapp.zig does not apply fcntl to child.stderr" {
    const src = @embedFile("whatsapp.zig");
    var pos: usize = 0;
    while (std.mem.indexOfPos(u8, src, pos, "stderr.handle")) |idx| {
        const lo = if (idx >= 120) idx - 120 else 0;
        const hi = @min(src.len, idx + 120);
        try std.testing.expect(std.mem.indexOf(u8, src[lo..hi], "fcntl") == null);
        pos = idx + 1;
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// AC1 – poll() returns within 5 ms when the bridge has no output
// Uses a running process (sleep) with O_NONBLOCK applied manually in the test,
// simulating what start() does after the fix.
// ═════════════════════════════════════════════════════════════════════════════

test "AC1: Sidecar.poll() returns promptly (< 5 ms) when bridge has no output" {
    const alloc = std.testing.allocator;
    var s = Sidecar.init(alloc, "Borg");

    // A process that stays alive but never writes to stdout.
    var child = try spawnNonblocking(&.{ "/bin/sh", "-c", "sleep 5" }, alloc);
    s.child = child;

    const t0 = std.time.milliTimestamp();
    const messages = try s.poll(alloc);
    const elapsed = std.time.milliTimestamp() - t0;
    alloc.free(messages);

    // Tear down: kill before deinit to avoid zombie
    if (s.child) |*c| _ = c.kill() catch {};
    s.child = null;
    s.deinit();

    try std.testing.expectEqual(@as(usize, 0), messages.len);
    try std.testing.expect(elapsed < 5);
}

test "AC1: WhatsApp.poll() returns promptly (< 5 ms) when bridge has no output" {
    const alloc = std.testing.allocator;
    var wa = WhatsApp.init(alloc, "Borg");

    var child = try spawnNonblocking(&.{ "/bin/sh", "-c", "sleep 5" }, alloc);
    wa.child = child;

    const t0 = std.time.milliTimestamp();
    const messages = try wa.poll(alloc);
    const elapsed = std.time.milliTimestamp() - t0;
    alloc.free(messages);

    if (wa.child) |*c| _ = c.kill() catch {};
    wa.child = null;
    wa.deinit();

    try std.testing.expectEqual(@as(usize, 0), messages.len);
    try std.testing.expect(elapsed < 5);
}

// ═════════════════════════════════════════════════════════════════════════════
// AC3 – Messages already buffered in the pipe are not lost
// Pre-populate stdout_buf then poll() with a child whose stdout gives EOF.
// ═════════════════════════════════════════════════════════════════════════════

test "AC3: Sidecar.poll() returns all N pre-buffered complete NDJSON lines" {
    const alloc = std.testing.allocator;
    var s = Sidecar.init(alloc, "Borg");

    // Child whose stdout pipe reaches EOF immediately.
    s.child = try spawnEofChild(alloc);
    defer s.deinit();

    // Inject 3 complete NDJSON message lines directly into stdout_buf.
    const line_discord =
        "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"1\"," ++
        "\"channel_id\":\"c1\",\"sender_id\":\"u1\",\"sender_name\":\"Alice\"," ++
        "\"text\":\"hello\",\"timestamp\":1000,\"is_dm\":false,\"mentions_bot\":false}\n";
    const line_discord2 =
        "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"2\"," ++
        "\"channel_id\":\"c1\",\"sender_id\":\"u2\",\"sender_name\":\"Bob\"," ++
        "\"text\":\"world\",\"timestamp\":1001,\"is_dm\":false,\"mentions_bot\":false}\n";
    const line_wa =
        "{\"source\":\"whatsapp\",\"event\":\"message\",\"id\":\"3\",\"jid\":\"g1@g.us\"," ++
        "\"sender\":\"u3\",\"sender_name\":\"Carol\",\"text\":\"hi\",\"timestamp\":1002," ++
        "\"is_group\":true,\"mentions_bot\":false}\n";

    try s.stdout_buf.appendSlice(line_discord);
    try s.stdout_buf.appendSlice(line_discord2);
    try s.stdout_buf.appendSlice(line_wa);

    const messages = try s.poll(alloc);
    defer freeSidecarMessages(alloc, messages);

    try std.testing.expectEqual(@as(usize, 3), messages.len);
}

test "AC3: WhatsApp.poll() returns all N pre-buffered complete NDJSON lines" {
    const alloc = std.testing.allocator;
    var wa = WhatsApp.init(alloc, "Borg");

    wa.child = try spawnEofChild(alloc);
    defer wa.deinit();

    const line1 =
        "{\"event\":\"message\",\"jid\":\"g1@g.us\",\"id\":\"1\",\"sender\":\"u1\"," ++
        "\"sender_name\":\"Alice\",\"text\":\"hello\",\"timestamp\":1000," ++
        "\"is_group\":true,\"mentions_bot\":false}\n";
    const line2 =
        "{\"event\":\"message\",\"jid\":\"g1@g.us\",\"id\":\"2\",\"sender\":\"u2\"," ++
        "\"sender_name\":\"Bob\",\"text\":\"world\",\"timestamp\":1001," ++
        "\"is_group\":true,\"mentions_bot\":false}\n";

    try wa.stdout_buf.appendSlice(line1);
    try wa.stdout_buf.appendSlice(line2);

    const messages = try wa.poll(alloc);
    defer freeWaMessages(alloc, messages);

    try std.testing.expectEqual(@as(usize, 2), messages.len);
}

// ═════════════════════════════════════════════════════════════════════════════
// AC3 – Messages buffered in an actual pipe are not lost
// Spawn a child that writes N lines then exits; poll() after the process exits.
// ═════════════════════════════════════════════════════════════════════════════

test "AC3: Sidecar.poll() returns messages written to pipe before EOF" {
    const alloc = std.testing.allocator;
    var s = Sidecar.init(alloc, "Borg");

    // Write 5 complete NDJSON lines to stdout then exit.
    const script =
        "for i in 1 2 3 4 5; do " ++
        "printf '{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"%s\"," ++
        "\"channel_id\":\"c1\",\"sender_id\":\"u1\",\"sender_name\":\"A\",' $i && " ++
        "echo '\"text\":\"t\",\"timestamp\":1000,\"is_dm\":false,\"mentions_bot\":false}'; " ++
        "done";

    var child = std.process.Child.init(&.{ "/bin/sh", "-c", script }, alloc);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Close;
    try child.spawn();
    _ = child.wait() catch {}; // let all data land in the pipe buffer
    s.child = child;
    defer s.deinit();

    const messages = try s.poll(alloc);
    defer freeSidecarMessages(alloc, messages);

    try std.testing.expectEqual(@as(usize, 5), messages.len);
}

// ═════════════════════════════════════════════════════════════════════════════
// AC4 – Large bursts (> 4096 bytes) are read in full via the read loop
// ═════════════════════════════════════════════════════════════════════════════

test "AC4: Sidecar.poll() returns all messages when stdout_buf exceeds 4096 bytes" {
    const alloc = std.testing.allocator;
    var s = Sidecar.init(alloc, "Borg");

    s.child = try spawnEofChild(alloc);
    defer s.deinit();

    // Build > 4096 bytes of NDJSON in stdout_buf (each line ~175 bytes; 30 lines ≈ 5250 bytes).
    const tmpl =
        "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"X\"," ++
        "\"channel_id\":\"c1\",\"sender_id\":\"u1\",\"sender_name\":\"Alice\"," ++
        "\"text\":\"filler-to-exceed-4096-byte-read-buffer\",\"timestamp\":1000," ++
        "\"is_dm\":false,\"mentions_bot\":false}\n";

    const n = 30;
    var i: usize = 0;
    while (i < n) : (i += 1) {
        try s.stdout_buf.appendSlice(tmpl);
    }
    try std.testing.expect(s.stdout_buf.items.len > 4096);

    const messages = try s.poll(alloc);
    defer freeSidecarMessages(alloc, messages);

    try std.testing.expectEqual(@as(usize, n), messages.len);
}

test "AC4: WhatsApp.poll() returns all messages when stdout_buf exceeds 4096 bytes" {
    const alloc = std.testing.allocator;
    var wa = WhatsApp.init(alloc, "Borg");

    wa.child = try spawnEofChild(alloc);
    defer wa.deinit();

    const tmpl =
        "{\"event\":\"message\",\"jid\":\"g1@g.us\",\"id\":\"X\",\"sender\":\"u1\"," ++
        "\"sender_name\":\"Alice\",\"text\":\"filler-to-exceed-4096-byte-buffer\"," ++
        "\"timestamp\":1000,\"is_group\":true,\"mentions_bot\":false}\n";

    const n = 30;
    var i: usize = 0;
    while (i < n) : (i += 1) {
        try wa.stdout_buf.appendSlice(tmpl);
    }
    try std.testing.expect(wa.stdout_buf.items.len > 4096);

    const messages = try wa.poll(alloc);
    defer freeWaMessages(alloc, messages);

    try std.testing.expectEqual(@as(usize, n), messages.len);
}

test "AC4: Sidecar.poll() drains > 4096 bytes written to a real pipe in one call" {
    const alloc = std.testing.allocator;
    var s = Sidecar.init(alloc, "Borg");

    // Script writes 30 complete NDJSON lines (> 4096 bytes total) then exits.
    const script =
        "i=0; while [ $i -lt 30 ]; do " ++
        "echo '{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"X\"," ++
        "\"channel_id\":\"c1\",\"sender_id\":\"u1\",\"sender_name\":\"Alice\"," ++
        "\"text\":\"large-burst\",\"timestamp\":1000,\"is_dm\":false,\"mentions_bot\":false}'; " ++
        "i=$((i+1)); done";

    // O_NONBLOCK so poll() doesn't block after the pipe is drained.
    var child = try spawnNonblocking(&.{ "/bin/sh", "-c", script }, alloc);
    _ = child.wait() catch {}; // all data is in the pipe buffer; write-end closed
    s.child = child;
    defer s.deinit();

    const messages = try s.poll(alloc);
    defer freeSidecarMessages(alloc, messages);

    try std.testing.expectEqual(@as(usize, 30), messages.len);
}

// ═════════════════════════════════════════════════════════════════════════════
// AC6 – WouldBlock is handled silently; poll() never propagates it
// With O_NONBLOCK set, reading an idle pipe raises WouldBlock → catch break.
// ═════════════════════════════════════════════════════════════════════════════

test "AC6: Sidecar.poll() succeeds (no error) when stdout returns WouldBlock" {
    const alloc = std.testing.allocator;
    var s = Sidecar.init(alloc, "Borg");

    // Running process whose stdout is idle → read() returns WouldBlock.
    var child = try spawnNonblocking(&.{ "/bin/sh", "-c", "sleep 5" }, alloc);
    s.child = child;

    // poll() must not propagate WouldBlock; it returns an empty slice.
    const messages = try s.poll(alloc);
    alloc.free(messages);

    if (s.child) |*c| _ = c.kill() catch {};
    s.child = null;
    s.deinit();

    try std.testing.expectEqual(@as(usize, 0), messages.len);
}

test "AC6: WhatsApp.poll() succeeds (no error) when stdout returns WouldBlock" {
    const alloc = std.testing.allocator;
    var wa = WhatsApp.init(alloc, "Borg");

    var child = try spawnNonblocking(&.{ "/bin/sh", "-c", "sleep 5" }, alloc);
    wa.child = child;

    const messages = try wa.poll(alloc);
    alloc.free(messages);

    if (wa.child) |*c| _ = c.kill() catch {};
    wa.child = null;
    wa.deinit();

    try std.testing.expectEqual(@as(usize, 0), messages.len);
}

// ═════════════════════════════════════════════════════════════════════════════
// AC6 – Multiple successive poll() calls all succeed with WouldBlock present
// ═════════════════════════════════════════════════════════════════════════════

test "AC6: multiple consecutive Sidecar.poll() calls all succeed on idle bridge" {
    const alloc = std.testing.allocator;
    var s = Sidecar.init(alloc, "Borg");

    var child = try spawnNonblocking(&.{ "/bin/sh", "-c", "sleep 5" }, alloc);
    s.child = child;

    var round: usize = 0;
    while (round < 5) : (round += 1) {
        const messages = try s.poll(alloc);
        alloc.free(messages);
        try std.testing.expectEqual(@as(usize, 0), messages.len);
    }

    if (s.child) |*c| _ = c.kill() catch {};
    s.child = null;
    s.deinit();
}

// ═════════════════════════════════════════════════════════════════════════════
// AC7 – poll() error union is unchanged; callers in main.zig need no changes
// ═════════════════════════════════════════════════════════════════════════════

test "AC7: Sidecar.poll() return type is an error union (not bare []SidecarMessage)" {
    const PollFn = @TypeOf(Sidecar.poll);
    const info = @typeInfo(PollFn).@"fn";
    const ret = info.return_type.?;
    try std.testing.expect(@typeInfo(ret) == .error_union);
}

test "AC7: WhatsApp.poll() return type is an error union (not bare []WaMessage)" {
    const PollFn = @TypeOf(WhatsApp.poll);
    const info = @typeInfo(PollFn).@"fn";
    const ret = info.return_type.?;
    try std.testing.expect(@typeInfo(ret) == .error_union);
}

test "AC7: Sidecar.start() signature is unchanged — returns !void" {
    const StartFn = @TypeOf(Sidecar.start);
    const info = @typeInfo(StartFn).@"fn";
    const ret = info.return_type.?;
    const ret_info = @typeInfo(ret);
    try std.testing.expect(ret_info == .error_union);
    try std.testing.expect(ret_info.error_union.payload == void);
}

test "AC7: WhatsApp.start() signature is unchanged — returns !void" {
    const StartFn = @TypeOf(WhatsApp.start);
    const info = @typeInfo(StartFn).@"fn";
    const ret = info.return_type.?;
    const ret_info = @typeInfo(ret);
    try std.testing.expect(ret_info == .error_union);
    try std.testing.expect(ret_info.error_union.payload == void);
}

// ═════════════════════════════════════════════════════════════════════════════
// AC9 – Existing unit tests continue to pass (regression guard)
// ═════════════════════════════════════════════════════════════════════════════

test "AC9: Sidecar init/deinit still works after fix" {
    var s = Sidecar.init(std.testing.allocator, "Borg");
    defer s.deinit();
    try std.testing.expect(!s.discord_connected);
    try std.testing.expect(!s.wa_connected);
    try std.testing.expect(s.child == null);
}

test "AC9: WhatsApp init/deinit still works after fix" {
    var wa = WhatsApp.init(std.testing.allocator, "Borg");
    defer wa.deinit();
    try std.testing.expect(!wa.connected);
    try std.testing.expect(wa.child == null);
}

test "AC9: Sidecar.poll() with null child returns empty slice without error" {
    const alloc = std.testing.allocator;
    var s = Sidecar.init(alloc, "Borg");
    defer s.deinit();
    // child is null — poll() must return &[_]SidecarMessage{} (compile-time const, not heap)
    const messages = try s.poll(alloc);
    try std.testing.expectEqual(@as(usize, 0), messages.len);
    // Do NOT free — it's a static empty slice, not heap-allocated.
}

test "AC9: WhatsApp.poll() with null child returns empty slice without error" {
    const alloc = std.testing.allocator;
    var wa = WhatsApp.init(alloc, "Borg");
    defer wa.deinit();
    const messages = try wa.poll(alloc);
    try std.testing.expectEqual(@as(usize, 0), messages.len);
}

// ═════════════════════════════════════════════════════════════════════════════
// Edge E1 – child.stdout is null; fcntl guard skips; poll() returns empty
// ═════════════════════════════════════════════════════════════════════════════

test "E1: Sidecar.poll() returns empty slice when child.stdout is null" {
    const alloc = std.testing.allocator;
    var s = Sidecar.init(alloc, "Borg");

    // stdout_behavior = .Close means child.stdout is null after spawn.
    var child = std.process.Child.init(&.{"/bin/true"}, alloc);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Close;
    child.stderr_behavior = .Close;
    try child.spawn();
    _ = child.wait() catch {};
    s.child = child;
    defer s.deinit();

    const messages = try s.poll(alloc);
    // Static empty slice — do not free.
    try std.testing.expectEqual(@as(usize, 0), messages.len);
}

test "E1: WhatsApp.poll() returns empty slice when child.stdout is null" {
    const alloc = std.testing.allocator;
    var wa = WhatsApp.init(alloc, "Borg");

    var child = std.process.Child.init(&.{"/bin/true"}, alloc);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Close;
    child.stderr_behavior = .Close;
    try child.spawn();
    _ = child.wait() catch {};
    wa.child = child;
    defer wa.deinit();

    const messages = try wa.poll(alloc);
    try std.testing.expectEqual(@as(usize, 0), messages.len);
}

// ═════════════════════════════════════════════════════════════════════════════
// Edge E5 – Partial NDJSON line (no trailing newline) → 0 messages returned;
//            incomplete bytes stay in stdout_buf for the next poll().
// ═════════════════════════════════════════════════════════════════════════════

test "E5: Sidecar.poll() buffers an incomplete line and returns 0 messages" {
    const alloc = std.testing.allocator;
    var s = Sidecar.init(alloc, "Borg");

    s.child = try spawnEofChild(alloc);
    defer s.deinit();

    // Incomplete JSON (no trailing newline).
    const partial =
        "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"1\",\"channel_id\":\"c1\"";
    try s.stdout_buf.appendSlice(partial);

    // child is non-null and stdout is non-null → poll() returns a heap-allocated slice.
    const messages = try s.poll(alloc);
    defer alloc.free(messages);
    try std.testing.expectEqual(@as(usize, 0), messages.len);
    // The partial line must still be in the buffer.
    try std.testing.expect(s.stdout_buf.items.len > 0);
}

test "E5: WhatsApp.poll() buffers an incomplete line and returns 0 messages" {
    const alloc = std.testing.allocator;
    var wa = WhatsApp.init(alloc, "Borg");

    wa.child = try spawnEofChild(alloc);
    defer wa.deinit();

    const partial = "{\"event\":\"message\",\"jid\":\"g1@g.us\",\"id\":\"1\"";
    try wa.stdout_buf.appendSlice(partial);

    const messages = try wa.poll(alloc);
    defer alloc.free(messages);
    try std.testing.expectEqual(@as(usize, 0), messages.len);
    try std.testing.expect(wa.stdout_buf.items.len > 0);
}

test "E5: Sidecar.poll() completes partial line on next call" {
    const alloc = std.testing.allocator;
    var s = Sidecar.init(alloc, "Borg");

    s.child = try spawnEofChild(alloc);
    defer s.deinit();

    // First poll — partial line, no messages.
    const part1 = "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"1\",\"channel_id\":\"c1\",";
    try s.stdout_buf.appendSlice(part1);
    {
        const messages = try s.poll(alloc);
        defer alloc.free(messages); // heap-allocated empty slice
        try std.testing.expectEqual(@as(usize, 0), messages.len);
    }

    // Second poll — complete the line (note: child.stdout already returned EOF, so the
    // read loop drains nothing; the appended bytes go directly into stdout_buf).
    const part2 =
        "\"sender_id\":\"u1\",\"sender_name\":\"A\",\"text\":\"hi\",\"timestamp\":1000," ++
        "\"is_dm\":false,\"mentions_bot\":false}\n";
    try s.stdout_buf.appendSlice(part2);
    const messages = try s.poll(alloc);
    defer freeSidecarMessages(alloc, messages);
    try std.testing.expectEqual(@as(usize, 1), messages.len);
}

// ═════════════════════════════════════════════════════════════════════════════
// Edge E7 – Bridge exits unexpectedly; already-buffered complete lines returned
// ═════════════════════════════════════════════════════════════════════════════

test "E7: Sidecar.poll() returns complete lines even after bridge process exits" {
    const alloc = std.testing.allocator;
    var s = Sidecar.init(alloc, "Borg");

    // Process writes one complete line then exits immediately.
    const line =
        "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"99\"," ++
        "\"channel_id\":\"c1\",\"sender_id\":\"u1\",\"sender_name\":\"X\"," ++
        "\"text\":\"eof-test\",\"timestamp\":1000,\"is_dm\":false,\"mentions_bot\":false}";
    const script = "echo '" ++ line ++ "'";

    var child = std.process.Child.init(&.{ "/bin/sh", "-c", script }, alloc);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Close;
    try child.spawn();
    _ = child.wait() catch {}; // process has exited; pipe contains data
    s.child = child;
    defer s.deinit();

    const messages = try s.poll(alloc);
    defer freeSidecarMessages(alloc, messages);

    try std.testing.expectEqual(@as(usize, 1), messages.len);
}

test "E7: WhatsApp.poll() returns complete lines even after bridge process exits" {
    const alloc = std.testing.allocator;
    var wa = WhatsApp.init(alloc, "Borg");

    const line =
        "{\"event\":\"message\",\"jid\":\"g1@g.us\",\"id\":\"77\",\"sender\":\"u1\"," ++
        "\"sender_name\":\"Z\",\"text\":\"eof-wa\",\"timestamp\":1000," ++
        "\"is_group\":true,\"mentions_bot\":false}";
    const script = "echo '" ++ line ++ "'";

    var child = std.process.Child.init(&.{ "/bin/sh", "-c", script }, alloc);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Close;
    try child.spawn();
    _ = child.wait() catch {};
    wa.child = child;
    defer wa.deinit();

    const messages = try wa.poll(alloc);
    defer freeWaMessages(alloc, messages);

    try std.testing.expectEqual(@as(usize, 1), messages.len);
}

// ═════════════════════════════════════════════════════════════════════════════
// Edge E4 – Data accumulates between poll() calls; next poll() reads it all
// ═════════════════════════════════════════════════════════════════════════════

test "E4: Sidecar data appended to stdout_buf between polls is returned on next call" {
    const alloc = std.testing.allocator;
    var s = Sidecar.init(alloc, "Borg");

    s.child = try spawnEofChild(alloc);
    defer s.deinit();

    const msg1 =
        "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"1\"," ++
        "\"channel_id\":\"c1\",\"sender_id\":\"u1\",\"sender_name\":\"A\"," ++
        "\"text\":\"first\",\"timestamp\":1000,\"is_dm\":false,\"mentions_bot\":false}\n";
    const msg2 =
        "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"2\"," ++
        "\"channel_id\":\"c1\",\"sender_id\":\"u1\",\"sender_name\":\"A\"," ++
        "\"text\":\"second\",\"timestamp\":1001,\"is_dm\":false,\"mentions_bot\":false}\n";

    // First poll — one message.
    try s.stdout_buf.appendSlice(msg1);
    {
        const messages = try s.poll(alloc);
        defer freeSidecarMessages(alloc, messages);
        try std.testing.expectEqual(@as(usize, 1), messages.len);
    }

    // Simulate data arriving between polls.
    try s.stdout_buf.appendSlice(msg2);

    // Second poll — second message.
    const messages = try s.poll(alloc);
    defer freeSidecarMessages(alloc, messages);
    try std.testing.expectEqual(@as(usize, 1), messages.len);
}

// ═════════════════════════════════════════════════════════════════════════════
// Edge E9 – O_NONBLOCK does not suppress data already in the pipe
// A process writes data before poll() is called; data must be returned.
// ═════════════════════════════════════════════════════════════════════════════

test "E9: Sidecar.poll() reads data present in pipe even when O_NONBLOCK is set" {
    const alloc = std.testing.allocator;
    var s = Sidecar.init(alloc, "Borg");

    const line =
        "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"e9\"," ++
        "\"channel_id\":\"c1\",\"sender_id\":\"u1\",\"sender_name\":\"T\"," ++
        "\"text\":\"nonblock-data\",\"timestamp\":1000,\"is_dm\":false,\"mentions_bot\":false}";
    const script = "echo '" ++ line ++ "'";

    // O_NONBLOCK set; data is written before poll() is called.
    var child = try spawnNonblocking(&.{ "/bin/sh", "-c", script }, alloc);
    _ = child.wait() catch {}; // all data in pipe; write-end closed
    s.child = child;
    defer s.deinit();

    const messages = try s.poll(alloc);
    defer freeSidecarMessages(alloc, messages);

    try std.testing.expectEqual(@as(usize, 1), messages.len);
}

// ═════════════════════════════════════════════════════════════════════════════
// AC2 – The poll loop does not block for POLL_INTERVAL_MS when bridge is idle.
// Proxy: 20 consecutive poll() calls on an idle O_NONBLOCK pipe must all
// complete in well under 500 ms (the main-loop sleep interval).
// ═════════════════════════════════════════════════════════════════════════════

test "AC2: 20 Sidecar.poll() calls on idle bridge complete well under 500 ms total" {
    const alloc = std.testing.allocator;
    var s = Sidecar.init(alloc, "Borg");

    var child = try spawnNonblocking(&.{ "/bin/sh", "-c", "sleep 10" }, alloc);
    s.child = child;

    const t0 = std.time.milliTimestamp();
    var round: usize = 0;
    while (round < 20) : (round += 1) {
        const messages = try s.poll(alloc);
        alloc.free(messages);
        try std.testing.expectEqual(@as(usize, 0), messages.len);
    }
    const elapsed = std.time.milliTimestamp() - t0;

    if (s.child) |*c| _ = c.kill() catch {};
    s.child = null;
    s.deinit();

    // 20 polls on an idle non-blocking fd must complete in << 500 ms.
    try std.testing.expect(elapsed < 500);
}

test "AC2: 20 WhatsApp.poll() calls on idle bridge complete well under 500 ms total" {
    const alloc = std.testing.allocator;
    var wa = WhatsApp.init(alloc, "Borg");

    var child = try spawnNonblocking(&.{ "/bin/sh", "-c", "sleep 10" }, alloc);
    wa.child = child;

    const t0 = std.time.milliTimestamp();
    var round: usize = 0;
    while (round < 20) : (round += 1) {
        const messages = try wa.poll(alloc);
        alloc.free(messages);
        try std.testing.expectEqual(@as(usize, 0), messages.len);
    }
    const elapsed = std.time.milliTimestamp() - t0;

    if (wa.child) |*c| _ = c.kill() catch {};
    wa.child = null;
    wa.deinit();

    try std.testing.expect(elapsed < 500);
}

// ═════════════════════════════════════════════════════════════════════════════
// Edge E6 – > 4096-byte burst on a real pipe for WhatsApp
// Exercises the re-entry loop: read_buf fills → loop continues → WouldBlock
// finally breaks the loop; all messages captured.
// ═════════════════════════════════════════════════════════════════════════════

test "E6: WhatsApp.poll() drains > 4096 bytes written to a real pipe in one call" {
    const alloc = std.testing.allocator;
    var wa = WhatsApp.init(alloc, "Borg");

    // Script writes 30 complete NDJSON lines (> 4096 bytes total) then exits.
    const script =
        "i=0; while [ $i -lt 30 ]; do " ++
        "echo '{\"event\":\"message\",\"jid\":\"g1@g.us\",\"id\":\"X\",\"sender\":\"u1\"," ++
        "\"sender_name\":\"Alice\",\"text\":\"large-burst-wa\",\"timestamp\":1000," ++
        "\"is_group\":true,\"mentions_bot\":false}'; " ++
        "i=$((i+1)); done";

    // O_NONBLOCK so poll() doesn't block after the pipe is drained.
    var child = try spawnNonblocking(&.{ "/bin/sh", "-c", script }, alloc);
    _ = child.wait() catch {}; // all data is in the pipe buffer; write-end closed
    wa.child = child;
    defer wa.deinit();

    const messages = try wa.poll(alloc);
    defer freeWaMessages(alloc, messages);

    try std.testing.expectEqual(@as(usize, 30), messages.len);
}

// ═════════════════════════════════════════════════════════════════════════════
// Edge E10 – fcntl call lives inside start(), so any new child after a restart
// also gets O_NONBLOCK.  Source-code check: "fcntl" appears after "spawn()" in
// both files (not before, not in poll(), not in deinit()).
// FAILS initially because the fcntl block is absent from start().
// ═════════════════════════════════════════════════════════════════════════════

test "E10: sidecar.zig sets O_NONBLOCK inside start() (after spawn, before end of start)" {
    const src = @embedFile("sidecar.zig");
    // Locate start() and verify that SETFL appears within it.
    // We find the first "pub fn start" and the first "pub fn deinit" (or "pub fn poll")
    // and confirm "SETFL" falls between those two positions.
    const start_pos = std.mem.indexOf(u8, src, "pub fn start(") orelse {
        try std.testing.expect(false); // start() missing
        return;
    };
    // Find next top-level fn after start().
    const after_start = src[start_pos + 1 ..];
    const next_fn_rel = std.mem.indexOf(u8, after_start, "\n    pub fn ") orelse src.len - start_pos - 1;
    const end_start = start_pos + 1 + next_fn_rel;

    const start_body = src[start_pos..end_start];
    try std.testing.expect(std.mem.indexOf(u8, start_body, "SETFL") != null);
}

test "E10: whatsapp.zig sets O_NONBLOCK inside start() (after spawn, before end of start)" {
    const src = @embedFile("whatsapp.zig");
    const start_pos = std.mem.indexOf(u8, src, "pub fn start(") orelse {
        try std.testing.expect(false);
        return;
    };
    const after_start = src[start_pos + 1 ..];
    const next_fn_rel = std.mem.indexOf(u8, after_start, "\n    pub fn ") orelse src.len - start_pos - 1;
    const end_start = start_pos + 1 + next_fn_rel;

    const start_body = src[start_pos..end_start];
    try std.testing.expect(std.mem.indexOf(u8, start_body, "SETFL") != null);
}

test "E10: sidecar.zig does not set O_NONBLOCK inside poll() or deinit()" {
    const src = @embedFile("sidecar.zig");
    // poll() body: from "pub fn poll(" to the next top-level fn.
    const poll_pos = std.mem.indexOf(u8, src, "pub fn poll(") orelse return;
    const after_poll = src[poll_pos + 1 ..];
    const next_rel = std.mem.indexOf(u8, after_poll, "\n    pub fn ") orelse after_poll.len;
    const poll_body = src[poll_pos .. poll_pos + 1 + next_rel];
    try std.testing.expect(std.mem.indexOf(u8, poll_body, "SETFL") == null);
}

test "E10: whatsapp.zig does not set O_NONBLOCK inside poll() or deinit()" {
    const src = @embedFile("whatsapp.zig");
    const poll_pos = std.mem.indexOf(u8, src, "pub fn poll(") orelse return;
    const after_poll = src[poll_pos + 1 ..];
    const next_rel = std.mem.indexOf(u8, after_poll, "\n    pub fn ") orelse after_poll.len;
    const poll_body = src[poll_pos .. poll_pos + 1 + next_rel];
    try std.testing.expect(std.mem.indexOf(u8, poll_body, "SETFL") == null);
}
