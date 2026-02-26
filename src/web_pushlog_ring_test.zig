// Tests for: pushLog ring buffer wrap-around, count cap, and message truncation,
// and Task #75: Fix stream history cap to check size before appending all slices.
//
// Verifies the four core behaviours of the WebServer log ring buffer:
//   1. Single push increments count and advances head
//   2. Filling the ring caps log_count at LOG_RING_SIZE
//   3. One push beyond full silently overwrites the oldest slot
//   4. Messages longer than 512 bytes are stored truncated, not panicked
//
// Also verifies (Task #75) that broadcastTaskStream's 2MB history cap accounts
// for the 8-byte SSE frame overhead ("data: " + "\n\n") before appending, so
// a near-2MB line cannot push history past the cap.
//
// To wire this file into the build, add inside the `test { … }` block at
// the bottom of src/web.zig:
//   _ = @import("web_pushlog_ring_test.zig");

const std = @import("std");
const web_mod = @import("web.zig");
const WebServer = web_mod.WebServer;

// Mirror the values from web.zig (not pub-exported, so hardcoded here).
const LOG_RING_SIZE: usize = 500;
const LOG_MSG_CAP: usize = 512;

// ── Helpers ─────────────────────────────────────────────────────────────

/// Minimal WebServer for ring-buffer tests.
/// db and config are fake pointers — never dereferenced when level is "info".
fn makeTestServer(alloc: std.mem.Allocator) WebServer {
    return WebServer.init(
        alloc,
        @ptrFromInt(0x10000), // fake *Db  (not touched for "info" level)
        @ptrFromInt(0x10000), // fake *Config (never dereferenced in pushLog)
        0,
        "127.0.0.1",
    );
}

/// Free all ArrayList memory allocated by WebServer.init.
fn cleanupTestServer(ws: *WebServer) void {
    for (ws.sse_clients.items) |c| c.close();
    ws.sse_clients.deinit();
    for (ws.chat_sse_clients.items) |c| c.close();
    ws.chat_sse_clients.deinit();
    ws.chat_queue.deinit();
    ws.task_streams.deinit();
}

// ── AC1: Single push sets count to 1 and advances head ──────────────────

test "AC1: single pushLog sets log_count to 1" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    ws.pushLog("info", "hello");

    try std.testing.expectEqual(@as(usize, 1), ws.log_count);
}

test "AC1: single pushLog advances log_head to 1" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    ws.pushLog("info", "hello");

    try std.testing.expectEqual(@as(usize, 1), ws.log_head);
}

test "AC1: single pushLog stores message correctly in slot 0" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    ws.pushLog("info", "hello");

    const entry = ws.log_ring[0];
    try std.testing.expectEqual(@as(u16, 5), entry.message_len);
    try std.testing.expectEqualStrings("hello", entry.message[0..5]);
    try std.testing.expect(entry.active);
}

// ── AC2: Filling the ring caps count at LOG_RING_SIZE ───────────────────

test "AC2: filling ring caps log_count at LOG_RING_SIZE" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    for (0..LOG_RING_SIZE) |_| {
        ws.pushLog("info", "fill");
    }

    try std.testing.expectEqual(LOG_RING_SIZE, ws.log_count);
}

test "AC2: filling ring wraps log_head back to 0" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    for (0..LOG_RING_SIZE) |_| {
        ws.pushLog("info", "fill");
    }

    try std.testing.expectEqual(@as(usize, 0), ws.log_head);
}

test "AC2: log_count does not exceed LOG_RING_SIZE after extra pushes" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    // Push one more than the ring can hold
    for (0..LOG_RING_SIZE + 1) |_| {
        ws.pushLog("info", "extra");
    }

    try std.testing.expectEqual(LOG_RING_SIZE, ws.log_count);
}

// ── AC3: One push beyond full overwrites the oldest entry ───────────────

test "AC3: push beyond full leaves log_count capped" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    for (0..LOG_RING_SIZE) |_| {
        ws.pushLog("info", "old");
    }
    ws.pushLog("info", "newest");

    try std.testing.expectEqual(LOG_RING_SIZE, ws.log_count);
}

test "AC3: push beyond full advances log_head to 1" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    for (0..LOG_RING_SIZE) |_| {
        ws.pushLog("info", "old");
    }
    // At this point log_head == 0; one more push writes to slot 0 then advances to 1
    ws.pushLog("info", "newest");

    try std.testing.expectEqual(@as(usize, 1), ws.log_head);
}

