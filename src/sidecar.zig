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
        child.stderr_behavior = .Pipe;

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
                            self.discord_bot_id = bot_id;
                            std.log.info("Discord connected as bot {s}", .{bot_id});
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

// Pull in whatsapp and nonblocking pipe tests so they are discovered by `zig build test`
test {
    _ = @import("whatsapp.zig");
    _ = @import("nonblocking_pipe_test.zig");
}

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

// ── Non-blocking pipe tests (spec: stdout blocking fix) ──────────────

/// Helper: create a Sidecar with a real pipe injected as child stdout.
/// Returns the sidecar and the write-end fd for feeding test data.
fn testSidecarWithPipe(allocator: std.mem.Allocator) !struct { sidecar: Sidecar, write_fd: std.posix.fd_t } {
    var s = Sidecar.init(allocator, "TestBot");
    const pipe_fds = try std.posix.pipe();
    // Use Child.init with a dummy command, then override stdout with our pipe
    var child = std.process.Child.init(&.{"true"}, allocator);
    child.stdout = std.fs.File{ .handle = pipe_fds[0] };
    s.child = child;
    return .{ .sidecar = s, .write_fd = pipe_fds[1] };
}

test "Sidecar stdout pipe has O_NONBLOCK after start" {
    // AC1: After Sidecar.start(), the stdout pipe fd must have O_NONBLOCK set.
    // We can't call start() (needs bun), so we verify the flag on an injected pipe
    // to test that the implementation sets it. This test will FAIL until the
    // O_NONBLOCK fix is applied to start().
    //
    // Strategy: create a pipe, inject it as child.stdout, then check flags.
    // The implementation should set O_NONBLOCK during start(). Since we bypass
    // start() here, we directly test that the fd does NOT have O_NONBLOCK
    // (proving our test harness works), then call the expected helper.
    const allocator = std.testing.allocator;
    var result = try testSidecarWithPipe(allocator);
    defer {
        std.posix.close(result.write_fd);
        if (result.sidecar.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
        result.sidecar.child = null;
        result.sidecar.deinit();
    }

    const stdout_fd = result.sidecar.child.?.stdout.?.handle;
    const flags = std.posix.fcntl(stdout_fd, .GET_FL) catch 0;
    const nonblock_flag = @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true }));

    // After start() with the fix applied, O_NONBLOCK must be set.
    // This FAILS before implementation because a fresh pipe is blocking.
    try std.testing.expect((@as(u32, @intCast(flags)) & nonblock_flag) != 0);
}

test "Sidecar poll returns immediately when no data available" {
    // AC3: poll() returns empty slice within bounded time (< 10ms)
    // when child has produced no output.
    const allocator = std.testing.allocator;
    var result = try testSidecarWithPipe(allocator);
    defer {
        std.posix.close(result.write_fd);
        if (result.sidecar.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
        result.sidecar.child = null;
        result.sidecar.deinit();
    }

    // Set O_NONBLOCK on the read end (simulating what start() should do)
    const stdout_fd = result.sidecar.child.?.stdout.?.handle;
    const current_flags = std.posix.fcntl(stdout_fd, .GET_FL) catch 0;
    const nonblock_val = @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true }));
    _ = std.posix.fcntl(stdout_fd, .SET_FL, .{ .flags = @as(u32, @intCast(current_flags)) | nonblock_val }) catch {};

    const before = std.time.nanoTimestamp();
    const msgs = try result.sidecar.poll(allocator);
    const elapsed_ns = std.time.nanoTimestamp() - before;

    // Must return empty and complete within 10ms
    try std.testing.expectEqual(@as(usize, 0), msgs.len);
    try std.testing.expect(elapsed_ns < 10 * std.time.ns_per_ms);
}

test "Sidecar poll reads NDJSON data correctly" {
    // AC4: When child writes NDJSON, poll() parses into SidecarMessage slices.
    const allocator = std.testing.allocator;
    var result = try testSidecarWithPipe(allocator);
    defer {
        std.posix.close(result.write_fd);
        if (result.sidecar.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
        result.sidecar.child = null;
        result.sidecar.deinit();
    }

    // Set O_NONBLOCK
    const stdout_fd = result.sidecar.child.?.stdout.?.handle;
    const current_flags = std.posix.fcntl(stdout_fd, .GET_FL) catch 0;
    const nonblock_val = @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true }));
    _ = std.posix.fcntl(stdout_fd, .SET_FL, .{ .flags = @as(u32, @intCast(current_flags)) | nonblock_val }) catch {};

    // Write a complete NDJSON message line to the pipe
    const json_line = "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"123\",\"channel_id\":\"ch1\",\"sender_id\":\"u1\",\"sender_name\":\"Alice\",\"text\":\"hello\",\"timestamp\":1700000000,\"is_dm\":false,\"mentions_bot\":true}\n";
    _ = try std.posix.write(result.write_fd, json_line);

    const msgs = try result.sidecar.poll(allocator);
    defer allocator.free(msgs);

    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expectEqualStrings("ch1", msgs[0].chat_id);
    try std.testing.expectEqualStrings("Alice", msgs[0].sender_name);
    try std.testing.expectEqualStrings("hello", msgs[0].text);
    try std.testing.expect(msgs[0].mentions_bot);
    try std.testing.expectEqual(Source.discord, msgs[0].source);

    // Free duped strings
    for (msgs) |m| {
        allocator.free(m.id);
        allocator.free(m.chat_id);
        allocator.free(m.sender);
        allocator.free(m.sender_name);
        allocator.free(m.text);
    }
}

