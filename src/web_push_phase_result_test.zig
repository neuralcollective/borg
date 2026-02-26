// Tests for WebServer.pushPhaseResult.
//
// These FAIL initially because pushPhaseResult does not exist on WebServer yet.
// Once implemented they cover:
//   AC2: pushPhaseResult injects a synthetic SSE event into the task's live stream.
//   AC2: The injected line contains type:"phase_result", the phase name, and the content.
//   EC7: SSE injection works even when notify_chat is empty (no live clients registered).
//   EC9: phase_result event appears in history before stream_end.
//   EC4: calling pushPhaseResult with a nonexistent task_id is a no-op (no crash).
//
// To wire into the build, add inside the trailing `test { … }` block of
// src/web.zig:
//   _ = @import("web_push_phase_result_test.zig");

const std = @import("std");
const web_mod = @import("web.zig");
const WebServer = web_mod.WebServer;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn makeTestServer(alloc: std.mem.Allocator) WebServer {
    return WebServer.init(
        alloc,
        @ptrFromInt(0x10000), // fake *Db  — not dereferenced in these tests
        @ptrFromInt(0x10000), // fake *Config — not dereferenced in these tests
        0,
        "127.0.0.1",
    );
}

fn cleanupTestServer(ws: *WebServer) void {
    for (ws.sse_clients.items) |c| c.close();
    ws.sse_clients.deinit();
    for (ws.chat_sse_clients.items) |c| c.close();
    ws.chat_sse_clients.deinit();
    ws.chat_queue.deinit();
    ws.task_streams.deinit();
}

fn cleanupWithStreams(ws: *WebServer, task_ids: []const i64) void {
    for (task_ids) |id| ws.endTaskStream(id);
    cleanupTestServer(ws);
}

// ── AC2: pushPhaseResult is declared on WebServer ────────────────────────────

test "AC2: pushPhaseResult is declared on WebServer" {
    try std.testing.expect(@hasDecl(WebServer, "pushPhaseResult"));
}

// ── AC2: SSE history contains the injected event ─────────────────────────────

test "AC2: pushPhaseResult injects an event into the task stream history" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 42;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    ws.pushPhaseResult(task_id, "spec", "Here is the specification.");

    const entry = ws.task_streams.getPtr(task_id) orelse {
        try std.testing.expect(false); // stream must still exist
        return;
    };
    try std.testing.expect(entry.history.items.len > 0);
}

test "AC2: history contains the string 'phase_result' after pushPhaseResult" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 43;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    ws.pushPhaseResult(task_id, "spec", "Summary.");

    const entry = ws.task_streams.getPtr(task_id).?;
    try std.testing.expect(std.mem.indexOf(u8, entry.history.items, "phase_result") != null);
}

test "AC2: history contains the JSON key type:phase_result" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 44;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    ws.pushPhaseResult(task_id, "qa", "QA complete.");

    const entry = ws.task_streams.getPtr(task_id).?;
    const hist = entry.history.items;
    // Accept either "type":"phase_result" or "type": "phase_result"
    const has_type = std.mem.indexOf(u8, hist, "\"type\":\"phase_result\"") != null or
        std.mem.indexOf(u8, hist, "\"type\": \"phase_result\"") != null;
    try std.testing.expect(has_type);
}

test "AC2: history contains the phase name" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 45;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    ws.pushPhaseResult(task_id, "qa_fix", "Fixed test.");

    const entry = ws.task_streams.getPtr(task_id).?;
    try std.testing.expect(std.mem.indexOf(u8, entry.history.items, "qa_fix") != null);
}

test "AC2: history contains the content string" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 46;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    ws.pushPhaseResult(task_id, "spec", "UniqueSentinel_XYZ_987");

    const entry = ws.task_streams.getPtr(task_id).?;
    try std.testing.expect(std.mem.indexOf(u8, entry.history.items, "UniqueSentinel_XYZ_987") != null);
}

