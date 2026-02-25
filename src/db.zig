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

    pub fn deinit(self: PipelineTask, allocator: std.mem.Allocator) void {
        const fields = .{ self.title, self.description, self.repo_path, self.branch, self.status, self.last_error, self.created_by, self.notify_chat, self.created_at, self.session_id };
        inline for (fields) |f| allocator.free(f);
    }
};

pub const QueueEntry = struct {
    id: i64,
    task_id: i64,
    branch: []const u8,
    repo_path: []const u8,
    status: []const u8,
    queued_at: []const u8,
    pr_number: i64,
};

pub const Proposal = struct {
    id: i64,
    repo_path: []const u8,
    title: []const u8,
    description: []const u8,
    rationale: []const u8,
    status: []const u8, // proposed, approved, dismissed
    created_at: []const u8,
    triage_score: i64,
    triage_impact: i64,
    triage_feasibility: i64,
    triage_risk: i64,
    triage_effort: i64,
    triage_reasoning: []const u8,
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
            \\  max_attempts INTEGER DEFAULT 5,
            \\  last_error TEXT DEFAULT '',
            \\  created_by TEXT DEFAULT '',
            \\  notify_chat TEXT DEFAULT '',
            \\  session_id TEXT DEFAULT '',
            \\  dispatched_at TEXT DEFAULT '',
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
            \\  unknown_retries INTEGER DEFAULT 0,
            \\  queued_at TEXT DEFAULT (datetime('now'))
            \\);
        );
        try self.sqlite_db.exec(
            \\CREATE TABLE IF NOT EXISTS task_outputs (
            \\  id INTEGER PRIMARY KEY AUTOINCREMENT,
            \\  task_id INTEGER NOT NULL,
            \\  phase TEXT NOT NULL,
            \\  output TEXT NOT NULL,
            \\  raw_stream TEXT DEFAULT '',
            \\  exit_code INTEGER DEFAULT 0,
            \\  created_at TEXT DEFAULT (datetime('now'))
            \\);
        );
        try self.sqlite_db.exec(
            \\CREATE INDEX IF NOT EXISTS idx_task_outputs_task ON task_outputs(task_id);
        );
        try self.sqlite_db.exec(
            \\CREATE TABLE IF NOT EXISTS proposals (
            \\  id INTEGER PRIMARY KEY AUTOINCREMENT,
            \\  repo_path TEXT NOT NULL,
            \\  title TEXT NOT NULL,
            \\  description TEXT NOT NULL DEFAULT '',
            \\  rationale TEXT NOT NULL DEFAULT '',
            \\  status TEXT NOT NULL DEFAULT 'proposed',
            \\  created_at TEXT DEFAULT (datetime('now'))
            \\);
        );

        try self.sqlite_db.exec(
            \\CREATE TABLE IF NOT EXISTS chat_agent_runs (
            \\  id INTEGER PRIMARY KEY AUTOINCREMENT,
            \\  jid TEXT NOT NULL,
            \\  status TEXT NOT NULL DEFAULT 'running',
            \\  transport TEXT DEFAULT '',
            \\  original_id TEXT DEFAULT '',
            \\  trigger_msg_id TEXT DEFAULT '',
            \\  folder TEXT DEFAULT '',
            \\  output TEXT DEFAULT '',
            \\  new_session_id TEXT DEFAULT '',
            \\  last_msg_timestamp TEXT DEFAULT '',
            \\  started_at TEXT DEFAULT (datetime('now')),
            \\  completed_at TEXT
            \\);
        );

        try self.sqlite_db.exec(
            \\CREATE TABLE IF NOT EXISTS events (
            \\  id INTEGER PRIMARY KEY AUTOINCREMENT,
            \\  ts INTEGER NOT NULL,
            \\  level TEXT NOT NULL DEFAULT 'info',
            \\  category TEXT NOT NULL DEFAULT 'system',
            \\  message TEXT NOT NULL,
            \\  metadata TEXT DEFAULT ''
            \\);
        );
        try self.sqlite_db.exec(
            \\CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts);
        );
        try self.sqlite_db.exec(
            \\CREATE INDEX IF NOT EXISTS idx_events_category ON events(category, ts);
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
            "INSERT INTO state (key, value) VALUES ('schema_version', '0')",
            .{},
        );
    }

    fn runMigrations(self: *Db) !void {
        // Always try all ALTER TABLEs — they're idempotent (duplicate column is caught).
        // This handles the case where schema_version was set but columns weren't actually added.
        const migrations = [_][*:0]const u8{
            "ALTER TABLE pipeline_tasks ADD COLUMN session_id TEXT DEFAULT ''",
            "ALTER TABLE integration_queue ADD COLUMN repo_path TEXT DEFAULT ''",
            "ALTER TABLE integration_queue ADD COLUMN pr_number INTEGER DEFAULT 0",
            "ALTER TABLE task_outputs ADD COLUMN raw_stream TEXT DEFAULT ''",
            "CREATE TABLE IF NOT EXISTS proposals (id INTEGER PRIMARY KEY AUTOINCREMENT, repo_path TEXT NOT NULL, title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', rationale TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'proposed', created_at TEXT DEFAULT (datetime('now')))",
            "UPDATE proposals SET status = 'proposed' WHERE status = 'pending'",
            "CREATE TABLE IF NOT EXISTS chat_agent_runs (id INTEGER PRIMARY KEY AUTOINCREMENT, jid TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'running', transport TEXT DEFAULT '', original_id TEXT DEFAULT '', trigger_msg_id TEXT DEFAULT '', folder TEXT DEFAULT '', output TEXT DEFAULT '', new_session_id TEXT DEFAULT '', last_msg_timestamp TEXT DEFAULT '', started_at TEXT DEFAULT (datetime('now')), completed_at TEXT)",
            "ALTER TABLE pipeline_tasks ADD COLUMN dispatched_at TEXT DEFAULT ''",
            "ALTER TABLE integration_queue ADD COLUMN unknown_retries INTEGER DEFAULT 0",
            "CREATE TABLE IF NOT EXISTS events (id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, level TEXT NOT NULL DEFAULT 'info', category TEXT NOT NULL DEFAULT 'system', message TEXT NOT NULL, metadata TEXT DEFAULT '')",
            "CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts)",
            "CREATE INDEX IF NOT EXISTS idx_events_category ON events(category, ts)",
            "ALTER TABLE proposals ADD COLUMN triage_score INTEGER DEFAULT 0",
            "ALTER TABLE proposals ADD COLUMN triage_impact INTEGER DEFAULT 0",
            "ALTER TABLE proposals ADD COLUMN triage_feasibility INTEGER DEFAULT 0",
            "ALTER TABLE proposals ADD COLUMN triage_risk INTEGER DEFAULT 0",
            "ALTER TABLE proposals ADD COLUMN triage_effort INTEGER DEFAULT 0",
            "ALTER TABLE proposals ADD COLUMN triage_reasoning TEXT DEFAULT ''",
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

    // --- Chat Agent Runs ---

    pub fn createChatAgentRun(self: *Db, jid: []const u8, transport: []const u8, original_id: []const u8, trigger_msg_id: []const u8, folder: []const u8) !i64 {
        try self.sqlite_db.execute(
            "INSERT INTO chat_agent_runs (jid, status, transport, original_id, trigger_msg_id, folder) VALUES (?1, 'running', ?2, ?3, ?4, ?5)",
            .{ jid, transport, original_id, trigger_msg_id, folder },
        );
        return self.sqlite_db.lastInsertRowId();
    }

    pub fn completeChatAgentRun(self: *Db, run_id: i64, output: []const u8, new_session_id: []const u8, last_msg_ts: []const u8, success: bool) !void {
        const status = if (success) "completed" else "failed";
        try self.sqlite_db.execute(
            "UPDATE chat_agent_runs SET status = ?1, output = ?2, new_session_id = ?3, last_msg_timestamp = ?4, completed_at = datetime('now') WHERE id = ?5",
            .{ status, output, new_session_id, last_msg_ts, run_id },
        );
    }

    pub fn markChatAgentRunDelivered(self: *Db, run_id: i64) !void {
        try self.sqlite_db.execute(
            "UPDATE chat_agent_runs SET status = 'delivered' WHERE id = ?1",
            .{run_id},
        );
    }

    pub const ChatAgentRun = struct {
        id: i64,
        jid: []const u8,
        status: []const u8,
        transport: []const u8,
        original_id: []const u8,
        trigger_msg_id: []const u8,
        folder: []const u8,
        output: []const u8,
        new_session_id: []const u8,
        last_msg_timestamp: []const u8,
    };

    pub fn getUndeliveredRuns(self: *Db, allocator: std.mem.Allocator) ![]ChatAgentRun {
        var rows = try self.sqlite_db.query(
            allocator,
            "SELECT id, jid, status, transport, original_id, trigger_msg_id, folder, output, COALESCE(new_session_id, ''), COALESCE(last_msg_timestamp, '') FROM chat_agent_runs WHERE status IN ('completed', 'failed')",
            .{},
        );
        defer rows.deinit();

        var runs = std.ArrayList(ChatAgentRun).init(allocator);
        for (rows.items) |row| {
            try runs.append(.{
                .id = std.fmt.parseInt(i64, row.get(0) orelse "0", 10) catch 0,
                .jid = try allocator.dupe(u8, row.get(1) orelse ""),
                .status = try allocator.dupe(u8, row.get(2) orelse ""),
                .transport = try allocator.dupe(u8, row.get(3) orelse ""),
                .original_id = try allocator.dupe(u8, row.get(4) orelse ""),
                .trigger_msg_id = try allocator.dupe(u8, row.get(5) orelse ""),
                .folder = try allocator.dupe(u8, row.get(6) orelse ""),
                .output = try allocator.dupe(u8, row.get(7) orelse ""),
                .new_session_id = try allocator.dupe(u8, row.get(8) orelse ""),
                .last_msg_timestamp = try allocator.dupe(u8, row.get(9) orelse ""),
            });
        }
        return runs.toOwnedSlice();
    }

    pub fn abandonRunningAgents(self: *Db) !void {
        try self.sqlite_db.execute(
            "UPDATE chat_agent_runs SET status = 'abandoned', completed_at = datetime('now') WHERE status = 'running'",
            .{},
        );
    }

    pub const UnansweredEntry = struct { jid: []const u8, last_user_ts: []const u8 };

    pub fn getUnansweredMessages(self: *Db, allocator: std.mem.Allocator, max_age_s: i64) ![]UnansweredEntry {
        var results = std.ArrayList(UnansweredEntry).init(allocator);

        // For each registered group, find the latest user message and latest bot message
        var groups = try self.sqlite_db.query(allocator, "SELECT jid FROM registered_groups", .{});
        defer groups.deinit();

        for (groups.items) |grow| {
            const jid = grow.get(0) orelse continue;

            // Latest user message timestamp
            var user_rows = try self.sqlite_db.query(allocator,
                "SELECT timestamp FROM messages WHERE chat_jid = ?1 AND is_bot_message = 0 ORDER BY timestamp DESC LIMIT 1",
                .{jid},
            );
            defer user_rows.deinit();
            const user_ts = if (user_rows.items.len > 0) user_rows.items[0].get(0) orelse "" else "";
            if (user_ts.len == 0) continue;

            // Latest bot message timestamp
            var bot_rows = try self.sqlite_db.query(allocator,
                "SELECT timestamp FROM messages WHERE chat_jid = ?1 AND is_bot_message = 1 ORDER BY timestamp DESC LIMIT 1",
                .{jid},
            );
            defer bot_rows.deinit();
            const bot_ts = if (bot_rows.items.len > 0) bot_rows.items[0].get(0) orelse "" else "";

            // If no bot reply, or user message is newer than bot reply
            const unanswered = bot_ts.len == 0 or std.mem.order(u8, user_ts, bot_ts) == .gt;
            if (!unanswered) continue;

            // Skip messages older than max_age_s
            var age_rows = try self.sqlite_db.query(allocator,
                "SELECT 1 WHERE datetime(?1) >= datetime('now', printf('-%d seconds', ?2))",
                .{ user_ts, max_age_s },
            );
            const recent = age_rows.items.len > 0;
            age_rows.deinit();
            if (!recent) continue;

            try results.append(.{
                .jid = try allocator.dupe(u8, jid),
                .last_user_ts = try allocator.dupe(u8, user_ts),
            });
        }
        return results.toOwnedSlice();
    }

    // --- Pipeline Task Inflight Tracking ---

    pub fn markTaskDispatched(self: *Db, task_id: i64) !void {
        try self.sqlite_db.execute(
            "UPDATE pipeline_tasks SET dispatched_at = datetime('now') WHERE id = ?1",
            .{task_id},
        );
    }

    pub fn clearTaskDispatched(self: *Db, task_id: i64) !void {
        try self.sqlite_db.execute(
            "UPDATE pipeline_tasks SET dispatched_at = '' WHERE id = ?1",
            .{task_id},
        );
    }

    pub fn isTaskDispatched(self: *Db, task_id: i64) bool {
        var rows = self.sqlite_db.query(
            self.allocator,
            "SELECT dispatched_at FROM pipeline_tasks WHERE id = ?1",
            .{task_id},
        ) catch return false;
        defer rows.deinit();
        if (rows.items.len == 0) return false;
        const val = rows.items[0].get(0) orelse "";
        return val.len > 0;
    }

    pub fn clearAllDispatched(self: *Db) !void {
        try self.sqlite_db.execute(
            "UPDATE pipeline_tasks SET dispatched_at = '' WHERE dispatched_at != ''",
            .{},
        );
    }

    // --- Integration Queue Unknown Retries ---

    pub fn getUnknownRetries(self: *Db, queue_id: i64) u32 {
        var rows = self.sqlite_db.query(
            self.allocator,
            "SELECT unknown_retries FROM integration_queue WHERE id = ?1",
            .{queue_id},
        ) catch return 0;
        defer rows.deinit();
        if (rows.items.len == 0) return 0;
        return std.fmt.parseInt(u32, rows.items[0].get(0) orelse "0", 10) catch 0;
    }

    pub fn incrementUnknownRetries(self: *Db, queue_id: i64) !void {
        try self.sqlite_db.execute(
            "UPDATE integration_queue SET unknown_retries = unknown_retries + 1 WHERE id = ?1",
            .{queue_id},
        );
    }

    pub fn resetUnknownRetries(self: *Db, queue_id: i64) !void {
        try self.sqlite_db.execute(
            "UPDATE integration_queue SET unknown_retries = 0 WHERE id = ?1",
            .{queue_id},
        );
    }

    // --- Events ---

    pub fn logEvent(self: *Db, level: []const u8, category: []const u8, message: []const u8, metadata: []const u8) void {
        const ts = std.time.timestamp();
        self.sqlite_db.execute(
            "INSERT INTO events (ts, level, category, message, metadata) VALUES (?1, ?2, ?3, ?4, ?5)",
            .{ ts, level, category, message, metadata },
        ) catch {};
        // Auto-prune old events (keep last 10000)
        self.sqlite_db.execute(
            "DELETE FROM events WHERE id <= (SELECT id FROM events ORDER BY id DESC LIMIT 1 OFFSET 10000)",
            .{},
        ) catch {};
    }

    pub const EventRow = struct {
        id: i64,
        ts: i64,
        level: []const u8,
        category: []const u8,
        message: []const u8,
        metadata: []const u8,
    };

    pub fn getEvents(self: *Db, allocator: std.mem.Allocator, category: ?[]const u8, level: ?[]const u8, since_ts: i64, limit_n: i64) ![]EventRow {
        var result = std.ArrayList(EventRow).init(allocator);

        if (category) |cat| {
            if (level) |lvl| {
                var rows = try self.sqlite_db.query(
                    allocator,
                    "SELECT id, ts, level, category, message, COALESCE(metadata,'') FROM events WHERE category = ?1 AND level = ?2 AND ts >= ?3 ORDER BY ts DESC LIMIT ?4",
                    .{ cat, lvl, since_ts, limit_n },
                );
                defer rows.deinit();
                for (rows.items) |row| {
                    try result.append(.{
                        .id = std.fmt.parseInt(i64, row.get(0) orelse "0", 10) catch 0,
                        .ts = std.fmt.parseInt(i64, row.get(1) orelse "0", 10) catch 0,
                        .level = try allocator.dupe(u8, row.get(2) orelse "info"),
                        .category = try allocator.dupe(u8, row.get(3) orelse "system"),
                        .message = try allocator.dupe(u8, row.get(4) orelse ""),
                        .metadata = try allocator.dupe(u8, row.get(5) orelse ""),
                    });
                }
            } else {
                var rows = try self.sqlite_db.query(
                    allocator,
                    "SELECT id, ts, level, category, message, COALESCE(metadata,'') FROM events WHERE category = ?1 AND ts >= ?2 ORDER BY ts DESC LIMIT ?3",
                    .{ cat, since_ts, limit_n },
                );
                defer rows.deinit();
                for (rows.items) |row| {
                    try result.append(.{
                        .id = std.fmt.parseInt(i64, row.get(0) orelse "0", 10) catch 0,
                        .ts = std.fmt.parseInt(i64, row.get(1) orelse "0", 10) catch 0,
                        .level = try allocator.dupe(u8, row.get(2) orelse "info"),
                        .category = try allocator.dupe(u8, row.get(3) orelse "system"),
                        .message = try allocator.dupe(u8, row.get(4) orelse ""),
                        .metadata = try allocator.dupe(u8, row.get(5) orelse ""),
                    });
                }
            }
        } else if (level) |lvl| {
            var rows = try self.sqlite_db.query(
                allocator,
                "SELECT id, ts, level, category, message, COALESCE(metadata,'') FROM events WHERE level = ?1 AND ts >= ?2 ORDER BY ts DESC LIMIT ?3",
                .{ lvl, since_ts, limit_n },
            );
            defer rows.deinit();
            for (rows.items) |row| {
                try result.append(.{
                    .id = std.fmt.parseInt(i64, row.get(0) orelse "0", 10) catch 0,
                    .ts = std.fmt.parseInt(i64, row.get(1) orelse "0", 10) catch 0,
                    .level = try allocator.dupe(u8, row.get(2) orelse "info"),
                    .category = try allocator.dupe(u8, row.get(3) orelse "system"),
                    .message = try allocator.dupe(u8, row.get(4) orelse ""),
                    .metadata = try allocator.dupe(u8, row.get(5) orelse ""),
                });
            }
        } else {
            var rows = try self.sqlite_db.query(
                allocator,
                "SELECT id, ts, level, category, message, COALESCE(metadata,'') FROM events WHERE ts >= ?1 ORDER BY ts DESC LIMIT ?2",
                .{ since_ts, limit_n },
            );
            defer rows.deinit();
            for (rows.items) |row| {
                try result.append(.{
                    .id = std.fmt.parseInt(i64, row.get(0) orelse "0", 10) catch 0,
                    .ts = std.fmt.parseInt(i64, row.get(1) orelse "0", 10) catch 0,
                    .level = try allocator.dupe(u8, row.get(2) orelse "info"),
                    .category = try allocator.dupe(u8, row.get(3) orelse "system"),
                    .message = try allocator.dupe(u8, row.get(4) orelse ""),
                    .metadata = try allocator.dupe(u8, row.get(5) orelse ""),
                });
            }
        }

        return result.toOwnedSlice();
    }

    // --- Pipeline Tasks ---

    pub fn createPipelineTask(self: *Db, title: []const u8, description: []const u8, repo_path: []const u8, created_by: []const u8, notify_chat: []const u8) !i64 {
        try self.sqlite_db.execute(
            "INSERT INTO pipeline_tasks (title, description, repo_path, created_by, notify_chat) VALUES (?1, ?2, ?3, ?4, ?5)",
            .{ title, description, repo_path, created_by, notify_chat },
        );
        const id = self.sqlite_db.lastInsertRowId();
        self.logEvent("info", "pipeline", title[0..@min(title.len, 200)], "created");
        return id;
    }

    pub fn getNextPipelineTask(self: *Db, allocator: std.mem.Allocator) !?PipelineTask {
        // Priority: rebase > retry > impl > qa > spec > backlog
        var rows = try self.sqlite_db.query(
            allocator,
            \\SELECT id, title, description, repo_path, branch, status, attempt, max_attempts, last_error, created_by, notify_chat, created_at, COALESCE(session_id, '')
            \\FROM pipeline_tasks
            \\WHERE status IN ('backlog', 'spec', 'qa', 'qa_fix', 'impl', 'retry', 'rebase')
            \\ORDER BY
            \\  CASE status
            \\    WHEN 'rebase' THEN 0
            \\    WHEN 'retry' THEN 1
            \\    WHEN 'impl' THEN 2
            \\    WHEN 'qa_fix' THEN 3
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
            \\WHERE status IN ('backlog', 'spec', 'qa', 'qa_fix', 'impl', 'retry', 'rebase')
            \\ORDER BY
            \\  CASE status
            \\    WHEN 'rebase' THEN 0
            \\    WHEN 'retry' THEN 1
            \\    WHEN 'impl' THEN 2
            \\    WHEN 'qa_fix' THEN 3
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
        var buf: [64]u8 = undefined;
        const meta = std.fmt.bufPrint(&buf, "task_id={d}", .{task_id}) catch "";
        const level: []const u8 = if (std.mem.eql(u8, status, "failed")) "error" else "info";
        self.logEvent(level, "pipeline", status, meta);
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

    pub fn recycleFailedTasks(self: *Db) !void {
        try self.sqlite_db.execute(
            "UPDATE pipeline_tasks SET status = 'backlog', attempt = 0, branch = '', session_id = '', updated_at = datetime('now') WHERE status = 'failed'",
            .{},
        );
    }

    pub fn deletePipelineTask(self: *Db, task_id: i64) !void {
        try self.sqlite_db.execute(
            "DELETE FROM pipeline_tasks WHERE id = ?1",
            .{task_id},
        );
    }

    pub fn resetTaskAttempt(self: *Db, task_id: i64) !void {
        try self.sqlite_db.execute(
            "UPDATE pipeline_tasks SET attempt = 0, branch = '', session_id = '', updated_at = datetime('now') WHERE id = ?1",
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
            .max_attempts = row.getInt(7) orelse 5,
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
        dispatched: i64,
    };

    pub fn getPipelineStats(self: *Db) !PipelineStats {
        var rows = try self.sqlite_db.query(self.allocator,
            \\SELECT
            \\  COUNT(*) AS total,
            \\  COUNT(CASE WHEN status IN ('backlog', 'spec', 'qa', 'qa_fix', 'impl', 'retry', 'rebase') THEN 1 END) AS active,
            \\  COUNT(CASE WHEN status = 'merged' THEN 1 END) AS merged,
            \\  COUNT(CASE WHEN status = 'failed' THEN 1 END) AS failed,
            \\  COUNT(CASE WHEN dispatched_at != '' THEN 1 END) AS dispatched
            \\FROM pipeline_tasks
        , .{});
        defer rows.deinit();
        if (rows.items.len == 0) return .{ .total = 0, .active = 0, .merged = 0, .failed = 0, .dispatched = 0 };
        const row = rows.items[0];
        return .{
            .total = row.getInt(0) orelse 0,
            .active = row.getInt(1) orelse 0,
            .merged = row.getInt(2) orelse 0,
            .failed = row.getInt(3) orelse 0,
            .dispatched = row.getInt(4) orelse 0,
        };
    }

    pub fn getActivePipelineTaskCount(self: *Db) !i64 {
        var rows = try self.sqlite_db.query(
            self.allocator,
            "SELECT COUNT(*) FROM pipeline_tasks WHERE status IN ('backlog', 'spec', 'qa', 'qa_fix', 'impl', 'retry', 'rebase')",
            .{},
        );
        defer rows.deinit();
        if (rows.items.len == 0) return 0;
        return rows.items[0].getInt(0) orelse 0;
    }

    pub fn getQueuedIntegrationCount(self: *Db) !i64 {
        var rows = try self.sqlite_db.query(
            self.allocator,
            "SELECT COUNT(*) FROM integration_queue WHERE status = 'queued'",
            .{},
        );
        defer rows.deinit();
        if (rows.items.len == 0) return 0;
        return rows.items[0].getInt(0) orelse 0;
    }

    pub fn getUnmergedBacklogCount(self: *Db) !i64 {
        var rows = try self.sqlite_db.query(
            self.allocator,
            "SELECT COUNT(*) FROM pipeline_tasks WHERE created_by = 'backlog' AND status != 'merged' AND status != 'failed'",
            .{},
        );
        defer rows.deinit();
        if (rows.items.len == 0) return 0;
        return rows.items[0].getInt(0) orelse 0;
    }

    // --- Integration Queue ---

    pub fn enqueueForIntegration(self: *Db, task_id: i64, branch: []const u8, repo_path: []const u8) !void {
        // Don't re-enqueue if a merged entry already exists for this task
        var merged_rows = try self.sqlite_db.query(self.allocator,
            "SELECT 1 FROM integration_queue WHERE task_id = ?1 AND status = 'merged' LIMIT 1",
            .{task_id},
        );
        const already_merged = merged_rows.items.len > 0;
        merged_rows.deinit();
        if (already_merged) return;

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
            "SELECT id, task_id, branch, COALESCE(repo_path, ''), status, queued_at, COALESCE(pr_number, 0) FROM integration_queue WHERE status = 'queued' ORDER BY queued_at ASC",
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
                .pr_number = row.getInt(6) orelse 0,
            });
        }
        return entries.toOwnedSlice();
    }

    pub fn getQueuedBranchesForRepo(self: *Db, allocator: std.mem.Allocator, repo_path: []const u8) ![]QueueEntry {
        var rows = try self.sqlite_db.query(
            allocator,
            "SELECT id, task_id, branch, COALESCE(repo_path, ''), status, queued_at, COALESCE(pr_number, 0) FROM integration_queue WHERE status = 'queued' AND repo_path = ?1 ORDER BY queued_at ASC",
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
                .pr_number = row.getInt(6) orelse 0,
            });
        }
        return entries.toOwnedSlice();
    }

    pub fn updateQueuePrNumber(self: *Db, entry_id: i64, pr_number: i64) !void {
        try self.sqlite_db.execute(
            "UPDATE integration_queue SET pr_number = ?1 WHERE id = ?2",
            .{ pr_number, entry_id },
        );
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

    pub fn storeTaskOutputFull(self: *Db, task_id: i64, phase: []const u8, output: []const u8, raw_stream: []const u8, exit_code: i64) !void {
        const truncated = output[0..@min(output.len, 32000)];
        try self.sqlite_db.execute(
            "INSERT INTO task_outputs (task_id, phase, output, raw_stream, exit_code) VALUES (?1, ?2, ?3, ?4, ?5)",
            .{ task_id, phase, truncated, raw_stream, exit_code },
        );
    }

    pub const TaskOutput = struct {
        id: i64,
        phase: []const u8,
        output: []const u8,
        raw_stream: []const u8,
        exit_code: i64,
        created_at: []const u8,
    };

    pub fn getTaskOutputs(self: *Db, allocator: std.mem.Allocator, task_id: i64) ![]TaskOutput {
        var rows = try self.sqlite_db.query(
            allocator,
            "SELECT id, phase, output, exit_code, created_at, COALESCE(raw_stream, '') FROM task_outputs WHERE task_id = ?1 ORDER BY created_at ASC",
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
                .raw_stream = try allocator.dupe(u8, row.get(5) orelse ""),
            });
        }
        return outputs.toOwnedSlice();
    }

    // --- Proposals ---

    pub fn createProposal(self: *Db, repo_path: []const u8, title: []const u8, description: []const u8, rationale: []const u8) !i64 {
        try self.sqlite_db.execute(
            "INSERT INTO proposals (repo_path, title, description, rationale) VALUES (?1, ?2, ?3, ?4)",
            .{ repo_path, title, description, rationale },
        );
        return self.sqlite_db.lastInsertRowId();
    }

    pub fn getProposals(self: *Db, allocator: std.mem.Allocator, status_filter: ?[]const u8, limit: i64) ![]Proposal {
        var rows = if (status_filter) |sf|
            try self.sqlite_db.query(
                allocator,
                "SELECT id, repo_path, title, description, rationale, status, created_at, triage_score, triage_impact, triage_feasibility, triage_risk, triage_effort, triage_reasoning FROM proposals WHERE status = ?1 ORDER BY created_at DESC LIMIT ?2",
                .{ sf, limit },
            )
        else
            try self.sqlite_db.query(
                allocator,
                "SELECT id, repo_path, title, description, rationale, status, created_at, triage_score, triage_impact, triage_feasibility, triage_risk, triage_effort, triage_reasoning FROM proposals ORDER BY created_at DESC LIMIT ?1",
                .{limit},
            );
        defer rows.deinit();

        var proposals = std.ArrayList(Proposal).init(allocator);
        for (rows.items) |row| {
            try proposals.append(proposalFromRow(allocator, row));
        }
        return proposals.toOwnedSlice();
    }

    pub fn updateProposalStatus(self: *Db, proposal_id: i64, status: []const u8) !void {
        try self.sqlite_db.execute(
            "UPDATE proposals SET status = ?1 WHERE id = ?2",
            .{ status, proposal_id },
        );
    }

    pub fn getProposal(self: *Db, allocator: std.mem.Allocator, proposal_id: i64) !?Proposal {
        var rows = try self.sqlite_db.query(
            allocator,
            "SELECT id, repo_path, title, description, rationale, status, created_at, triage_score, triage_impact, triage_feasibility, triage_risk, triage_effort, triage_reasoning FROM proposals WHERE id = ?1",
            .{proposal_id},
        );
        defer rows.deinit();
        if (rows.items.len == 0) return null;
        return proposalFromRow(allocator, rows.items[0]);
    }

    pub fn updateProposalTriage(self: *Db, proposal_id: i64, score: i64, impact: i64, feasibility: i64, risk: i64, effort: i64, reasoning: []const u8) !void {
        try self.sqlite_db.execute(
            "UPDATE proposals SET triage_score = ?1, triage_impact = ?2, triage_feasibility = ?3, triage_risk = ?4, triage_effort = ?5, triage_reasoning = ?6 WHERE id = ?7",
            .{ score, impact, feasibility, risk, effort, reasoning, proposal_id },
        );
    }
};

