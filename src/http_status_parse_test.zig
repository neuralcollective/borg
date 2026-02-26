// Tests for the HTTP status-line bounds check fix (Task #63).
//
// The parsing logic in unixRequest must be extracted into a testable helper:
//
//   pub fn parseStatusLine(first_line: []const u8) std.http.Status
//
// and the bounds guard changed from `> 9` to `>= 12`.
//
// To include in the build, add inside http.zig's test section:
//   _ = @import("http_status_parse_test.zig");
//
// All tests marked AC1–AC5 map directly to the acceptance criteria in spec.md.
// Edge-case tests cover the boundary values and non-numeric input.

const std = @import("std");
const http = @import("http.zig");

// =============================================================================
// AC1: first_line.len == 9 — guard is false, status defaults to .ok
// "HTTP/1.1 " is exactly 9 characters. Neither the old guard (> 9) nor the
// new guard (>= 12) fires, so the default .ok is returned without any slice.
// =============================================================================

test "AC1: first_line len=9 does not panic, returns ok" {
    const line = "HTTP/1.1 "; // exactly 9 bytes
    comptime std.debug.assert(line.len == 9);
    const status = http.parseStatusLine(line);
    try std.testing.expectEqual(std.http.Status.ok, status);
}

// =============================================================================
// AC2: first_line.len == 10 — CRASH with old guard, safe with new guard
// "HTTP/1.1 2" is 10 characters. The old guard `> 9` is true, so it attempts
// first_line[9..12] on a 10-byte slice — out-of-bounds panic.
// The new guard `>= 12` is false, so it falls through to .ok safely.
// =============================================================================

test "AC2: first_line len=10 does not panic, returns ok" {
    const line = "HTTP/1.1 2"; // 10 bytes — triggers crash with old guard
    comptime std.debug.assert(line.len == 10);
    const status = http.parseStatusLine(line);
    try std.testing.expectEqual(std.http.Status.ok, status);
}

// =============================================================================
// AC2 (part 2): first_line.len == 11 — CRASH with old guard, safe with new
// "HTTP/1.1 20" is 11 characters. Same crash vector as len=10.
// =============================================================================

test "AC2: first_line len=11 does not panic, returns ok" {
    const line = "HTTP/1.1 20"; // 11 bytes — triggers crash with old guard
    comptime std.debug.assert(line.len == 11);
    const status = http.parseStatusLine(line);
    try std.testing.expectEqual(std.http.Status.ok, status);
}

// =============================================================================
// AC3: well-formed 200 response is parsed correctly
// "HTTP/1.1 200 OK" — first_line.len >= 12, status code "200" parses to .ok.
// =============================================================================

test "AC3: HTTP/1.1 200 OK parses as .ok" {
    const line = "HTTP/1.1 200 OK";
    comptime std.debug.assert(line.len >= 12);
    const status = http.parseStatusLine(line);
    try std.testing.expectEqual(std.http.Status.ok, status);
}

// =============================================================================
// AC4: 404 response is parsed correctly
// =============================================================================

test "AC4: HTTP/1.1 404 Not Found parses as .not_found" {
    const line = "HTTP/1.1 404 Not Found";
    const status = http.parseStatusLine(line);
    try std.testing.expectEqual(std.http.Status.not_found, status);
}

// =============================================================================
// AC5: 500 response is parsed correctly
// =============================================================================

test "AC5: HTTP/1.1 500 Internal Server Error parses as .internal_server_error" {
    const line = "HTTP/1.1 500 Internal Server Error";
    const status = http.parseStatusLine(line);
    try std.testing.expectEqual(std.http.Status.internal_server_error, status);
}

// =============================================================================
// Edge: first_line.len == 0 — guard is false, status defaults to .ok
// =============================================================================

test "Edge: empty first_line returns ok" {
    const status = http.parseStatusLine("");
    try std.testing.expectEqual(std.http.Status.ok, status);
}

// =============================================================================
// Edge: first_line.len == 12 — minimum valid length "HTTP/1.1 XYZ"
// The new guard `>= 12` accepts this; "XYZ" is non-numeric so parseInt falls
// back to 200, returning .ok.
// =============================================================================

test "Edge: first_line len=12 non-numeric code falls back to ok" {
    const line = "HTTP/1.1 XYZ"; // exactly 12 bytes, non-numeric code
    comptime std.debug.assert(line.len == 12);
    const status = http.parseStatusLine(line);
    try std.testing.expectEqual(std.http.Status.ok, status);
}

// =============================================================================
// Edge: non-numeric status bytes — parseInt catch 200 fires, returns .ok
// =============================================================================

test "Edge: non-numeric status bytes fall back to ok" {
    const line = "HTTP/1.1 abc remainder";
    const status = http.parseStatusLine(line);
    try std.testing.expectEqual(std.http.Status.ok, status);
}

// =============================================================================
// Edge: 201 Created parses correctly
// =============================================================================

test "Edge: HTTP/1.1 201 Created parses as .created" {
    const line = "HTTP/1.1 201 Created";
    const status = http.parseStatusLine(line);
    try std.testing.expectEqual(std.http.Status.created, status);
}

// =============================================================================
// Edge: 204 No Content parses correctly
// =============================================================================

test "Edge: HTTP/1.1 204 No Content parses as .no_content" {
    const line = "HTTP/1.1 204 No Content";
    const status = http.parseStatusLine(line);
    try std.testing.expectEqual(std.http.Status.no_content, status);
}

// =============================================================================
// Edge: 301 Moved Permanently parses correctly
// =============================================================================

test "Edge: HTTP/1.1 301 Moved Permanently parses correctly" {
    const line = "HTTP/1.1 301 Moved Permanently";
    const status = http.parseStatusLine(line);
    try std.testing.expectEqual(std.http.Status.moved_permanently, status);
}

// =============================================================================
// Edge: 401 Unauthorized parses correctly
// =============================================================================

test "Edge: HTTP/1.1 401 Unauthorized parses correctly" {
    const line = "HTTP/1.1 401 Unauthorized";
    const status = http.parseStatusLine(line);
    try std.testing.expectEqual(std.http.Status.unauthorized, status);
}

// =============================================================================
// Edge: first_line with no reason phrase (bare "HTTP/1.1 200")
// len=12, exactly at the boundary — parses "200" correctly.
// =============================================================================

test "Edge: bare HTTP/1.1 200 without reason phrase parses as ok" {
    const line = "HTTP/1.1 200"; // exactly 12 bytes, no reason phrase
    comptime std.debug.assert(line.len == 12);
    const status = http.parseStatusLine(line);
    try std.testing.expectEqual(std.http.Status.ok, status);
}
