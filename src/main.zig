const std = @import("std");
const Config = @import("config.zig").Config;
const db_mod = @import("db.zig");
const Db = db_mod.Db;
const tg_mod = @import("telegram.zig");
const Telegram = tg_mod.Telegram;
const TgMessage = tg_mod.TgMessage;
const docker_mod = @import("docker.zig");
const Docker = docker_mod.Docker;
const json_mod = @import("json.zig");
const git_mod = @import("git.zig");
const agent_mod = @import("agent.zig");
const pipeline_mod = @import("pipeline.zig");

const POLL_INTERVAL_MS = 1000;
const MAX_AGENT_RETRIES = 3;
const RETRY_BASE_MS = 1000;

const GroupState = struct {
    last_agent_timestamp: ?[]const u8,
    consecutive_errors: u32,
    agent_running: bool,
};

pub fn main() !void {
    var gpa = std.heap.GeneralPurposeAllocator(.{}){};
    defer _ = gpa.deinit();
    const allocator = gpa.allocator();

    var config = try Config.load(allocator);
    const start_time = std.time.timestamp();

    if (config.telegram_token.len == 0) {
        std.log.err("TELEGRAM_BOT_TOKEN not set", .{});
        return;
    }
    if (config.oauth_token.len == 0) {
        std.log.err("OAuth token not found (check ~/.claude/.credentials.json or CLAUDE_CODE_OAUTH_TOKEN in .env)", .{});
        return;
    }

    std.fs.cwd().makePath("store") catch {};
    std.fs.cwd().makePath("data/sessions") catch {};
    std.fs.cwd().makePath("data/ipc") catch {};

    var db = try Db.init(allocator, "store/borg.db");
    defer db.deinit();

    var tg = Telegram.init(allocator, config.telegram_token);
    try tg.connect();

    var docker = Docker.init(allocator);
    docker.cleanupOrphans() catch {};

    // Start pipeline thread if repo is configured
    var pipeline_db: ?Db = null;
    var pipeline: ?pipeline_mod.Pipeline = null;
    var pipeline_thread: ?std.Thread = null;
    defer {
        if (pipeline) |*p| {
            p.stop();
            if (pipeline_thread) |t| t.join();
        }
        if (pipeline_db) |*pdb| pdb.deinit();
    }

    if (config.pipeline_repo.len > 0) {
        pipeline_db = try Db.init(allocator, "store/borg.db");
        pipeline = pipeline_mod.Pipeline.init(
            allocator,
            &pipeline_db.?,
            &docker,
            &tg,
            &config,
        );
        pipeline_thread = try std.Thread.spawn(.{}, pipeline_mod.Pipeline.run, .{&pipeline.?});
        std.log.info("Pipeline thread started for: {s}", .{config.pipeline_repo});
    }

    var groups_list = std.ArrayList(db_mod.RegisteredGroup).init(allocator);
    defer groups_list.deinit();
    {
        const loaded = try db.getAllGroups(allocator);
        try groups_list.appendSlice(loaded);
        allocator.free(loaded);
    }
    std.log.info("Borg online | assistant: {s} | groups: {d}", .{ config.assistant_name, groups_list.items.len });

    var group_states = std.StringHashMap(GroupState).init(allocator);
    defer group_states.deinit();
    for (groups_list.items) |group| {
        try group_states.put(group.jid, GroupState{
            .last_agent_timestamp = null,
            .consecutive_errors = 0,
            .agent_running = false,
        });
    }

    while (true) {
        var arena = std.heap.ArenaAllocator.init(allocator);
        defer arena.deinit();
        const cycle_alloc = arena.allocator();

        db.expireSessions(config.session_max_age_hours) catch {};

        const messages = tg.getUpdates(cycle_alloc) catch |err| {
            std.log.err("Telegram poll error: {}", .{err});
            std.time.sleep(POLL_INTERVAL_MS * std.time.ns_per_ms);
            continue;
        };

        for (messages) |msg| {
            const chat_jid = try std.fmt.allocPrint(cycle_alloc, "tg:{s}", .{msg.chat_id});

            db.storeMessage(.{
                .id = msg.message_id,
                .chat_jid = chat_jid,
                .sender = msg.sender_id,
                .sender_name = msg.sender_name,
                .content = msg.text,
                .timestamp = try formatTimestamp(cycle_alloc, msg.date),
                .is_from_me = false,
            }) catch |err| {
                std.log.err("Store message: {}", .{err});
                continue;
            };

            // Handle commands
            if (msg.text.len > 0 and msg.text[0] == '/') {
                handleCommand(allocator, &db, &tg, msg, chat_jid, &groups_list, &group_states, &config, start_time, if (pipeline_db) |*pdb| pdb else null) catch |err| {
                    std.log.err("Command error: {}", .{err});
                };
                continue;
            }

            // Check if registered
            var registered_group: ?db_mod.RegisteredGroup = null;
            for (groups_list.items) |g| {
                if (std.mem.eql(u8, g.jid, chat_jid)) {
                    registered_group = g;
                    break;
                }
            }
            const group = registered_group orelse continue;

            // Check trigger
            if (group.requires_trigger) {
                if (!msg.mentions_bot and !containsTrigger(msg.text, config.assistant_name)) {
                    continue;
                }
            }

            // Check if agent already running for this group
            if (group_states.get(chat_jid)) |state| {
                if (state.agent_running) {
                    std.log.info("Agent already running for {s}, queueing", .{chat_jid});
                    continue;
                }
            }

            std.log.info("Triggered: \"{s}\" from {s}", .{ msg.text[0..@min(msg.text.len, 60)], msg.sender_name });
            tg.sendTyping(msg.chat_id) catch {};

            const state = group_states.get(chat_jid) orelse continue;
            const since = state.last_agent_timestamp orelse "";
            const pending = db.getMessagesSince(cycle_alloc, chat_jid, since) catch continue;
            if (pending.len == 0) continue;

            config.refreshOAuthToken();

            const prompt = try formatPrompt(cycle_alloc, pending, config.assistant_name);
            const session_id = db.getSession(cycle_alloc, group.folder) catch null;

            // Mark agent as running
            if (group_states.getPtr(chat_jid)) |s| {
                s.agent_running = true;
            }

            const result = runAgentWithRetry(allocator, &docker, config, group, prompt, session_id);

            // Mark agent as done
            if (group_states.getPtr(chat_jid)) |s| {
                s.agent_running = false;
            }

            if (result) |res| {
                defer allocator.free(res.output);
                defer if (res.new_session_id) |sid| allocator.free(sid);

                if (group_states.getPtr(chat_jid)) |s| {
                    s.consecutive_errors = 0;
                    if (s.last_agent_timestamp) |old| allocator.free(old);
                    s.last_agent_timestamp = allocator.dupe(u8, pending[pending.len - 1].timestamp) catch null;
                }

                if (res.new_session_id) |new_sid| {
                    db.setSession(group.folder, new_sid) catch {};
                }

                if (res.output.len > 0) {
                    db.storeMessage(.{
                        .id = try std.fmt.allocPrint(cycle_alloc, "bot-{d}", .{std.time.timestamp()}),
                        .chat_jid = chat_jid,
                        .sender = "borg",
                        .sender_name = config.assistant_name,
                        .content = res.output,
                        .timestamp = try formatTimestamp(cycle_alloc, std.time.timestamp()),
                        .is_from_me = true,
                    }) catch {};

                    tg.sendMessage(msg.chat_id, res.output, msg.message_id) catch |err| {
                        std.log.err("Send response: {}", .{err});
                    };
                }
            } else |err| {
                std.log.err("Agent failed after retries: {}", .{err});

                if (group_states.getPtr(chat_jid)) |s| {
                    s.consecutive_errors += 1;
                    if (s.consecutive_errors >= config.max_consecutive_errors) {
                        s.consecutive_errors = 0;
                        if (s.last_agent_timestamp) |old| allocator.free(old);
                        s.last_agent_timestamp = allocator.dupe(u8, pending[pending.len - 1].timestamp) catch null;
                    }
                }

                tg.sendMessage(msg.chat_id, "Sorry, I encountered an error processing your message. Please try again.", msg.message_id) catch {};
            }
        }

        std.time.sleep(POLL_INTERVAL_MS * std.time.ns_per_ms);
    }
}