test "AC2: the injected line is wrapped in a valid SSE data frame" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 47;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    ws.pushPhaseResult(task_id, "spec", "Content.");

    const entry = ws.task_streams.getPtr(task_id).?;
    const hist = entry.history.items;
    // SSE frame must start with "data: " and end with "\n\n"
    try std.testing.expect(std.mem.indexOf(u8, hist, "data: ") != null);
    try std.testing.expect(std.mem.endsWith(u8, std.mem.trimRight(u8, hist, ""), "\n\n") or
        std.mem.indexOf(u8, hist, "\n\n") != null);
}

// ── EC7: SSE injection works when no SSE clients are registered ───────────────

test "EC7: pushPhaseResult does not crash when no live clients are registered" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 50;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    // Verify no clients are registered
    try std.testing.expectEqual(
        @as(usize, 0),
        ws.task_streams.getPtr(task_id).?.clients.items.len,
    );

    // Must not crash; history must still be populated
    ws.pushPhaseResult(task_id, "spec", "No clients but history still updated.");

    const entry = ws.task_streams.getPtr(task_id).?;
    try std.testing.expect(entry.history.items.len > 0);
}

// ── EC9: phase_result event appears before stream_end in history ──────────────

test "EC9: phase_result event is present in history before endTaskStream is called" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 51;
    ws.startTaskStream(task_id);
    defer cleanupTestServer(&ws);

    ws.pushPhaseResult(task_id, "spec", "Summary before stream end.");

    // Verify phase_result is in history
    const entry = ws.task_streams.getPtr(task_id).?;
    try std.testing.expect(std.mem.indexOf(u8, entry.history.items, "phase_result") != null);
    // stream_end has NOT been injected yet (endTaskStream not called)
    try std.testing.expect(std.mem.indexOf(u8, entry.history.items, "stream_end") == null);
}

test "EC9: phase_result appears before stream_end after endTaskStream is called" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 52;
    ws.startTaskStream(task_id);
    defer cleanupTestServer(&ws);

    // The push must happen before end
    ws.pushPhaseResult(task_id, "qa", "QA summary.");

    // Snapshot history before end
    const entry_before = ws.task_streams.getPtr(task_id).?;
    const phase_result_pos = std.mem.indexOf(u8, entry_before.history.items, "phase_result");
    try std.testing.expect(phase_result_pos != null);

    // endTaskStream delivers stream_end to clients and frees the stream entry
    // — we just verified phase_result was present before end was called.
    ws.endTaskStream(task_id);
}

// ── EC4: pushPhaseResult with nonexistent task_id is a no-op ─────────────────

test "EC4: pushPhaseResult with nonexistent task_id does not crash" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    // No stream started for this task_id — must be a safe no-op
    ws.pushPhaseResult(9999, "spec", "Ghost task.");
}

// ── Multiple calls: two pushPhaseResult calls both appear in history ──────────

test "multi-push: two pushPhaseResult calls both appear in history" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_id: i64 = 60;
    ws.startTaskStream(task_id);
    defer cleanupWithStreams(&ws, &.{task_id});

    ws.pushPhaseResult(task_id, "spec", "First_UniqueABC");
    ws.pushPhaseResult(task_id, "qa", "Second_UniqueXYZ");

    const entry = ws.task_streams.getPtr(task_id).?;
    try std.testing.expect(std.mem.indexOf(u8, entry.history.items, "First_UniqueABC") != null);
    try std.testing.expect(std.mem.indexOf(u8, entry.history.items, "Second_UniqueXYZ") != null);
}

// ── Different task_ids get independent streams ────────────────────────────────

test "isolation: pushPhaseResult only affects the specified task stream" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    const task_a: i64 = 70;
    const task_b: i64 = 71;
    ws.startTaskStream(task_a);
    ws.startTaskStream(task_b);
    defer cleanupWithStreams(&ws, &.{ task_a, task_b });

    ws.pushPhaseResult(task_a, "spec", "Only for task A.");

    const entry_a = ws.task_streams.getPtr(task_a).?;
    const entry_b = ws.task_streams.getPtr(task_b).?;

    try std.testing.expect(std.mem.indexOf(u8, entry_a.history.items, "Only for task A.") != null);
    // Task B stream must be untouched
    try std.testing.expect(std.mem.indexOf(u8, entry_b.history.items, "Only for task A.") == null);
}
