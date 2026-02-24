const std = @import("std");
const json_mod = @import("json.zig");

pub const WaMessage = struct {
    jid: []const u8,
    id: []const u8,
    sender: []const u8,
    sender_name: []const u8,
    text: []const u8,
    timestamp: i64,
    is_group: bool,
    mentions_bot: bool,
};

pub const WaEvent = union(enum) {
    message: WaMessage,
    connected: []const u8,
    qr: []const u8,
    disconnected: []const u8,
    err: []const u8,
};

pub const WhatsApp = struct {
    allocator: std.mem.Allocator,
    child: ?std.process.Child,
    stdout_buf: std.ArrayList(u8),
    pending_events: std.ArrayList(WaEvent),
    connected: bool,
    self_jid: []const u8,

    assistant_name: []const u8,

    pub fn init(allocator: std.mem.Allocator, assistant_name: []const u8) WhatsApp {
        return .{
            .allocator = allocator,
            .child = null,
            .stdout_buf = std.ArrayList(u8).init(allocator),
            .pending_events = std.ArrayList(WaEvent).init(allocator),
            .connected = false,
            .self_jid = "",
            .assistant_name = assistant_name,
        };
    }

    pub fn start(self: *WhatsApp) !void {
        var child = std.process.Child.init(
            &.{ "bun", "whatsapp/bridge.js", self.assistant_name },
            self.allocator,
        );
        child.stdin_behavior = .Pipe;
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Pipe;
        try child.spawn();
        self.child = child;
        std.log.info("WhatsApp bridge process started", .{});
    }

    pub fn deinit(self: *WhatsApp) void {
        if (self.child) |*c| {
            if (c.stdin) |stdin| {
                stdin.close();
                c.stdin = null;
            }
            _ = c.kill() catch {};
        }
        self.stdout_buf.deinit();
        self.pending_events.deinit();
    }

    /// Read available events from the bridge process (non-blocking).
    pub fn poll(self: *WhatsApp, allocator: std.mem.Allocator) ![]WaMessage {
        const child = &(self.child orelse return &[_]WaMessage{});
        const stdout = child.stdout orelse return &[_]WaMessage{};

        // Read available data
        var read_buf: [4096]u8 = undefined;
        while (true) {
            const n = stdout.read(&read_buf) catch break;
            if (n == 0) break;
            try self.stdout_buf.appendSlice(read_buf[0..n]);
            if (n < read_buf.len) break;
        }

        // Parse complete lines
        var messages = std.ArrayList(WaMessage).init(allocator);
        while (std.mem.indexOf(u8, self.stdout_buf.items, "\n")) |nl| {
            const line = self.stdout_buf.items[0..nl];
            if (line.len > 0) {
                if (self.parseEvent(allocator, line)) |event| {
                    switch (event) {
                        .message => |msg| try messages.append(msg),
                        .connected => |jid| {
                            self.connected = true;
                            self.self_jid = jid;
                            std.log.info("WhatsApp connected as {s}", .{jid});
                        },
                        .qr => |_| {
                            std.log.info("WhatsApp QR code generated - scan with phone", .{});
                        },
                        .disconnected => |reason| {
                            self.connected = false;
                            std.log.warn("WhatsApp disconnected: {s}", .{reason});
                        },
                        .err => |msg| {
                            std.log.err("WhatsApp bridge error: {s}", .{msg});
                        },
                    }
                }
            }
            // Remove processed line + newline
            const remaining = self.stdout_buf.items[nl + 1 ..];
            std.mem.copyForwards(u8, self.stdout_buf.items[0..remaining.len], remaining);
            self.stdout_buf.shrinkRetainingCapacity(remaining.len);
        }

        return messages.toOwnedSlice();
    }

    fn parseEvent(self: *WhatsApp, allocator: std.mem.Allocator, line: []const u8) ?WaEvent {
        _ = self;
        var parsed = json_mod.parse(allocator, line) catch return null;
        defer parsed.deinit();

        const event_type = json_mod.getString(parsed.value, "event") orelse return null;

        if (std.mem.eql(u8, event_type, "message")) {
            return WaEvent{ .message = WaMessage{
                .jid = allocator.dupe(u8, json_mod.getString(parsed.value, "jid") orelse "") catch return null,
                .id = allocator.dupe(u8, json_mod.getString(parsed.value, "id") orelse "") catch return null,
                .sender = allocator.dupe(u8, json_mod.getString(parsed.value, "sender") orelse "") catch return null,
                .sender_name = allocator.dupe(u8, json_mod.getString(parsed.value, "sender_name") orelse "") catch return null,
                .text = allocator.dupe(u8, json_mod.getString(parsed.value, "text") orelse "") catch return null,
                .timestamp = json_mod.getInt(parsed.value, "timestamp") orelse std.time.timestamp(),
                .is_group = json_mod.getBool(parsed.value, "is_group") orelse false,
                .mentions_bot = json_mod.getBool(parsed.value, "mentions_bot") orelse false,
            } };
        } else if (std.mem.eql(u8, event_type, "connected")) {
            return WaEvent{ .connected = allocator.dupe(u8, json_mod.getString(parsed.value, "jid") orelse "") catch return null };
        } else if (std.mem.eql(u8, event_type, "qr")) {
            return WaEvent{ .qr = allocator.dupe(u8, json_mod.getString(parsed.value, "data") orelse "") catch return null };
        } else if (std.mem.eql(u8, event_type, "disconnected")) {
            return WaEvent{ .disconnected = allocator.dupe(u8, json_mod.getString(parsed.value, "reason") orelse "") catch return null };
        } else if (std.mem.eql(u8, event_type, "error")) {
            return WaEvent{ .err = allocator.dupe(u8, json_mod.getString(parsed.value, "message") orelse "") catch return null };
        }

        return null;
    }

    pub fn sendMessage(self: *WhatsApp, jid: []const u8, text: []const u8, quote_id: ?[]const u8) !void {
        const child = &(self.child orelse return error.NotConnected);
        const stdin = child.stdin orelse return error.NotConnected;

        var buf = std.ArrayList(u8).init(self.allocator);
        defer buf.deinit();
        const w = buf.writer();

        const esc_text = try json_mod.escapeString(self.allocator, text);
        defer self.allocator.free(esc_text);

        try w.print("{{\"cmd\":\"send\",\"jid\":\"{s}\",\"text\":\"{s}\"", .{ jid, esc_text });
        if (quote_id) |qid| {
            try w.print(",\"quote_id\":\"{s}\"", .{qid});
        }
        try w.writeAll("}\n");

        try stdin.writeAll(buf.items);
    }

    pub fn sendTyping(self: *WhatsApp, jid: []const u8) !void {
        const child = &(self.child orelse return error.NotConnected);
        const stdin = child.stdin orelse return error.NotConnected;

        var buf: [256]u8 = undefined;
        const cmd = try std.fmt.bufPrint(&buf, "{{\"cmd\":\"typing\",\"jid\":\"{s}\"}}\n", .{jid});
        try stdin.writeAll(cmd);
    }

    pub const WhatsAppError = error{
        NotConnected,
        OutOfMemory,
        BrokenPipe,
        DiskQuota,
        FileTooBig,
        InputOutput,
        NoSpaceLeft,
        DeviceBusy,
        InvalidArgument,
        OperationAborted,
        NotOpenForWriting,
        LockViolation,
        WouldBlock,
        ConnectionResetByPeer,
        Unexpected,
        AccessDenied,
    };
};