fn handleCommand(
    allocator: std.mem.Allocator,
    db: *Db,
    tg: *Telegram,
    msg: TgMessage,
    chat_jid: []const u8,
    groups_list: *std.ArrayList(db_mod.RegisteredGroup),
    group_states: *std.StringHashMap(GroupState),
    config: *Config,
    start_time: i64,
    pipeline_db: ?*Db,
) !void {
    if (std.mem.startsWith(u8, msg.text, "/register")) {
        for (groups_list.items) |g| {
            if (std.mem.eql(u8, g.jid, chat_jid)) {
                try tg.sendMessage(msg.chat_id, "Already registered.", msg.message_id);
                return;
            }
        }

        const folder = try sanitizeFolder(allocator, msg.chat_title);
        const trigger = try std.fmt.allocPrint(allocator, "@{s}", .{config.assistant_name});
        const jid_dupe = try allocator.dupe(u8, chat_jid);
        const name_dupe = try allocator.dupe(u8, msg.chat_title);

        try db.registerGroup(jid_dupe, name_dupe, folder, trigger, true);

        try groups_list.append(db_mod.RegisteredGroup{
            .jid = jid_dupe,
            .name = name_dupe,
            .folder = folder,
            .trigger = trigger,
            .requires_trigger = true,
        });
        try group_states.put(jid_dupe, GroupState{
            .last_agent_timestamp = null,
            .consecutive_errors = 0,
            .agent_running = false,
        });

        var buf: [256]u8 = undefined;
        const reply = try std.fmt.bufPrint(&buf, "Registered! Mention @{s} to talk to me.", .{config.assistant_name});
        try tg.sendMessage(msg.chat_id, reply, msg.message_id);
        std.log.info("Registered: {s} ({s})", .{ msg.chat_title, chat_jid });
    } else if (std.mem.startsWith(u8, msg.text, "/unregister")) {
        var found_idx: ?usize = null;
        for (groups_list.items, 0..) |g, idx| {
            if (std.mem.eql(u8, g.jid, chat_jid)) {
                found_idx = idx;
                break;
            }
        }

        if (found_idx) |idx| {
            _ = groups_list.orderedRemove(idx);
            _ = group_states.remove(chat_jid);
            db.unregisterGroup(chat_jid) catch {};
            try tg.sendMessage(msg.chat_id, "Unregistered. I'll stop responding here.", msg.message_id);
            std.log.info("Unregistered: {s}", .{chat_jid});
        } else {
            try tg.sendMessage(msg.chat_id, "This chat is not registered.", msg.message_id);
        }
    } else if (std.mem.startsWith(u8, msg.text, "/status")) {
        const uptime_secs = std.time.timestamp() - start_time;
        const hours = @divTrunc(uptime_secs, 3600);
        const mins = @divTrunc(@mod(uptime_secs, 3600), 60);

        var active_agents: u32 = 0;
        var it = group_states.iterator();
        while (it.next()) |entry| {
            if (entry.value_ptr.agent_running) active_agents += 1;
        }

        var buf = std.ArrayList(u8).init(allocator);
        defer buf.deinit();
        try buf.writer().print(
            \\*Borg Status*
            \\Uptime: {d}h {d}m
            \\Groups: {d}
            \\Active agents: {d}
            \\Model: {s}
        , .{ hours, mins, groups_list.items.len, active_agents, config.model });

        if (config.pipeline_repo.len > 0) {
            try buf.writer().print("\nPipeline: active ({s})", .{config.pipeline_repo});
        }

        try tg.sendMessage(msg.chat_id, buf.items, msg.message_id);
    } else if (std.mem.startsWith(u8, msg.text, "/groups")) {
        if (groups_list.items.len == 0) {
            try tg.sendMessage(msg.chat_id, "No groups registered.", msg.message_id);
            return;
        }

        var buf = std.ArrayList(u8).init(allocator);
        defer buf.deinit();
        try buf.appendSlice("*Registered groups:*\n");
        for (groups_list.items) |g| {
            const status = if (group_states.get(g.jid)) |s| (if (s.agent_running) " (running)" else "") else "";
            try buf.writer().print("- {s} `{s}`{s}\n", .{ g.name, g.jid, status });
        }
        try tg.sendMessage(msg.chat_id, buf.items, msg.message_id);
    } else if (std.mem.startsWith(u8, msg.text, "/task ")) {
        const pdb = pipeline_db orelse {
            try tg.sendMessage(msg.chat_id, "Pipeline not configured. Set PIPELINE_REPO in .env", msg.message_id);
            return;
        };

        // Parse: /task <title> or /task <title>\n<description>
        const rest = std.mem.trim(u8, msg.text[6..], &[_]u8{ ' ', '\t' });
        if (rest.len == 0) {
            try tg.sendMessage(msg.chat_id, "Usage: /task <title> [description on next line]", msg.message_id);
            return;
        }

        var title: []const u8 = rest;
        var description: []const u8 = rest;
        if (std.mem.indexOf(u8, rest, "\n")) |nl| {
            title = std.mem.trim(u8, rest[0..nl], &[_]u8{ ' ', '\t', '\r' });
            description = std.mem.trim(u8, rest[nl + 1 ..], &[_]u8{ ' ', '\t', '\r' });
        }

        const task_id = try pdb.createPipelineTask(
            title,
            description,
            config.pipeline_repo,
            msg.sender_name,
            chat_jid,
        );

        var reply_buf: [256]u8 = undefined;
        const reply = try std.fmt.bufPrint(&reply_buf, "Task #{d} created: {s}", .{ task_id, title[0..@min(title.len, 100)] });
        try tg.sendMessage(msg.chat_id, reply, msg.message_id);
        std.log.info("Pipeline task #{d} created by {s}: {s}", .{ task_id, msg.sender_name, title });
    } else if (std.mem.startsWith(u8, msg.text, "/tasks")) {
        const pdb = pipeline_db orelse {
            try tg.sendMessage(msg.chat_id, "Pipeline not configured.", msg.message_id);
            return;
        };

        const tasks = try pdb.getAllPipelineTasks(allocator, 20);
        defer {
            for (tasks) |t| {
                allocator.free(t.title);
                allocator.free(t.description);
                allocator.free(t.repo_path);
                allocator.free(t.branch);
                allocator.free(t.status);
                allocator.free(t.last_error);
                allocator.free(t.created_by);
                allocator.free(t.notify_chat);
                allocator.free(t.created_at);
            }
            allocator.free(tasks);
        }

        if (tasks.len == 0) {
            try tg.sendMessage(msg.chat_id, "No pipeline tasks.", msg.message_id);
            return;
        }

        var buf = std.ArrayList(u8).init(allocator);
        defer buf.deinit();
        try buf.appendSlice("*Pipeline Tasks*\n");
        for (tasks) |t| {
            const status_icon = if (std.mem.eql(u8, t.status, "done") or std.mem.eql(u8, t.status, "merged"))
                "+"
            else if (std.mem.eql(u8, t.status, "failed"))
                "x"
            else
                "~";
            try buf.writer().print("[{s}] #{d} {s} ({s})\n", .{ status_icon, t.id, t.title[0..@min(t.title.len, 50)], t.status });
        }
        try tg.sendMessage(msg.chat_id, buf.items, msg.message_id);
    } else if (std.mem.startsWith(u8, msg.text, "/pipeline")) {
        if (config.pipeline_repo.len == 0) {
            try tg.sendMessage(msg.chat_id, "Pipeline not configured. Set PIPELINE_REPO in .env", msg.message_id);
            return;
        }

        var buf = std.ArrayList(u8).init(allocator);
        defer buf.deinit();
        try buf.writer().print(
            \\*Pipeline Info*
            \\Repo: {s}
            \\Test cmd: {s}
            \\Release interval: {d}h
            \\Model: {s}
        , .{ config.pipeline_repo, config.pipeline_test_cmd, config.release_interval_hours, config.model });
        try tg.sendMessage(msg.chat_id, buf.items, msg.message_id);
    } else if (std.mem.startsWith(u8, msg.text, "/chatid")) {
        var buf: [256]u8 = undefined;
        const reply = try std.fmt.bufPrint(&buf, "Chat: `{s}`\nType: {s}\nName: {s}", .{ chat_jid, msg.chat_type, msg.chat_title });
        try tg.sendMessage(msg.chat_id, reply, msg.message_id);
    } else if (std.mem.startsWith(u8, msg.text, "/ping")) {
        try tg.sendMessage(msg.chat_id, "Borg online.", msg.message_id);
    } else if (std.mem.startsWith(u8, msg.text, "/help") or std.mem.startsWith(u8, msg.text, "/start")) {
        try tg.sendMessage(msg.chat_id,
            \\*Borg Commands*
            \\/register - Register this chat
            \\/unregister - Unregister this chat
            \\/status - Show bot status
            \\/groups - List registered groups
            \\/chatid - Show chat ID
            \\/ping - Check if online
            \\
            \\*Pipeline*
            \\/task <title> - Create engineering task
            \\/tasks - List pipeline tasks
            \\/pipeline - Show pipeline info
        , msg.message_id);
    }
}