test "Sidecar poll handles partial NDJSON lines" {
    // Edge case 3: partial line is buffered until next poll completes it.
    const allocator = std.testing.allocator;
    var result = try testSidecarWithPipe(allocator);
    defer {
        std.posix.close(result.write_fd);
        if (result.sidecar.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
        result.sidecar.child = null;
        result.sidecar.deinit();
    }

    // Set O_NONBLOCK
    const stdout_fd = result.sidecar.child.?.stdout.?.handle;
    const current_flags = std.posix.fcntl(stdout_fd, .GET_FL) catch 0;
    const nonblock_val = @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true }));
    _ = std.posix.fcntl(stdout_fd, .SET_FL, .{ .flags = @as(u32, @intCast(current_flags)) | nonblock_val }) catch {};

    // Write partial JSON (no newline)
    const partial = "{\"source\":\"whatsapp\",\"event\":\"message\"";
    _ = try std.posix.write(result.write_fd, partial);

    // First poll: no complete line, should return empty
    const msgs1 = try result.sidecar.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), msgs1.len);

    // Buffer should have the partial data
    try std.testing.expect(result.sidecar.stdout_buf.items.len > 0);

    // Write the rest including newline
    const rest = ",\"id\":\"m1\",\"jid\":\"j1\",\"sender\":\"s1\",\"sender_name\":\"Bob\",\"text\":\"hi\",\"timestamp\":1700000000,\"is_group\":true,\"mentions_bot\":false}\n";
    _ = try std.posix.write(result.write_fd, rest);

    // Second poll: now the line is complete
    const msgs2 = try result.sidecar.poll(allocator);
    defer allocator.free(msgs2);

    try std.testing.expectEqual(@as(usize, 1), msgs2.len);
    try std.testing.expectEqualStrings("Bob", msgs2[0].sender_name);
    try std.testing.expectEqual(Source.whatsapp, msgs2[0].source);

    for (msgs2) |m| {
        allocator.free(m.id);
        allocator.free(m.chat_id);
        allocator.free(m.sender);
        allocator.free(m.sender_name);
        allocator.free(m.text);
    }
}

test "Sidecar poll with null child returns empty" {
    // Edge case 6: null child/stdout handled gracefully.
    const allocator = std.testing.allocator;
    var s = Sidecar.init(allocator, "TestBot");
    defer s.deinit();

    // child is null
    const msgs = try s.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), msgs.len);
}

test "Sidecar poll handles multiple lines in burst" {
    // Edge case 5: large burst of data — multiple NDJSON lines in one read.
    const allocator = std.testing.allocator;
    var result = try testSidecarWithPipe(allocator);
    defer {
        std.posix.close(result.write_fd);
        if (result.sidecar.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
        result.sidecar.child = null;
        result.sidecar.deinit();
    }

    // Set O_NONBLOCK
    const stdout_fd = result.sidecar.child.?.stdout.?.handle;
    const current_flags = std.posix.fcntl(stdout_fd, .GET_FL) catch 0;
    const nonblock_val = @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true }));
    _ = std.posix.fcntl(stdout_fd, .SET_FL, .{ .flags = @as(u32, @intCast(current_flags)) | nonblock_val }) catch {};

    // Write two complete messages at once
    const line1 = "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"a1\",\"channel_id\":\"c1\",\"sender_id\":\"u1\",\"sender_name\":\"A\",\"text\":\"first\",\"timestamp\":1700000000,\"is_dm\":false,\"mentions_bot\":false}\n";
    const line2 = "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"a2\",\"channel_id\":\"c2\",\"sender_id\":\"u2\",\"sender_name\":\"B\",\"text\":\"second\",\"timestamp\":1700000001,\"is_dm\":false,\"mentions_bot\":true}\n";
    _ = try std.posix.write(result.write_fd, line1 ++ line2);

    const msgs = try result.sidecar.poll(allocator);
    defer allocator.free(msgs);

    try std.testing.expectEqual(@as(usize, 2), msgs.len);
    try std.testing.expectEqualStrings("first", msgs[0].text);
    try std.testing.expectEqualStrings("second", msgs[1].text);

    for (msgs) |m| {
        allocator.free(m.id);
        allocator.free(m.chat_id);
        allocator.free(m.sender);
        allocator.free(m.sender_name);
        allocator.free(m.text);
    }
}

test "Sidecar rapid successive polls with no data return empty" {
    // Edge case 4: successive polls with no data each return empty immediately.
    const allocator = std.testing.allocator;
    var result = try testSidecarWithPipe(allocator);
    defer {
        std.posix.close(result.write_fd);
        if (result.sidecar.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
        result.sidecar.child = null;
        result.sidecar.deinit();
    }

    // Set O_NONBLOCK
    const stdout_fd = result.sidecar.child.?.stdout.?.handle;
    const current_flags = std.posix.fcntl(stdout_fd, .GET_FL) catch 0;
    const nonblock_val = @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true }));
    _ = std.posix.fcntl(stdout_fd, .SET_FL, .{ .flags = @as(u32, @intCast(current_flags)) | nonblock_val }) catch {};

    // Poll multiple times rapidly — all should return empty, none should block
    var i: usize = 0;
    while (i < 10) : (i += 1) {
        const msgs = try result.sidecar.poll(allocator);
        try std.testing.expectEqual(@as(usize, 0), msgs.len);
    }
}
