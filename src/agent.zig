const std = @import("std");
const json_mod = @import("json.zig");

pub const AgentResult = struct {
    output: []const u8,
    new_session_id: ?[]const u8,
};

/// Parse NDJSON stream output from Claude Code CLI.
/// Extracts the final result text and session_id for resumption.
pub fn parseNdjson(allocator: std.mem.Allocator, data: []const u8) !AgentResult {
    var output_text = std.ArrayList(u8).init(allocator);
    var new_session_id: ?[]const u8 = null;

    var lines = std.mem.splitScalar(u8, data, '\n');
    while (lines.next()) |line| {
        if (line.len == 0) continue;
        var parsed = json_mod.parse(allocator, line) catch continue;
        defer parsed.deinit();

        const msg_type = json_mod.getString(parsed.value, "type") orelse continue;

        if (std.mem.eql(u8, msg_type, "result")) {
            if (json_mod.getString(parsed.value, "result")) |text| {
                output_text.clearRetainingCapacity();
                try output_text.appendSlice(text);
            }
            if (json_mod.getString(parsed.value, "session_id")) |sid| {
                if (new_session_id) |old| allocator.free(old);
                new_session_id = try allocator.dupe(u8, sid);
            }
        } else if (std.mem.eql(u8, msg_type, "system")) {
            if (json_mod.getString(parsed.value, "session_id")) |sid| {
                if (new_session_id) |old| allocator.free(old);
                new_session_id = try allocator.dupe(u8, sid);
            }
        }
    }

    return AgentResult{
        .output = try output_text.toOwnedSlice(),
        .new_session_id = new_session_id,
    };
}

// ── Tests ──────────────────────────────────────────────────────────────

test "parseNdjson extracts result and session_id" {
    const alloc = std.testing.allocator;
    const data =
        \\{"type":"system","subtype":"init","session_id":"sess-abc"}
        \\{"type":"assistant","message":{"content":"thinking"}}
        \\{"type":"result","subtype":"success","session_id":"sess-abc","result":"Hello!"}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("Hello!", result.output);
    try std.testing.expectEqualStrings("sess-abc", result.new_session_id.?);
}

test "parseNdjson handles empty and invalid lines" {
    const alloc = std.testing.allocator;
    const data = "\n\nnot json at all\n{\"type\":\"result\",\"result\":\"ok\"}\n";
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("ok", result.output);
    try std.testing.expect(result.new_session_id == null);
}

test "parseNdjson last result wins" {
    const alloc = std.testing.allocator;
    const data =
        \\{"type":"result","result":"first"}
        \\{"type":"result","result":"second","session_id":"s2"}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("second", result.output);
    try std.testing.expectEqualStrings("s2", result.new_session_id.?);
}
