const std = @import("std");
const sqlite = @import("sqlite.zig");

pub const RegisteredGroup = struct {
    jid: []const u8,
    name: []const u8,
    folder: []const u8,
    trigger: []const u8,
    requires_trigger: bool,
};

pub const Message = struct {
    id: []const u8,
    chat_jid: []const u8,
    sender: []const u8,
    sender_name: []const u8,
    content: []const u8,
    timestamp: []const u8,
    is_from_me: bool,
};

pub const Session = struct {
    folder: []const u8,
    session_id: []const u8,
    created_at: []const u8,
};

pub const Db = struct {
    sqlite_db: sqlite.Database,
    allocator: std.mem.Allocator,

    pub fn init(allocator: std.mem.Allocator, path: [:0]const u8) !Db {
        var db = Db{
            .sqlite_db = try sqlite.Database.open(path),
            .allocator = allocator,
        };
        try db.migrate();
        return db;
    }

    pub fn deinit(self: *Db) void {
        self.sqlite_db.close();
    }

    fn migrate(self: *Db) !void {
        try self.sqlite_db.exec(
            \\CREATE TABLE IF NOT EXISTS registered_groups (
            \\  jid TEXT PRIMARY KEY,
            \\  name TEXT NOT NULL,
            \\  folder TEXT NOT NULL UNIQUE,
            \\  trigger_pattern TEXT DEFAULT '@Borg',
            \\  added_at TEXT DEFAULT (datetime('now')),
            \\  requires_trigger INTEGER DEFAULT 1
            \\);
        );
        try self.sqlite_db.exec(
            \\CREATE TABLE IF NOT EXISTS messages (
            \\  id TEXT NOT NULL,
            \\  chat_jid TEXT NOT NULL,
            \\  sender TEXT,
            \\  sender_name TEXT,
            \\  content TEXT NOT NULL,
            \\  timestamp TEXT NOT NULL,
            \\  is_from_me INTEGER DEFAULT 0,
            \\  is_bot_message INTEGER DEFAULT 0,
            \\  PRIMARY KEY (chat_jid, id)
            \\);
        );
        try self.sqlite_db.exec(
            \\CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(chat_jid, timestamp);
        );
        try self.sqlite_db.exec(
            \\CREATE TABLE IF NOT EXISTS sessions (
            \\  folder TEXT PRIMARY KEY,
            \\  session_id TEXT NOT NULL,
            \\  created_at TEXT DEFAULT (datetime('now'))
            \\);
        );
        try self.sqlite_db.exec(
            \\CREATE TABLE IF NOT EXISTS scheduled_tasks (
            \\  id INTEGER PRIMARY KEY AUTOINCREMENT,
            \\  chat_jid TEXT NOT NULL,
            \\  description TEXT NOT NULL,
            \\  cron_expr TEXT NOT NULL,
            \\  next_run TEXT,
            \\  last_run TEXT,
            \\  enabled INTEGER DEFAULT 1
            \\);
        );
        try self.sqlite_db.exec(
            \\CREATE TABLE IF NOT EXISTS state (
            \\  key TEXT PRIMARY KEY,
            \\  value TEXT NOT NULL
            \\);
        );
    }

    // --- Registered Groups ---

    pub fn getAllGroups(self: *Db, allocator: std.mem.Allocator) ![]RegisteredGroup {
        var rows = try self.sqlite_db.query(
            allocator,
            "SELECT jid, name, folder, trigger_pattern, requires_trigger FROM registered_groups",
            .{},
        );
        defer rows.deinit();

        var groups = std.ArrayList(RegisteredGroup).init(allocator);
        for (rows.items) |row| {
            try groups.append(RegisteredGroup{
                .jid = try allocator.dupe(u8, row.get(0) orelse ""),
                .name = try allocator.dupe(u8, row.get(1) orelse ""),
                .folder = try allocator.dupe(u8, row.get(2) orelse ""),
                .trigger = try allocator.dupe(u8, row.get(3) orelse "@Borg"),
                .requires_trigger = (row.getInt(4) orelse 1) == 1,
            });
        }
        return groups.toOwnedSlice();
    }

    pub fn registerGroup(self: *Db, jid: []const u8, name: []const u8, folder: []const u8, trigger: []const u8, requires_trigger: bool) !void {
        try self.sqlite_db.execute(
            "INSERT OR REPLACE INTO registered_groups (jid, name, folder, trigger_pattern, requires_trigger) VALUES (?1, ?2, ?3, ?4, ?5)",
            .{ jid, name, folder, trigger, @as(i64, if (requires_trigger) 1 else 0) },
        );
    }

    pub fn unregisterGroup(self: *Db, jid: []const u8) !void {
        try self.sqlite_db.execute(
            "DELETE FROM registered_groups WHERE jid = ?1",
            .{jid},
        );
    }

    // --- Messages ---

    pub fn storeMessage(self: *Db, msg: Message) !void {
        try self.sqlite_db.execute(
            "INSERT OR IGNORE INTO messages (id, chat_jid, sender, sender_name, content, timestamp, is_from_me, is_bot_message) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            .{
                msg.id,
                msg.chat_jid,
                msg.sender,
                msg.sender_name,
                msg.content,
                msg.timestamp,
                @as(i64, if (msg.is_from_me) 1 else 0),
                @as(i64, if (msg.is_from_me) 1 else 0),
            },
        );
    }

    pub fn getMessagesSince(self: *Db, allocator: std.mem.Allocator, chat_jid: []const u8, since: []const u8) ![]Message {
        var rows = try self.sqlite_db.query(
            allocator,
            "SELECT id, chat_jid, sender, sender_name, content, timestamp, is_from_me FROM messages WHERE chat_jid = ?1 AND timestamp > ?2 ORDER BY timestamp ASC LIMIT 50",
            .{ chat_jid, since },
        );
        defer rows.deinit();

        var messages = std.ArrayList(Message).init(allocator);
        for (rows.items) |row| {
            try messages.append(Message{
                .id = try allocator.dupe(u8, row.get(0) orelse ""),
                .chat_jid = try allocator.dupe(u8, row.get(1) orelse ""),
                .sender = try allocator.dupe(u8, row.get(2) orelse ""),
                .sender_name = try allocator.dupe(u8, row.get(3) orelse ""),
                .content = try allocator.dupe(u8, row.get(4) orelse ""),
                .timestamp = try allocator.dupe(u8, row.get(5) orelse ""),
                .is_from_me = (row.getInt(6) orelse 0) == 1,
            });
        }
        return messages.toOwnedSlice();
    }

    // --- Sessions ---

    pub fn getSession(self: *Db, allocator: std.mem.Allocator, folder: []const u8) !?[]const u8 {
        var rows = try self.sqlite_db.query(
            allocator,
            "SELECT session_id FROM sessions WHERE folder = ?1",
            .{folder},
        );
        defer rows.deinit();

        if (rows.items.len == 0) return null;
        return try allocator.dupe(u8, rows.items[0].get(0) orelse return null);
    }

    pub fn setSession(self: *Db, folder: []const u8, session_id: []const u8) !void {
        try self.sqlite_db.execute(
            "INSERT OR REPLACE INTO sessions (folder, session_id, created_at) VALUES (?1, ?2, datetime('now'))",
            .{ folder, session_id },
        );
    }

    pub fn expireSessions(self: *Db, max_age_hours: i64) !void {
        var buf: [64]u8 = undefined;
        const hours_str = std.fmt.bufPrint(&buf, "-{d} hours", .{max_age_hours}) catch return;
        try self.sqlite_db.execute(
            "DELETE FROM sessions WHERE created_at < datetime('now', ?1)",
            .{hours_str},
        );
    }

    // --- State ---

    pub fn getState(self: *Db, allocator: std.mem.Allocator, key: []const u8) !?[]const u8 {
        var rows = try self.sqlite_db.query(
            allocator,
            "SELECT value FROM state WHERE key = ?1",
            .{key},
        );
        defer rows.deinit();

        if (rows.items.len == 0) return null;
        return try allocator.dupe(u8, rows.items[0].get(0) orelse return null);
    }

    pub fn setState(self: *Db, key: []const u8, value: []const u8) !void {
        try self.sqlite_db.execute(
            "INSERT OR REPLACE INTO state (key, value) VALUES (?1, ?2)",
            .{ key, value },
        );
    }
};

