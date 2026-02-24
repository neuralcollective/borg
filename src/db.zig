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
