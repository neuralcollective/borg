const std = @import("std");
const json_mod = @import("json.zig");
const http_mod = @import("http.zig");
const telegram_mod = @import("telegram.zig");

const HAIKU = "claude-haiku-4-5-20251001";
const MAX_LOG_BYTES = 50_000;

pub const Severity = enum(u2) {
    low = 0,
    medium = 1,
    high = 2,
    critical = 3,

    fn fromStr(s: []const u8) Severity {
        if (std.mem.eql(u8, s, "low")) return .low;
        if (std.mem.eql(u8, s, "high")) return .high;
        if (std.mem.eql(u8, s, "critical")) return .critical;
        return .medium;
    }

    fn toStr(self: Severity) []const u8 {
        return switch (self) {
            .low => "low",
            .medium => "medium",
            .high => "high",
            .critical => "critical",
        };
    }
};

const Source = union(enum) {
    journalctl: []const u8, // unit name
    file_tail: []const u8, // file path
    command: []const u8, // shell command
};

const Action = union(enum) {
    alert: []const u8, // Telegram chat_id (raw or tg: prefix)
    command: []const u8, // shell command
    webhook: []const u8, // URL
};

const Entry = struct {
    name: []const u8,
    source: Source,
    window_lines: u32,
    interval_s: i64,
    prompt: []const u8,
    cooldown_s: i64,
    severity_threshold: Severity,
    actions: []Action,
    last_run: i64 = 0,
    last_triggered: i64 = 0,
};

const AnalysisResult = struct {
    triggered: bool,
    severity: Severity,
    summary: []const u8,
    recommendation: []const u8,
};

