const std = @import("std");
const http = @import("http.zig");
const json = @import("json.zig");

pub const TgMessage = struct {
    message_id: []const u8,
    chat_id: []const u8,
    chat_type: []const u8,
    chat_title: []const u8,
    sender_id: []const u8,
    sender_name: []const u8,
    text: []const u8,
    date: i64,
    mentions_bot: bool,
    reply_to_text: ?[]const u8,
    reply_to_author: ?[]const u8,
};

pub const Telegram = struct {
    token: []const u8,
    bot_username: []const u8,
    allocator: std.mem.Allocator,
    last_update_id: i64,

    pub fn init(allocator: std.mem.Allocator, token: []const u8) Telegram {
        return Telegram{
            .token = token,
            .bot_username = "",
            .allocator = allocator,
            .last_update_id = 0,
        };
    }

    fn apiUrl(self: *Telegram, buf: []u8, method: []const u8) ![]const u8 {
        return std.fmt.bufPrint(buf, "https://api.telegram.org/bot{s}/{s}", .{ self.token, method });
    }

    pub fn connect(self: *Telegram) !void {
        var url_buf: [512]u8 = undefined;
        const url = try self.apiUrl(&url_buf, "getMe");
        var resp = try http.get(self.allocator, url);
        defer resp.deinit();

        var parsed = try json.parse(self.allocator, resp.body);
        defer parsed.deinit();

        if (json.getObject(parsed.value, "result")) |result| {
            if (json.getString(result, "username")) |username| {
                self.bot_username = try self.allocator.dupe(u8, username);
                std.log.info("Telegram bot connected: @{s}", .{self.bot_username});
            }
        }
    }

    /// Poll for new messages. Returned TgMessages are allocated with `alloc`.
    pub fn getUpdates(self: *Telegram, alloc: std.mem.Allocator) ![]TgMessage {
        var url_buf: [512]u8 = undefined;
        var params_buf: [256]u8 = undefined;
        const params = try std.fmt.bufPrint(&params_buf, "getUpdates?timeout=30&offset={d}&allowed_updates=[\"message\"]", .{self.last_update_id + 1});
        const url = try self.apiUrl(&url_buf, params);

        var resp = try http.get(self.allocator, url);
        defer resp.deinit();

        if (resp.status != .ok) return &.{};

        var parsed = try json.parse(self.allocator, resp.body);
        defer parsed.deinit();

        const result_array = json.getArray(parsed.value, "result") orelse return &.{};
        var messages = std.ArrayList(TgMessage).init(alloc);

        for (result_array) |update| {
            const update_id = json.getInt(update, "update_id") orelse continue;
            if (update_id > self.last_update_id) {
                self.last_update_id = update_id;
            }

            const msg_obj = json.getObject(update, "message") orelse continue;
            const text = json.getString(msg_obj, "text") orelse continue;
            const chat_obj = json.getObject(msg_obj, "chat") orelse continue;
            const from_obj = json.getObject(msg_obj, "from") orelse continue;

            const chat_id_int = json.getInt(chat_obj, "id") orelse continue;
            const sender_id_int = json.getInt(from_obj, "id") orelse continue;
            const msg_id_int = json.getInt(msg_obj, "message_id") orelse continue;
            const date = json.getInt(msg_obj, "date") orelse 0;

            var chat_id_buf: [32]u8 = undefined;
            const chat_id_str = try std.fmt.bufPrint(&chat_id_buf, "{d}", .{chat_id_int});
            var sender_id_buf: [32]u8 = undefined;
            const sender_id_str = try std.fmt.bufPrint(&sender_id_buf, "{d}", .{sender_id_int});
            var msg_id_buf: [32]u8 = undefined;
            const msg_id_str = try std.fmt.bufPrint(&msg_id_buf, "{d}", .{msg_id_int});

            const chat_type = json.getString(chat_obj, "type") orelse "private";
            const chat_title = json.getString(chat_obj, "title") orelse
                json.getString(from_obj, "first_name") orelse "Unknown";
            const sender_name = json.getString(from_obj, "first_name") orelse
                json.getString(from_obj, "username") orelse "Unknown";

            // Check @bot_username mentions
            var mentions_bot = false;
            if (json.getArray(msg_obj, "entities")) |entities| {
                for (entities) |entity| {
                    if (json.getString(entity, "type")) |etype| {
                        if (std.mem.eql(u8, etype, "mention")) {
                            const offset: usize = @intCast(json.getInt(entity, "offset") orelse continue);
                            const length: usize = @intCast(json.getInt(entity, "length") orelse continue);
                            if (offset + length <= text.len and length > 1) {
                                const mention = text[offset + 1 .. offset + length];
                                if (std.ascii.eqlIgnoreCase(mention, self.bot_username)) {
                                    mentions_bot = true;
                                }
                            }
                        }
                    }
                }
            }

            // Reply context
            var reply_to_text: ?[]const u8 = null;
            var reply_to_author: ?[]const u8 = null;
            if (json.getObject(msg_obj, "reply_to_message")) |reply_msg| {
                reply_to_text = json.getString(reply_msg, "text");
                if (json.getObject(reply_msg, "from")) |reply_from| {
                    reply_to_author = json.getString(reply_from, "first_name");
                }
            }

            try messages.append(TgMessage{
                .message_id = try alloc.dupe(u8, msg_id_str),
                .chat_id = try alloc.dupe(u8, chat_id_str),
                .chat_type = try alloc.dupe(u8, chat_type),
                .chat_title = try alloc.dupe(u8, chat_title),
                .sender_id = try alloc.dupe(u8, sender_id_str),
                .sender_name = try alloc.dupe(u8, sender_name),
                .text = try alloc.dupe(u8, text),
                .date = date,
                .mentions_bot = mentions_bot,
                .reply_to_text = if (reply_to_text) |t| try alloc.dupe(u8, t) else null,
                .reply_to_author = if (reply_to_author) |a| try alloc.dupe(u8, a) else null,
            });
        }

        return messages.toOwnedSlice();
    }

    pub fn sendMessage(self: *Telegram, chat_id: []const u8, text: []const u8, reply_to: ?[]const u8) !void {
        const max_len = 4000;
        var offset: usize = 0;
        while (offset < text.len) {
            const end = @min(offset + max_len, text.len);
            const chunk = text[offset..end];
            try self.sendSingleMessage(chat_id, chunk, if (offset == 0) reply_to else null);
            offset = end;
        }
    }

    fn sendSingleMessage(self: *Telegram, chat_id: []const u8, text: []const u8, reply_to: ?[]const u8) !void {
        var url_buf: [512]u8 = undefined;
        const url = try self.apiUrl(&url_buf, "sendMessage");

        const escaped = try json.escapeString(self.allocator, text);
        defer self.allocator.free(escaped);

        var body = std.ArrayList(u8).init(self.allocator);
        defer body.deinit();

        try body.writer().print("{{\"chat_id\":{s},\"text\":\"{s}\",\"parse_mode\":\"Markdown\"", .{ chat_id, escaped });
        if (reply_to) |rid| {
            try body.writer().print(",\"reply_to_message_id\":{s}", .{rid});
        }
        try body.appendSlice("}");

        var resp = try http.postJson(self.allocator, url, body.items);
        defer resp.deinit();

        if (resp.status != .ok) {
            body.clearRetainingCapacity();
            try body.writer().print("{{\"chat_id\":{s},\"text\":\"{s}\"", .{ chat_id, escaped });
            if (reply_to) |rid| {
                try body.writer().print(",\"reply_to_message_id\":{s}", .{rid});
            }
            try body.appendSlice("}");
            var resp2 = try http.postJson(self.allocator, url, body.items);
            defer resp2.deinit();
        }
    }

    pub fn sendTyping(self: *Telegram, chat_id: []const u8) !void {
        var url_buf: [512]u8 = undefined;
        const url = try self.apiUrl(&url_buf, "sendChatAction");
        var body_buf: [256]u8 = undefined;
        const body = try std.fmt.bufPrint(&body_buf, "{{\"chat_id\":{s},\"action\":\"typing\"}}", .{chat_id});
        var resp = try http.postJson(self.allocator, url, body);
        defer resp.deinit();
    }
};