// ── Tests ──────────────────────────────────────────────────────────────

test "group registration round trip" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("tg:123", "Test Group", "test-group", "@Bot", true);
    try db.registerGroup("tg:456", "Other", "other", "@Bot", false);

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 2), groups.len);
    try std.testing.expectEqualStrings("tg:123", groups[0].jid);
    try std.testing.expectEqualStrings("test-group", groups[0].folder);
    try std.testing.expect(groups[0].requires_trigger);
    try std.testing.expect(!groups[1].requires_trigger);
}

test "registerGroup upserts on conflict" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("tg:123", "Old Name", "folder", "@Bot", true);
    try db.registerGroup("tg:123", "New Name", "folder", "@Bot", false);

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    try std.testing.expectEqualStrings("New Name", groups[0].name);
    try std.testing.expect(!groups[0].requires_trigger);
}

test "message storage and timestamp filtering" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{ .id = "1", .chat_jid = "tg:1", .sender = "u1", .sender_name = "Alice", .content = "First", .timestamp = "2024-01-01T00:00:00Z", .is_from_me = false });
    try db.storeMessage(.{ .id = "2", .chat_jid = "tg:1", .sender = "u2", .sender_name = "Bob", .content = "Second", .timestamp = "2024-01-01T00:01:00Z", .is_from_me = false });
    try db.storeMessage(.{ .id = "3", .chat_jid = "tg:2", .sender = "u1", .sender_name = "Alice", .content = "Other chat", .timestamp = "2024-01-01T00:01:00Z", .is_from_me = false });

    // All messages for tg:1
    const all = try db.getMessagesSince(alloc, "tg:1", "");
    try std.testing.expectEqual(@as(usize, 2), all.len);
    try std.testing.expectEqualStrings("First", all[0].content);
    try std.testing.expectEqualStrings("Second", all[1].content);

    // Only messages after first timestamp
    const after = try db.getMessagesSince(alloc, "tg:1", "2024-01-01T00:00:00Z");
    try std.testing.expectEqual(@as(usize, 1), after.len);
    try std.testing.expectEqualStrings("Second", after[0].content);

    // Different chat isolation
    const other = try db.getMessagesSince(alloc, "tg:2", "");
    try std.testing.expectEqual(@as(usize, 1), other.len);
}