fn sanitizeFolder(allocator: std.mem.Allocator, name: []const u8) ![]const u8 {
    var buf = std.ArrayList(u8).init(allocator);
    for (name) |ch| {
        if (std.ascii.isAlphanumeric(ch)) {
            try buf.append(std.ascii.toLower(ch));
        } else if (ch == ' ' or ch == '-' or ch == '_') {
            if (buf.items.len > 0 and buf.items[buf.items.len - 1] != '-') {
                try buf.append('-');
            }
        }
    }
    while (buf.items.len > 0 and buf.items[buf.items.len - 1] == '-') {
        _ = buf.pop();
    }
    if (buf.items.len == 0) try buf.appendSlice("chat");
    return buf.toOwnedSlice();
}

const AgentResult = agent_mod.AgentResult;
const parseNdjson = agent_mod.parseNdjson;

/// Run agent with exponential backoff retry
fn runAgentWithRetry(
    allocator: std.mem.Allocator,
    docker: *Docker,
    config: Config,
    group: db_mod.RegisteredGroup,
    prompt: []const u8,
    session_id: ?[]const u8,
) !AgentResult {
    var last_err: anyerror = error.DockerCreateFailed;

    for (0..MAX_AGENT_RETRIES) |attempt| {
        const result = runAgent(allocator, docker, config, group, prompt, session_id) catch |err| {
            last_err = err;
            const delay = RETRY_BASE_MS * (@as(u64, 1) << @intCast(attempt));
            std.log.warn("Agent attempt {d}/{d} failed: {} (retry in {d}ms)", .{ attempt + 1, MAX_AGENT_RETRIES, err, delay });
            std.time.sleep(delay * std.time.ns_per_ms);
            continue;
        };
        return result;
    }

    return last_err;
}

