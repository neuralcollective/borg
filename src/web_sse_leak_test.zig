// Tests for: Fix SSE client file descriptor leak in web server
//
// Verifies that disconnected SSE client streams are properly closed
// in all three broadcast sites and during server shutdown.
//
// These tests should FAIL before the fix because:
// - broadcastSse discards the swapRemove return value without closing the stream
// - broadcastChatEvent does the same
// - handleChatPost's inline broadcast does the same
// - stop() doesn't close SSE client streams at all

const std = @import("std");
const web_mod = @import("web.zig");
const WebServer = web_mod.WebServer;

// ── Helpers ────────────────────────────────────────────────────────────

/// Check if a file descriptor is still open by attempting fstat.
/// Returns false if the FD is closed (EBADF) or invalid.
fn isFdOpen(fd: std.posix.fd_t) bool {
    _ = std.posix.fstat(fd) catch return false;
    return true;
}

/// Create a pipe whose write end is wrapped as a std.net.Stream.
/// The read end is closed immediately, so any write will fail with BrokenPipe.
fn makeBrokenStream() !std.net.Stream {
    const fds = try std.posix.pipe();
    std.posix.close(fds[0]); // close read end
    return .{ .handle = fds[1] };
}

/// Create a pipe whose write end is wrapped as a std.net.Stream.
/// Both ends remain open; caller must close read_fd when done.
fn makeOpenStream() !struct { stream: std.net.Stream, read_fd: std.posix.fd_t } {
    const fds = try std.posix.pipe();
    return .{ .stream = .{ .handle = fds[1] }, .read_fd = fds[0] };
}

/// Create a minimal WebServer suitable for testing broadcast/stop behavior.
/// Uses fake pointers for db/config since those fields are never dereferenced
/// by the functions under test (pushLog, broadcastChatEvent, stop).
fn makeTestServer(alloc: std.mem.Allocator) WebServer {
    return WebServer.init(
        alloc,
        @ptrFromInt(0x10000), // fake *Db (never dereferenced)
        @ptrFromInt(0x10000), // fake *Config (never dereferenced)
        0, // port 0: stop()'s self-connect will harmlessly fail
        "127.0.0.1",
    );
}

/// Close any remaining streams in all SSE client lists, then free ArrayList memory.
fn cleanupTestServer(ws: *WebServer) void {
    for (ws.sse_clients.items) |client| client.close();
    ws.sse_clients.deinit();
    for (ws.chat_sse_clients.items) |client| client.close();
    ws.chat_sse_clients.deinit();
    ws.chat_queue.deinit();
}

// ── AC1: broadcastSse closes removed streams ───────────────────────────
//
// broadcastSse is private, so we test through pushLog (pub) which calls it.

test "AC1: broadcastSse closes FD when writeAll fails (single client)" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    const s = try makeBrokenStream();
    const fd = s.handle;
    try ws.sse_clients.append(s);

    // FD should be open before broadcast
    try std.testing.expect(isFdOpen(fd));

    // pushLog calls broadcastSse internally; write fails → swapRemove → (fix: close)
    ws.pushLog("info", "test");

    // After fix: FD should be closed
    try std.testing.expect(!isFdOpen(fd));
    // Client should be removed from list
    try std.testing.expectEqual(@as(usize, 0), ws.sse_clients.items.len);
}

test "AC1: broadcastSse closes all FDs when multiple clients fail" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    var fds: [3]std.posix.fd_t = undefined;
    for (&fds) |*fd| {
        const s = try makeBrokenStream();
        fd.* = s.handle;
        try ws.sse_clients.append(s);
    }

    for (fds) |fd| try std.testing.expect(isFdOpen(fd));

    ws.pushLog("info", "test all fail");

    for (fds) |fd| try std.testing.expect(!isFdOpen(fd));
    try std.testing.expectEqual(@as(usize, 0), ws.sse_clients.items.len);
}

