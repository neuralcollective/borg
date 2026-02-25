// Tests for json.zig: malformed-input error paths, getString/getInt coverage,
// nested traversal, and escapeString character-class completeness.
//
// To include in the build, add to json.zig (inside the existing test section):
//   test { _ = @import("json_test.zig"); }
//
// All AC1 tests verify that parse() returns an error for malformed JSON and
// that the leak-detecting allocator sees no leaked memory.

const std = @import("std");
const json = @import("json.zig");

// ── AC1: parse returns error on malformed JSON ─────────────────────────

test "parse error: empty input" {
    const alloc = std.testing.allocator;
    const result = json.parse(alloc, "");
    try std.testing.expectError(error.UnexpectedEndOfInput, result);
}

test "parse error: unclosed object brace" {
    const alloc = std.testing.allocator;
    const result = json.parse(alloc, "{");
    try std.testing.expect(std.meta.isError(result));
}

test "parse error: bare closing brace" {
    const alloc = std.testing.allocator;
    const result = json.parse(alloc, "}");
    try std.testing.expect(std.meta.isError(result));
}

test "parse error: unquoted object key" {
    const alloc = std.testing.allocator;
    // {invalid} — bare word as key without quotes
    const result = json.parse(alloc, "{invalid}");
    try std.testing.expect(std.meta.isError(result));
}

test "parse error: missing value after colon" {
    const alloc = std.testing.allocator;
    const result = json.parse(alloc, "{\"k\":}");
    try std.testing.expect(std.meta.isError(result));
}

test "parse error: truncated array" {
    const alloc = std.testing.allocator;
    const result = json.parse(alloc, "[1,2,");
    try std.testing.expect(std.meta.isError(result));
}

test "parse error: truncated value after colon" {
    const alloc = std.testing.allocator;
    const result = json.parse(alloc, "{\"k\":");
    try std.testing.expect(std.meta.isError(result));
}

test "parse error: trailing garbage after valid object" {
    const alloc = std.testing.allocator;
    // std.json.parseFromSlice with default options rejects trailing non-whitespace
    const result = json.parse(alloc, "{}garbage");
    try std.testing.expect(std.meta.isError(result));
}

test "parse error: bare word true-ish but misspelled" {
    const alloc = std.testing.allocator;
    const result = json.parse(alloc, "tru");
    try std.testing.expect(std.meta.isError(result));
}

test "parse error: unclosed string" {
    const alloc = std.testing.allocator;
    const result = json.parse(alloc, "\"unclosed");
    try std.testing.expect(std.meta.isError(result));
}

// ── AC2: parse succeeds on valid JSON ─────────────────────────────────

test "parse: empty object produces empty .object" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{}");
    defer parsed.deinit();
    try std.testing.expect(parsed.value == .object);
    try std.testing.expectEqual(@as(usize, 0), parsed.value.object.count());
}

test "parse: top-level null" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "null");
    defer parsed.deinit();
    try std.testing.expect(parsed.value == .null);
}

test "parse: top-level string" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "\"str\"");
    defer parsed.deinit();
    try std.testing.expect(parsed.value == .string);
    try std.testing.expectEqualStrings("str", parsed.value.string);
}

test "parse: simple object getString" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"k\":\"v\"}");
    defer parsed.deinit();
    try std.testing.expectEqualStrings("v", json.getString(parsed.value, "k").?);
}

// ── AC3: getString branch coverage ────────────────────────────────────

test "getString: returns value for present string key" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"x\":\"hi\"}");
    defer parsed.deinit();
    try std.testing.expectEqualStrings("hi", json.getString(parsed.value, "x").?);
}

test "getString: returns null for missing key" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"x\":\"hi\"}");
    defer parsed.deinit();
    try std.testing.expect(json.getString(parsed.value, "missing") == null);
}

test "getString: returns null when value is integer" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"n\":42}");
    defer parsed.deinit();
    try std.testing.expect(json.getString(parsed.value, "n") == null);
}

test "getString: returns empty string (non-null) for empty string value" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"e\":\"\"}");
    defer parsed.deinit();
    const result = json.getString(parsed.value, "e");
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("", result.?);
}

test "getString: returns null for non-object top-level value" {
    // Construct Values directly — no allocation needed
    try std.testing.expect(json.getString(json.Value{ .string = "bare" }, "k") == null);
    try std.testing.expect(json.getString(json.Value{ .integer = 1 }, "k") == null);
    try std.testing.expect(json.getString(json.Value{ .bool = true }, "k") == null);
    try std.testing.expect(json.getString(json.Value{ .null = {} }, "k") == null);
}

// ── AC4: getInt branch coverage ────────────────────────────────────────

test "getInt: returns value for present integer key" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"n\":42}");
    defer parsed.deinit();
    try std.testing.expectEqual(@as(i64, 42), json.getInt(parsed.value, "n").?);
}

test "getInt: returns negative value" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"n\":-7}");
    defer parsed.deinit();
    try std.testing.expectEqual(@as(i64, -7), json.getInt(parsed.value, "n").?);
}

test "getInt: returns zero" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"n\":0}");
    defer parsed.deinit();
    try std.testing.expectEqual(@as(i64, 0), json.getInt(parsed.value, "n").?);
}

test "getInt: float 3.0 coerced to 3" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"n\":3.0}");
    defer parsed.deinit();
    try std.testing.expectEqual(@as(i64, 3), json.getInt(parsed.value, "n").?);
}

test "getInt: returns null for missing key" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"n\":1}");
    defer parsed.deinit();
    try std.testing.expect(json.getInt(parsed.value, "missing") == null);
}

test "getInt: returns null when value is string" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"s\":\"str\"}");
    defer parsed.deinit();
    try std.testing.expect(json.getInt(parsed.value, "s") == null);
}

