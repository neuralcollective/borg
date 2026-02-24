const std = @import("std");
const json_mod = @import("json.zig");

pub const DiscordMessage = struct {
    channel_id: []const u8,
    message_id: []const u8,
    sender_id: []const u8,
    sender_name: []const u8,
    text: []const u8,
    timestamp: i64,
    is_dm: bool,
    mentions_bot: bool,
};

pub const DiscordEvent = union(enum) {
    message: DiscordMessage,
    ready: []const u8,
    err: []const u8,
};

pub const Discord = struct {
    allocator: std.mem.Allocator,
    token: []const u8,
    child: ?std.process.Child,
    stdout_buf: std.ArrayList(u8),
    connected: bool,
    bot_id: []const u8,
    assistant_name: []const u8,

    pub fn init(allocator: std.mem.Allocator, token: []const u8, assistant_name: []const u8) Discord {
        return .{
            .allocator = allocator,
            .token = token,
            .child = null,
            .stdout_buf = std.ArrayList(u8).init(allocator),
            .connected = false,
            .bot_id = "",
            .assistant_name = assistant_name,
        };
    }

    pub fn start(self: *Discord) !void {
        var child = std.process.Child.init(
            &.{ "node", "discord/bridge.js", self.assistant_name },
            self.allocator,
        );
        child.stdin_behavior = .Pipe;
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Pipe;

        // Pass DISCORD_TOKEN via environment
        var env_map = std.process.EnvMap.init(self.allocator);
        defer env_map.deinit();
        // Inherit current env
        const env_vars = std.process.getEnvMap(self.allocator) catch |err| {
            std.log.err("Discord: failed to get env: {}", .{err});
            return err;
        };
        defer {
            var env_copy = env_vars;
            env_copy.deinit();
        }
        var it = env_vars.iterator();
        while (it.next()) |entry| {
            try env_map.put(entry.key_ptr.*, entry.value_ptr.*);
        }
        try env_map.put("DISCORD_TOKEN", self.token);
        child.env_map = &env_map;

        try child.spawn();
        self.child = child;
        std.log.info("Discord bridge process started", .{});
    }

    pub fn deinit(self: *Discord) void {
        if (self.child) |*c| {
            if (c.stdin) |stdin| {
                stdin.close();
                c.stdin = null;
            }
            _ = c.kill() catch {};
        }
        self.stdout_buf.deinit();
    }

    pub fn poll(self: *Discord, allocator: std.mem.Allocator) ![]DiscordMessage {
        const child = &(self.child orelse return &[_]DiscordMessage{});
        const stdout = child.stdout orelse return &[_]DiscordMessage{};

        var read_buf: [4096]u8 = undefined;
        while (true) {
            const n = stdout.read(&read_buf) catch break;
            if (n == 0) break;
            try self.stdout_buf.appendSlice(read_buf[0..n]);
            if (n < read_buf.len) break;
        }

        var messages = std.ArrayList(DiscordMessage).init(allocator);
        while (std.mem.indexOf(u8, self.stdout_buf.items, "\n")) |nl| {
            const line = self.stdout_buf.items[0..nl];
            if (line.len > 0) {
                if (self.parseEvent(allocator, line)) |event| {
                    switch (event) {
                        .message => |msg| try messages.append(msg),
                        .ready => |bot_id| {
                            self.connected = true;
                            self.bot_id = bot_id;
                            std.log.info("Discord connected as bot {s}", .{bot_id});
                        },
                        .err => |msg| {
                            std.log.err("Discord bridge error: {s}", .{msg});
                        },
                    }
                }
            }
            const remaining = self.stdout_buf.items[nl + 1 ..];
            std.mem.copyForwards(u8, self.stdout_buf.items[0..remaining.len], remaining);
            self.stdout_buf.shrinkRetainingCapacity(remaining.len);
        }

        return messages.toOwnedSlice();
    }

    fn parseEvent(_: *Discord, allocator: std.mem.Allocator, line: []const u8) ?DiscordEvent {
        var parsed = json_mod.parse(allocator, line) catch return null;
        defer parsed.deinit();

        const event_type = json_mod.getString(parsed.value, "event") orelse return null;

        if (std.mem.eql(u8, event_type, "message")) {
            return DiscordEvent{ .message = DiscordMessage{
                .channel_id = allocator.dupe(u8, json_mod.getString(parsed.value, "channel_id") orelse "") catch return null,
                .message_id = allocator.dupe(u8, json_mod.getString(parsed.value, "message_id") orelse "") catch return null,
                .sender_id = allocator.dupe(u8, json_mod.getString(parsed.value, "sender_id") orelse "") catch return null,
                .sender_name = allocator.dupe(u8, json_mod.getString(parsed.value, "sender_name") orelse "") catch return null,
                .text = allocator.dupe(u8, json_mod.getString(parsed.value, "text") orelse "") catch return null,
                .timestamp = json_mod.getInt(parsed.value, "timestamp") orelse std.time.timestamp(),
                .is_dm = json_mod.getBool(parsed.value, "is_dm") orelse false,
                .mentions_bot = json_mod.getBool(parsed.value, "mentions_bot") orelse false,
            } };
        } else if (std.mem.eql(u8, event_type, "ready")) {
            return DiscordEvent{ .ready = allocator.dupe(u8, json_mod.getString(parsed.value, "bot_id") orelse "") catch return null };
        } else if (std.mem.eql(u8, event_type, "error")) {
            return DiscordEvent{ .err = allocator.dupe(u8, json_mod.getString(parsed.value, "message") orelse "") catch return null };
        }

        return null;
    }

    pub fn sendMessage(self: *Discord, channel_id: []const u8, text: []const u8, reply_to: ?[]const u8) !void {
        const child = &(self.child orelse return error.NotConnected);
        const stdin = child.stdin orelse return error.NotConnected;

        var buf = std.ArrayList(u8).init(self.allocator);
        defer buf.deinit();
        const w = buf.writer();

        const esc_text = try json_mod.escapeString(self.allocator, text);
        defer self.allocator.free(esc_text);

        try w.print("{{\"cmd\":\"send\",\"channel_id\":\"{s}\",\"text\":\"{s}\"", .{ channel_id, esc_text });
        if (reply_to) |rid| {
            try w.print(",\"reply_to\":\"{s}\"", .{rid});
        }
        try w.writeAll("}\n");

        try stdin.writeAll(buf.items);
    }

    pub fn sendTyping(self: *Discord, channel_id: []const u8) !void {
        const child = &(self.child orelse return error.NotConnected);
        const stdin = child.stdin orelse return error.NotConnected;

        var buf: [256]u8 = undefined;
        const cmd = try std.fmt.bufPrint(&buf, "{{\"cmd\":\"typing\",\"channel_id\":\"{s}\"}}\n", .{channel_id});
        try stdin.writeAll(cmd);
    }
};

// ── Tests ──────────────────────────────────────────────────────────────

test "Discord init/deinit" {
    var d = Discord.init(std.testing.allocator, "test-token", "Borg");
    defer d.deinit();
    try std.testing.expect(!d.connected);
    try std.testing.expect(d.child == null);
}

test "DiscordEvent union size" {
    const event = DiscordEvent{ .ready = "123456" };
    switch (event) {
        .ready => |id| try std.testing.expectEqualStrings("123456", id),
        else => unreachable,
    }
}