test "AC3: push beyond full overwrites slot 0 with the newest message" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    for (0..LOG_RING_SIZE) |_| {
        ws.pushLog("info", "old");
    }
    ws.pushLog("info", "newest");

    const entry = ws.log_ring[0];
    try std.testing.expectEqual(@as(u16, 6), entry.message_len);
    try std.testing.expectEqualStrings("newest", entry.message[0..6]);
}

// ── AC4: Over-length message is stored truncated, no bounds error ────────

test "AC4: message longer than LOG_MSG_CAP is stored with message_len capped" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    var long_msg: [LOG_MSG_CAP + 100]u8 = undefined;
    @memset(&long_msg, 'A');

    ws.pushLog("info", &long_msg);

    try std.testing.expectEqual(@as(u16, LOG_MSG_CAP), ws.log_ring[0].message_len);
}

test "AC4: truncated message contains only the first LOG_MSG_CAP bytes" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    var long_msg: [LOG_MSG_CAP + 100]u8 = undefined;
    @memset(&long_msg, 'A');

    ws.pushLog("info", &long_msg);

    const stored = ws.log_ring[0].message[0..LOG_MSG_CAP];
    for (stored) |byte| {
        try std.testing.expectEqual(@as(u8, 'A'), byte);
    }
}

test "AC4: over-length push still increments log_count" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    var long_msg: [LOG_MSG_CAP + 100]u8 = undefined;
    @memset(&long_msg, 'A');

    ws.pushLog("info", &long_msg);

    try std.testing.expectEqual(@as(usize, 1), ws.log_count);
}

// ── EC1: Level field truncation ──────────────────────────────────────────

test "EC1: level longer than 8 bytes is stored without panic, level_len capped at 8" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    ws.pushLog("verylongLevel", "msg");

    // level array is [8]u8; level_len must be <= 8
    try std.testing.expect(ws.log_ring[0].level_len <= 8);
    try std.testing.expectEqual(@as(u8, 8), ws.log_ring[0].level_len);
}

// ── EC2: Empty message ───────────────────────────────────────────────────

test "EC2: empty message is stored with message_len 0 and log_count 1" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    ws.pushLog("info", "");

    try std.testing.expectEqual(@as(u16, 0), ws.log_ring[0].message_len);
    try std.testing.expectEqual(@as(usize, 1), ws.log_count);
}

// ── EC3: Exact-boundary message (exactly LOG_MSG_CAP bytes) ─────────────

test "EC3: message of exactly LOG_MSG_CAP bytes is stored without truncation" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    var exact_msg: [LOG_MSG_CAP]u8 = undefined;
    @memset(&exact_msg, 'B');

    ws.pushLog("info", &exact_msg);

    try std.testing.expectEqual(@as(u16, LOG_MSG_CAP), ws.log_ring[0].message_len);
    for (ws.log_ring[0].message[0..LOG_MSG_CAP]) |byte| {
        try std.testing.expectEqual(@as(u8, 'B'), byte);
    }
}

// ── EC5: Multiple wrap-arounds ───────────────────────────────────────────

test "EC5: two full cycles plus one push leaves log_count capped and log_head at 1" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    for (0..2 * LOG_RING_SIZE + 1) |_| {
        ws.pushLog("info", "cycle");
    }

    try std.testing.expectEqual(LOG_RING_SIZE, ws.log_count);
    try std.testing.expectEqual(@as(usize, 1), ws.log_head);
}

// ── EC6: log_head arithmetic stays in range ──────────────────────────────

test "EC6: log_head is always less than LOG_RING_SIZE after filling the ring" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    for (0..LOG_RING_SIZE) |_| {
        ws.pushLog("info", "x");
    }

    try std.testing.expect(ws.log_head < LOG_RING_SIZE);
}

test "EC6: log_head is always less than LOG_RING_SIZE after overflow" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    for (0..LOG_RING_SIZE + 50) |_| {
        ws.pushLog("info", "x");
    }

    try std.testing.expect(ws.log_head < LOG_RING_SIZE);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Task #75 — Stream history cap: check total bytes before appending
//
// The corrected guard is:
//   entry.history.items.len + line.len + 8 < 2 * 1024 * 1024
//
// where 8 = len("data: ") + len("\n\n").
//
// Tests AC1–AC5 and all edge cases from spec.md.
// Tests marked "FAILS initially" will fail against the buggy implementation.
// ═══════════════════════════════════════════════════════════════════════════════

const HIST_CAP: usize = 2 * 1024 * 1024;
const SSE_FRAME_OVERHEAD: usize = 8; // "data: " (6) + "\n\n" (2)

/// Open a Unix pipe; return write end as std.net.Stream and keep read fd.
fn makeTestPipe() !struct { stream: std.net.Stream, read_fd: std.posix.fd_t } {
    const fds = try std.posix.pipe();
    return .{ .stream = .{ .handle = fds[1] }, .read_fd = fds[0] };
}

