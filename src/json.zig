const std = @import("std");

/// Thin wrappers around std.json for convenient field access on parsed JSON values.

pub const Value = std.json.Value;
pub const ParseError = std.json.Error;

pub fn parse(allocator: std.mem.Allocator, source: []const u8) !std.json.Parsed(Value) {
    return std.json.parseFromSlice(Value, allocator, source, .{
        .allocate = .alloc_always,
    });
}

pub fn getString(obj: Value, key: []const u8) ?[]const u8 {
    if (obj != .object) return null;
    const val = obj.object.get(key) orelse return null;
    return switch (val) {
        .string => |s| s,
        else => null,
    };
}

pub fn getInt(obj: Value, key: []const u8) ?i64 {
    if (obj != .object) return null;
    const val = obj.object.get(key) orelse return null;
    return switch (val) {
        .integer => |i| i,
        .float => |f| @intFromFloat(f),
        else => null,
    };
}

pub fn getBool(obj: Value, key: []const u8) ?bool {
    if (obj != .object) return null;
    const val = obj.object.get(key) orelse return null;
    return switch (val) {
        .bool => |b| b,
        else => null,
    };
}

pub fn getObject(obj: Value, key: []const u8) ?Value {
    if (obj != .object) return null;
    const val = obj.object.get(key) orelse return null;
    return switch (val) {
        .object => val,
        else => null,
    };
}

pub fn getArray(obj: Value, key: []const u8) ?[]Value {
    if (obj != .object) return null;
    const val = obj.object.get(key) orelse return null;
    return switch (val) {
        .array => |a| a.items,
        else => null,
    };
}

/// Escape a string for JSON output
pub fn escapeString(allocator: std.mem.Allocator, input: []const u8) ![]const u8 {
    var result = std.ArrayList(u8).init(allocator);
    for (input) |ch| {
        switch (ch) {
            '"' => try result.appendSlice("\\\""),
            '\\' => try result.appendSlice("\\\\"),
            '\n' => try result.appendSlice("\\n"),
            '\r' => try result.appendSlice("\\r"),
            '\t' => try result.appendSlice("\\t"),
            else => {
                if (ch < 0x20) {
                    try result.writer().print("\\u{x:0>4}", .{ch});
                } else {
                    try result.append(ch);
                }
            },
        }
    }
    return result.toOwnedSlice();
}

/// Stringify a JSON value
pub fn stringify(allocator: std.mem.Allocator, value: Value) ![]const u8 {
    var buf = std.ArrayList(u8).init(allocator);
    try std.json.stringify(value, .{}, buf.writer());
    return buf.toOwnedSlice();
}

// ── Tests ──────────────────────────────────────────────────────────────

test "parse and access typed fields" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc,
        \\{"name":"test","count":42,"active":true,"nested":{"key":"val"},"items":[1,2,3]}
    );
    defer parsed.deinit();

    try std.testing.expectEqualStrings("test", getString(parsed.value, "name").?);
    try std.testing.expectEqual(@as(i64, 42), getInt(parsed.value, "count").?);
    try std.testing.expectEqual(true, getBool(parsed.value, "active").?);

    const nested = getObject(parsed.value, "nested").?;
    try std.testing.expectEqualStrings("val", getString(nested, "key").?);

    const items = getArray(parsed.value, "items").?;
    try std.testing.expectEqual(@as(usize, 3), items.len);
}

test "escapeString handles special characters" {
    const alloc = std.testing.allocator;
    // Input: a"b\c<newline>d<tab>e
    const result = try escapeString(alloc, "a\"b\\c\nd\te");
    defer alloc.free(result);
    try std.testing.expectEqualStrings("a\\\"b\\\\c\\nd\\te", result);
}

test "escapeString handles control characters" {
    const alloc = std.testing.allocator;
    const input = &[_]u8{ 0x01, 0x1f };
    const result = try escapeString(alloc, input);
    defer alloc.free(result);
    try std.testing.expectEqualStrings("\\u0001\\u001f", result);
}

test "getString returns null for missing key and wrong type" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc, "{\"count\":42,\"flag\":true}");
    defer parsed.deinit();

    try std.testing.expect(getString(parsed.value, "missing") == null);
    try std.testing.expect(getString(parsed.value, "count") == null);
    try std.testing.expect(getString(parsed.value, "flag") == null);
    try std.testing.expectEqual(@as(i64, 42), getInt(parsed.value, "count").?);
}

// ── New escapeString tests ─────────────────────────────────────────────

