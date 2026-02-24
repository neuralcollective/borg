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
    is_bot_message: bool,
};

pub const Session = struct {
    folder: []const u8,
    session_id: []const u8,
    created_at: []const u8,
};

pub const PipelineTask = struct {
    id: i64,
    title: []const u8,
    description: []const u8,
    repo_path: []const u8,
    branch: []const u8,
    status: []const u8,
    attempt: i64,
    max_attempts: i64,
    last_error: []const u8,
    created_by: []const u8,
    notify_chat: []const u8,
    created_at: []const u8,
    session_id: []const u8,
};

pub const QueueEntry = struct {
    id: i64,
    task_id: i64,
    branch: []const u8,
    repo_path: []const u8,
    status: []const u8,
    queued_at: []const u8,
};

// Must match the number of entries in runMigrations()
const SCHEMA_VERSION = "2";

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
        // Base schema (all CREATE IF NOT EXISTS — safe to rerun)
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
        try self.sqlite_db.exec(
            \\CREATE TABLE IF NOT EXISTS pipeline_tasks (
            \\  id INTEGER PRIMARY KEY AUTOINCREMENT,
            \\  title TEXT NOT NULL,
            \\  description TEXT NOT NULL,
            \\  repo_path TEXT NOT NULL,
            \\  branch TEXT DEFAULT '',
            \\  status TEXT NOT NULL DEFAULT 'backlog',
            \\  attempt INTEGER DEFAULT 0,
            \\  max_attempts INTEGER DEFAULT 3,
            \\  last_error TEXT DEFAULT '',
            \\  created_by TEXT DEFAULT '',
            \\  notify_chat TEXT DEFAULT '',
            \\  session_id TEXT DEFAULT '',
            \\  created_at TEXT DEFAULT (datetime('now')),
            \\  updated_at TEXT DEFAULT (datetime('now'))
            \\);
        );
        try self.sqlite_db.exec(
            \\CREATE INDEX IF NOT EXISTS idx_pipeline_status ON pipeline_tasks(status);
        );
        try self.sqlite_db.exec(
            \\CREATE TABLE IF NOT EXISTS integration_queue (
            \\  id INTEGER PRIMARY KEY AUTOINCREMENT,
            \\  task_id INTEGER NOT NULL,
            \\  branch TEXT NOT NULL,
            \\  repo_path TEXT DEFAULT '',
            \\  status TEXT DEFAULT 'queued',
            \\  error_msg TEXT DEFAULT '',
            \\  queued_at TEXT DEFAULT (datetime('now'))
            \\);
        );
        try self.sqlite_db.exec(
            \\CREATE TABLE IF NOT EXISTS task_outputs (
            \\  id INTEGER PRIMARY KEY AUTOINCREMENT,
            \\  task_id INTEGER NOT NULL,
            \\  phase TEXT NOT NULL,
            \\  output TEXT NOT NULL,
            \\  exit_code INTEGER DEFAULT 0,
            \\  created_at TEXT DEFAULT (datetime('now'))
            \\);
        );
        try self.sqlite_db.exec(
            \\CREATE INDEX IF NOT EXISTS idx_task_outputs_task ON task_outputs(task_id);
        );

        // For fresh installs, mark schema as current so ALTER migrations are skipped.
        try self.initSchemaVersion();
        // For existing databases, run any new ALTER TABLE migrations.
        try self.runMigrations();
    }

    /// On a truly fresh DB (empty state table), set schema_version to latest
    /// so ALTER migrations are skipped (the CREATE TABLE already has all columns).
    fn initSchemaVersion(self: *Db) !void {
        // Check if state table has ANY rows — if so, this is an existing DB
        var rows = try self.sqlite_db.query(
            self.allocator,
            "SELECT COUNT(*) FROM state",
            .{},
        );
        defer rows.deinit();
        const count = if (rows.items.len > 0)
            std.fmt.parseInt(usize, rows.items[0].get(0) orelse "0", 10) catch 0
        else
            0;
        if (count > 0) return; // existing DB, let runMigrations handle it

        try self.sqlite_db.execute(
            "INSERT INTO state (key, value) VALUES ('schema_version', ?1)",
            .{SCHEMA_VERSION},
        );
    }

    fn runMigrations(self: *Db) !void {
        // Always try all ALTER TABLEs — they're idempotent (duplicate column is caught).
        // This handles the case where schema_version was set but columns weren't actually added.
        const migrations = [_][*:0]const u8{
            "ALTER TABLE pipeline_tasks ADD COLUMN session_id TEXT DEFAULT ''",
            "ALTER TABLE integration_queue ADD COLUMN repo_path TEXT DEFAULT ''",
        };

        for (migrations, 1..) |sql, i| {
            self.sqlite_db.execQuiet(sql) catch {
                continue;
            };
            std.log.info("Applied migration {d}/{d}", .{ i, migrations.len });
        }

        var buf: [16]u8 = undefined;
        const ver_str = std.fmt.bufPrint(&buf, "{d}", .{migrations.len}) catch "0";
        try self.sqlite_db.execute(
            "INSERT OR REPLACE INTO state (key, value) VALUES ('schema_version', ?1)",
            .{ver_str},
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
                @as(i64, if (msg.is_bot_message) 1 else 0),
            },
        );
    }

    pub fn getMessagesSince(self: *Db, allocator: std.mem.Allocator, chat_jid: []const u8, since: []const u8) ![]Message {
        var rows = try self.sqlite_db.query(
            allocator,
            "SELECT id, chat_jid, sender, sender_name, content, timestamp, is_from_me, is_bot_message FROM messages WHERE chat_jid = ?1 AND timestamp > ?2 ORDER BY timestamp ASC LIMIT 50",
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
                .is_bot_message = (row.getInt(7) orelse 0) == 1,
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

    // --- Pipeline Tasks ---

    pub fn createPipelineTask(self: *Db, title: []const u8, description: []const u8, repo_path: []const u8, created_by: []const u8, notify_chat: []const u8) !i64 {
        try self.sqlite_db.execute(
            "INSERT INTO pipeline_tasks (title, description, repo_path, created_by, notify_chat) VALUES (?1, ?2, ?3, ?4, ?5)",
            .{ title, description, repo_path, created_by, notify_chat },
        );
        return self.sqlite_db.lastInsertRowId();
    }

    pub fn getNextPipelineTask(self: *Db, allocator: std.mem.Allocator) !?PipelineTask {
        // Priority: rebase > retry > impl > qa > spec > backlog
        var rows = try self.sqlite_db.query(
            allocator,
            \\SELECT id, title, description, repo_path, branch, status, attempt, max_attempts, last_error, created_by, notify_chat, created_at, COALESCE(session_id, '')
            \\FROM pipeline_tasks
            \\WHERE status IN ('backlog', 'spec', 'qa', 'impl', 'retry', 'rebase')
            \\ORDER BY
            \\  CASE status
            \\    WHEN 'rebase' THEN 0
            \\    WHEN 'retry' THEN 1
            \\    WHEN 'impl' THEN 2
            \\    WHEN 'qa' THEN 3
            \\    WHEN 'spec' THEN 4
            \\    WHEN 'backlog' THEN 5
            \\  END,
            \\  created_at ASC
            \\LIMIT 1
            ,
            .{},
        );
        defer rows.deinit();
        if (rows.items.len == 0) return null;
        return try rowToPipelineTask(allocator, rows.items[0]);
    }

    pub fn getActivePipelineTasks(self: *Db, allocator: std.mem.Allocator, limit: i64) ![]PipelineTask {
        var rows = try self.sqlite_db.query(
            allocator,
            \\SELECT id, title, description, repo_path, branch, status, attempt, max_attempts, last_error, created_by, notify_chat, created_at, COALESCE(session_id, '')
            \\FROM pipeline_tasks
            \\WHERE status IN ('backlog', 'spec', 'qa', 'impl', 'retry', 'rebase')
            \\ORDER BY
            \\  CASE status
            \\    WHEN 'rebase' THEN 0
            \\    WHEN 'retry' THEN 1
            \\    WHEN 'impl' THEN 2
            \\    WHEN 'qa' THEN 3
            \\    WHEN 'spec' THEN 4
            \\    WHEN 'backlog' THEN 5
            \\  END,
            \\  created_at ASC
            \\LIMIT ?1
            ,
            .{limit},
        );
        defer rows.deinit();

        var tasks = std.ArrayList(PipelineTask).init(allocator);
        for (rows.items) |row| {
            tasks.append(rowToPipelineTask(allocator, row) catch continue) catch continue;
        }
        return tasks.toOwnedSlice();
    }

    pub fn getPipelineTask(self: *Db, allocator: std.mem.Allocator, task_id: i64) !?PipelineTask {
        var rows = try self.sqlite_db.query(
            allocator,
            "SELECT id, title, description, repo_path, branch, status, attempt, max_attempts, last_error, created_by, notify_chat, created_at, COALESCE(session_id, '') FROM pipeline_tasks WHERE id = ?1",
            .{task_id},
        );
        defer rows.deinit();
        if (rows.items.len == 0) return null;
        return try rowToPipelineTask(allocator, rows.items[0]);
    }

    pub fn updateTaskStatus(self: *Db, task_id: i64, status: []const u8) !void {
        try self.sqlite_db.execute(
            "UPDATE pipeline_tasks SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
            .{ status, task_id },
        );
    }

    pub fn updateTaskBranch(self: *Db, task_id: i64, branch: []const u8) !void {
        try self.sqlite_db.execute(
            "UPDATE pipeline_tasks SET branch = ?1, updated_at = datetime('now') WHERE id = ?2",
            .{ branch, task_id },
        );
    }

    pub fn updateTaskError(self: *Db, task_id: i64, err_log: []const u8) !void {
        try self.sqlite_db.execute(
            "UPDATE pipeline_tasks SET last_error = ?1, updated_at = datetime('now') WHERE id = ?2",
            .{ err_log, task_id },
        );
    }

    pub fn setTaskSessionId(self: *Db, task_id: i64, session_id: []const u8) !void {
        try self.sqlite_db.execute(
            "UPDATE pipeline_tasks SET session_id = ?1, updated_at = datetime('now') WHERE id = ?2",
            .{ session_id, task_id },
        );
    }

    pub fn incrementTaskAttempt(self: *Db, task_id: i64) !void {
        try self.sqlite_db.execute(
            "UPDATE pipeline_tasks SET attempt = attempt + 1, updated_at = datetime('now') WHERE id = ?1",
            .{task_id},
        );
    }

    pub fn getAllPipelineTasks(self: *Db, allocator: std.mem.Allocator, limit: i64) ![]PipelineTask {
        var rows = try self.sqlite_db.query(
            allocator,
            "SELECT id, title, description, repo_path, branch, status, attempt, max_attempts, last_error, created_by, notify_chat, created_at, COALESCE(session_id, '') FROM pipeline_tasks ORDER BY created_at DESC LIMIT ?1",
            .{limit},
        );
        defer rows.deinit();

        var tasks = std.ArrayList(PipelineTask).init(allocator);
        for (rows.items) |row| {
            try tasks.append(try rowToPipelineTask(allocator, row));
        }
        return tasks.toOwnedSlice();
    }

    fn rowToPipelineTask(allocator: std.mem.Allocator, row: sqlite.Row) !PipelineTask {
        return PipelineTask{
            .id = row.getInt(0) orelse 0,
            .title = try allocator.dupe(u8, row.get(1) orelse ""),
            .description = try allocator.dupe(u8, row.get(2) orelse ""),
            .repo_path = try allocator.dupe(u8, row.get(3) orelse ""),
            .branch = try allocator.dupe(u8, row.get(4) orelse ""),
            .status = try allocator.dupe(u8, row.get(5) orelse "backlog"),
            .attempt = row.getInt(6) orelse 0,
            .max_attempts = row.getInt(7) orelse 3,
            .last_error = try allocator.dupe(u8, row.get(8) orelse ""),
            .created_by = try allocator.dupe(u8, row.get(9) orelse ""),
            .notify_chat = try allocator.dupe(u8, row.get(10) orelse ""),
            .created_at = try allocator.dupe(u8, row.get(11) orelse ""),
            .session_id = try allocator.dupe(u8, row.get(12) orelse ""),
        };
    }

    pub const PipelineStats = struct {
        active: i64,
        merged: i64,
        failed: i64,
        total: i64,
    };

    pub fn getPipelineStats(self: *Db) !PipelineStats {
        var total_rows = try self.sqlite_db.query(self.allocator, "SELECT COUNT(*) FROM pipeline_tasks", .{});
        defer total_rows.deinit();
        var active_rows = try self.sqlite_db.query(self.allocator, "SELECT COUNT(*) FROM pipeline_tasks WHERE status IN ('backlog', 'spec', 'qa', 'impl', 'retry', 'rebase')", .{});
        defer active_rows.deinit();
        var merged_rows = try self.sqlite_db.query(self.allocator, "SELECT COUNT(*) FROM pipeline_tasks WHERE status = 'merged'", .{});
        defer merged_rows.deinit();
        var failed_rows = try self.sqlite_db.query(self.allocator, "SELECT COUNT(*) FROM pipeline_tasks WHERE status = 'failed'", .{});
        defer failed_rows.deinit();

        return .{
            .total = if (total_rows.items.len > 0) total_rows.items[0].getInt(0) orelse 0 else 0,
            .active = if (active_rows.items.len > 0) active_rows.items[0].getInt(0) orelse 0 else 0,
            .merged = if (merged_rows.items.len > 0) merged_rows.items[0].getInt(0) orelse 0 else 0,
            .failed = if (failed_rows.items.len > 0) failed_rows.items[0].getInt(0) orelse 0 else 0,
        };
    }

    pub fn getActivePipelineTaskCount(self: *Db) !i64 {
        var rows = try self.sqlite_db.query(
            self.allocator,
            "SELECT COUNT(*) FROM pipeline_tasks WHERE status IN ('backlog', 'spec', 'qa', 'impl', 'retry', 'rebase')",
            .{},
        );
        defer rows.deinit();
        if (rows.items.len == 0) return 0;
        return rows.items[0].getInt(0) orelse 0;
    }

    // --- Integration Queue ---

    pub fn enqueueForIntegration(self: *Db, task_id: i64, branch: []const u8, repo_path: []const u8) !void {
        try self.sqlite_db.execute(
            "DELETE FROM integration_queue WHERE task_id = ?1 AND status = 'queued'",
            .{task_id},
        );
        try self.sqlite_db.execute(
            "INSERT INTO integration_queue (task_id, branch, repo_path) VALUES (?1, ?2, ?3)",
            .{ task_id, branch, repo_path },
        );
    }

    pub fn getQueuedBranches(self: *Db, allocator: std.mem.Allocator) ![]QueueEntry {
        var rows = try self.sqlite_db.query(
            allocator,
            "SELECT id, task_id, branch, COALESCE(repo_path, ''), status, queued_at FROM integration_queue WHERE status = 'queued' ORDER BY queued_at ASC",
            .{},
        );
        defer rows.deinit();

        var entries = std.ArrayList(QueueEntry).init(allocator);
        for (rows.items) |row| {
            try entries.append(QueueEntry{
                .id = row.getInt(0) orelse 0,
                .task_id = row.getInt(1) orelse 0,
                .branch = try allocator.dupe(u8, row.get(2) orelse ""),
                .repo_path = try allocator.dupe(u8, row.get(3) orelse ""),
                .status = try allocator.dupe(u8, row.get(4) orelse "queued"),
                .queued_at = try allocator.dupe(u8, row.get(5) orelse ""),
            });
        }
        return entries.toOwnedSlice();
    }

    pub fn getQueuedBranchesForRepo(self: *Db, allocator: std.mem.Allocator, repo_path: []const u8) ![]QueueEntry {
        var rows = try self.sqlite_db.query(
            allocator,
            "SELECT id, task_id, branch, COALESCE(repo_path, ''), status, queued_at FROM integration_queue WHERE status = 'queued' AND repo_path = ?1 ORDER BY queued_at ASC",
            .{repo_path},
        );
        defer rows.deinit();

        var entries = std.ArrayList(QueueEntry).init(allocator);
        for (rows.items) |row| {
            try entries.append(QueueEntry{
                .id = row.getInt(0) orelse 0,
                .task_id = row.getInt(1) orelse 0,
                .branch = try allocator.dupe(u8, row.get(2) orelse ""),
                .repo_path = try allocator.dupe(u8, row.get(3) orelse ""),
                .status = try allocator.dupe(u8, row.get(4) orelse "queued"),
                .queued_at = try allocator.dupe(u8, row.get(5) orelse ""),
            });
        }
        return entries.toOwnedSlice();
    }

    pub fn resetStuckQueueEntries(self: *Db) !void {
        try self.sqlite_db.execute(
            "UPDATE integration_queue SET status = 'queued' WHERE status = 'merging'",
            .{},
        );
    }

    pub fn updateQueueStatus(self: *Db, entry_id: i64, status: []const u8, error_msg: ?[]const u8) !void {
        try self.sqlite_db.execute(
            "UPDATE integration_queue SET status = ?1, error_msg = ?2 WHERE id = ?3",
            .{ status, error_msg orelse "", entry_id },
        );
    }

    // --- Task Outputs ---

    pub fn storeTaskOutput(self: *Db, task_id: i64, phase: []const u8, output: []const u8, exit_code: i64) !void {
        const truncated = output[0..@min(output.len, 32000)];
        try self.sqlite_db.execute(
            "INSERT INTO task_outputs (task_id, phase, output, exit_code) VALUES (?1, ?2, ?3, ?4)",
            .{ task_id, phase, truncated, exit_code },
        );
    }

    pub const TaskOutput = struct {
        id: i64,
        phase: []const u8,
        output: []const u8,
        exit_code: i64,
        created_at: []const u8,
    };

    pub fn getTaskOutputs(self: *Db, allocator: std.mem.Allocator, task_id: i64) ![]TaskOutput {
        var rows = try self.sqlite_db.query(
            allocator,
            "SELECT id, phase, output, exit_code, created_at FROM task_outputs WHERE task_id = ?1 ORDER BY created_at ASC",
            .{task_id},
        );
        defer rows.deinit();

        var outputs = std.ArrayList(TaskOutput).init(allocator);
        for (rows.items) |row| {
            try outputs.append(.{
                .id = row.getInt(0) orelse 0,
                .phase = try allocator.dupe(u8, row.get(1) orelse ""),
                .output = try allocator.dupe(u8, row.get(2) orelse ""),
                .exit_code = row.getInt(3) orelse 0,
                .created_at = try allocator.dupe(u8, row.get(4) orelse ""),
            });
        }
        return outputs.toOwnedSlice();
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

    try db.storeMessage(.{ .id = "1", .chat_jid = "tg:1", .sender = "u1", .sender_name = "Alice", .content = "First", .timestamp = "2024-01-01T00:00:00Z", .is_from_me = false, .is_bot_message = false });
    try db.storeMessage(.{ .id = "2", .chat_jid = "tg:1", .sender = "u2", .sender_name = "Bob", .content = "Second", .timestamp = "2024-01-01T00:01:00Z", .is_from_me = false, .is_bot_message = false });
    try db.storeMessage(.{ .id = "3", .chat_jid = "tg:2", .sender = "u1", .sender_name = "Alice", .content = "Other chat", .timestamp = "2024-01-01T00:01:00Z", .is_from_me = false, .is_bot_message = false });

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

    try db.storeMessage(.{ .id = "1", .chat_jid = "tg:1", .sender = "u1", .sender_name = "A", .content = "Original", .timestamp = "2024-01-01T00:00:00Z", .is_from_me = false, .is_bot_message = false });
    try db.storeMessage(.{ .id = "1", .chat_jid = "tg:1", .sender = "u1", .sender_name = "A", .content = "Duplicate", .timestamp = "2024-01-01T00:00:00Z", .is_from_me = false, .is_bot_message = false });

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

test "pipeline task lifecycle" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Create tasks
    const id1 = try db.createPipelineTask("Add feature X", "Detailed description", "/tmp/repo", "alice", "tg:123");
    const id2 = try db.createPipelineTask("Fix bug Y", "Bug details", "/tmp/repo", "bob", "tg:456");
    try std.testing.expect(id1 > 0);
    try std.testing.expect(id2 > id1);

    // Get next task (oldest first)
    const next = try db.getNextPipelineTask(alloc);
    try std.testing.expect(next != null);
    try std.testing.expectEqualStrings("Add feature X", next.?.title);
    try std.testing.expectEqualStrings("backlog", next.?.status);

    // Update status
    try db.updateTaskStatus(id1, "spec");
    try db.updateTaskBranch(id1, "feature/task-1");
    const updated = (try db.getPipelineTask(alloc, id1)).?;
    try std.testing.expectEqualStrings("spec", updated.status);
    try std.testing.expectEqualStrings("feature/task-1", updated.branch);

    // Increment attempt
    try db.incrementTaskAttempt(id1);
    const after_inc = (try db.getPipelineTask(alloc, id1)).?;
    try std.testing.expectEqual(@as(i64, 1), after_inc.attempt);

    // Update error
    try db.updateTaskError(id1, "test failed: assertion error");
    const with_err = (try db.getPipelineTask(alloc, id1)).?;
    try std.testing.expectEqualStrings("test failed: assertion error", with_err.last_error);

    // List all tasks
    const all = try db.getAllPipelineTasks(alloc, 10);
    try std.testing.expectEqual(@as(usize, 2), all.len);
}

test "integration queue operations" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("Task 1", "desc", "/repo", "", "");
    const id2 = try db.createPipelineTask("Task 2", "desc", "/repo", "", "");

    try db.enqueueForIntegration(id1, "feature/task-1", "/repo");
    try db.enqueueForIntegration(id2, "feature/task-2", "/repo");

    var queued = try db.getQueuedBranches(alloc);
    try std.testing.expectEqual(@as(usize, 2), queued.len);
    try std.testing.expectEqualStrings("feature/task-1", queued[0].branch);

    // Mark first as merged
    try db.updateQueueStatus(queued[0].id, "merged", null);
    queued = try db.getQueuedBranches(alloc);
    try std.testing.expectEqual(@as(usize, 1), queued.len);
    try std.testing.expectEqualStrings("feature/task-2", queued[0].branch);

    // Exclude second
    try db.updateQueueStatus(queued[0].id, "excluded", "merge conflict");
    queued = try db.getQueuedBranches(alloc);
    try std.testing.expectEqual(@as(usize, 0), queued.len);
}

test {
    _ = @import("is_bot_message_test.zig");
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

// ── AC1: Round-trip preserves all five fields ──────────────────────────
test "registerGroup round-trip preserves all five fields" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("grp:rt1", "Round Trip Group", "rt-folder", "@TestBot", true);

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    try std.testing.expectEqualStrings("grp:rt1", groups[0].jid);
    try std.testing.expectEqualStrings("Round Trip Group", groups[0].name);
    try std.testing.expectEqualStrings("rt-folder", groups[0].folder);
    try std.testing.expectEqualStrings("@TestBot", groups[0].trigger);
    try std.testing.expectEqual(true, groups[0].requires_trigger);
}

// ── AC2: Round-trip with requires_trigger=false ────────────────────────
test "registerGroup round-trip with requires_trigger false" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("grp:rt2", "No Trigger Group", "nt-folder", "@NTBot", false);

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    try std.testing.expectEqualStrings("grp:rt2", groups[0].jid);
    try std.testing.expectEqualStrings("No Trigger Group", groups[0].name);
    try std.testing.expectEqualStrings("nt-folder", groups[0].folder);
    try std.testing.expectEqualStrings("@NTBot", groups[0].trigger);
    try std.testing.expectEqual(false, groups[0].requires_trigger);
}