fn runAgent(
    allocator: std.mem.Allocator,
    docker: *Docker,
    config: Config,
    group: db_mod.RegisteredGroup,
    prompt: []const u8,
    session_id: ?[]const u8,
) !AgentResult {
    var arena = std.heap.ArenaAllocator.init(allocator);
    defer arena.deinit();
    const tmp = arena.allocator();

    var input_json = std.ArrayList(u8).init(tmp);
    const escaped_prompt = try json_mod.escapeString(tmp, prompt);
    try input_json.writer().print("{{\"prompt\":\"{s}\"", .{escaped_prompt});
    if (session_id) |sid| {
        try input_json.writer().print(",\"sessionId\":\"{s}\"", .{sid});
    }
    try input_json.writer().print(",\"model\":\"{s}\",\"assistantName\":\"{s}\"}}", .{ config.model, config.assistant_name });

    var name_buf: [128]u8 = undefined;
    const container_name = try std.fmt.bufPrint(&name_buf, "borg-{s}-{d}", .{ group.folder, std.time.timestamp() });

    var oauth_buf: [4096]u8 = undefined;
    const oauth_env = try std.fmt.bufPrint(&oauth_buf, "CLAUDE_CODE_OAUTH_TOKEN={s}", .{config.oauth_token});
    var model_buf: [256]u8 = undefined;
    const model_env = try std.fmt.bufPrint(&model_buf, "CLAUDE_MODEL={s}", .{config.model});

    const env = [_][]const u8{
        oauth_env,
        model_env,
        "HOME=/home/node",
        "NODE_OPTIONS=--max-old-space-size=384",
    };

    var cwd_buf: [512]u8 = undefined;
    const cwd = try std.fs.cwd().realpath(".", &cwd_buf);

    var session_dir_buf: [512]u8 = undefined;
    const session_dir = try std.fmt.bufPrint(&session_dir_buf, "data/sessions/{s}", .{group.folder});
    std.fs.cwd().makePath(session_dir) catch {};

    var session_bind_buf: [1024]u8 = undefined;
    const session_bind = try std.fmt.bufPrint(&session_bind_buf, "{s}/{s}:/home/node/.claude/projects/{s}", .{ cwd, session_dir, group.folder });

    var ipc_dir_buf: [512]u8 = undefined;
    const ipc_dir = try std.fmt.bufPrint(&ipc_dir_buf, "data/ipc/{s}", .{group.folder});
    std.fs.cwd().makePath(ipc_dir) catch {};

    var ipc_bind_buf: [1024]u8 = undefined;
    const ipc_bind = try std.fmt.bufPrint(&ipc_bind_buf, "{s}/{s}:/workspace/ipc", .{ cwd, ipc_dir });

    const binds = [_][]const u8{
        session_bind,
        ipc_bind,
    };

    std.log.info("Spawning agent: {s}", .{container_name});

    var run_result = try docker.runWithStdio(docker_mod.ContainerConfig{
        .image = config.container_image,
        .name = container_name,
        .env = &env,
        .binds = &binds,
    }, input_json.items);
    defer run_result.deinit();

    std.log.info("Agent done (exit={d}, {d} bytes)", .{ run_result.exit_code, run_result.stdout.len });

    return try parseNdjson(allocator, run_result.stdout);
}

