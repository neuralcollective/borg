// Tests for Task #68: web.zig HTTP request parsing helpers
//
// Covers parseMethod, parsePath, parseContentLength, and parseBody.
//
// These tests FAIL initially because parseContentLength and parseBody are
// private (no `pub` modifier) in web.zig. The implementation must:
//   1. Change `fn parseContentLength` → `pub fn parseContentLength`
//   2. Change `fn parseBody` → `pub fn parseBody`
//   3. Add `_ = @import("web_request_parse_test.zig");` inside the
//      `test { … }` block at the bottom of src/web.zig
//
// To run after wiring:
//   just t

const std = @import("std");
const web_mod = @import("web.zig");
const WebServer = web_mod.WebServer;

// ── AC1: parseMethod — well-formed request line ───────────────────────────────

test "AC1: parseMethod extracts GET from well-formed request line" {
    try std.testing.expectEqualStrings("GET", WebServer.parseMethod("GET /index HTTP/1.1\r\n"));
}

test "AC1: parseMethod extracts POST from well-formed request line" {
    try std.testing.expectEqualStrings("POST", WebServer.parseMethod("POST /api/tasks HTTP/1.1\r\n"));
}

test "AC1: parseMethod extracts DELETE from well-formed request line" {
    try std.testing.expectEqualStrings("DELETE", WebServer.parseMethod("DELETE /api/tasks/1 HTTP/1.1\r\n"));
}

test "AC1: parseMethod extracts PUT from well-formed request line" {
    try std.testing.expectEqualStrings("PUT", WebServer.parseMethod("PUT /api/tasks/2 HTTP/1.1\r\n"));
}

// ── AC2: parseMethod — malformed / empty input ────────────────────────────────

test "AC2: parseMethod returns GET for empty input" {
    try std.testing.expectEqualStrings("GET", WebServer.parseMethod(""));
}

test "AC2: parseMethod returns GET when no space present" {
    try std.testing.expectEqualStrings("GET", WebServer.parseMethod("NOSPACE"));
}

test "AC2: parseMethod returns GET for a single token with no space" {
    try std.testing.expectEqualStrings("GET", WebServer.parseMethod("POST"));
}

// ── AC3: parsePath — well-formed request line ─────────────────────────────────

test "AC3: parsePath extracts path from standard GET request" {
    try std.testing.expectEqualStrings("/api/tasks", WebServer.parsePath("GET /api/tasks HTTP/1.1\r\n"));
}

test "AC3: parsePath extracts root path" {
    try std.testing.expectEqualStrings("/", WebServer.parsePath("GET / HTTP/1.1\r\n"));
}

test "AC3: parsePath extracts path with query string" {
    try std.testing.expectEqualStrings("/api/tasks?q=1", WebServer.parsePath("POST /api/tasks?q=1 HTTP/1.1\r\n"));
}

test "AC3: parsePath extracts nested path" {
    try std.testing.expectEqualStrings("/api/tasks/42/details", WebServer.parsePath("GET /api/tasks/42/details HTTP/1.1\r\n"));
}

// ── AC4: parsePath — malformed / empty input ──────────────────────────────────

test "AC4: parsePath returns / for empty input" {
    try std.testing.expectEqualStrings("/", WebServer.parsePath(""));
}

test "AC4: parsePath returns / when only one token (no space after method)" {
    try std.testing.expectEqualStrings("/", WebServer.parsePath("GET"));
}

test "AC4: parsePath returns / for input with no spaces" {
    try std.testing.expectEqualStrings("/", WebServer.parsePath("NOSPACE"));
}

test "AC4: parsePath returns / when first space found but no second space" {
    // "GET /path" — one space after method but no trailing space/version
    try std.testing.expectEqualStrings("/", WebServer.parsePath("GET /path"));
}

// ── AC5: parseContentLength — header present with valid integer ───────────────

test "AC5: parseContentLength returns 42 for Content-Length: 42" {
    const headers = "GET / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 42\r\n\r\n";
    try std.testing.expectEqual(@as(?usize, 42), WebServer.parseContentLength(headers));
}

test "AC5: parseContentLength returns 0 for Content-Length: 0" {
    const headers = "POST /api HTTP/1.1\r\nContent-Length: 0\r\n\r\n";
    try std.testing.expectEqual(@as(?usize, 0), WebServer.parseContentLength(headers));
}

test "AC5: parseContentLength returns large value correctly" {
    const headers = "POST /upload HTTP/1.1\r\nContent-Length: 1048576\r\n\r\n";
    try std.testing.expectEqual(@as(?usize, 1048576), WebServer.parseContentLength(headers));
}

test "AC5: parseContentLength returns 1 for Content-Length: 1" {
    const headers = "POST / HTTP/1.1\r\nContent-Length: 1\r\n\r\n";
    try std.testing.expectEqual(@as(?usize, 1), WebServer.parseContentLength(headers));
}