// ── AC3: Round-trip with custom trigger pattern ────────────────────────
test "registerGroup round-trip with custom trigger pattern" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("grp:cmd", "Cmd Group", "cmd-folder", "!cmd", true);

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    try std.testing.expectEqualStrings("!cmd", groups[0].trigger);
}

// ── AC4: unregisterGroup removes group from getAllGroups ────────────────
test "unregisterGroup removes group from getAllGroups" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("grp:a", "Group A", "folder-a", "@A", true);
    try db.registerGroup("grp:b", "Group B", "folder-b", "@B", false);

    const before = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 2), before.len);

    try db.unregisterGroup("grp:a");

    const after = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 1), after.len);
    try std.testing.expectEqualStrings("grp:b", after[0].jid);
}

// ── AC5: unregisterGroup on non-existent JID does not error ────────────
test "unregisterGroup on non-existent JID does not error" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Must not return an error
    try db.unregisterGroup("nonexistent:jid");

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 0), groups.len);
}

// ── AC6: getAllGroups on empty database returns empty slice ─────────────
test "getAllGroups on empty database returns empty slice" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 0), groups.len);
}

// ── AC7: Register, unregister, re-register round-trip ──────────────────
test "register unregister re-register round-trip" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("grp:cycle", "Original Name", "cycle-folder", "@OrigBot", true);
    try db.unregisterGroup("grp:cycle");
    try db.registerGroup("grp:cycle", "New Name", "cycle-folder", "@NewBot", false);

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    try std.testing.expectEqualStrings("grp:cycle", groups[0].jid);
    try std.testing.expectEqualStrings("New Name", groups[0].name);
    try std.testing.expectEqualStrings("@NewBot", groups[0].trigger);
    try std.testing.expectEqual(false, groups[0].requires_trigger);
}