// ── Tests ──────────────────────────────────────────────────────────────

test "WhatsApp init/deinit" {
    var wa = WhatsApp.init(std.testing.allocator, "Borg");
    defer wa.deinit();
    try std.testing.expect(!wa.connected);
    try std.testing.expect(wa.child == null);
}

test "WaEvent union size" {
    // Just verify the types compile correctly
    const event = WaEvent{ .connected = "test" };
    switch (event) {
        .connected => |jid| try std.testing.expectEqualStrings("test", jid),
        else => unreachable,
    }
}

// ── Non-blocking pipe tests (spec: stdout blocking fix) ──────────────

/// Helper: create a WhatsApp with a real pipe injected as child stdout.
fn testWhatsAppWithPipe(allocator: std.mem.Allocator) !struct { wa: WhatsApp, write_fd: std.posix.fd_t } {
    var wa = WhatsApp.init(allocator, "TestBot");
    const pipe_fds = try std.posix.pipe();
    var child = std.process.Child.init(&.{"true"}, allocator);
    child.stdout = std.fs.File{ .handle = pipe_fds[0] };
    wa.child = child;
    return .{ .wa = wa, .write_fd = pipe_fds[1] };
}

test "WhatsApp stdout pipe has O_NONBLOCK after start" {
    // AC2: After WhatsApp.start(), the stdout pipe fd must have O_NONBLOCK set.
    // This test FAILS until the O_NONBLOCK fix is applied to start().
    const allocator = std.testing.allocator;
    var result = try testWhatsAppWithPipe(allocator);
    defer {
        std.posix.close(result.write_fd);
        if (result.wa.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
        result.wa.child = null;
        result.wa.deinit();
    }

    const stdout_fd = result.wa.child.?.stdout.?.handle;
    const flags = std.posix.fcntl(stdout_fd, .GET_FL) catch 0;
    const nonblock_flag = @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true }));

    // Must have O_NONBLOCK set after start(). FAILS before implementation.
    try std.testing.expect((@as(u32, @intCast(flags)) & nonblock_flag) != 0);
}

