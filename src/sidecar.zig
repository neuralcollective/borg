const std = @import("std");
const json_mod = @import("json.zig");

// Unified message type for both Discord and WhatsApp
pub const SidecarMessage = struct {
    source: Source,
    // Common fields
    id: []const u8,
    chat_id: []const u8,
    sender: []const u8,
    sender_name: []const u8,
    text: []const u8,
    timestamp: i64,
    is_group: bool,
    mentions_bot: bool,
};

pub const Source = enum { discord, whatsapp };

pub const SidecarEvent = union(enum) {
    message: SidecarMessage,
    discord_ready: []const u8,
    wa_connected: []const u8,
    wa_qr: []const u8,
    disconnected: struct { source: Source, reason: []const u8 },
    err: struct { source: Source, message: []const u8 },
};

pub const Sidecar = struct {
    allocator: std.mem.Allocator,
    child: ?std.process.Child,
    stdout_buf: std.ArrayList(u8),
    assistant_name: []const u8,
    discord_connected: bool,
    wa_connected: bool,
    discord_bot_id: []const u8,

    pub fn init(allocator: std.mem.Allocator, assistant_name: []const u8) Sidecar {
        return .{
            .allocator = allocator,
            .child = null,
            .stdout_buf = std.ArrayList(u8).init(allocator),
            .assistant_name = assistant_name,
            .discord_connected = false,
            .wa_connected = false,
            .discord_bot_id = "",
        };
    }

    pub fn start(self: *Sidecar, discord_token: []const u8, wa_auth_dir: []const u8, wa_disabled: bool) !void {
        var child = std.process.Child.init(
            &.{ "bun", "sidecar/bridge.js", self.assistant_name },
            self.allocator,
        );
        child.stdin_behavior = .Pipe;
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Inherit;

        var env_map = std.process.EnvMap.init(self.allocator);
        defer env_map.deinit();
        const env_vars = std.process.getEnvMap(self.allocator) catch |err| {
            std.log.err("Sidecar: failed to get env: {}", .{err});
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
        if (discord_token.len > 0) try env_map.put("DISCORD_TOKEN", discord_token);
        if (wa_auth_dir.len > 0) try env_map.put("WA_AUTH_DIR", wa_auth_dir);
        if (wa_disabled) try env_map.put("WA_DISABLED", "true");
        child.env_map = &env_map;

        try child.spawn();
        self.child = child;
        std.log.info("Sidecar process started", .{});
    }

    pub fn deinit(self: *Sidecar) void {
        if (self.child) |*c| {
            if (c.stdin) |stdin| {
                stdin.close();
                c.stdin = null;
            }
            _ = c.kill() catch {};
        }
        self.stdout_buf.deinit();
    }

    pub fn poll(self: *Sidecar, allocator: std.mem.Allocator) ![]SidecarMessage {
        const child = &(self.child orelse return &[_]SidecarMessage{});
        const stdout = child.stdout orelse return &[_]SidecarMessage{};

        var read_buf: [4096]u8 = undefined;
        while (true) {
            const n = stdout.read(&read_buf) catch break;
            if (n == 0) break;
            try self.stdout_buf.appendSlice(read_buf[0..n]);
            if (n < read_buf.len) break;
        }

        var messages = std.ArrayList(SidecarMessage).init(allocator);
        while (std.mem.indexOf(u8, self.stdout_buf.items, "\n")) |nl| {
            const line = self.stdout_buf.items[0..nl];
            if (line.len > 0) {
                if (self.parseEvent(allocator, line)) |event| {
                    switch (event) {
                        .message => |msg| try messages.append(msg),
                        .discord_ready => |bot_id| {
                            self.discord_connected = true;
                            self.discord_bot_id = self.allocator.dupe(u8, bot_id) catch "";
                            std.log.info("Discord connected as bot {s}", .{self.discord_bot_id});
                        },
                        .wa_connected => |jid| {
                            self.wa_connected = true;
                            std.log.info("WhatsApp connected as {s}", .{jid});
                        },
                        .wa_qr => {
                            std.log.info("WhatsApp QR code generated - scan with phone", .{});
                        },
                        .disconnected => |d| {
                            if (d.source == .discord) self.discord_connected = false else self.wa_connected = false;
                            std.log.warn("{s} disconnected: {s}", .{ @tagName(d.source), d.reason });
                        },
                        .err => |e| {
                            std.log.err("{s} error: {s}", .{ @tagName(e.source), e.message });
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

    fn parseSource(source_str: []const u8) ?Source {
        if (std.mem.eql(u8, source_str, "discord")) return .discord;
        if (std.mem.eql(u8, source_str, "whatsapp")) return .whatsapp;
        return null;
    }

    fn parseEvent(self: *Sidecar, allocator: std.mem.Allocator, line: []const u8) ?SidecarEvent {
        _ = self;
        var parsed = json_mod.parse(allocator, line) catch return null;
        defer parsed.deinit();

        const source_str = json_mod.getString(parsed.value, "source") orelse return null;
        const event_type = json_mod.getString(parsed.value, "event") orelse return null;
        const source = parseSource(source_str) orelse return null;

        if (std.mem.eql(u8, event_type, "message")) {
            if (source == .discord) {
                return SidecarEvent{ .message = SidecarMessage{
                    .source = .discord,
                    .id = allocator.dupe(u8, json_mod.getString(parsed.value, "message_id") orelse "") catch return null,
                    .chat_id = allocator.dupe(u8, json_mod.getString(parsed.value, "channel_id") orelse "") catch return null,
                    .sender = allocator.dupe(u8, json_mod.getString(parsed.value, "sender_id") orelse "") catch return null,
                    .sender_name = allocator.dupe(u8, json_mod.getString(parsed.value, "sender_name") orelse "") catch return null,
                    .text = allocator.dupe(u8, json_mod.getString(parsed.value, "text") orelse "") catch return null,
                    .timestamp = json_mod.getInt(parsed.value, "timestamp") orelse std.time.timestamp(),
                    .is_group = !(json_mod.getBool(parsed.value, "is_dm") orelse false),
                    .mentions_bot = json_mod.getBool(parsed.value, "mentions_bot") orelse false,
                } };
            } else {
                return SidecarEvent{ .message = SidecarMessage{
                    .source = .whatsapp,
                    .id = allocator.dupe(u8, json_mod.getString(parsed.value, "id") orelse "") catch return null,
                    .chat_id = allocator.dupe(u8, json_mod.getString(parsed.value, "jid") orelse "") catch return null,
                    .sender = allocator.dupe(u8, json_mod.getString(parsed.value, "sender") orelse "") catch return null,
                    .sender_name = allocator.dupe(u8, json_mod.getString(parsed.value, "sender_name") orelse "") catch return null,
                    .text = allocator.dupe(u8, json_mod.getString(parsed.value, "text") orelse "") catch return null,
                    .timestamp = json_mod.getInt(parsed.value, "timestamp") orelse std.time.timestamp(),
                    .is_group = json_mod.getBool(parsed.value, "is_group") orelse false,
                    .mentions_bot = json_mod.getBool(parsed.value, "mentions_bot") orelse false,
                } };
            }
        } else if (std.mem.eql(u8, event_type, "ready")) {
            return SidecarEvent{ .discord_ready = allocator.dupe(u8, json_mod.getString(parsed.value, "bot_id") orelse "") catch return null };
        } else if (std.mem.eql(u8, event_type, "connected")) {
            return SidecarEvent{ .wa_connected = allocator.dupe(u8, json_mod.getString(parsed.value, "jid") orelse "") catch return null };
        } else if (std.mem.eql(u8, event_type, "qr")) {
            return SidecarEvent{ .wa_qr = allocator.dupe(u8, json_mod.getString(parsed.value, "data") orelse "") catch return null };
        } else if (std.mem.eql(u8, event_type, "disconnected")) {
            return SidecarEvent{ .disconnected = .{
                .source = source,
                .reason = allocator.dupe(u8, json_mod.getString(parsed.value, "reason") orelse "") catch return null,
            } };
        } else if (std.mem.eql(u8, event_type, "error")) {
            return SidecarEvent{ .err = .{
                .source = source,
                .message = allocator.dupe(u8, json_mod.getString(parsed.value, "message") orelse "") catch return null,
            } };
        }

        return null;
    }

    pub fn sendDiscord(self: *Sidecar, channel_id: []const u8, text: []const u8, reply_to: ?[]const u8) !void {
        const child = &(self.child orelse return error.NotConnected);
        const stdin = child.stdin orelse return error.NotConnected;

        var buf = std.ArrayList(u8).init(self.allocator);
        defer buf.deinit();
        const w = buf.writer();

        const esc_text = try json_mod.escapeString(self.allocator, text);
        defer self.allocator.free(esc_text);

        try w.print("{{\"target\":\"discord\",\"cmd\":\"send\",\"channel_id\":\"{s}\",\"text\":\"{s}\"", .{ channel_id, esc_text });
        if (reply_to) |rid| {
            try w.print(",\"reply_to\":\"{s}\"", .{rid});
        }
        try w.writeAll("}\n");
        try stdin.writeAll(buf.items);
    }

    pub fn sendWhatsApp(self: *Sidecar, jid: []const u8, text: []const u8, quote_id: ?[]const u8) !void {
        const child = &(self.child orelse return error.NotConnected);
        const stdin = child.stdin orelse return error.NotConnected;

        var buf = std.ArrayList(u8).init(self.allocator);
        defer buf.deinit();
        const w = buf.writer();

        const esc_text = try json_mod.escapeString(self.allocator, text);
        defer self.allocator.free(esc_text);

        try w.print("{{\"target\":\"whatsapp\",\"cmd\":\"send\",\"jid\":\"{s}\",\"text\":\"{s}\"", .{ jid, esc_text });
        if (quote_id) |qid| {
            try w.print(",\"quote_id\":\"{s}\"", .{qid});
        }
        try w.writeAll("}\n");
        try stdin.writeAll(buf.items);
    }

    pub fn sendDiscordTyping(self: *Sidecar, channel_id: []const u8) !void {
        const child = &(self.child orelse return error.NotConnected);
        const stdin = child.stdin orelse return error.NotConnected;

        var buf: [256]u8 = undefined;
        const cmd = try std.fmt.bufPrint(&buf, "{{\"target\":\"discord\",\"cmd\":\"typing\",\"channel_id\":\"{s}\"}}\n", .{channel_id});
        try stdin.writeAll(cmd);
    }

    pub fn sendWhatsAppTyping(self: *Sidecar, jid: []const u8) !void {
        const child = &(self.child orelse return error.NotConnected);
        const stdin = child.stdin orelse return error.NotConnected;

        var buf: [256]u8 = undefined;
        const cmd = try std.fmt.bufPrint(&buf, "{{\"target\":\"whatsapp\",\"cmd\":\"typing\",\"jid\":\"{s}\"}}\n", .{jid});
        try stdin.writeAll(cmd);
    }
};

// ── Tests ──────────────────────────────────────────────────────────────

test "Sidecar init/deinit" {
    var s = Sidecar.init(std.testing.allocator, "Borg");
    defer s.deinit();
    try std.testing.expect(!s.discord_connected);
    try std.testing.expect(!s.wa_connected);
    try std.testing.expect(s.child == null);
}

test "parseSource" {
    try std.testing.expectEqual(Source.discord, Sidecar.parseSource("discord").?);
    try std.testing.expectEqual(Source.whatsapp, Sidecar.parseSource("whatsapp").?);
    try std.testing.expect(Sidecar.parseSource("unknown") == null);
}