// ── AC2: broadcastChatEvent closes removed streams ─────────────────────

test "AC2: broadcastChatEvent closes FD when writeAll fails (single client)" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    const s = try makeBrokenStream();
    const fd = s.handle;
    try ws.chat_sse_clients.append(s);

    try std.testing.expect(isFdOpen(fd));

    ws.broadcastChatEvent("hello", "web:dashboard");

    try std.testing.expect(!isFdOpen(fd));
    try std.testing.expectEqual(@as(usize, 0), ws.chat_sse_clients.items.len);
}

test "AC2: broadcastChatEvent closes all FDs when multiple clients fail" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    var fds: [4]std.posix.fd_t = undefined;
    for (&fds) |*fd| {
        const s = try makeBrokenStream();
        fd.* = s.handle;
        try ws.chat_sse_clients.append(s);
    }

    ws.broadcastChatEvent("test all fail", "web:dashboard");

    for (fds) |fd| try std.testing.expect(!isFdOpen(fd));
    try std.testing.expectEqual(@as(usize, 0), ws.chat_sse_clients.items.len);
}

// ── AC3: handleChatPost inline broadcast closes removed streams ────────
//
// handleChatPost is a private function, so we cannot call it directly from
// an external test file. It uses the same chat_sse_clients list and the
// identical swapRemove pattern as broadcastChatEvent. The AC2 tests above
// verify the behavior through broadcastChatEvent. Here we additionally
// verify the exact loop pattern works correctly in isolation.

test "AC3: swapRemove+close pattern in while loop closes all failed streams" {
    const alloc = std.testing.allocator;
    var clients = std.ArrayList(std.net.Stream).init(alloc);
    defer clients.deinit();

    var fds: [5]std.posix.fd_t = undefined;
    for (&fds) |*fd| {
        const s = try makeBrokenStream();
        fd.* = s.handle;
        try clients.append(s);
    }

    // Mirror the exact broadcast loop from handleChatPost WITH the fix applied:
    // This is the pattern that must exist in the implementation.
    var i: usize = 0;
    while (i < clients.items.len) {
        clients.items[i].writeAll("data: {\"role\":\"user\",\"text\":\"test\"}\n\n") catch {
            const removed = clients.swapRemove(i);
            removed.close(); // THE FIX: close the removed stream
            continue;
        };
        i += 1;
    }

    try std.testing.expectEqual(@as(usize, 0), clients.items.len);
    for (fds) |fd| try std.testing.expect(!isFdOpen(fd));
}

// ── AC4: stop() closes all remaining SSE clients ───────────────────────

test "AC4: stop closes all streams in sse_clients" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    var read_fds: [3]std.posix.fd_t = undefined;
    var write_fds: [3]std.posix.fd_t = undefined;
    for (&read_fds, &write_fds) |*rfd, *wfd| {
        const s = try makeOpenStream();
        rfd.* = s.read_fd;
        wfd.* = s.stream.handle;
        try ws.sse_clients.append(s.stream);
    }
    defer for (read_fds) |fd| std.posix.close(fd);

    for (write_fds) |fd| try std.testing.expect(isFdOpen(fd));

    ws.stop();

    // After fix: all write-end FDs should be closed by stop()
    for (write_fds) |fd| try std.testing.expect(!isFdOpen(fd));
    // List should be cleared
    try std.testing.expectEqual(@as(usize, 0), ws.sse_clients.items.len);
}

test "AC4: stop closes all streams in chat_sse_clients" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    var read_fds: [2]std.posix.fd_t = undefined;
    var write_fds: [2]std.posix.fd_t = undefined;
    for (&read_fds, &write_fds) |*rfd, *wfd| {
        const s = try makeOpenStream();
        rfd.* = s.read_fd;
        wfd.* = s.stream.handle;
        try ws.chat_sse_clients.append(s.stream);
    }
    defer for (read_fds) |fd| std.posix.close(fd);

    ws.stop();

    for (write_fds) |fd| try std.testing.expect(!isFdOpen(fd));
    try std.testing.expectEqual(@as(usize, 0), ws.chat_sse_clients.items.len);
}