test "WhatsApp poll returns immediately when no data available" {
    // AC3 (whatsapp): poll() returns empty within bounded time.
    const allocator = std.testing.allocator;
    var result = try testWhatsAppWithPipe(allocator);
    defer {
        std.posix.close(result.write_fd);
        if (result.wa.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
        result.wa.child = null;
        result.wa.deinit();
    }

    // Set O_NONBLOCK (simulating what start() should do)
    const stdout_fd = result.wa.child.?.stdout.?.handle;
    const current_flags = std.posix.fcntl(stdout_fd, .GET_FL) catch 0;
    const nonblock_val = @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true }));
    _ = std.posix.fcntl(stdout_fd, .SET_FL, .{ .flags = @as(u32, @intCast(current_flags)) | nonblock_val }) catch {};

    const before = std.time.nanoTimestamp();
    const msgs = try result.wa.poll(allocator);
    const elapsed_ns = std.time.nanoTimestamp() - before;

    try std.testing.expectEqual(@as(usize, 0), msgs.len);
    try std.testing.expect(elapsed_ns < 10 * std.time.ns_per_ms);
}

test "WhatsApp poll reads NDJSON data correctly" {
    // AC4 (whatsapp): poll() parses NDJSON into WaMessage slices.
    const allocator = std.testing.allocator;
    var result = try testWhatsAppWithPipe(allocator);
    defer {
        std.posix.close(result.write_fd);
        if (result.wa.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
        result.wa.child = null;
        result.wa.deinit();
    }

    // Set O_NONBLOCK
    const stdout_fd = result.wa.child.?.stdout.?.handle;
    const current_flags = std.posix.fcntl(stdout_fd, .GET_FL) catch 0;
    const nonblock_val = @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true }));
    _ = std.posix.fcntl(stdout_fd, .SET_FL, .{ .flags = @as(u32, @intCast(current_flags)) | nonblock_val }) catch {};

    const json_line = "{\"event\":\"message\",\"jid\":\"j1@s.whatsapp.net\",\"id\":\"msg1\",\"sender\":\"s1\",\"sender_name\":\"Alice\",\"text\":\"hello\",\"timestamp\":1700000000,\"is_group\":true,\"mentions_bot\":false}\n";
    _ = try std.posix.write(result.write_fd, json_line);

    const msgs = try result.wa.poll(allocator);
    defer allocator.free(msgs);

    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expectEqualStrings("j1@s.whatsapp.net", msgs[0].jid);
    try std.testing.expectEqualStrings("Alice", msgs[0].sender_name);
    try std.testing.expectEqualStrings("hello", msgs[0].text);
    try std.testing.expect(msgs[0].is_group);
    try std.testing.expect(!msgs[0].mentions_bot);

    for (msgs) |m| {
        allocator.free(m.jid);
        allocator.free(m.id);
        allocator.free(m.sender);
        allocator.free(m.sender_name);
        allocator.free(m.text);
    }
}