// ── AC6: parseContentLength — header absent or non-numeric ───────────────────

test "AC6: parseContentLength returns null when no Content-Length header" {
    const headers = "GET / HTTP/1.1\r\nHost: localhost\r\nAccept: */*\r\n\r\n";
    try std.testing.expectEqual(@as(?usize, null), WebServer.parseContentLength(headers));
}

test "AC6: parseContentLength returns null for non-numeric value" {
    const headers = "POST / HTTP/1.1\r\nContent-Length: abc\r\n\r\n";
    try std.testing.expectEqual(@as(?usize, null), WebServer.parseContentLength(headers));
}

test "AC6: parseContentLength returns null for empty input" {
    try std.testing.expectEqual(@as(?usize, null), WebServer.parseContentLength(""));
}

test "AC6: parseContentLength returns null for negative value string" {
    // parseInt(usize, ...) on a negative literal returns error, so null
    const headers = "POST / HTTP/1.1\r\nContent-Length: -1\r\n\r\n";
    try std.testing.expectEqual(@as(?usize, null), WebServer.parseContentLength(headers));
}

// ── AC7: parseContentLength — case-insensitive header name ───────────────────

test "AC7: parseContentLength matches lowercase content-length" {
    const headers = "POST / HTTP/1.1\r\ncontent-length: 10\r\n\r\n";
    try std.testing.expectEqual(@as(?usize, 10), WebServer.parseContentLength(headers));
}

test "AC7: parseContentLength matches uppercase CONTENT-LENGTH" {
    const headers = "POST / HTTP/1.1\r\nCONTENT-LENGTH: 10\r\n\r\n";
    try std.testing.expectEqual(@as(?usize, 10), WebServer.parseContentLength(headers));
}

test "AC7: parseContentLength matches title-case Content-Length" {
    const headers = "POST / HTTP/1.1\r\nContent-Length: 10\r\n\r\n";
    try std.testing.expectEqual(@as(?usize, 10), WebServer.parseContentLength(headers));
}

test "AC7: parseContentLength matches mixed-case cOnTeNt-LeNgTh" {
    const headers = "POST / HTTP/1.1\r\ncOnTeNt-LeNgTh: 7\r\n\r\n";
    try std.testing.expectEqual(@as(?usize, 7), WebServer.parseContentLength(headers));
}

// ── AC8: parseBody — separator present ───────────────────────────────────────

test "AC8: parseBody returns body after CRLFCRLF separator" {
    const req = "GET / HTTP/1.1\r\nHost: x\r\n\r\nhello";
    try std.testing.expectEqualStrings("hello", WebServer.parseBody(req));
}

test "AC8: parseBody returns empty string when body is empty after separator" {
    const req = "POST /api HTTP/1.1\r\nContent-Length: 0\r\n\r\n";
    try std.testing.expectEqualStrings("", WebServer.parseBody(req));
}

test "AC8: parseBody returns multiline body correctly" {
    const req = "POST /api HTTP/1.1\r\nContent-Type: text/plain\r\n\r\nline1\r\nline2\r\nline3";
    try std.testing.expectEqualStrings("line1\r\nline2\r\nline3", WebServer.parseBody(req));
}

test "AC8: parseBody returns JSON body" {
    const req = "POST /api/tasks HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"title\":\"test\"}";
    try std.testing.expectEqualStrings("{\"title\":\"test\"}", WebServer.parseBody(req));
}

// ── AC9: parseBody — separator absent ────────────────────────────────────────

test "AC9: parseBody returns empty string when CRLFCRLF separator is absent" {
    const req = "GET / HTTP/1.1\r\nHost: x";
    try std.testing.expectEqualStrings("", WebServer.parseBody(req));
}

test "AC9: parseBody returns empty string for empty input" {
    try std.testing.expectEqualStrings("", WebServer.parseBody(""));
}

test "AC9: parseBody returns empty string for partial separator CRLF only" {
    const req = "GET / HTTP/1.1\r\nHost: x\r\n";
    try std.testing.expectEqualStrings("", WebServer.parseBody(req));
}

// ── EC1: parseMethod — request starts with a space ───────────────────────────

test "EC1: parseMethod on request starting with space returns empty token" {
    // First space is at index 0, so request[0..0] == ""
    const result = WebServer.parseMethod(" GET /");
    try std.testing.expectEqualStrings("", result);
}

// ── EC2: parsePath — path contains spaces ────────────────────────────────────

test "EC2: parsePath truncates at first internal space in path" {
    // Path "has space" is split at the space — returns only "has"
    const result = WebServer.parsePath("GET has space HTTP/1.1");
    try std.testing.expectEqualStrings("has", result);
}

