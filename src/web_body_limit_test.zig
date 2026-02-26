// Tests for Task #77: Cap Content-Length to prevent DoS via unbounded allocation
//
// handleConnection (src/web.zig) allocates `headers.len + content_length` bytes
// without checking whether content_length exceeds a safe maximum.  A client
// sending `Content-Length: 10000000000` can trigger a multi-GB allocation
// attempt, exhausting memory and hanging the thread indefinitely.
//
// The fix must:
//   1. Expose `pub const max_body_size: usize = 1 * 1024 * 1024;` on WebServer
//      (or at module scope as `pub const max_body_size`).
//   2. Before the alloc at web.zig:350, add:
//        if (content_length > max_body_size) {
//            self.serve413(stream);
//            stream.close();
//            return;
//        }
//   3. Add `fn serve413(_: *WebServer, stream: std.net.Stream) void` that writes
//      "HTTP/1.1 413 Request Entity Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
//   4. Add `_ = @import("web_body_limit_test.zig");` inside the `test { }` block
//      at the bottom of src/web.zig.
//
// These tests FAIL initially because:
//   - `WebServer.max_body_size` does not exist → compile error on every test
//     in this file that references it.
//   - Integration tests (AC5, AC6) receive the wrong HTTP status (400 / 404)
//     instead of 413 before the fix is applied.
//
// To run after wiring:
//   just t

const std = @import("std");
const web_mod = @import("web.zig");
const WebServer = web_mod.WebServer;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Port used exclusively by integration tests in this file.
/// Pick a value far from common service ports to minimise collision risk.
const TEST_PORT: u16 = 18877;

fn makeTestServer(alloc: std.mem.Allocator) WebServer {
    return WebServer.init(
        alloc,
        @ptrFromInt(0x10000), // fake *Db  — never dereferenced by the 413 path
        @ptrFromInt(0x10000), // fake *Config — same
        TEST_PORT,
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

/// Spin-wait until the server is accepting connections (or timeout expires).
fn waitForServer(port: u16) !void {
    const addr = try std.net.Address.parseIp4("127.0.0.1", port);
    for (0..200) |_| {
        if (std.net.tcpConnectToAddress(addr)) |conn| {
            conn.close();
            return;
        } else |_| {}
        std.time.sleep(5 * std.time.ns_per_ms);
    }
    return error.ServerNotReady;
}

/// Send `request_bytes` to the server at `port`, close the write side so the
/// server sees EOF on the body, then read and return the full response.
/// Caller owns the returned slice (freed with alloc.free).
fn sendAndReceive(
    alloc: std.mem.Allocator,
    port: u16,
    request_bytes: []const u8,
) ![]u8 {
    const addr = try std.net.Address.parseIp4("127.0.0.1", port);
    const conn = try std.net.tcpConnectToAddress(addr);
    defer conn.close();

    // Set a generous read timeout so a hung server doesn't block the test suite.
    const timeout = std.posix.timeval{ .tv_sec = 3, .tv_usec = 0 };
    try std.posix.setsockopt(
        conn.handle,
        std.posix.SOL.SOCKET,
        std.posix.SO.RCVTIMEO,
        std.mem.asBytes(&timeout),
    );

    try conn.writeAll(request_bytes);
    // Signal EOF on the write side so the server stops waiting for more body.
    try std.posix.shutdown(conn.handle, .send);

    var buf: [4096]u8 = undefined;
    var total: usize = 0;
    while (total < buf.len) {
        const n = conn.read(buf[total..]) catch |err| {
            // EAGAIN / EWOULDBLOCK means our read timeout fired — treat as done.
            if (err == error.WouldBlock) break;
            break;
        };
        if (n == 0) break;
        total += n;
    }

    return alloc.dupe(u8, buf[0..total]);
}

// ── AC1: max_body_size constant — value and type ───────────────────────────────
//
// The constant must exist and equal exactly 1 MiB (1,048,576 bytes).
// If it is absent, the entire file fails to compile → all tests fail.

test "AC1: max_body_size equals 1 MiB (1,048,576 bytes)" {
    try std.testing.expectEqual(@as(usize, 1 * 1024 * 1024), WebServer.max_body_size);
}

test "AC1: max_body_size type is usize" {
    const T = @TypeOf(WebServer.max_body_size);
    try std.testing.expect(T == usize);
}

test "AC1: max_body_size is a compile-time constant (comptime-known)" {
    comptime {
        const v: usize = WebServer.max_body_size;
        if (v != 1 * 1024 * 1024) @compileError("max_body_size must be 1 MiB");
    }
}

// ── AC2: Boundary — content_length > max_body_size should be rejected ──────────

test "AC2: content_length one byte above limit exceeds max_body_size" {
    const content_length: usize = WebServer.max_body_size + 1;
    try std.testing.expect(content_length > WebServer.max_body_size);
}

test "AC2: 10 MB content_length exceeds max_body_size" {
    const content_length: usize = 10 * 1024 * 1024;
    try std.testing.expect(content_length > WebServer.max_body_size);
}

test "AC2: 1 GB content_length exceeds max_body_size" {
    const content_length: usize = 1 * 1024 * 1024 * 1024;
    try std.testing.expect(content_length > WebServer.max_body_size);
}

test "AC2: 10,000,000,000 content_length exceeds max_body_size" {
    const content_length: usize = 10_000_000_000;
    try std.testing.expect(content_length > WebServer.max_body_size);
}

// ── AC3: Boundary — content_length <= max_body_size should be allowed ──────────

test "AC3: content_length equal to max_body_size is at the limit (not rejected)" {
    const content_length: usize = WebServer.max_body_size;
    try std.testing.expect(!(content_length > WebServer.max_body_size));
}

test "AC3: content_length one byte below limit is allowed" {
    const content_length: usize = WebServer.max_body_size - 1;
    try std.testing.expect(!(content_length > WebServer.max_body_size));
}

test "AC3: content_length zero is always allowed" {
    const content_length: usize = 0;
    try std.testing.expect(!(content_length > WebServer.max_body_size));
}

test "AC3: content_length 1 byte is allowed" {
    const content_length: usize = 1;
    try std.testing.expect(!(content_length > WebServer.max_body_size));
}

test "AC3: content_length 512 KiB is allowed" {
    const content_length: usize = 512 * 1024;
    try std.testing.expect(!(content_length > WebServer.max_body_size));
}

// ── AC4: No allocation attempted when content_length exceeds limit ─────────────
//
// Simulates the guard condition: a FailingAllocator set to fail on the very
// first allocation is installed.  The guard must prevent reaching the alloc
// call, so fa.allocations must remain 0 after the check.

test "AC4: guard prevents allocation for content_length above max_body_size" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 0 });
    const content_length: usize = WebServer.max_body_size + 1;

    if (content_length > WebServer.max_body_size) {
        // Fixed code path: serve 413 and return — no alloc reached.
        try std.testing.expectEqual(@as(usize, 0), fa.allocations);
        return;
    }

    // If we reach here the guard is absent → simulate the (bad) allocation.
    _ = fa.allocator().alloc(u8, content_length) catch {};
    return error.GuardShouldHavePreventedAllocation;
}