/// End all named task streams then call cleanupTestServer.
/// Must be used instead of cleanupTestServer whenever startTaskStream was called,
/// to free the per-stream ArrayList allocations before the map is deinit'd.
fn cleanupWithStreams(ws: *WebServer, task_ids: []const i64) void {
    for (task_ids) |id| ws.endTaskStream(id);
    cleanupTestServer(ws);
}

// ── AC75-1: near-2MB line rejected when history already has data ─────────────
//
// FAILS initially: the buggy check `history.items.len < CAP` passes (tiny < CAP)
// and appends a nearly-2MB line, blowing past the cap.

test "AC75-1: line of CAP-1 bytes is rejected when history already has data" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 1001;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    // Seed history with a small line ("data: init\n\n" = 13 bytes).
    ws.broadcastTaskStream(task_id, "init\n");
    const after_init = ws.task_streams.getPtr(task_id).?.history.items.len;
    try std.testing.expectEqual(@as(usize, 13), after_init);

    // A line of CAP-1 bytes: after_init + (CAP-1) + 8 = CAP + 20 >= CAP → reject.
    const line_len = HIST_CAP - 1;
    const buf = try alloc.alloc(u8, line_len + 1);
    defer alloc.free(buf);
    @memset(buf[0..line_len], 'A');
    buf[line_len] = '\n';

    ws.broadcastTaskStream(task_id, buf);

    // Fixed: history stays at 13. Buggy: history >> CAP.
    try std.testing.expectEqual(after_init, ws.task_streams.getPtr(task_id).?.history.items.len);
}

// ── AC75-2: line that brings total to exactly CAP-1 bytes IS appended ───────
//
// Passes with both old and new code (regression / documentation test).

test "AC75-2: line bringing total to exactly CAP-1 bytes is appended" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 1002;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    // 0 + line_len + 8 = CAP - 1  →  line_len = CAP - 9
    const line_len = HIST_CAP - SSE_FRAME_OVERHEAD - 1;
    const buf = try alloc.alloc(u8, line_len + 1);
    defer alloc.free(buf);
    @memset(buf[0..line_len], 'B');
    buf[line_len] = '\n';

    ws.broadcastTaskStream(task_id, buf);

    // history = "data: " + line_len bytes + "\n\n" = CAP - 1 bytes
    try std.testing.expectEqual(HIST_CAP - 1, ws.task_streams.getPtr(task_id).?.history.items.len);
}

// ── AC75-3: line that brings total to exactly CAP bytes is NOT appended ──────
//
// FAILS initially: buggy check `0 < CAP` passes and appends, making history = CAP.

test "AC75-3: line that would bring total to exactly 2MB bytes is rejected" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 1003;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    // 0 + line_len + 8 = CAP  →  line_len = CAP - 8  →  not < CAP → reject.
    const line_len = HIST_CAP - SSE_FRAME_OVERHEAD;
    const buf = try alloc.alloc(u8, line_len + 1);
    defer alloc.free(buf);
    @memset(buf[0..line_len], 'C');
    buf[line_len] = '\n';

    ws.broadcastTaskStream(task_id, buf);

    // Fixed: history stays at 0. Buggy: history = CAP.
    try std.testing.expectEqual(@as(usize, 0), ws.task_streams.getPtr(task_id).?.history.items.len);
}

// ── AC75-4: small line from empty history is appended normally ───────────────
//
// Passes with both old and new code (regression / documentation test).

test "AC75-4: small line from empty history is appended" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 1004;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    ws.broadcastTaskStream(task_id, "hello\n");

    // "data: hello\n\n" = 6 + 5 + 2 = 13 bytes
    const hist = ws.task_streams.getPtr(task_id).?.history.items;
    try std.testing.expectEqual(@as(usize, 13), hist.len);
    try std.testing.expectEqualStrings("data: hello\n\n", hist);
}

// ── AC75-5: live client delivery unaffected when line is rejected from history ─
//
// Even when the history cap rejects a line, broadcastTaskStream must still
// deliver it to registered live clients.  Passes with both old and new code
// (regression test — the fix must not accidentally move delivery inside the if).