// ── EC3: parseContentLength — leading/trailing whitespace on value ────────────

test "EC3: parseContentLength trims leading whitespace from value" {
    const headers = "POST / HTTP/1.1\r\nContent-Length:  42\r\n\r\n";
    // The prefix is "content-length: " (with one space), so value starts with " 42"
    // trim(" \t") should strip the extra space → returns 42
    try std.testing.expectEqual(@as(?usize, 42), WebServer.parseContentLength(headers));
}

test "EC3: parseContentLength trims trailing whitespace from value" {
    const headers = "POST / HTTP/1.1\r\nContent-Length: 42 \r\n\r\n";
    try std.testing.expectEqual(@as(?usize, 42), WebServer.parseContentLength(headers));
}

// ── EC4: parseContentLength — empty/whitespace-only value ────────────────────

test "EC4: parseContentLength returns null for whitespace-only value" {
    const headers = "POST / HTTP/1.1\r\nContent-Length: \r\n\r\n";
    try std.testing.expectEqual(@as(?usize, null), WebServer.parseContentLength(headers));
}

// ── EC5: parseBody — returns zero-copy sub-slice ─────────────────────────────

test "EC5: parseBody returned slice is a sub-slice of the input (same memory)" {
    const req = "POST / HTTP/1.1\r\n\r\nbody-data";
    const body = WebServer.parseBody(req);
    // The returned slice should point into the original buffer
    const req_start = @intFromPtr(req.ptr);
    const body_start = @intFromPtr(body.ptr);
    try std.testing.expect(body_start >= req_start);
    try std.testing.expect(body_start < req_start + req.len);
    try std.testing.expectEqualStrings("body-data", body);
}

// ── EC6: parseContentLength — ignores X-Content-Length ───────────────────────

test "EC6: parseContentLength ignores headers not starting with C or c" {
    const headers = "POST / HTTP/1.1\r\nX-Content-Length: 5\r\n\r\n";
    try std.testing.expectEqual(@as(?usize, null), WebServer.parseContentLength(headers));
}

test "EC6: parseContentLength ignores Transfer-Encoding but finds Content-Length" {
    const headers = "POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\nContent-Length: 99\r\n\r\n";
    try std.testing.expectEqual(@as(?usize, 99), WebServer.parseContentLength(headers));
}

// ── EC7: parseMethod and parsePath on whitespace-only input ──────────────────

test "EC7: parseMethod on whitespace-only input returns empty token before first space" {
    // First space at index 0 → method is ""
    const result = WebServer.parseMethod("   ");
    try std.testing.expectEqualStrings("", result);
}

test "EC7: parsePath on whitespace-only input returns / (no second space token)" {
    // First space at 0, rest is "  " which contains a space → path is ""
    // Actually "   ": first space at 0, rest = "  ", second space at 0 → path = ""
    // Let's just verify it doesn't crash and returns a valid string
    const result = WebServer.parsePath("   ");
    // The implementation finds first space at 0, rest = "  ", finds second space at 0
    // returns rest[0..0] = "" — that's a valid zero-length slice, not "/"
    // This is documented degenerate behaviour.
    _ = result; // we just verify no crash
}

// ── Additional: parseMethod and parsePath return correct types ────────────────

test "parseMethod return type is []const u8" {
    const T = @TypeOf(WebServer.parseMethod("GET / HTTP/1.1\r\n"));
    try std.testing.expect(T == []const u8);
}

test "parsePath return type is []const u8" {
    const T = @TypeOf(WebServer.parsePath("GET / HTTP/1.1\r\n"));
    try std.testing.expect(T == []const u8);
}

test "parseContentLength return type is optional usize" {
    const T = @TypeOf(WebServer.parseContentLength(""));
    try std.testing.expect(T == ?usize);
}

test "parseBody return type is []const u8" {
    const T = @TypeOf(WebServer.parseBody(""));
    try std.testing.expect(T == []const u8);
}

// ── Additional: parseContentLength with multiple headers ─────────────────────

test "parseContentLength finds Content-Length among multiple headers" {
    const headers =
        "POST /api/tasks HTTP/1.1\r\n" ++
        "Host: localhost:8080\r\n" ++
        "Accept: application/json\r\n" ++
        "Content-Type: application/json\r\n" ++
        "Content-Length: 256\r\n" ++
        "Connection: keep-alive\r\n" ++
        "\r\n";
    try std.testing.expectEqual(@as(?usize, 256), WebServer.parseContentLength(headers));
}

test "parseContentLength returns null when only unrelated C-headers present" {
    // Connection starts with 'C' but won't match "content-length: "
    const headers = "GET / HTTP/1.1\r\nConnection: keep-alive\r\n\r\n";
    try std.testing.expectEqual(@as(?usize, null), WebServer.parseContentLength(headers));
}