pub const Observer = struct {
    allocator: std.mem.Allocator,
    config_arena: std.heap.ArenaAllocator,
    entries: []Entry,
    api_key: []const u8,
    tg: ?*telegram_mod.Telegram,
    running: std.atomic.Value(bool),

    pub fn init(
        allocator: std.mem.Allocator,
        config_path: []const u8,
        api_key: []const u8,
        tg: ?*telegram_mod.Telegram,
    ) !Observer {
        var arena = std.heap.ArenaAllocator.init(allocator);
        errdefer arena.deinit();
        const aa = arena.allocator();

        const entries = try loadEntries(aa, config_path);
        const key = try aa.dupe(u8, api_key);

        return .{
            .allocator = allocator,
            .config_arena = arena,
            .entries = entries,
            .api_key = key,
            .tg = tg,
            .running = std.atomic.Value(bool).init(true),
        };
    }

    pub fn deinit(self: *Observer) void {
        self.config_arena.deinit();
    }

    pub fn stop(self: *Observer) void {
        self.running.store(false, .monotonic);
    }

    pub fn run(self: *Observer) void {
        while (self.running.load(.monotonic)) {
            const now = std.time.timestamp();
            for (self.entries) |*entry| {
                if (now - entry.last_run < entry.interval_s) continue;
                entry.last_run = now;
                self.runEntry(entry, now) catch |err| {
                    std.log.warn("Observer [{s}]: {}", .{ entry.name, err });
                };
            }
            std.time.sleep(10 * std.time.ns_per_s);
        }
    }

    fn runEntry(self: *Observer, entry: *Entry, now: i64) !void {
        var tmp = std.heap.ArenaAllocator.init(self.allocator);
        defer tmp.deinit();
        const ta = tmp.allocator();

        const logs = try collectLogs(ta, entry.*);
        if (logs.len == 0) return;

        const result = try self.analyze(ta, entry.*, logs);

        if (!result.triggered) return;
        if (@intFromEnum(result.severity) < @intFromEnum(entry.severity_threshold)) return;
        if (now - entry.last_triggered < entry.cooldown_s) return;

        entry.last_triggered = now;
        std.log.warn("Observer [{s}] triggered ({s}): {s}", .{ entry.name, result.severity.toStr(), result.summary });

        for (entry.actions) |action| {
            self.executeAction(ta, entry.name, action, result) catch |err| {
                std.log.warn("Observer [{s}] action failed: {}", .{ entry.name, err });
            };
        }
    }

    fn analyze(self: *Observer, ta: std.mem.Allocator, entry: Entry, logs: []const u8) !AnalysisResult {
        const escaped_logs = try json_mod.escapeString(ta, logs);
        const escaped_prompt = try json_mod.escapeString(ta, entry.prompt);

        // Keep last MAX_LOG_BYTES to stay within context limits
        const log_slice = if (escaped_logs.len > MAX_LOG_BYTES)
            escaped_logs[escaped_logs.len - MAX_LOG_BYTES ..]
        else
            escaped_logs;

        const user_content = try std.fmt.allocPrint(ta,
            "You are a log monitor.\n\n{s}\n\nRecent log output:\n```\n{s}\n```\n\nRespond ONLY with JSON. If something concerning is found: {{\"triggered\":true,\"severity\":\"low|medium|high|critical\",\"summary\":\"one sentence\",\"recommendation\":\"one sentence\"}}. If nothing to flag: {{\"triggered\":false}}",
            .{ escaped_prompt, log_slice },
        );

        const body = try std.fmt.allocPrint(ta,
            \\{{"model":"{s}","max_tokens":256,"messages":[{{"role":"user","content":"{s}"}}]}}
        , .{ HAIKU, user_content });

        const resp = try http_mod.post(ta, "https://api.anthropic.com/v1/messages", body, &.{
            .{ .name = "content-type", .value = "application/json" },
            .{ .name = "x-api-key", .value = self.api_key },
            .{ .name = "anthropic-version", .value = "2023-06-01" },
        });

        if (resp.status != .ok) {
            std.log.warn("Observer [{s}] API error {}: {s}", .{ entry.name, resp.status, resp.body });
            return error.ApiError;
        }

        // Response: {"content":[{"type":"text","text":"...json..."}],...}
        const outer = try json_mod.parse(ta, resp.body);
        const content = json_mod.getArray(outer.value, "content") orelse return error.ParseError;
        if (content.len == 0) return error.ParseError;
        const text = json_mod.getString(content[0], "text") orelse return error.ParseError;

        const inner = json_mod.parse(ta, stripFences(text)) catch return .{
            .triggered = false,
            .severity = .low,
            .summary = "",
            .recommendation = "",
        };

        const triggered = json_mod.getBool(inner.value, "triggered") orelse false;
        if (!triggered) return .{ .triggered = false, .severity = .low, .summary = "", .recommendation = "" };

        return .{
            .triggered = true,
            .severity = Severity.fromStr(json_mod.getString(inner.value, "severity") orelse "medium"),
            .summary = json_mod.getString(inner.value, "summary") orelse "",
            .recommendation = json_mod.getString(inner.value, "recommendation") orelse "",
        };
    }

    fn executeAction(self: *Observer, ta: std.mem.Allocator, name: []const u8, action: Action, result: AnalysisResult) !void {
        switch (action) {
            .alert => |chat_id| {
                const tg = self.tg orelse return;
                const raw_id = if (std.mem.startsWith(u8, chat_id, "tg:")) chat_id[3..] else chat_id;
                const msg = try std.fmt.allocPrint(ta, "[Observer: {s}] {s}\n\n{s}\n\nRecommendation: {s}", .{
                    name, result.severity.toStr(), result.summary, result.recommendation,
                });
                try tg.sendMessage(raw_id, msg, null);
            },
            .command => |cmd| {
                var child = std.process.Child.init(&.{ "/bin/sh", "-c", cmd }, self.allocator);
                child.stdout_behavior = .Ignore;
                child.stderr_behavior = .Ignore;
                try child.spawn();
                _ = try child.wait();
            },
            .webhook => |url| {
                const esc_name = try json_mod.escapeString(ta, name);
                const esc_summary = try json_mod.escapeString(ta, result.summary);
                const esc_rec = try json_mod.escapeString(ta, result.recommendation);
                const body = try std.fmt.allocPrint(ta,
                    \\{{"observer":"{s}","severity":"{s}","summary":"{s}","recommendation":"{s}"}}
                , .{ esc_name, result.severity.toStr(), esc_summary, esc_rec });
                const resp = try http_mod.postJson(ta, url, body);
                _ = resp;
            },
        }
    }
};

fn stripFences(text: []const u8) []const u8 {
    const t = std.mem.trim(u8, text, " \t\r\n");
    if (!std.mem.startsWith(u8, t, "```")) return t;
    const nl = std.mem.indexOfScalar(u8, t, '\n') orelse return t;
    const inner = t[nl + 1 ..];
    return if (std.mem.endsWith(u8, inner, "```"))
        std.mem.trimRight(u8, inner[0 .. inner.len - 3], " \t\r\n")
    else
        inner;
}