test "escapeString returns empty output for empty input" {
    const alloc = std.testing.allocator;
    const result = try escapeString(alloc, "");
    defer alloc.free(result);
    try std.testing.expectEqualStrings("", result);
    try std.testing.expectEqual(@as(usize, 0), result.len);
}

test "escapeString escapes carriage return" {
    const alloc = std.testing.allocator;
    const result = try escapeString(alloc, "\r");
    defer alloc.free(result);
    try std.testing.expectEqualStrings("\\r", result);
}

test "escapeString passes through normal ASCII unchanged" {
    const alloc = std.testing.allocator;
    const input = "Hello, world! 0123 ABC abc ~";
    const result = try escapeString(alloc, input);
    defer alloc.free(result);
    try std.testing.expectEqualStrings(input, result);
}

test "escapeString passes through multi-byte UTF-8 unchanged" {
    const alloc = std.testing.allocator;
    const input = "café 日本語 🚀";
    const result = try escapeString(alloc, input);
    defer alloc.free(result);
    try std.testing.expectEqualStrings(input, result);
}

test "escapeString escapes null byte" {
    const alloc = std.testing.allocator;
    const result = try escapeString(alloc, &[_]u8{0x00});
    defer alloc.free(result);
    try std.testing.expectEqualStrings("\\u0000", result);
    try std.testing.expectEqual(@as(usize, 6), result.len);
}

test "escapeString escapes all control characters below 0x20" {
    const alloc = std.testing.allocator;
    const input = &[_]u8{
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
        0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
        0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
    };
    const result = try escapeString(alloc, input);
    defer alloc.free(result);

    // 3 named escapes (\t, \n, \r) at 2 chars each = 6
    // 29 hex escapes (\uXXXX) at 6 chars each = 174
    // Total = 180
    const expected =
        "\\u0000\\u0001\\u0002\\u0003\\u0004\\u0005\\u0006\\u0007" ++
        "\\u0008\\t\\n\\u000b\\u000c\\r\\u000e\\u000f" ++
        "\\u0010\\u0011\\u0012\\u0013\\u0014\\u0015\\u0016\\u0017" ++
        "\\u0018\\u0019\\u001a\\u001b\\u001c\\u001d\\u001e\\u001f";

    try std.testing.expectEqualStrings(expected, result);
    try std.testing.expectEqual(@as(usize, 180), result.len);
}

test "escapeString escapes each special character in isolation" {
    const alloc = std.testing.allocator;

    // Double quote
    const r1 = try escapeString(alloc, "\"");
    defer alloc.free(r1);
    try std.testing.expectEqualStrings("\\\"", r1);

    // Backslash
    const r2 = try escapeString(alloc, "\\");
    defer alloc.free(r2);
    try std.testing.expectEqualStrings("\\\\", r2);

    // Newline
    const r3 = try escapeString(alloc, "\n");
    defer alloc.free(r3);
    try std.testing.expectEqualStrings("\\n", r3);

    // Tab
    const r4 = try escapeString(alloc, "\t");
    defer alloc.free(r4);
    try std.testing.expectEqualStrings("\\t", r4);

    // Carriage return
    const r5 = try escapeString(alloc, "\r");
    defer alloc.free(r5);
    try std.testing.expectEqualStrings("\\r", r5);
}

test "escapeString handles mixed content with all escape types" {
    const alloc = std.testing.allocator;
    // Input: "hi" + 0x01 + tab + "é" (multi-byte UTF-8)
    const input = "hi\x01\t" ++ "é";
    const result = try escapeString(alloc, input);
    defer alloc.free(result);
    try std.testing.expectEqualStrings("hi\\u0001\\t" ++ "é", result);
}

test "escapeString does not escape space (0x20 boundary)" {
    const alloc = std.testing.allocator;
    // 0x20 (space) is the first character that should NOT be escaped
    const result = try escapeString(alloc, " ");
    defer alloc.free(result);
    try std.testing.expectEqualStrings(" ", result);
    try std.testing.expectEqual(@as(usize, 1), result.len);
}

test "escapeString escapes 0x1f (upper boundary of control range)" {
    const alloc = std.testing.allocator;
    const result = try escapeString(alloc, &[_]u8{0x1f});
    defer alloc.free(result);
    try std.testing.expectEqualStrings("\\u001f", result);
}

test "escapeString handles adjacent special characters" {
    const alloc = std.testing.allocator;
    // Input: quote followed by backslash (two special chars adjacent)
    const result = try escapeString(alloc, "\"\\");
    defer alloc.free(result);
    try std.testing.expectEqualStrings("\\\"\\\\", result);
}