fn proposalFromRow(allocator: std.mem.Allocator, row: sqlite.Row) Proposal {
    return Proposal{
        .id = row.getInt(0) orelse 0,
        .repo_path = allocator.dupe(u8, row.get(1) orelse "") catch "",
        .title = allocator.dupe(u8, row.get(2) orelse "") catch "",
        .description = allocator.dupe(u8, row.get(3) orelse "") catch "",
        .rationale = allocator.dupe(u8, row.get(4) orelse "") catch "",
        .status = allocator.dupe(u8, row.get(5) orelse "proposed") catch "proposed",
        .created_at = allocator.dupe(u8, row.get(6) orelse "") catch "",
        .triage_score = row.getInt(7) orelse 0,
        .triage_impact = row.getInt(8) orelse 0,
        .triage_feasibility = row.getInt(9) orelse 0,
        .triage_risk = row.getInt(10) orelse 0,
        .triage_effort = row.getInt(11) orelse 0,
        .triage_reasoning = allocator.dupe(u8, row.get(12) orelse "") catch "",
    };
}

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
    _ = @import("sqlite_bindparams_test.zig");
    _ = @import("pipeline_stats_test.zig");
    _ = @import("db_pipeline_query_test.zig");
    _ = @import("db_task_output_test.zig");
    _ = @import("db_proposal_test.zig");
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

test "registerGroup and getAllGroups round-trip preserves all fields" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("g1", "Test Group", "test-folder", "!help", false);

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    try std.testing.expectEqualStrings("g1", groups[0].jid);
    try std.testing.expectEqualStrings("Test Group", groups[0].name);
    try std.testing.expectEqualStrings("test-folder", groups[0].folder);
    try std.testing.expectEqualStrings("!help", groups[0].trigger);
    try std.testing.expect(!groups[0].requires_trigger);
}

test "unregisterGroup removes group from getAllGroups" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("g1", "Group One", "folder1", "@Bot", true);
    try db.registerGroup("g2", "Group Two", "folder2", "@Bot", true);

    try db.unregisterGroup("g1");

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    try std.testing.expectEqualStrings("g2", groups[0].jid);
}

test "unregisterGroup on nonexistent jid is a no-op" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.unregisterGroup("nonexistent");

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 0), groups.len);
}

test "getAllGroups returns empty slice when no groups registered" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const groups = try db.getAllGroups(alloc);
    try std.testing.expectEqual(@as(usize, 0), groups.len);
}
