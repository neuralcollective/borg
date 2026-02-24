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

pub const DirectAgentConfig = struct {
    model: []const u8,
    oauth_token: []const u8,
    session_id: ?[]const u8,
    session_dir: []const u8,
    assistant_name: []const u8,
};

/// Run claude CLI directly as a subprocess (no Docker container).
/// Returns AgentResult with output text and session_id.
pub fn runDirect(allocator: std.mem.Allocator, config: DirectAgentConfig, prompt: []const u8) !AgentResult {
    var argv = std.ArrayList([]const u8).init(allocator);
    defer argv.deinit();

    try argv.appendSlice(&.{
        "claude",
        "--print",
        "--output-format",
        "stream-json",
        "--model",
        config.model,
        "--verbose",
        "--permission-mode",
        "bypassPermissions",
    });

    if (config.session_id) |sid| {
        try argv.appendSlice(&.{ "--resume", sid });
    }

    var child = std.process.Child.init(argv.items, allocator);
    child.stdin_behavior = .Pipe;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;

    // Set environment: inherit current env + override OAuth token
    var env_map = std.process.EnvMap.init(allocator);
    defer env_map.deinit();

    // Copy current environment
    var env_iter = std.process.getEnvMap(allocator) catch |err| {
        std.log.err("Failed to get environment: {}", .{err});
        return err;
    };
    defer env_iter.deinit();
    var it = env_iter.iterator();
    while (it.next()) |entry| {
        try env_map.put(entry.key_ptr.*, entry.value_ptr.*);
    }
    try env_map.put("CLAUDE_CODE_OAUTH_TOKEN", config.oauth_token);

    child.env_map = &env_map;

    try child.spawn();

    // Write prompt to stdin
    if (child.stdin) |stdin| {
        stdin.writeAll(prompt) catch {};
        stdin.close();
        child.stdin = null;
    }

    // Read stdout
    var stdout_buf = std.ArrayList(u8).init(allocator);
    defer stdout_buf.deinit();
    if (child.stdout) |stdout| {
        var read_buf: [8192]u8 = undefined;
        while (true) {
            const n = stdout.read(&read_buf) catch break;
            if (n == 0) break;
            try stdout_buf.appendSlice(read_buf[0..n]);
        }
    }

    const term = try child.wait();
    const exit_code: u8 = switch (term) {
        .Exited => |code| code,
        else => 1,
    };

    if (exit_code != 0 and stdout_buf.items.len == 0) {
        return error.AgentFailed;
    }

    return try parseNdjson(allocator, stdout_buf.items);
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