// ── Edge Case 1: Empty string trigger ──────────────────────────────────
test "registerGroup preserves empty string trigger" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("grp:empty", "Empty Trigger", "empty-folder", "", false);

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    // Empty string from SQLite is non-null, so it must be preserved as ""
    // and NOT fall back to the schema default "@Borg".
    try std.testing.expectEqualStrings("", groups[0].trigger);
}

// ── Edge Case 2: Unicode in name and trigger ───────────────────────────
test "registerGroup preserves unicode in name and trigger" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("grp:uni", "Группа Тест", "uni-folder", "@Бот", true);

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    try std.testing.expectEqualStrings("Группа Тест", groups[0].name);
    try std.testing.expectEqualStrings("@Бот", groups[0].trigger);
}

// ── Edge Case 3: Unregister middle of multiple groups ──────────────────
test "unregisterGroup removes middle group leaving others intact" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("grp:A", "Group A", "folder-A", "@A", true);
    try db.registerGroup("grp:B", "Group B", "folder-B", "@B", false);
    try db.registerGroup("grp:C", "Group C", "folder-C", "@C", true);

    try db.unregisterGroup("grp:B");

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 2), groups.len);

    // Collect JIDs to verify A and C remain (order not guaranteed by SQL)
    var found_a = false;
    var found_c = false;
    for (groups) |g| {
        if (std.mem.eql(u8, g.jid, "grp:A")) {
            found_a = true;
            try std.testing.expectEqualStrings("Group A", g.name);
            try std.testing.expectEqualStrings("folder-A", g.folder);
            try std.testing.expectEqualStrings("@A", g.trigger);
            try std.testing.expectEqual(true, g.requires_trigger);
        } else if (std.mem.eql(u8, g.jid, "grp:C")) {
            found_c = true;
            try std.testing.expectEqualStrings("Group C", g.name);
            try std.testing.expectEqualStrings("folder-C", g.folder);
            try std.testing.expectEqualStrings("@C", g.trigger);
            try std.testing.expectEqual(true, g.requires_trigger);
        }
    }
    try std.testing.expect(found_a);
    try std.testing.expect(found_c);
}

// ── Edge Case 4: Double unregister ─────────────────────────────────────
test "double unregisterGroup does not error" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("grp:double", "Double", "double-folder", "@D", true);
    try db.unregisterGroup("grp:double");
    // Second unregister on already-removed JID must not error
    try db.unregisterGroup("grp:double");

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 0), groups.len);
}

// ── Edge Case 5: Upsert updates trigger and folder fields ──────────────
test "registerGroup upsert updates trigger and folder fields" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("grp:upsert", "V1", "folder-v1", "@TrigV1", true);
    try db.registerGroup("grp:upsert", "V2", "folder-v2", "@TrigV2", false);

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    try std.testing.expectEqualStrings("grp:upsert", groups[0].jid);
    try std.testing.expectEqualStrings("V2", groups[0].name);
    try std.testing.expectEqualStrings("folder-v2", groups[0].folder);
    try std.testing.expectEqualStrings("@TrigV2", groups[0].trigger);
    try std.testing.expectEqual(false, groups[0].requires_trigger);
}