fn formatPrompt(allocator: std.mem.Allocator, messages: []const db_mod.Message, assistant_name: []const u8) ![]const u8 {
    var buf = std.ArrayList(u8).init(allocator);
    try buf.writer().print("You are {s}, a helpful AI assistant in a group chat. Respond naturally and concisely.\n\nRecent messages:\n", .{assistant_name});
    for (messages) |msg| {
        if (msg.is_from_me) {
            try buf.writer().print("[{s}] {s} (you): {s}\n", .{ msg.timestamp, msg.sender_name, msg.content });
        } else {
            try buf.writer().print("[{s}] {s}: {s}\n", .{ msg.timestamp, msg.sender_name, msg.content });
        }
    }
    try buf.appendSlice("\nRespond to the latest message. Be concise.");
    return buf.toOwnedSlice();
}

fn containsTrigger(text: []const u8, assistant_name: []const u8) bool {
    if (text.len < assistant_name.len + 1) return false;
    var i: usize = 0;
    while (i < text.len) : (i += 1) {
        if (text[i] == '@' and i + 1 + assistant_name.len <= text.len) {
            if (std.ascii.eqlIgnoreCase(text[i + 1 .. i + 1 + assistant_name.len], assistant_name)) {
                return true;
            }
        }
    }
    return false;
}