test "message deduplication preserves original" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{ .id = "1", .chat_jid = "tg:1", .sender = "u1", .sender_name = "A", .content = "Original", .timestamp = "2024-01-01T00:00:00Z", .is_from_me = false });
    try db.storeMessage(.{ .id = "1", .chat_jid = "tg:1", .sender = "u1", .sender_name = "A", .content = "Duplicate", .timestamp = "2024-01-01T00:00:00Z", .is_from_me = false });

    const msgs = try db.getMessagesSince(alloc, "tg:1", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expectEqualStrings("Original", msgs[0].content);
}

test "session set get and overwrite" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try std.testing.expect((try db.getSession(alloc, "folder1")) == null);

    try db.setSession("folder1", "session-aaa");
    try std.testing.expectEqualStrings("session-aaa", (try db.getSession(alloc, "folder1")).?);

    try db.setSession("folder1", "session-bbb");
    try std.testing.expectEqualStrings("session-bbb", (try db.getSession(alloc, "folder1")).?);
}

test "session expiry removes old sessions" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Insert old session via raw SQL (no params — exec doesn't bind)
    try db.sqlite_db.exec(
        "INSERT INTO sessions (folder, session_id, created_at) VALUES ('old', 'old-sess', datetime('now', '-25 hours'))"
    );
    try db.setSession("fresh", "fresh-sess");

    try std.testing.expect((try db.getSession(alloc, "old")) != null);
    try std.testing.expect((try db.getSession(alloc, "fresh")) != null);

    try db.expireSessions(4);

    try std.testing.expect((try db.getSession(alloc, "old")) == null);
    try std.testing.expect((try db.getSession(alloc, "fresh")) != null);
}

test "state key-value store" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try std.testing.expect((try db.getState(alloc, "k")) == null);
    try db.setState("k", "v1");
    try std.testing.expectEqualStrings("v1", (try db.getState(alloc, "k")).?);
    try db.setState("k", "v2");
    try std.testing.expectEqualStrings("v2", (try db.getState(alloc, "k")).?);
}
