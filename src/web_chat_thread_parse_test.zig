// Tests for Task #83: fix query-parameter parsing in serveChatMessages.
//
// The production code in web.zig currently extracts the `thread` query
// parameter with an unbounded slice:
//
//   path[pos + "?thread=".len ..]
//
// which incorrectly includes any subsequent `&`-delimited parameters in
// the thread ID (e.g. "?thread=foo&other=bar" yields "foo&other=bar").
//
// The fix must terminate the slice at the next `&` or end-of-string using
// std.mem.indexOfScalarPos.
//
// To wire these tests into the build:
//   Add `_ = @import("web_chat_thread_parse_test.zig");` inside the
//   `test { … }` block at the bottom of src/web.zig.
//
// To run after wiring:
//   just t

const std = @import("std");

/// Mirror of the corrected extraction expression in serveChatMessages.
/// Tests exercise this helper directly because the production logic is an
/// inline expression rather than an exported function.
fn extractThreadId(path: []const u8) []const u8 {
    if (std.mem.indexOf(u8, path, "?thread=")) |pos| {
        const start = pos + "?thread=".len;
        const end = std.mem.indexOfScalarPos(u8, path, start, '&') orelse path.len;
        return path[start..end];
    }
    return "web:dashboard";
}

// ── AC1: single param ─────────────────────────────────────────────────────────

test "AC1: single thread param returns value only" {
    try std.testing.expectEqualStrings(
        "foo",
        extractThreadId("/api/chat/messages?thread=foo"),
    );
}

test "AC1: single thread param with numeric value" {
    try std.testing.expectEqualStrings(
        "42",
        extractThreadId("/api/chat/messages?thread=42"),
    );
}

// ── AC2: thread first, extra param after (the core bug) ───────────────────────

test "AC2: trailing param is not included in thread ID" {
    try std.testing.expectEqualStrings(
        "foo",
        extractThreadId("/api/chat/messages?thread=foo&other=bar"),
    );
}

test "AC2: result must not contain ampersand" {
    const result = extractThreadId("/api/chat/messages?thread=foo&other=bar");
    try std.testing.expect(std.mem.indexOfScalar(u8, result, '&') == null);
}

test "AC2: bugfix — previous behaviour returned foo&other=bar not foo" {
    // This is the canonical regression test for the bug described in the task.
    const result = extractThreadId("/api/chat/messages?thread=foo&other=bar");
    try std.testing.expectEqualStrings("foo", result);
    try std.testing.expect(!std.mem.eql(u8, result, "foo&other=bar"));
}

// ── AC3: multiple extra params ────────────────────────────────────────────────

test "AC3: two extra params after thread" {
    try std.testing.expectEqualStrings(
        "abc",
        extractThreadId("/api/chat/messages?thread=abc&x=1&y=2"),
    );
}

test "AC3: many extra params after thread" {
    try std.testing.expectEqualStrings(
        "t",
        extractThreadId("/api/chat/messages?thread=t&a=1&b=2&c=3&d=4"),
    );
}

// ── AC4: thread not first ─────────────────────────────────────────────────────

test "AC4: thread param preceded by another param" {
    try std.testing.expectEqualStrings(
        "xyz",
        extractThreadId("/api/chat/messages?a=1&thread=xyz&b=2"),
    );
}

test "AC4: thread param last among several" {
    try std.testing.expectEqualStrings(
        "last",
        extractThreadId("/api/chat/messages?a=1&b=2&thread=last"),
    );
}

// ── AC5: no thread param ──────────────────────────────────────────────────────

test "AC5: no query string returns default" {
    try std.testing.expectEqualStrings(
        "web:dashboard",
        extractThreadId("/api/chat/messages"),
    );
}

test "AC5: unrelated query param returns default" {
    try std.testing.expectEqualStrings(
        "web:dashboard",
        extractThreadId("/api/chat/messages?other=bar"),
    );
}

test "AC5: empty string returns default" {
    try std.testing.expectEqualStrings("web:dashboard", extractThreadId(""));
}

// ── AC6: empty thread value ───────────────────────────────────────────────────

