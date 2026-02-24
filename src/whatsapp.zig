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
            &.{ "node", "whatsapp/bridge.js", self.assistant_name },
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