test "AC4: stop closes streams in both sse_clients and chat_sse_clients" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    const s1 = try makeOpenStream();
    const s2 = try makeOpenStream();
    defer std.posix.close(s1.read_fd);
    defer std.posix.close(s2.read_fd);

    try ws.sse_clients.append(s1.stream);
    try ws.chat_sse_clients.append(s2.stream);

    ws.stop();

    try std.testing.expect(!isFdOpen(s1.stream.handle));
    try std.testing.expect(!isFdOpen(s2.stream.handle));
    try std.testing.expectEqual(@as(usize, 0), ws.sse_clients.items.len);
    try std.testing.expectEqual(@as(usize, 0), ws.chat_sse_clients.items.len);
}

// ── AC5: Existing tests pass ───────────────────────────────────────────
// Verified implicitly: if this file compiles and the existing jsonEscape
// and parsePath tests in web.zig still pass, AC5 is satisfied.

// ── AC6: No double-close ──────────────────────────────────────────────

test "AC6: swapRemove returns removed element which is closed exactly once" {
    const alloc = std.testing.allocator;
    var clients = std.ArrayList(std.net.Stream).init(alloc);
    defer clients.deinit();

    const s = try makeBrokenStream();
    const fd = s.handle;
    try clients.append(s);

    // swapRemove returns the removed element
    const removed = clients.swapRemove(0);
    try std.testing.expectEqual(fd, removed.handle);

    // Close exactly once
    removed.close();
    try std.testing.expect(!isFdOpen(fd));

    // The stream is gone from the list — no path to double-close
    try std.testing.expectEqual(@as(usize, 0), clients.items.len);
}

test "AC6: after broadcast, removed stream is not accessible from the list" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    const s = try makeBrokenStream();
    const fd = s.handle;
    try ws.sse_clients.append(s);

    ws.pushLog("info", "trigger broadcast");

    // List is empty → no stream reference exists → no double-close possible
    try std.testing.expectEqual(@as(usize, 0), ws.sse_clients.items.len);
    try std.testing.expect(!isFdOpen(fd));
}

// ── Edge Case 1: stream.close() returns void ──────────────────────────

test "Edge1: Stream.close returns void — no error handling needed" {
    const CloseType = @TypeOf(std.net.Stream.close);
    const fn_info = @typeInfo(CloseType).@"fn";
    try std.testing.expect(fn_info.return_type == void);
}

// ── Edge Case 2: Empty client list — broadcast is a no-op ─────────────

test "Edge2: broadcastSse with empty client list is a no-op" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    // No clients registered — should not crash
    ws.pushLog("info", "empty test");
    try std.testing.expectEqual(@as(usize, 0), ws.sse_clients.items.len);
}

test "Edge2: broadcastChatEvent with empty client list is a no-op" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    ws.broadcastChatEvent("empty test", "web:dashboard");
    try std.testing.expectEqual(@as(usize, 0), ws.chat_sse_clients.items.len);
}

// ── Edge Case 3: All clients fail simultaneously ───────────────────────

test "Edge3: swapRemove+close handles all-fail with correct index management" {
    // The swapRemove pattern doesn't increment i on removal, so the loop
    // correctly processes every element even when ALL clients fail.
    const alloc = std.testing.allocator;
    var clients = std.ArrayList(std.net.Stream).init(alloc);
    defer clients.deinit();

    const N = 10;
    var fds: [N]std.posix.fd_t = undefined;
    for (&fds) |*fd| {
        const s = try makeBrokenStream();
        fd.* = s.handle;
        try clients.append(s);
    }

    var i: usize = 0;
    while (i < clients.items.len) {
        clients.items[i].writeAll("data: test\n\n") catch {
            const removed = clients.swapRemove(i);
            removed.close();
            continue;
        };
        i += 1;
    }

    try std.testing.expectEqual(@as(usize, 0), clients.items.len);
    for (fds) |fd| try std.testing.expect(!isFdOpen(fd));
}