fn formatTimestamp(allocator: std.mem.Allocator, unix_ts: i64) ![]const u8 {
    const epoch = std.time.epoch.EpochSeconds{ .secs = @intCast(unix_ts) };
    const day_seconds = epoch.getDaySeconds();
    const year_day = epoch.getEpochDay().calculateYearDay();
    const month_day = year_day.calculateMonthDay();
    return std.fmt.allocPrint(allocator, "{d:0>4}-{d:0>2}-{d:0>2}T{d:0>2}:{d:0>2}:{d:0>2}Z", .{
        year_day.year,
        @intFromEnum(month_day.month),
        month_day.day_index + 1,
        day_seconds.getHoursIntoDay(),
        day_seconds.getMinutesIntoHour(),
        day_seconds.getSecondsIntoMinute(),
    });
}

// ── Tests ──────────────────────────────────────────────────────────────

test "containsTrigger" {
    try std.testing.expect(containsTrigger("Hey @Borg do something", "Borg"));
    try std.testing.expect(containsTrigger("@borg help", "Borg"));
    try std.testing.expect(!containsTrigger("Hello there", "Borg"));
    try std.testing.expect(!containsTrigger("@Bo", "Borg"));
    try std.testing.expect(containsTrigger("text @MiniShulgin more", "MiniShulgin"));
    try std.testing.expect(!containsTrigger("@", "Borg"));
    try std.testing.expect(!containsTrigger("", "Borg"));
}