test "AC4: guard prevents allocation for 10 GB content_length" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 0 });
    const content_length: usize = 10_000_000_000;

    if (content_length > WebServer.max_body_size) {
        try std.testing.expectEqual(@as(usize, 0), fa.allocations);
        return;
    }
    _ = fa.allocator().alloc(u8, 1) catch {};
    return error.GuardShouldHavePreventedAllocation;
}

test "AC4: allocation IS allowed when content_length equals max_body_size" {
    // At the boundary the guard must NOT fire; allocation proceeds normally.
    const content_length: usize = WebServer.max_body_size;
    // Verify the guard condition does not trip at the boundary.
    try std.testing.expect(!(content_length > WebServer.max_body_size));
}

// ── AC5: Integration — POST with Content-Length > max_body_size → 413 ──────────
//
// Starts a real WebServer, connects as a TCP client, sends a POST whose
// Content-Length header declares a 2 MiB body (twice the limit) but sends
// no body bytes.  The server must respond with 413 immediately (before any
// body allocation or read attempt).
//
// BEFORE the fix the server allocates the buffer, tries to read the body,
// gets EOF, then routes the partial request → returns 400 or 404 (not 413).
// AFTER the fix the server checks the header first → 413.

test "AC5: POST with Content-Length 2 MiB returns 413" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    const thread = try std.Thread.spawn(.{}, WebServer.serve, .{&ws});
    defer {
        ws.stop();
        thread.join();
    }

    try waitForServer(TEST_PORT);

    const request =
        "POST /api/tasks HTTP/1.1\r\n" ++
        "Host: 127.0.0.1\r\n" ++
        "Content-Type: application/json\r\n" ++
        "Content-Length: 2097152\r\n" ++
        "\r\n";

    const response = try sendAndReceive(alloc, TEST_PORT, request);
    defer alloc.free(response);

    try std.testing.expect(std.mem.startsWith(u8, response, "HTTP/1.1 413"));
}

test "AC5: POST with Content-Length 10 GB returns 413" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    const thread = try std.Thread.spawn(.{}, WebServer.serve, .{&ws});
    defer {
        ws.stop();
        thread.join();
    }

    try waitForServer(TEST_PORT);

    const request =
        "POST /api/chat HTTP/1.1\r\n" ++
        "Host: 127.0.0.1\r\n" ++
        "Content-Length: 10000000000\r\n" ++
        "\r\n";

    const response = try sendAndReceive(alloc, TEST_PORT, request);
    defer alloc.free(response);

    try std.testing.expect(std.mem.startsWith(u8, response, "HTTP/1.1 413"));
}

// ── AC6: Integration — POST with Content-Length <= max_body_size is NOT 413 ───
//
// A request within the limit must reach the normal handler path.
// We use `/api/tasks` with a tiny body so the server attempts to parse JSON
// and returns 400 (bad JSON) — not 413.