fn collectLogs(ta: std.mem.Allocator, entry: Entry) ![]u8 {
    const lines_str = try std.fmt.allocPrint(ta, "{d}", .{entry.window_lines});
    var argv = std.ArrayList([]const u8).init(ta);

    switch (entry.source) {
        .journalctl => |unit| try argv.appendSlice(&.{ "journalctl", "-u", unit, "-n", lines_str, "--no-pager", "--output=short-precise" }),
        .file_tail => |path| try argv.appendSlice(&.{ "tail", "-n", lines_str, path }),
        .command => |cmd| try argv.appendSlice(&.{ "/bin/sh", "-c", cmd }),
    }

    var child = std.process.Child.init(argv.items, ta);
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Ignore;
    try child.spawn();
    const output = try child.stdout.?.reader().readAllAlloc(ta, 4 * 1024 * 1024);
    _ = try child.wait();
    return output;
}

fn loadEntries(aa: std.mem.Allocator, path: []const u8) ![]Entry {
    const data = std.fs.cwd().readFileAlloc(aa, path, 64 * 1024) catch |err| {
        std.log.warn("Observer: can't read {s}: {}", .{ path, err });
        return &.{};
    };

    const parsed = try json_mod.parse(aa, data);
    if (parsed.value != .array) return error.InvalidObserverConfig;

    const items = parsed.value.array.items;
    const entries = try aa.alloc(Entry, items.len);
    var count: usize = 0;

    for (items) |item| {
        entries[count] = parseEntry(aa, item) catch |err| {
            std.log.warn("Observer: skipping invalid entry: {}", .{err});
            continue;
        };
        count += 1;
    }

    std.log.info("Observer: loaded {d} observer(s) from {s}", .{ count, path });
    return entries[0..count];
}

fn parseEntry(aa: std.mem.Allocator, v: json_mod.Value) !Entry {
    const name = json_mod.getString(v, "name") orelse return error.MissingName;
    const prompt = json_mod.getString(v, "prompt") orelse return error.MissingPrompt;
    const src_obj = json_mod.getObject(v, "source") orelse return error.MissingSource;
    const src_type = json_mod.getString(src_obj, "type") orelse return error.MissingSourceType;

    const source: Source = if (std.mem.eql(u8, src_type, "journalctl")) blk: {
        break :blk .{ .journalctl = try aa.dupe(u8, json_mod.getString(src_obj, "unit") orelse return error.MissingUnit) };
    } else if (std.mem.eql(u8, src_type, "file_tail")) blk: {
        break :blk .{ .file_tail = try aa.dupe(u8, json_mod.getString(src_obj, "path") orelse return error.MissingPath) };
    } else if (std.mem.eql(u8, src_type, "command")) blk: {
        break :blk .{ .command = try aa.dupe(u8, json_mod.getString(src_obj, "cmd") orelse return error.MissingCmd) };
    } else return error.UnknownSourceType;

    const actions_arr = json_mod.getArray(v, "actions") orelse &.{};
    const actions_buf = try aa.alloc(Action, actions_arr.len);
    var action_count: usize = 0;

    for (actions_arr) |av| {
        const atype = json_mod.getString(av, "type") orelse continue;
        actions_buf[action_count] = if (std.mem.eql(u8, atype, "alert")) blk: {
            break :blk .{ .alert = try aa.dupe(u8, json_mod.getString(av, "chat_id") orelse continue) };
        } else if (std.mem.eql(u8, atype, "command")) blk: {
            break :blk .{ .command = try aa.dupe(u8, json_mod.getString(av, "cmd") orelse continue) };
        } else if (std.mem.eql(u8, atype, "webhook")) blk: {
            break :blk .{ .webhook = try aa.dupe(u8, json_mod.getString(av, "url") orelse continue) };
        } else continue;
        action_count += 1;
    }

    return Entry{
        .name = try aa.dupe(u8, name),
        .source = source,
        .window_lines = @intCast(json_mod.getInt(v, "window_lines") orelse 200),
        .interval_s = json_mod.getInt(v, "interval_s") orelse 60,
        .prompt = try aa.dupe(u8, prompt),
        .cooldown_s = json_mod.getInt(v, "cooldown_s") orelse 300,
        .severity_threshold = Severity.fromStr(json_mod.getString(v, "severity_threshold") orelse "medium"),
        .actions = actions_buf[0..action_count],
    };
}