test "getInt: returns null when value is bool" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"b\":true}");
    defer parsed.deinit();
    try std.testing.expect(json.getInt(parsed.value, "b") == null);
}

test "getInt: returns null when value is JSON null" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"x\":null}");
    defer parsed.deinit();
    try std.testing.expect(json.getInt(parsed.value, "x") == null);
}

test "getInt: returns null for non-object top-level value" {
    try std.testing.expect(json.getInt(json.Value{ .integer = 99 }, "k") == null);
    try std.testing.expect(json.getInt(json.Value{ .string = "s" }, "k") == null);
    try std.testing.expect(json.getInt(json.Value{ .null = {} }, "k") == null);
}

// ── AC5: escapeString character-class completeness ─────────────────────

test "escapeString: double quote" {
    const alloc = std.testing.allocator;
    const r = try json.escapeString(alloc, "\"");
    defer alloc.free(r);
    try std.testing.expectEqualStrings("\\\"", r);
}

test "escapeString: backslash" {
    const alloc = std.testing.allocator;
    const r = try json.escapeString(alloc, "\\");
    defer alloc.free(r);
    try std.testing.expectEqualStrings("\\\\", r);
}

test "escapeString: newline" {
    const alloc = std.testing.allocator;
    const r = try json.escapeString(alloc, "\n");
    defer alloc.free(r);
    try std.testing.expectEqualStrings("\\n", r);
}

test "escapeString: carriage return" {
    const alloc = std.testing.allocator;
    const r = try json.escapeString(alloc, "\r");
    defer alloc.free(r);
    try std.testing.expectEqualStrings("\\r", r);
}

test "escapeString: tab" {
    const alloc = std.testing.allocator;
    const r = try json.escapeString(alloc, "\t");
    defer alloc.free(r);
    try std.testing.expectEqualStrings("\\t", r);
}

test "escapeString: null byte 0x00" {
    const alloc = std.testing.allocator;
    const r = try json.escapeString(alloc, &[_]u8{0x00});
    defer alloc.free(r);
    try std.testing.expectEqualStrings("\\u0000", r);
}

test "escapeString: control byte 0x01" {
    const alloc = std.testing.allocator;
    const r = try json.escapeString(alloc, &[_]u8{0x01});
    defer alloc.free(r);
    try std.testing.expectEqualStrings("\\u0001", r);
}

test "escapeString: control byte 0x1f" {
    const alloc = std.testing.allocator;
    const r = try json.escapeString(alloc, &[_]u8{0x1f});
    defer alloc.free(r);
    try std.testing.expectEqualStrings("\\u001f", r);
}

test "escapeString: space 0x20 passes through" {
    const alloc = std.testing.allocator;
    const r = try json.escapeString(alloc, " ");
    defer alloc.free(r);
    try std.testing.expectEqualStrings(" ", r);
}

test "escapeString: boundary 0x1f then 0x20" {
    const alloc = std.testing.allocator;
    const r = try json.escapeString(alloc, &[_]u8{ 0x1f, 0x20 });
    defer alloc.free(r);
    try std.testing.expectEqualStrings("\\u001f ", r);
}

test "escapeString: multi-byte UTF-8 passes through unchanged" {
    const alloc = std.testing.allocator;
    const input = "héllo 世界";
    const r = try json.escapeString(alloc, input);
    defer alloc.free(r);
    try std.testing.expectEqualStrings(input, r);
    try std.testing.expectEqual(input.len, r.len);
}

test "escapeString: empty input yields empty output" {
    const alloc = std.testing.allocator;
    const r = try json.escapeString(alloc, "");
    defer alloc.free(r);
    try std.testing.expectEqualStrings("", r);
    try std.testing.expectEqual(@as(usize, 0), r.len);
}

// ── Edge case: deep nested traversal ──────────────────────────────────

test "nested traversal: three-level getObject chain then getString" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"a\":{\"b\":{\"c\":\"deep\"}}}");
    defer parsed.deinit();

    const a = json.getObject(parsed.value, "a");
    try std.testing.expect(a != null);
    const b = json.getObject(a.?, "b");
    try std.testing.expect(b != null);
    const result = json.getString(b.?, "c");
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("deep", result.?);
}

// ── Edge case: all accessors return null for array top-level value ─────

test "all accessors return null for top-level array value" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "[1,2,3]");
    defer parsed.deinit();

    try std.testing.expect(json.getString(parsed.value, "k") == null);
    try std.testing.expect(json.getInt(parsed.value, "k") == null);
    try std.testing.expect(json.getBool(parsed.value, "k") == null);
    try std.testing.expect(json.getObject(parsed.value, "k") == null);
    try std.testing.expect(json.getArray(parsed.value, "k") == null);
}

// ── Edge case: getArray returns correct slice ──────────────────────────

test "getArray: returns items for present array key" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"list\":[10,20,30]}");
    defer parsed.deinit();

    const items = json.getArray(parsed.value, "list");
    try std.testing.expect(items != null);
    try std.testing.expectEqual(@as(usize, 3), items.?.len);
    try std.testing.expectEqual(@as(i64, 10), items.?[0].integer);
    try std.testing.expectEqual(@as(i64, 20), items.?[1].integer);
    try std.testing.expectEqual(@as(i64, 30), items.?[2].integer);
}

test "getArray: returns null for missing key" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"list\":[1]}");
    defer parsed.deinit();
    try std.testing.expect(json.getArray(parsed.value, "missing") == null);
}

test "getArray: returns null when value is not an array" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"x\":42}");
    defer parsed.deinit();
    try std.testing.expect(json.getArray(parsed.value, "x") == null);
}
