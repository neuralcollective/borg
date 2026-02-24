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

test "escapeString empty string" {
    const alloc = std.testing.allocator;
    const result = try escapeString(alloc, "");
    defer alloc.free(result);
    try std.testing.expectEqualStrings("", result);
    try std.testing.expectEqual(@as(usize, 0), result.len);
}

test "escapeString double quote standalone" {
    const alloc = std.testing.allocator;
    const result = try escapeString(alloc, "\"");
    defer alloc.free(result);
    try std.testing.expectEqualStrings("\\\"", result);
}

test "escapeString backslash standalone" {
    const alloc = std.testing.allocator;
    const result = try escapeString(alloc, "\\");
    defer alloc.free(result);
    try std.testing.expectEqualStrings("\\\\", result);
}

test "escapeString newline standalone" {
    const alloc = std.testing.allocator;
    const result = try escapeString(alloc, "\n");
    defer alloc.free(result);
    try std.testing.expectEqualStrings("\\n", result);
}

test "escapeString carriage return standalone" {
    const alloc = std.testing.allocator;
    const result = try escapeString(alloc, "\r");
    defer alloc.free(result);
    try std.testing.expectEqualStrings("\\r", result);
}

test "escapeString tab standalone" {
    const alloc = std.testing.allocator;
    const result = try escapeString(alloc, "\t");
    defer alloc.free(result);
    try std.testing.expectEqualStrings("\\t", result);
}

test "escapeString control char null 0x00" {
    const alloc = std.testing.allocator;
    const input = &[_]u8{0x00};
    const result = try escapeString(alloc, input);
    defer alloc.free(result);
    try std.testing.expectEqualStrings("\\u0000", result);
}

test "escapeString control char bell 0x07" {
    const alloc = std.testing.allocator;
    const input = &[_]u8{0x07};
    const result = try escapeString(alloc, input);
    defer alloc.free(result);
    try std.testing.expectEqualStrings("\\u0007", result);
}

test "escapeString control char form feed 0x0C" {
    const alloc = std.testing.allocator;
    const input = &[_]u8{0x0C};
    const result = try escapeString(alloc, input);
    defer alloc.free(result);
    try std.testing.expectEqualStrings("\\u000c", result);
}

test "escapeString control char 0x1F" {
    const alloc = std.testing.allocator;
    const input = &[_]u8{0x1F};
    const result = try escapeString(alloc, input);
    defer alloc.free(result);
    try std.testing.expectEqualStrings("\\u001f", result);
}

test "escapeString normal ASCII passthrough" {
    const alloc = std.testing.allocator;
    const input = "hello world 123!@#";
    const result = try escapeString(alloc, input);
    defer alloc.free(result);
    try std.testing.expectEqualStrings(input, result);
}

test "escapeString multi-byte UTF-8 passthrough" {
    const alloc = std.testing.allocator;
    const input = "héllo 世界";
    const result = try escapeString(alloc, input);
    defer alloc.free(result);
    try std.testing.expectEqualStrings(input, result);
    try std.testing.expectEqual(input.len, result.len);
}

test "escapeString mixed content" {
    const alloc = std.testing.allocator;
    // Mix of: normal text, double quote, backslash, newline, carriage return, tab, control char 0x07
    const input = "hi\"there\\foo\nbar\rbaz\tEnd" ++ &[_]u8{0x07};
    const result = try escapeString(alloc, input);
    defer alloc.free(result);
    try std.testing.expectEqualStrings("hi\\\"there\\\\foo\\nbar\\rbaz\\tEnd\\u0007", result);
}

test "escapeString boundary 0x1F vs 0x20" {
    const alloc = std.testing.allocator;
    // 0x1F should be escaped, 0x20 (space) should pass through
    const input = &[_]u8{ 0x1F, 0x20 };
    const result = try escapeString(alloc, input);
    defer alloc.free(result);
    try std.testing.expectEqualStrings("\\u001f ", result);
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
