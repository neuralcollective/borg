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

// ── getString tests ────────────────────────────────────────────────────

test "getString returns correct value for present key" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc,
        \\{"greeting":"hello","empty":""}
    );
    defer parsed.deinit();

    try std.testing.expectEqualStrings("hello", getString(parsed.value, "greeting").?);
}

test "getString returns null for missing key" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc,
        \\{"name":"alice"}
    );
    defer parsed.deinit();

    try std.testing.expect(getString(parsed.value, "missing") == null);
    try std.testing.expect(getString(parsed.value, "nonexistent") == null);
}

test "getString returns null for wrong value type" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc,
        \\{"count":42,"flag":true,"pi":3.14,"nothing":null}
    );
    defer parsed.deinit();

    try std.testing.expect(getString(parsed.value, "count") == null);
    try std.testing.expect(getString(parsed.value, "flag") == null);
    try std.testing.expect(getString(parsed.value, "pi") == null);
    try std.testing.expect(getString(parsed.value, "nothing") == null);
}

test "getString returns null for non-object value" {
    const alloc = std.testing.allocator;

    // String value
    var parsed_str = try parse(alloc,
        \\"just a string"
    );
    defer parsed_str.deinit();
    try std.testing.expect(getString(parsed_str.value, "key") == null);

    // Integer value
    var parsed_int = try parse(alloc, "123");
    defer parsed_int.deinit();
    try std.testing.expect(getString(parsed_int.value, "key") == null);

    // Null value
    var parsed_null = try parse(alloc, "null");
    defer parsed_null.deinit();
    try std.testing.expect(getString(parsed_null.value, "key") == null);
}

// ── getInt tests ───────────────────────────────────────────────────────

test "getInt returns correct value for present key" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc,
        \\{"count":42}
    );
    defer parsed.deinit();

    try std.testing.expectEqual(@as(i64, 42), getInt(parsed.value, "count").?);
}

test "getInt returns null for missing key" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc,
        \\{"count":42}
    );
    defer parsed.deinit();

    try std.testing.expect(getInt(parsed.value, "missing") == null);
    try std.testing.expect(getInt(parsed.value, "nonexistent") == null);
}

test "getInt returns null for wrong value type" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc,
        \\{"name":"alice","flag":true,"nothing":null}
    );
    defer parsed.deinit();

    try std.testing.expect(getInt(parsed.value, "name") == null);
    try std.testing.expect(getInt(parsed.value, "flag") == null);
    try std.testing.expect(getInt(parsed.value, "nothing") == null);
}

test "getInt coerces float to int" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc,
        \\{"value":3.0}
    );
    defer parsed.deinit();

    try std.testing.expectEqual(@as(i64, 3), getInt(parsed.value, "value").?);
}

test "getInt returns null for non-object value" {
    const alloc = std.testing.allocator;

    // String value
    var parsed_str = try parse(alloc,
        \\"just a string"
    );
    defer parsed_str.deinit();
    try std.testing.expect(getInt(parsed_str.value, "key") == null);

    // Integer value (top-level integer is not an object)
    var parsed_int = try parse(alloc, "99");
    defer parsed_int.deinit();
    try std.testing.expect(getInt(parsed_int.value, "key") == null);

    // Null value
    var parsed_null = try parse(alloc, "null");
    defer parsed_null.deinit();
    try std.testing.expect(getInt(parsed_null.value, "key") == null);
}

// ── getBool tests ──────────────────────────────────────────────────────

test "getBool returns correct value for present key" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc,
        \\{"active":true,"disabled":false}
    );
    defer parsed.deinit();

    try std.testing.expectEqual(true, getBool(parsed.value, "active").?);
    try std.testing.expectEqual(false, getBool(parsed.value, "disabled").?);
}

test "getBool returns null for missing key" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc,
        \\{"active":true}
    );
    defer parsed.deinit();

    try std.testing.expect(getBool(parsed.value, "missing") == null);
    try std.testing.expect(getBool(parsed.value, "nonexistent") == null);
}

test "getBool returns null for wrong value type" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc,
        \\{"name":"alice","count":42,"nothing":null}
    );
    defer parsed.deinit();

    try std.testing.expect(getBool(parsed.value, "name") == null);
    try std.testing.expect(getBool(parsed.value, "count") == null);
    try std.testing.expect(getBool(parsed.value, "nothing") == null);
}

test "getBool returns null for non-object value" {
    const alloc = std.testing.allocator;

    // String value
    var parsed_str = try parse(alloc,
        \\"just a string"
    );
    defer parsed_str.deinit();
    try std.testing.expect(getBool(parsed_str.value, "key") == null);

    // Boolean value (top-level bool is not an object)
    var parsed_bool = try parse(alloc, "true");
    defer parsed_bool.deinit();
    try std.testing.expect(getBool(parsed_bool.value, "key") == null);

    // Null value
    var parsed_null = try parse(alloc, "null");
    defer parsed_null.deinit();
    try std.testing.expect(getBool(parsed_null.value, "key") == null);
}

// ── Edge case tests ────────────────────────────────────────────────────

test "all getters return null for null JSON value" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc,
        \\{"key":null}
    );
    defer parsed.deinit();

    try std.testing.expect(getString(parsed.value, "key") == null);
    try std.testing.expect(getInt(parsed.value, "key") == null);
    try std.testing.expect(getBool(parsed.value, "key") == null);
}

test "all getters return null for empty object" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc, "{}");
    defer parsed.deinit();

    try std.testing.expect(getString(parsed.value, "anything") == null);
    try std.testing.expect(getInt(parsed.value, "anything") == null);
    try std.testing.expect(getBool(parsed.value, "anything") == null);
}

test "getString returns empty string for empty string value" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc,
        \\{"empty":""}
    );
    defer parsed.deinit();

    const result = getString(parsed.value, "empty");
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("", result.?);
}

test "getInt handles negative and zero values" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc,
        \\{"negative":-7,"zero":0}
    );
    defer parsed.deinit();

    try std.testing.expectEqual(@as(i64, -7), getInt(parsed.value, "negative").?);
    try std.testing.expectEqual(@as(i64, 0), getInt(parsed.value, "zero").?);
}

test "getBool distinguishes false from null" {
    const alloc = std.testing.allocator;
    var parsed = try parse(alloc,
        \\{"present_false":false,"null_val":null}
    );
    defer parsed.deinit();

    // false should be returned as ?bool = false, not null
    const result_false = getBool(parsed.value, "present_false");
    try std.testing.expect(result_false != null);
    try std.testing.expectEqual(false, result_false.?);

    // null JSON value should return ?bool = null
    const result_null = getBool(parsed.value, "null_val");
    try std.testing.expect(result_null == null);

    // missing key should return ?bool = null
    const result_missing = getBool(parsed.value, "no_such_key");
    try std.testing.expect(result_missing == null);
}
