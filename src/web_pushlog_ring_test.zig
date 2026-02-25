// Tests for: pushLog ring buffer wrap-around, count cap, and message truncation
//
// Verifies the four core behaviours of the WebServer log ring buffer:
//   1. Single push increments count and advances head
//   2. Filling the ring caps log_count at LOG_RING_SIZE
//   3. One push beyond full silently overwrites the oldest slot
//   4. Messages longer than 512 bytes are stored truncated, not panicked
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