test "AC6: empty thread value returns empty string" {
    try std.testing.expectEqualStrings(
        "",
        extractThreadId("/api/chat/messages?thread="),
    );
}

// ── AC7: empty thread value with trailing param ───────────────────────────────

test "AC7: empty thread value before another param returns empty string" {
    try std.testing.expectEqualStrings(
        "",
        extractThreadId("/api/chat/messages?thread=&other=bar"),
    );
}

test "AC7: empty thread value before multiple params returns empty string" {
    try std.testing.expectEqualStrings(
        "",
        extractThreadId("/api/chat/messages?thread=&a=1&b=2"),
    );
}

// ── AC8: thread ID containing colons ─────────────────────────────────────────

test "AC8: thread ID with colon separator" {
    try std.testing.expectEqualStrings(
        "web:dashboard",
        extractThreadId("/api/chat/messages?thread=web:dashboard"),
    );
}

test "AC8: thread ID with multiple colons" {
    try std.testing.expectEqualStrings(
        "tg:group:12345",
        extractThreadId("/api/chat/messages?thread=tg:group:12345"),
    );
}

test "AC8: thread ID with colon and trailing param" {
    try std.testing.expectEqualStrings(
        "tg:chat:99",
        extractThreadId("/api/chat/messages?thread=tg:chat:99&extra=1"),
    );
}

// ── AC9: returned slice is a sub-slice of input (zero-copy) ──────────────────

test "AC9: returned slice points into original path buffer" {
    const path = "/api/chat/messages?thread=mythread&other=val";
    const result = extractThreadId(path);
    const path_start = @intFromPtr(path.ptr);
    const result_start = @intFromPtr(result.ptr);
    // result must start at or after path start
    try std.testing.expect(result_start >= path_start);
    // result must end within path
    try std.testing.expect(result_start + result.len <= path_start + path.len);
    try std.testing.expectEqualStrings("mythread", result);
}

test "AC9: single-param result is a sub-slice of input" {
    const path = "/api/chat/messages?thread=hello";
    const result = extractThreadId(path);
    const path_start = @intFromPtr(path.ptr);
    const result_start = @intFromPtr(result.ptr);
    try std.testing.expect(result_start >= path_start);
    try std.testing.expect(result_start + result.len <= path_start + path.len);
    try std.testing.expectEqualStrings("hello", result);
}

// ── Edge cases ────────────────────────────────────────────────────────────────

test "EC1: ampersand immediately after ?thread= yields empty value" {
    try std.testing.expectEqualStrings(
        "",
        extractThreadId("/api/chat/messages?thread=&foo=1"),
    );
}

test "EC2: ?thread= at end of string yields empty value" {
    try std.testing.expectEqualStrings(
        "",
        extractThreadId("/api/chat/messages?thread="),
    );
}

test "EC3: first ?thread= occurrence is used when duplicate keys present" {
    // std.mem.indexOf finds the first occurrence
    try std.testing.expectEqualStrings(
        "first",
        extractThreadId("/api/chat/messages?thread=first&thread=second"),
    );
}

test "EC4: URL-encoded characters are preserved as-is" {
    try std.testing.expectEqualStrings(
        "tg%3Achat%3A99",
        extractThreadId("/api/chat/messages?thread=tg%3Achat%3A99"),
    );
}

test "EC5: URL-encoded ampersand (%26) does not terminate extraction" {
    // %26 is the percent-encoding for '&'; it is two bytes '%' and '2', not '&'
    try std.testing.expectEqualStrings(
        "foo%26bar",
        extractThreadId("/api/chat/messages?thread=foo%26bar"),
    );
}

test "EC6: very long thread ID with no ampersand" {
    const long_id = "a" ** 2048;
    const path = "/api/chat/messages?thread=" ++ long_id;
    try std.testing.expectEqualStrings(long_id, extractThreadId(path));
}

test "EC7: path with no slash, just query string" {
    try std.testing.expectEqualStrings(
        "val",
        extractThreadId("?thread=val"),
    );
}

test "EC8: thread value containing equals sign" {
    // '=' is not a delimiter; only '&' terminates
    try std.testing.expectEqualStrings(
        "a=b",
        extractThreadId("/api/chat/messages?thread=a=b"),
    );
}