test "WhatsApp poll handles partial NDJSON lines" {
    // Edge case 3: partial line buffered until next poll.
    const allocator = std.testing.allocator;
    var result = try testWhatsAppWithPipe(allocator);
    defer {
        std.posix.close(result.write_fd);
        if (result.wa.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
        result.wa.child = null;
        result.wa.deinit();
    }

    // Set O_NONBLOCK
    const stdout_fd = result.wa.child.?.stdout.?.handle;
    const current_flags = std.posix.fcntl(stdout_fd, .GET_FL) catch 0;
    const nonblock_val = @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true }));
    _ = std.posix.fcntl(stdout_fd, .SET_FL, .{ .flags = @as(u32, @intCast(current_flags)) | nonblock_val }) catch {};

    // Write partial JSON (no newline)
    _ = try std.posix.write(result.write_fd, "{\"event\":\"message\"");

    const msgs1 = try result.wa.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), msgs1.len);
    try std.testing.expect(result.wa.stdout_buf.items.len > 0);

    // Complete the line
    _ = try std.posix.write(result.write_fd, ",\"jid\":\"j1\",\"id\":\"m1\",\"sender\":\"s1\",\"sender_name\":\"Bob\",\"text\":\"hi\",\"timestamp\":1700000000,\"is_group\":false,\"mentions_bot\":true}\n");

    const msgs2 = try result.wa.poll(allocator);
    defer allocator.free(msgs2);

    try std.testing.expectEqual(@as(usize, 1), msgs2.len);
    try std.testing.expectEqualStrings("Bob", msgs2[0].sender_name);

    for (msgs2) |m| {
        allocator.free(m.jid);
        allocator.free(m.id);
        allocator.free(m.sender);
        allocator.free(m.sender_name);
        allocator.free(m.text);
    }
}

test "WhatsApp poll with null child returns empty" {
    // Edge case 6: null child handled gracefully.
    const allocator = std.testing.allocator;
    var wa = WhatsApp.init(allocator, "TestBot");
    defer wa.deinit();

    const msgs = try wa.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), msgs.len);
}

test "WhatsApp poll handles multiple lines in burst" {
    // Edge case 5: multiple NDJSON lines written at once.
    const allocator = std.testing.allocator;
    var result = try testWhatsAppWithPipe(allocator);
    defer {
        std.posix.close(result.write_fd);
        if (result.wa.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
        result.wa.child = null;
        result.wa.deinit();
    }

    // Set O_NONBLOCK
    const stdout_fd = result.wa.child.?.stdout.?.handle;
    const current_flags = std.posix.fcntl(stdout_fd, .GET_FL) catch 0;
    const nonblock_val = @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true }));
    _ = std.posix.fcntl(stdout_fd, .SET_FL, .{ .flags = @as(u32, @intCast(current_flags)) | nonblock_val }) catch {};

    const line1 = "{\"event\":\"message\",\"jid\":\"j1\",\"id\":\"m1\",\"sender\":\"s1\",\"sender_name\":\"A\",\"text\":\"first\",\"timestamp\":1700000000,\"is_group\":false,\"mentions_bot\":false}\n";
    const line2 = "{\"event\":\"message\",\"jid\":\"j2\",\"id\":\"m2\",\"sender\":\"s2\",\"sender_name\":\"B\",\"text\":\"second\",\"timestamp\":1700000001,\"is_group\":true,\"mentions_bot\":true}\n";
    _ = try std.posix.write(result.write_fd, line1 ++ line2);

    const msgs = try result.wa.poll(allocator);
    defer allocator.free(msgs);

    try std.testing.expectEqual(@as(usize, 2), msgs.len);
    try std.testing.expectEqualStrings("first", msgs[0].text);
    try std.testing.expectEqualStrings("second", msgs[1].text);

    for (msgs) |m| {
        allocator.free(m.jid);
        allocator.free(m.id);
        allocator.free(m.sender);
        allocator.free(m.sender_name);
        allocator.free(m.text);
    }
}

test "WhatsApp rapid successive polls with no data return empty" {
    // Edge case 4: successive polls all return empty immediately.
    const allocator = std.testing.allocator;
    var result = try testWhatsAppWithPipe(allocator);
    defer {
        std.posix.close(result.write_fd);
        if (result.wa.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
        result.wa.child = null;
        result.wa.deinit();
    }

    // Set O_NONBLOCK
    const stdout_fd = result.wa.child.?.stdout.?.handle;
    const current_flags = std.posix.fcntl(stdout_fd, .GET_FL) catch 0;
    const nonblock_val = @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true }));
    _ = std.posix.fcntl(stdout_fd, .SET_FL, .{ .flags = @as(u32, @intCast(current_flags)) | nonblock_val }) catch {};

    var i: usize = 0;
    while (i < 10) : (i += 1) {
        const msgs = try result.wa.poll(allocator);
        try std.testing.expectEqual(@as(usize, 0), msgs.len);
    }
}