test "formatTimestamp" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    // 2024-02-26 00:00:00 UTC
    const ts = try formatTimestamp(arena.allocator(), 1708905600);
    try std.testing.expectEqualStrings("2024-02-26T00:00:00Z", ts);
}

test "sanitizeFolder" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const a = arena.allocator();

    try std.testing.expectEqualStrings("my-test-group", try sanitizeFolder(a, "My Test Group"));
    try std.testing.expectEqualStrings("hello", try sanitizeFolder(a, "hello"));
    try std.testing.expectEqualStrings("chat", try sanitizeFolder(a, "---"));
    try std.testing.expectEqualStrings("chat", try sanitizeFolder(a, "   "));
    try std.testing.expectEqualStrings("abc-123", try sanitizeFolder(a, "ABC 123!@#"));
    try std.testing.expectEqualStrings("a-b", try sanitizeFolder(a, "a---b"));
}

test "formatPrompt includes assistant identity and message context" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const a = arena.allocator();

    const msgs = [_]db_mod.Message{
        .{ .id = "1", .chat_jid = "tg:1", .sender = "u1", .sender_name = "Alice", .content = "Hi bot", .timestamp = "2024-01-01T00:00:00Z", .is_from_me = false },
        .{ .id = "2", .chat_jid = "tg:1", .sender = "bot", .sender_name = "Borg", .content = "Hello!", .timestamp = "2024-01-01T00:00:01Z", .is_from_me = true },
    };

    const prompt = try formatPrompt(a, &msgs, "Borg");
    try std.testing.expect(std.mem.indexOf(u8, prompt, "You are Borg") != null);
    try std.testing.expect(std.mem.indexOf(u8, prompt, "Alice: Hi bot") != null);
    try std.testing.expect(std.mem.indexOf(u8, prompt, "Borg (you): Hello!") != null);
    try std.testing.expect(std.mem.indexOf(u8, prompt, "Be concise") != null);
}
