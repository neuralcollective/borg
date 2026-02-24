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