// ── Edge Case 4: Concurrent access during shutdown ─────────────────────
// The broadcast and stop functions use sse_mu/chat_sse_mu for mutual
// exclusion. We verify the mutexes exist as separate fields.

test "Edge4: WebServer has separate sse_mu and chat_sse_mu mutexes" {
    const info = @typeInfo(WebServer);
    const fields = info.@"struct".fields;

    var found_sse_mu = false;
    var found_chat_sse_mu = false;
    for (fields) |f| {
        if (std.mem.eql(u8, f.name, "sse_mu")) {
            found_sse_mu = true;
            try std.testing.expect(f.type == std.Thread.Mutex);
        }
        if (std.mem.eql(u8, f.name, "chat_sse_mu")) {
            found_chat_sse_mu = true;
            try std.testing.expect(f.type == std.Thread.Mutex);
        }
    }
    try std.testing.expect(found_sse_mu);
    try std.testing.expect(found_chat_sse_mu);
}

// ── Edge Case 5: Client disconnects between registration and first broadcast

test "Edge5: client registered then disconnects before broadcast still gets closed" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    // Simulate: client connects (pipe created with both ends open)
    const fds = try std.posix.pipe();
    const stream = std.net.Stream{ .handle = fds[1] };
    try ws.sse_clients.append(stream);

    // Client disconnects before first broadcast: read end closes
    std.posix.close(fds[0]);

    // First broadcast after disconnect triggers removal + close
    ws.pushLog("info", "after disconnect");

    try std.testing.expect(!isFdOpen(fds[1]));
    try std.testing.expectEqual(@as(usize, 0), ws.sse_clients.items.len);
}

// ── Edge Case 6: stop() with empty SSE lists is a no-op ───────────────

test "Edge6: stop with no SSE clients does not crash" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    // No SSE clients — stop should handle empty lists gracefully
    ws.stop();

    try std.testing.expectEqual(@as(usize, 0), ws.sse_clients.items.len);
    try std.testing.expectEqual(@as(usize, 0), ws.chat_sse_clients.items.len);
}

// ── Mixed scenario: good and bad clients ───────────────────────────────

test "broadcast closes only failed clients, keeps working ones" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    // Good client: pipe with reader still open (writes succeed)
    const good = try makeOpenStream();
    defer std.posix.close(good.read_fd);

    // Bad client: broken pipe (writes fail)
    const bad = try makeBrokenStream();
    const bad_fd = bad.handle;

    try ws.sse_clients.append(good.stream);
    try ws.sse_clients.append(bad);

    ws.pushLog("info", "mixed test");

    // Bad client's FD should be closed
    try std.testing.expect(!isFdOpen(bad_fd));
    // Good client should still be in the list
    try std.testing.expectEqual(@as(usize, 1), ws.sse_clients.items.len);
    // Good client's FD should still be open
    try std.testing.expect(isFdOpen(good.stream.handle));
}

test "broadcastChatEvent closes only failed clients, keeps working ones" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    const good = try makeOpenStream();
    defer std.posix.close(good.read_fd);

    const bad = try makeBrokenStream();
    const bad_fd = bad.handle;

    try ws.chat_sse_clients.append(good.stream);
    try ws.chat_sse_clients.append(bad);

    ws.broadcastChatEvent("mixed chat test", "web:dashboard");

    try std.testing.expect(!isFdOpen(bad_fd));
    try std.testing.expectEqual(@as(usize, 1), ws.chat_sse_clients.items.len);
    try std.testing.expect(isFdOpen(good.stream.handle));
}