test "AC75-5: live client receives data even when history cap rejects the line" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 1005;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    // Register a live client via a Unix pipe.
    const pipe = try makeTestPipe();
    defer std.posix.close(pipe.read_fd);
    try ws.task_streams.getPtr(task_id).?.clients.append(pipe.stream);

    // Pre-fill history so the next short line would be rejected (total > CAP).
    // Direct append is safe in single-threaded tests.
    const entry = ws.task_streams.getPtr(task_id).?;
    try entry.history.appendNTimes('X', HIST_CAP - 5);

    // "msg\n": line_len=3, total = (CAP-5) + 3 + 8 = CAP + 6 >= CAP → rejected.
    ws.broadcastTaskStream(task_id, "msg\n");

    // History must not have grown.
    try std.testing.expectEqual(HIST_CAP - 5, ws.task_streams.getPtr(task_id).?.history.items.len);

    // The line must have been delivered to the live client regardless.
    // The write was synchronous, so the data is in the pipe buffer now.
    var read_buf: [32]u8 = undefined;
    const n = try std.posix.read(pipe.read_fd, &read_buf);
    try std.testing.expectEqualStrings("data: msg\n\n", read_buf[0..n]);
}

// ── Edge: exactly CAP-9 bytes admitted (total = CAP-1) ──────────────────────
//
// Passes with both old and new code.

test "AC75-edge-boundary: line of CAP-9 bytes from empty history is admitted" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 1006;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    // 0 + (CAP-9) + 8 = CAP-1 < CAP → admitted
    const line_len = HIST_CAP - SSE_FRAME_OVERHEAD - 1;
    const buf = try alloc.alloc(u8, line_len + 1);
    defer alloc.free(buf);
    @memset(buf[0..line_len], 'D');
    buf[line_len] = '\n';

    ws.broadcastTaskStream(task_id, buf);

    try std.testing.expectEqual(HIST_CAP - 1, ws.task_streams.getPtr(task_id).?.history.items.len);
}

// ── Edge: exactly CAP-8 bytes rejected (total = CAP, strict <) ──────────────
//
// FAILS initially: buggy `0 < CAP` passes and appends CAP bytes to history.

test "AC75-edge-boundary+1: line of CAP-8 bytes from empty history is rejected" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 1007;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    // 0 + (CAP-8) + 8 = CAP, NOT < CAP → rejected
    const line_len = HIST_CAP - SSE_FRAME_OVERHEAD;
    const buf = try alloc.alloc(u8, line_len + 1);
    defer alloc.free(buf);
    @memset(buf[0..line_len], 'E');
    buf[line_len] = '\n';

    ws.broadcastTaskStream(task_id, buf);

    // Fixed: 0 bytes. Buggy: CAP bytes.
    try std.testing.expectEqual(@as(usize, 0), ws.task_streams.getPtr(task_id).?.history.items.len);
}

// ── Edge: pre-filled buffer — short line rejected when history is near cap ───
//
// FAILS initially: `(CAP-10) < CAP` passes and appends the short line.

test "AC75-prefilled: short line rejected when history is already near cap" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 1008;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    // Pre-fill history directly (single-threaded test, no lock needed).
    const fill: usize = HIST_CAP - 10;
    const entry = ws.task_streams.getPtr(task_id).?;
    try entry.history.appendNTimes('X', fill);

    // "hello\n": line_len=5, total = (CAP-10) + 5 + 8 = CAP+3 >= CAP → rejected.
    ws.broadcastTaskStream(task_id, "hello\n");

    // Fixed: history stays at fill. Buggy: history grows by 13 bytes.
    try std.testing.expectEqual(fill, ws.task_streams.getPtr(task_id).?.history.items.len);
}

// ── Edge: consecutive lines — second rejected after first fills history ───────
//
// Each line is evaluated independently against the current buffer length.
// FAILS initially: after the first line history = CAP-1; the buggy check
// `(CAP-1) < CAP` still passes and appends the one-byte second line.

test "AC75-consecutive: second line rejected after first fills history to CAP-1" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 1009;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    // First line: CAP-9 bytes → total = CAP-1 → admitted.
    const line1_len = HIST_CAP - SSE_FRAME_OVERHEAD - 1;
    const buf1 = try alloc.alloc(u8, line1_len + 1);
    defer alloc.free(buf1);
    @memset(buf1[0..line1_len], 'F');
    buf1[line1_len] = '\n';
    ws.broadcastTaskStream(task_id, buf1);
    try std.testing.expectEqual(HIST_CAP - 1, ws.task_streams.getPtr(task_id).?.history.items.len);

    // Second line: "x\n" → line_len=1, total = (CAP-1) + 1 + 8 = CAP+8 → rejected.
    ws.broadcastTaskStream(task_id, "x\n");

    // Fixed: history stays at CAP-1. Buggy: history = CAP+8.
    try std.testing.expectEqual(HIST_CAP - 1, ws.task_streams.getPtr(task_id).?.history.items.len);
}