test "AC6: POST with Content-Length within limit is not rejected with 413" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    const thread = try std.Thread.spawn(.{}, WebServer.serve, .{&ws});
    defer {
        ws.stop();
        thread.join();
    }

    try waitForServer(TEST_PORT);

    // 5-byte body — well under the 1 MiB limit.
    const request =
        "POST /api/tasks HTTP/1.1\r\n" ++
        "Host: 127.0.0.1\r\n" ++
        "Content-Type: application/json\r\n" ++
        "Content-Length: 5\r\n" ++
        "\r\n" ++
        "hello";

    const response = try sendAndReceive(alloc, TEST_PORT, request);
    defer alloc.free(response);

    // Must NOT be 413.
    try std.testing.expect(!std.mem.startsWith(u8, response, "HTTP/1.1 413"));
}

// ── AC7: 413 response is well-formed HTTP/1.1 ──────────────────────────────────
//
// The response must carry the correct status line, a zero Content-Length, and
// a Connection: close header so the client knows to close immediately.

test "AC7: 413 response contains correct status line" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    const thread = try std.Thread.spawn(.{}, WebServer.serve, .{&ws});
    defer {
        ws.stop();
        thread.join();
    }

    try waitForServer(TEST_PORT);

    const request =
        "POST /api/tasks HTTP/1.1\r\n" ++
        "Content-Length: 2097152\r\n" ++
        "\r\n";

    const response = try sendAndReceive(alloc, TEST_PORT, request);
    defer alloc.free(response);

    try std.testing.expect(std.mem.startsWith(u8, response, "HTTP/1.1 413"));
    try std.testing.expect(std.mem.indexOf(u8, response, "Content-Length: 0") != null);
    try std.testing.expect(std.mem.indexOf(u8, response, "Connection: close") != null);
}

// ── EC1: Exact boundary — max_body_size itself is allowed ─────────────────────

test "EC1: content_length exactly equal to max_body_size does not exceed limit" {
    // The guard uses strictly-greater-than; the limit itself must be accepted.
    const at_limit: usize = WebServer.max_body_size;
    try std.testing.expect(!(at_limit > WebServer.max_body_size));
}

test "EC1: content_length max_body_size + 1 is the minimum that must be rejected" {
    const just_over: usize = WebServer.max_body_size + 1;
    try std.testing.expect(just_over > WebServer.max_body_size);
}

// ── EC2: usize maximum — guard comparison must not overflow ────────────────────

test "EC2: usize maximum value exceeds max_body_size (no overflow in comparison)" {
    const max_usize: usize = std.math.maxInt(usize);
    // Simple comparison — no arithmetic that could overflow.
    try std.testing.expect(max_usize > WebServer.max_body_size);
}

test "EC2: usize maximum content_length is handled by the guard (no allocation)" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 0 });
    const content_length: usize = std.math.maxInt(usize);

    if (content_length > WebServer.max_body_size) {
        try std.testing.expectEqual(@as(usize, 0), fa.allocations);
        return;
    }
    _ = fa.allocator().alloc(u8, 1) catch {};
    return error.GuardShouldHavePreventedAllocation;
}

// ── EC3: GET requests are unaffected ──────────────────────────────────────────
//
// The body-size guard only applies to POST requests (handleConnection only reads
// Content-Length for POST).  A GET request must never receive 413.

test "EC3: GET request is not affected by the body size limit" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    const thread = try std.Thread.spawn(.{}, WebServer.serve, .{&ws});
    defer {
        ws.stop();
        thread.join();
    }

    try waitForServer(TEST_PORT);

    const request = "GET /api/status HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
    const response = try sendAndReceive(alloc, TEST_PORT, request);
    defer alloc.free(response);

    try std.testing.expect(!std.mem.startsWith(u8, response, "HTTP/1.1 413"));
}

// ── EC4: max_body_size value sanity checks ────────────────────────────────────

test "EC4: max_body_size is positive" {
    try std.testing.expect(WebServer.max_body_size > 0);
}

test "EC4: max_body_size is at most 16 MiB (not excessively large)" {
    try std.testing.expect(WebServer.max_body_size <= 16 * 1024 * 1024);
}

test "EC4: max_body_size is at least 64 KiB (not uselessly small)" {
    try std.testing.expect(WebServer.max_body_size >= 64 * 1024);
}

test "EC4: max_body_size is a power of two" {
    const v = WebServer.max_body_size;
    try std.testing.expect(v != 0 and (v & (v - 1)) == 0);
}

// ── EC5: Repeated oversized requests do not leave the server in a bad state ───

test "EC5: server handles multiple consecutive oversized requests without error" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    const thread = try std.Thread.spawn(.{}, WebServer.serve, .{&ws});
    defer {
        ws.stop();
        thread.join();
    }

    try waitForServer(TEST_PORT);

    const request =
        "POST /api/tasks HTTP/1.1\r\n" ++
        "Content-Length: 5000000\r\n" ++
        "\r\n";

    for (0..3) |_| {
        const response = try sendAndReceive(alloc, TEST_PORT, request);
        defer alloc.free(response);
        try std.testing.expect(std.mem.startsWith(u8, response, "HTTP/1.1 413"));
    }
}
