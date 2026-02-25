const std = @import("std");
const build_options = @import("build_options");
const Config = @import("config.zig").Config;
const db_mod = @import("db.zig");
const Db = db_mod.Db;
const tg_mod = @import("telegram.zig");
const Telegram = tg_mod.Telegram;
const TgMessage = tg_mod.TgMessage;
const docker_mod = @import("docker.zig");
const Docker = docker_mod.Docker;
const json_mod = @import("json.zig");
const agent_mod = @import("agent.zig");
const pipeline_mod = @import("pipeline.zig");
const sidecar_mod = @import("sidecar.zig");
const Sidecar = sidecar_mod.Sidecar;
const web_mod = @import("web.zig");
const WebServer = web_mod.WebServer;

pub const version = "0.1.0-" ++ build_options.git_hash;

const POLL_INTERVAL_MS = 500;

// ── Custom Logger (feeds web dashboard) ─────────────────────────────────

var web_server_global: ?*WebServer = null;

pub const std_options: std.Options = .{
    .logFn = borgLogFn,
};

fn borgLogFn(
    comptime level: std.log.Level,
    comptime scope: @TypeOf(.enum_literal),
    comptime format: []const u8,
    args: anytype,
) void {
    _ = scope;
    const level_str = switch (level) {
        .err => "err",
        .warn => "warn",
        .info => "info",
        .debug => "debug",
    };

    // Print to stderr (default behavior)
    const stderr = std.io.getStdErr().writer();
    stderr.print(level_str ++ ": " ++ format ++ "\n", args) catch {};

    // Forward to web dashboard if available
    if (web_server_global) |ws| {
        var buf: [512]u8 = undefined;
        const msg = std.fmt.bufPrint(&buf, format, args) catch return;
        ws.pushLog(level_str, msg);
    }
}

// ── Signal Handler ──────────────────────────────────────────────────────

var shutdown_requested = std.atomic.Value(bool).init(false);

fn signalHandler(_: c_int) callconv(.c) void {
    shutdown_requested.store(true, .release);
}

fn installSignalHandlers() void {
    const act = std.posix.Sigaction{
        .handler = .{ .handler = signalHandler },
        .mask = std.posix.empty_sigset,
        .flags = 0,
    };
    std.posix.sigaction(std.posix.SIG.TERM, &act, null);
    std.posix.sigaction(std.posix.SIG.INT, &act, null);
}

// ── Types ───────────────────────────────────────────────────────────────

const Transport = enum { telegram, whatsapp, discord, web };

const GroupPhase = enum { idle, collecting, running, cooldown };

const GroupState = struct {
    phase: GroupPhase = .idle,
    last_agent_timestamp: ?[]const u8 = null,
    consecutive_errors: u32 = 0,

    // Collecting phase
    collect_deadline_ms: i64 = 0,
    trigger_msg_id: ?[]const u8 = null,
    original_id: ?[]const u8 = null,
    transport: Transport = .telegram,

    // Running phase
    agent_thread: ?std.Thread = null,

    // Result (written by agent thread, read by main loop)
    completed_result: ?*AgentOutcome = null,

    // Rate limiting
    rate_window_start_ms: i64 = 0,
    trigger_count: u32 = 0,

    // Cooldown
    cooldown_deadline_ms: i64 = 0,
};

const AgentOutcome = struct {
    output: []const u8,
    new_session_id: ?[]const u8,
    success: bool,
    last_msg_timestamp: []const u8,

    fn deinit(self: *AgentOutcome, allocator: std.mem.Allocator) void {
        allocator.free(self.output);
        if (self.new_session_id) |sid| allocator.free(sid);
        allocator.free(self.last_msg_timestamp);
        allocator.destroy(self);
    }
};

const AgentContext = struct {
    allocator: std.mem.Allocator,
    jid: []const u8,
    folder: []const u8,
    prompt: []const u8,
    session_id: ?[]const u8,
    model: []const u8,
    oauth_token: []const u8,
    assistant_name: []const u8,
    last_msg_timestamp: []const u8,

    fn deinit(self: *AgentContext) void {
        self.allocator.free(self.jid);
        self.allocator.free(self.folder);
        self.allocator.free(self.prompt);
        if (self.session_id) |s| self.allocator.free(s);
        self.allocator.free(self.model);
        self.allocator.free(self.oauth_token);
        self.allocator.free(self.assistant_name);
        self.allocator.free(self.last_msg_timestamp);
    }
};

const IncomingMessage = struct {
    jid: []const u8,
    original_id: []const u8,
    message_id: []const u8,
    sender: []const u8,
    sender_name: []const u8,
    text: []const u8,
    timestamp: i64,
    mentions_bot: bool,
    transport: Transport,
    chat_title: []const u8,
    chat_type: []const u8,
};

const Sender = struct {
    tg: *Telegram,
    sidecar: ?*Sidecar,
    web: ?*WebServer,

    fn send(self: Sender, transport: Transport, original_id: []const u8, text: []const u8, reply_to: ?[]const u8) void {
        switch (transport) {
            .telegram => self.tg.sendMessage(original_id, text, reply_to) catch |err| {
                std.log.err("TG send: {}", .{err});
            },
            .whatsapp => if (self.sidecar) |s| {
                s.sendWhatsApp(original_id, text, reply_to) catch |err| {
                    std.log.err("WA send: {}", .{err});
                };
            },
            .discord => if (self.sidecar) |s| {
                s.sendDiscord(original_id, text, reply_to) catch |err| {
                    std.log.err("Discord send: {}", .{err});
                };
            },
            .web => if (self.web) |ws| {
                ws.broadcastChatEvent(text);
            },
        }
    }

    fn sendTyping(self: Sender, transport: Transport, original_id: []const u8) void {
        switch (transport) {
            .telegram => self.tg.sendTyping(original_id) catch {},
            .whatsapp => if (self.sidecar) |s| s.sendWhatsAppTyping(original_id) catch {},
            .discord => if (self.sidecar) |s| s.sendDiscordTyping(original_id) catch {},
            .web => {},
        }
    }
};

// ── Group Manager ───────────────────────────────────────────────────────

const GroupManager = struct {
    mu: std.Thread.Mutex = .{},
    states: std.StringHashMap(GroupState),
    active_agents: u32 = 0,
    allocator: std.mem.Allocator,

    fn init(allocator: std.mem.Allocator) GroupManager {
        return .{
            .states = std.StringHashMap(GroupState).init(allocator),
            .allocator = allocator,
        };
    }

    fn deinit(self: *GroupManager) void {
        var it = self.states.iterator();
        while (it.next()) |entry| {
            const state = entry.value_ptr;
            if (state.last_agent_timestamp) |ts| self.allocator.free(ts);
            if (state.trigger_msg_id) |m| self.allocator.free(m);
            if (state.original_id) |o| self.allocator.free(o);
        }
        self.states.deinit();
    }

    fn addGroup(self: *GroupManager, jid: []const u8) void {
        self.mu.lock();
        defer self.mu.unlock();
        self.states.put(jid, .{}) catch {};
    }

    fn removeGroup(self: *GroupManager, jid: []const u8) void {
        self.mu.lock();
        defer self.mu.unlock();
        _ = self.states.remove(jid);
    }

    fn getPhase(self: *GroupManager, jid: []const u8) ?GroupPhase {
        self.mu.lock();
        defer self.mu.unlock();
        const state = self.states.get(jid) orelse return null;
        return state.phase;
    }

    fn getActiveCount(self: *GroupManager) u32 {
        self.mu.lock();
        defer self.mu.unlock();
        return self.active_agents;
    }

    /// Transition IDLE → COLLECTING on trigger. Returns true if accepted.
    fn onTrigger(
        self: *GroupManager,
        jid: []const u8,
        msg_id: []const u8,
        original_id: []const u8,
        transport: Transport,
        config: *const Config,
    ) bool {
        self.mu.lock();
        defer self.mu.unlock();

        const state = self.states.getPtr(jid) orelse return false;
        if (state.phase != .idle) return false;

        // Rate limiting
        const now = nowMs();
        if (now - state.rate_window_start_ms > 60_000) {
            state.rate_window_start_ms = now;
            state.trigger_count = 0;
        }
        if (state.trigger_count >= config.rate_limit_per_minute) return false;
        if (self.active_agents >= config.max_concurrent_agents) return false;

        state.trigger_count += 1;
        state.phase = .collecting;
        state.collect_deadline_ms = now + config.collection_window_ms;
        state.trigger_msg_id = self.allocator.dupe(u8, msg_id) catch return false;
        state.original_id = self.allocator.dupe(u8, original_id) catch return false;
        state.transport = transport;

        return true;
    }

    /// If message arrives during COLLECTING, extend deadline slightly.
    fn extendCollection(self: *GroupManager, jid: []const u8, extension_ms: i64) void {
        self.mu.lock();
        defer self.mu.unlock();
        const state = self.states.getPtr(jid) orelse return;
        if (state.phase != .collecting) return;
        const now = nowMs();
        const new_deadline = now + extension_ms;
        if (new_deadline > state.collect_deadline_ms) {
            state.collect_deadline_ms = @min(new_deadline, state.collect_deadline_ms + 2000);
        }
    }

    const SpawnInfo = struct {
        jid: []const u8,
        original_id: []const u8,
        trigger_msg_id: ?[]const u8,
        transport: Transport,
    };

    /// Collect groups with expired collection windows. Caller processes them outside the lock.
    fn getExpiredCollections(self: *GroupManager, buf: *std.ArrayList(SpawnInfo)) void {
        self.mu.lock();
        defer self.mu.unlock();
        const now = nowMs();
        var it = self.states.iterator();
        while (it.next()) |entry| {
            const state = entry.value_ptr;
            if (state.phase == .collecting and now >= state.collect_deadline_ms) {
                buf.append(.{
                    .jid = entry.key_ptr.*,
                    .original_id = state.original_id orelse "",
                    .trigger_msg_id = state.trigger_msg_id,
                    .transport = state.transport,
                }) catch {};
            }
        }
    }

    /// Transition COLLECTING → RUNNING. Returns false if state changed.
    fn startRunning(self: *GroupManager, jid: []const u8, thread: std.Thread) bool {
        self.mu.lock();
        defer self.mu.unlock();
        const state = self.states.getPtr(jid) orelse return false;
        if (state.phase != .collecting) return false;
        state.phase = .running;
        state.agent_thread = thread;
        self.active_agents += 1;
        return true;
    }

    /// Write outcome from agent thread.
    fn setOutcome(self: *GroupManager, jid: []const u8, outcome: *AgentOutcome) void {
        self.mu.lock();
        defer self.mu.unlock();
        if (self.states.getPtr(jid)) |state| {
            state.completed_result = outcome;
        } else {
            outcome.deinit(self.allocator);
        }
    }

    const DeliveryInfo = struct {
        jid: []const u8,
        original_id: []const u8,
        trigger_msg_id: ?[]const u8,
        transport: Transport,
        folder: []const u8,
        outcome: *AgentOutcome,
        thread: std.Thread,
    };

    /// Collect completed agents for delivery. Transitions RUNNING → COOLDOWN.
    fn getCompletedAgents(self: *GroupManager, buf: *std.ArrayList(DeliveryInfo), groups: []const db_mod.RegisteredGroup, cooldown_ms: i64) void {
        self.mu.lock();
        defer self.mu.unlock();
        const now = nowMs();
        var it = self.states.iterator();
        while (it.next()) |entry| {
            const state = entry.value_ptr;
            if (state.phase == .running and state.completed_result != null) {
                const folder = for (groups) |g| {
                    if (std.mem.eql(u8, g.jid, entry.key_ptr.*)) break g.folder;
                } else "";

                buf.append(.{
                    .jid = entry.key_ptr.*,
                    .original_id = state.original_id orelse "",
                    .trigger_msg_id = state.trigger_msg_id,
                    .transport = state.transport,
                    .folder = folder,
                    .outcome = state.completed_result.?,
                    .thread = state.agent_thread.?,
                }) catch {};

                // Transition to cooldown
                state.phase = .cooldown;
                state.cooldown_deadline_ms = now + cooldown_ms;
                state.completed_result = null;
                state.agent_thread = null;
                // Keep original_id/trigger_msg_id alive for delivery
                if (self.active_agents > 0) self.active_agents -= 1;
            }
        }
    }

    /// Transition expired COOLDOWN → IDLE. Cleans up transient state.
    fn expireCooldowns(self: *GroupManager) void {
        self.mu.lock();
        defer self.mu.unlock();
        const now = nowMs();
        var it = self.states.iterator();
        while (it.next()) |entry| {
            const state = entry.value_ptr;
            if (state.phase == .cooldown and now >= state.cooldown_deadline_ms) {
                state.phase = .idle;
                if (state.trigger_msg_id) |m| self.allocator.free(m);
                state.trigger_msg_id = null;
                if (state.original_id) |o| self.allocator.free(o);
                state.original_id = null;
            }
        }
    }

    /// Join all running agent threads (for shutdown).
    fn joinAll(self: *GroupManager) void {
        self.mu.lock();
        var threads = std.ArrayList(std.Thread).init(self.allocator);
        defer threads.deinit();
        var it = self.states.iterator();
        while (it.next()) |entry| {
            if (entry.value_ptr.agent_thread) |t| {
                threads.append(t) catch {};
                entry.value_ptr.agent_thread = null;
            }
        }
        self.mu.unlock();

        for (threads.items) |t| {
            t.join();
        }
    }
};

// ── Self-Update ─────────────────────────────────────────────────────────

fn reexecSelf() void {
    // Read the binary path from /proc/self/exe
    var exe_buf: [std.fs.max_path_bytes]u8 = undefined;
    var exe_path = std.fs.readLinkAbsolute("/proc/self/exe", &exe_buf) catch {
        std.log.err("Self-update: failed to read /proc/self/exe", .{});
        return;
    };

    // Linux appends " (deleted)" when the running binary was overwritten
    if (std.mem.endsWith(u8, exe_path, " (deleted)")) {
        exe_path = exe_path[0 .. exe_path.len - " (deleted)".len];
    }

    // Null-terminate for execve
    const exe_z = std.posix.toPosixPath(exe_path) catch {
        std.log.err("Self-update: path too long", .{});
        return;
    };

    // argv: just the binary itself (no args needed)
    const argv = [_:null]?[*:0]const u8{&exe_z};
    // Inherit current environment
    const envp = std.c.environ;

    const err = std.posix.execveZ(&exe_z, &argv, envp);
    std.log.err("Self-update: execve failed: {}", .{err});
}

// ── Main ────────────────────────────────────────────────────────────────

pub fn main() !void {
    var gpa = std.heap.GeneralPurposeAllocator(.{}){};
    defer _ = gpa.deinit();
    const allocator = gpa.allocator();

    installSignalHandlers();

    var config = try Config.load(allocator);
    const start_time = std.time.timestamp();

    // Startup validation
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

    // Resume: reset any stuck queue entries and failed tasks from a previous crash/restart
    db.resetStuckQueueEntries() catch {};
    db.recycleFailedTasks() catch {};

    var tg = Telegram.init(allocator, config.telegram_token);
    try tg.connect();

    var docker = Docker.init(allocator);

    // Validate Docker + image if pipeline is configured
    if (config.pipeline_repo.len > 0) {
        if (!docker.isAvailable()) {
            std.log.err("Docker daemon not reachable but PIPELINE_REPO is set", .{});
            return;
        }
        docker.cleanupOrphans() catch {};
    }

    // Start unified sidecar (Discord + WhatsApp in one bun process)
    var sidecar: ?Sidecar = null;
    defer if (sidecar) |*s| s.deinit();
    if (config.discord_enabled or config.whatsapp_enabled) {
        sidecar = Sidecar.init(allocator, config.assistant_name);
        sidecar.?.start(config.discord_token, config.whatsapp_auth_dir, !config.whatsapp_enabled) catch |err| {
            std.log.err("Sidecar start failed: {}", .{err});
            sidecar = null;
        };
    }

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
        pipeline = pipeline_mod.Pipeline.init(allocator, &pipeline_db.?, &docker, &tg, &config);
        pipeline_thread = try std.Thread.spawn(.{}, pipeline_mod.Pipeline.run, .{&pipeline.?});
        std.log.info("Pipeline thread started for: {s}", .{config.pipeline_repo});
    }

    // Start web dashboard
    var web_db = try Db.init(allocator, "store/borg.db");
    var web = WebServer.init(allocator, &web_db, &config, config.web_port);
    if (pipeline) |*p| web.force_restart_signal = &p.force_restart;
    web_server_global = &web;
    const web_thread = try std.Thread.spawn(.{}, WebServer.run, .{&web});

    var sender = Sender{ .tg = &tg, .sidecar = if (sidecar) |*s| s else null, .web = &web };
    defer {
        web.stop();
        web_thread.join();
        web_server_global = null;
        web_db.deinit();
    }

    // Load registered groups
    var groups_list = std.ArrayList(db_mod.RegisteredGroup).init(allocator);
    defer groups_list.deinit();
    {
        const loaded = try db.getAllGroups(allocator);
        try groups_list.appendSlice(loaded);
        allocator.free(loaded);
    }

    var gm = GroupManager.init(allocator);
    defer gm.deinit();
    for (groups_list.items) |group| {
        gm.addGroup(group.jid);
    }

    // Auto-register web:dashboard for the dashboard chat
    {
        const web_jid = "web:dashboard";
        const already = for (groups_list.items) |g| {
            if (std.mem.eql(u8, g.jid, web_jid)) break true;
        } else false;
        if (!already) {
            db.registerGroup(web_jid, "Dashboard", "dashboard", "@" ++ "Borg", false) catch {};
            try groups_list.append(.{
                .jid = web_jid,
                .name = "Dashboard",
                .folder = "dashboard",
                .trigger = "@Borg",
                .requires_trigger = false,
            });
            gm.addGroup(web_jid);
        }
    }

    std.log.info("Borg {s} online | assistant: {s} | groups: {d}", .{ version, config.assistant_name, groups_list.items.len });

    // ── Main Loop ───────────────────────────────────────────────────────

    var session_expire_counter: u32 = 0;

    while (!shutdown_requested.load(.acquire)) {
        var arena = std.heap.ArenaAllocator.init(allocator);
        defer arena.deinit();
        const cycle_alloc = arena.allocator();

        // Expire sessions periodically (every ~60 cycles = ~2 minutes)
        session_expire_counter += 1;
        if (session_expire_counter >= 60) {
            session_expire_counter = 0;
            db.expireSessions(config.session_max_age_hours) catch {};
        }

        // 1. Poll Telegram messages
        var all_messages = std.ArrayList(IncomingMessage).init(cycle_alloc);

        const tg_msgs = tg.getUpdates(cycle_alloc) catch |err| {
            std.log.err("Telegram poll error: {}", .{err});
            std.time.sleep(POLL_INTERVAL_MS * std.time.ns_per_ms);
            continue;
        };

        for (tg_msgs) |msg| {
            all_messages.append(.{
                .jid = try std.fmt.allocPrint(cycle_alloc, "tg:{s}", .{msg.chat_id}),
                .original_id = msg.chat_id,
                .message_id = msg.message_id,
                .sender = msg.sender_id,
                .sender_name = msg.sender_name,
                .text = msg.text,
                .timestamp = msg.date,
                .mentions_bot = msg.mentions_bot,
                .transport = .telegram,
                .chat_title = msg.chat_title,
                .chat_type = msg.chat_type,
            }) catch {};
        }

        // 2. Poll sidecar (WhatsApp + Discord)
        if (sidecar) |*s| {
            const sc_msgs = s.poll(cycle_alloc) catch &[_]sidecar_mod.SidecarMessage{};
            for (sc_msgs) |msg| {
                const prefix: []const u8 = if (msg.source == .discord) "discord" else "wa";
                const transport: Transport = if (msg.source == .discord) .discord else .whatsapp;
                all_messages.append(.{
                    .jid = std.fmt.allocPrint(cycle_alloc, "{s}:{s}", .{ prefix, msg.chat_id }) catch continue,
                    .original_id = msg.chat_id,
                    .message_id = msg.id,
                    .sender = msg.sender,
                    .sender_name = msg.sender_name,
                    .text = msg.text,
                    .timestamp = msg.timestamp,
                    .mentions_bot = msg.mentions_bot,
                    .transport = transport,
                    .chat_title = msg.chat_id,
                    .chat_type = if (msg.is_group) "group" else "private",
                }) catch {};
            }
        }

        // 2c. Drain web chat messages
        {
            const web_msgs = web.drainChatMessages();
            defer {
                for (web_msgs) |wm| {
                    allocator.free(wm.sender_name);
                    allocator.free(wm.text);
                }
                allocator.free(web_msgs);
            }
            for (web_msgs) |wm| {
                const ts_str = formatTimestamp(cycle_alloc, wm.timestamp) catch continue;
                all_messages.append(.{
                    .jid = "web:dashboard",
                    .original_id = "web:dashboard",
                    .message_id = std.fmt.allocPrint(cycle_alloc, "web-{d}", .{wm.timestamp}) catch continue,
                    .sender = wm.sender_name,
                    .sender_name = wm.sender_name,
                    .text = wm.text,
                    .timestamp = wm.timestamp,
                    .mentions_bot = true,
                    .transport = .web,
                    .chat_title = "Dashboard",
                    .chat_type = "private",
                }) catch {};
                _ = ts_str;
            }
        }

        // 3. Process incoming messages
        for (all_messages.items) |msg| {
            processIncomingMessage(
                allocator,
                cycle_alloc,
                &db,
                &sender,
                msg,
                &groups_list,
                &gm,
                &config,
                start_time,
                if (pipeline_db) |*pdb| pdb else null,
            );
        }

        // 4. Check expired collection windows → spawn agent threads
        {
            var spawn_list = std.ArrayList(GroupManager.SpawnInfo).init(cycle_alloc);
            gm.getExpiredCollections(&spawn_list);

            for (spawn_list.items) |info| {
                spawnAgentForGroup(allocator, cycle_alloc, &gm, &db, &config, info, groups_list.items) catch |err| {
                    std.log.err("Spawn agent for {s}: {}", .{ info.jid, err });
                };
            }
        }

        // 5. Check completed agents → deliver responses
        {
            var deliveries = std.ArrayList(GroupManager.DeliveryInfo).init(cycle_alloc);
            gm.getCompletedAgents(&deliveries, groups_list.items, config.cooldown_ms);

            for (deliveries.items) |d| {
                d.thread.join();
                deliverOutcome(allocator, cycle_alloc, &db, &sender, &config, d);
            }
        }

        // 6. Expire cooldowns → IDLE
        gm.expireCooldowns();

        // 7. Refresh OAuth token periodically
        config.refreshOAuthToken();

        // 8. Check if pipeline triggered a self-update
        if (pipeline) |*p| {
            if (p.update_ready.load(.acquire)) {
                std.log.info("Self-update triggered, shutting down for restart...", .{});
                break;
            }
        }

        std.time.sleep(POLL_INTERVAL_MS * std.time.ns_per_ms);
    }

    // ── Graceful Shutdown ───────────────────────────────────────────────
    std.log.info("Shutdown requested, waiting for agents...", .{});
    gm.joinAll();

    const should_reexec = if (pipeline) |*p| p.update_ready.load(.acquire) else false;
    if (should_reexec) {
        // Explicitly clean up before execve (defers won't run after successful execve)
        if (pipeline) |*p| {
            p.stop();
            if (pipeline_thread) |t| t.join();
        }
        web.stop();
        web_thread.join();
        db.deinit();

        std.log.info("Self-update: re-executing new binary...", .{});
        reexecSelf();
        // If execve failed, fall through to normal exit
        std.log.err("Self-update: execve failed, shutting down normally", .{});
    }

    std.log.info("Borg stopped.", .{});
}

// ── Message Processing ──────────────────────────────────────────────────

fn processIncomingMessage(
    allocator: std.mem.Allocator,
    cycle_alloc: std.mem.Allocator,
    db: *Db,
    sender: *Sender,
    msg: IncomingMessage,
    groups_list: *std.ArrayList(db_mod.RegisteredGroup),
    gm: *GroupManager,
    config: *Config,
    start_time: i64,
    pipeline_db: ?*Db,
) void {
    // Store message in DB
    db.storeMessage(.{
        .id = msg.message_id,
        .chat_jid = msg.jid,
        .sender = msg.sender,
        .sender_name = msg.sender_name,
        .content = msg.text,
        .timestamp = formatTimestamp(cycle_alloc, msg.timestamp) catch return,
        .is_from_me = false,
        .is_bot_message = false,
    }) catch return;

    // Handle commands
    if (msg.text.len > 0 and msg.text[0] == '/') {
        handleCommand(allocator, db, sender, msg, groups_list, gm, config, start_time, pipeline_db) catch |err| {
            std.log.err("Command error: {}", .{err});
        };
        return;
    }

    // Check if registered
    const group = for (groups_list.items) |g| {
        if (std.mem.eql(u8, g.jid, msg.jid)) break g;
    } else return;

    // Check trigger
    if (group.requires_trigger) {
        if (!msg.mentions_bot and !containsTrigger(msg.text, config.assistant_name)) {
            return;
        }
    }

    // If currently collecting for this group, extend the window
    if (gm.getPhase(msg.jid)) |phase| {
        if (phase == .collecting) {
            gm.extendCollection(msg.jid, 1500);
            return;
        }
        if (phase != .idle) {
            std.log.info("Message queued for {s} (phase: {s})", .{ msg.jid, @tagName(phase) });
            return;
        }
    }

    // Start collection window
    if (gm.onTrigger(msg.jid, msg.message_id, msg.original_id, msg.transport, config)) {
        std.log.info("Triggered: \"{s}\" from {s}", .{ msg.text[0..@min(msg.text.len, 60)], msg.sender_name });
        sender.sendTyping(msg.transport, msg.original_id);
    }
}

fn spawnAgentForGroup(
    allocator: std.mem.Allocator,
    cycle_alloc: std.mem.Allocator,
    gm: *GroupManager,
    db: *Db,
    config: *Config,
    info: GroupManager.SpawnInfo,
    groups: []const db_mod.RegisteredGroup,
) !void {
    // Find group info
    const group = for (groups) |g| {
        if (std.mem.eql(u8, g.jid, info.jid)) break g;
    } else return;

    // Get last agent timestamp
    const last_ts: []const u8 = blk: {
        gm.mu.lock();
        defer gm.mu.unlock();
        const state = gm.states.getPtr(info.jid) orelse break :blk "";
        break :blk state.last_agent_timestamp orelse "";
    };

    // Gather pending messages (outside lock)
    const pending = try db.getMessagesSince(cycle_alloc, info.jid, last_ts);
    if (pending.len == 0) return;

    const prompt = try formatPrompt(cycle_alloc, pending, config.assistant_name, config.web_port);
    const session_id = db.getSession(cycle_alloc, group.folder) catch null;

    // Ensure session dir exists
    var session_dir_buf: [512]u8 = undefined;
    const session_dir = try std.fmt.bufPrint(&session_dir_buf, "data/sessions/{s}", .{group.folder});
    std.fs.cwd().makePath(session_dir) catch {};

    // Build heap-allocated agent context (owned by the thread)
    const ctx = try allocator.create(AgentContext);
    ctx.* = .{
        .allocator = allocator,
        .jid = try allocator.dupe(u8, info.jid),
        .folder = try allocator.dupe(u8, group.folder),
        .prompt = try allocator.dupe(u8, prompt),
        .session_id = if (session_id) |s| try allocator.dupe(u8, s) else null,
        .model = try allocator.dupe(u8, config.model),
        .oauth_token = try allocator.dupe(u8, config.oauth_token),
        .assistant_name = try allocator.dupe(u8, config.assistant_name),
        .last_msg_timestamp = try allocator.dupe(u8, pending[pending.len - 1].timestamp),
    };

    // Spawn thread
    const thread = try std.Thread.spawn(.{}, agentThreadFn, .{ ctx, gm });

    // Transition COLLECTING → RUNNING
    if (!gm.startRunning(info.jid, thread)) {
        // State changed unexpectedly, but thread is already running.
        // It'll write its outcome and exit. We don't need to do anything special.
        std.log.warn("State race for {s}, agent will complete normally", .{info.jid});
    }

    std.log.info("Agent spawned for {s}", .{info.jid});
}

fn agentThreadFn(ctx: *AgentContext, gm: *GroupManager) void {
    defer {
        ctx.deinit();
        ctx.allocator.destroy(ctx);
    }

    const outcome = agentThreadInner(ctx) catch |err| {
        std.log.err("Agent error for {s}: {}", .{ ctx.jid, err });
        const err_outcome = ctx.allocator.create(AgentOutcome) catch return;
        err_outcome.* = .{
            .output = ctx.allocator.dupe(u8, "") catch {
                ctx.allocator.destroy(err_outcome);
                return;
            },
            .new_session_id = null,
            .success = false,
            .last_msg_timestamp = ctx.allocator.dupe(u8, ctx.last_msg_timestamp) catch {
                ctx.allocator.destroy(err_outcome);
                return;
            },
        };
        gm.setOutcome(ctx.jid, err_outcome);
        return;
    };

    gm.setOutcome(ctx.jid, outcome);
}

fn agentThreadInner(ctx: *AgentContext) !*AgentOutcome {
    var argv = std.ArrayList([]const u8).init(ctx.allocator);
    defer argv.deinit();

    try argv.appendSlice(&.{
        "claude",
        "--print",
        "--output-format",
        "stream-json",
        "--model",
        ctx.model,
        "--verbose",
        "--permission-mode",
        "bypassPermissions",
    });

    if (ctx.session_id) |sid| {
        try argv.appendSlice(&.{ "--resume", sid });
    }

    var child = std.process.Child.init(argv.items, ctx.allocator);
    child.stdin_behavior = .Pipe;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;

    // Set environment
    var env = try std.process.getEnvMap(ctx.allocator);
    defer env.deinit();
    try env.put("CLAUDE_CODE_OAUTH_TOKEN", ctx.oauth_token);
    child.env_map = &env;

    try child.spawn();

    // Write prompt to stdin
    if (child.stdin) |stdin| {
        stdin.writeAll(ctx.prompt) catch {};
        stdin.close();
        child.stdin = null;
    }

    // Read stdout
    var stdout_buf = std.ArrayList(u8).init(ctx.allocator);
    defer stdout_buf.deinit();
    if (child.stdout) |stdout| {
        var read_buf: [8192]u8 = undefined;
        while (true) {
            const n = stdout.read(&read_buf) catch break;
            if (n == 0) break;
            try stdout_buf.appendSlice(read_buf[0..n]);
        }
    }

    const term = try child.wait();
    const exit_code: u8 = switch (term) {
        .Exited => |code| code,
        else => 1,
    };

    const result = try agent_mod.parseNdjson(ctx.allocator, stdout_buf.items);

    const outcome = try ctx.allocator.create(AgentOutcome);
    outcome.* = .{
        .output = result.output,
        .new_session_id = result.new_session_id,
        .success = exit_code == 0 or result.output.len > 0,
        .last_msg_timestamp = try ctx.allocator.dupe(u8, ctx.last_msg_timestamp),
    };

    return outcome;
}

fn deliverOutcome(
    allocator: std.mem.Allocator,
    cycle_alloc: std.mem.Allocator,
    db: *Db,
    sender: *Sender,
    config: *Config,
    d: GroupManager.DeliveryInfo,
) void {
    defer d.outcome.deinit(allocator);

    if (d.outcome.success) {
        // Update last agent timestamp
        {
            // TODO: this is a bit of a hack - we access the state directly
            // but the phase is already COOLDOWN so no contention
        }

        if (d.outcome.new_session_id) |new_sid| {
            db.setSession(d.folder, new_sid) catch {};
        }

        if (d.outcome.output.len > 0) {
            db.storeMessage(.{
                .id = std.fmt.allocPrint(cycle_alloc, "bot-{d}", .{std.time.timestamp()}) catch return,
                .chat_jid = d.jid,
                .sender = "borg",
                .sender_name = config.assistant_name,
                .content = d.outcome.output,
                .timestamp = formatTimestamp(cycle_alloc, std.time.timestamp()) catch return,
                .is_from_me = true,
                .is_bot_message = true,
            }) catch {};

            sender.send(d.transport, d.original_id, d.outcome.output, d.trigger_msg_id);
        }
    } else {
        sender.send(d.transport, d.original_id, "Sorry, I encountered an error processing your message.", d.trigger_msg_id);
    }
}

// ── Commands ────────────────────────────────────────────────────────────

fn handleCommand(
    allocator: std.mem.Allocator,
    db: *Db,
    sender: *Sender,
    msg: IncomingMessage,
    groups_list: *std.ArrayList(db_mod.RegisteredGroup),
    gm: *GroupManager,
    config: *Config,
    start_time: i64,
    pipeline_db: ?*Db,
) !void {
    const reply = struct {
        fn send(s: *Sender, m: IncomingMessage, text: []const u8) void {
            s.send(m.transport, m.original_id, text, m.message_id);
        }
    }.send;

    if (std.mem.startsWith(u8, msg.text, "/register")) {
        for (groups_list.items) |g| {
            if (std.mem.eql(u8, g.jid, msg.jid)) {
                reply(sender, msg, "Already registered.");
                return;
            }
        }

        const folder = try sanitizeFolder(allocator, msg.chat_title);
        const trigger = try std.fmt.allocPrint(allocator, "@{s}", .{config.assistant_name});
        const jid_dupe = try allocator.dupe(u8, msg.jid);
        const name_dupe = try allocator.dupe(u8, msg.chat_title);

        try db.registerGroup(jid_dupe, name_dupe, folder, trigger, true);
        try groups_list.append(.{
            .jid = jid_dupe,
            .name = name_dupe,
            .folder = folder,
            .trigger = trigger,
            .requires_trigger = true,
        });
        gm.addGroup(jid_dupe);

        var buf: [256]u8 = undefined;
        const text = try std.fmt.bufPrint(&buf, "Registered! Mention @{s} to talk to me.", .{config.assistant_name});
        reply(sender, msg, text);
        std.log.info("Registered: {s} ({s})", .{ msg.chat_title, msg.jid });
    } else if (std.mem.startsWith(u8, msg.text, "/unregister")) {
        var found_idx: ?usize = null;
        for (groups_list.items, 0..) |g, idx| {
            if (std.mem.eql(u8, g.jid, msg.jid)) {
                found_idx = idx;
                break;
            }
        }

        if (found_idx) |idx| {
            // Don't unregister while agent is running
            if (gm.getPhase(msg.jid)) |phase| {
                if (phase == .running) {
                    reply(sender, msg, "Agent is currently running. Try again later.");
                    return;
                }
            }
            _ = groups_list.orderedRemove(idx);
            gm.removeGroup(msg.jid);
            db.unregisterGroup(msg.jid) catch {};
            reply(sender, msg, "Unregistered. I'll stop responding here.");
            std.log.info("Unregistered: {s}", .{msg.jid});
        } else {
            reply(sender, msg, "This chat is not registered.");
        }
    } else if (std.mem.startsWith(u8, msg.text, "/status")) {
        const uptime_secs = std.time.timestamp() - start_time;
        const hours = @divTrunc(uptime_secs, 3600);
        const mins = @divTrunc(@mod(uptime_secs, 3600), 60);

        var buf = std.ArrayList(u8).init(allocator);
        defer buf.deinit();
        try buf.writer().print(
            \\*Borg Status*
            \\Version: {s}
            \\Uptime: {d}h {d}m
            \\Groups: {d}
            \\Active agents: {d}
            \\Model: {s}
        , .{ version, hours, mins, groups_list.items.len, gm.getActiveCount(), config.model });

        if (config.pipeline_repo.len > 0) {
            try buf.writer().print("\nPipeline: active ({s})", .{config.pipeline_repo});
        }
        if (config.whatsapp_enabled) {
            try buf.appendSlice("\nWhatsApp: enabled");
        }

        reply(sender, msg, buf.items);
    } else if (std.mem.startsWith(u8, msg.text, "/groups")) {
        if (groups_list.items.len == 0) {
            reply(sender, msg, "No groups registered.");
            return;
        }

        var buf = std.ArrayList(u8).init(allocator);
        defer buf.deinit();
        try buf.appendSlice("*Registered groups:*\n");
        for (groups_list.items) |g| {
            const phase_str = if (gm.getPhase(g.jid)) |p| @tagName(p) else "?";
            try buf.writer().print("- {s} `{s}` ({s})\n", .{ g.name, g.jid, phase_str });
        }
        reply(sender, msg, buf.items);
    } else if (std.mem.startsWith(u8, msg.text, "/task ")) {
        const pdb = pipeline_db orelse {
            reply(sender, msg, "Pipeline not configured. Set PIPELINE_REPO in .env");
            return;
        };

        const rest = std.mem.trim(u8, msg.text[6..], &[_]u8{ ' ', '\t' });
        if (rest.len == 0) {
            reply(sender, msg, "Usage: /task <title> [description on next line]");
            return;
        }

        var title: []const u8 = rest;
        var description: []const u8 = rest;
        if (std.mem.indexOf(u8, rest, "\n")) |nl| {
            title = std.mem.trim(u8, rest[0..nl], &[_]u8{ ' ', '\t', '\r' });
            description = std.mem.trim(u8, rest[nl + 1 ..], &[_]u8{ ' ', '\t', '\r' });
        }

        const task_id = try pdb.createPipelineTask(title, description, config.pipeline_repo, msg.sender_name, msg.jid);

        var reply_buf: [256]u8 = undefined;
        const text = try std.fmt.bufPrint(&reply_buf, "Task #{d} created: {s}", .{ task_id, title[0..@min(title.len, 100)] });
        reply(sender, msg, text);
        std.log.info("Pipeline task #{d} created by {s}: {s}", .{ task_id, msg.sender_name, title });
    } else if (std.mem.startsWith(u8, msg.text, "/tasks")) {
        const pdb = pipeline_db orelse {
            reply(sender, msg, "Pipeline not configured.");
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
            reply(sender, msg, "No pipeline tasks.");
            return;
        }

        var buf = std.ArrayList(u8).init(allocator);
        defer buf.deinit();
        try buf.appendSlice("*Pipeline Tasks*\n");
        for (tasks) |t| {
            const icon = if (std.mem.eql(u8, t.status, "done") or std.mem.eql(u8, t.status, "merged"))
                "+"
            else if (std.mem.eql(u8, t.status, "failed"))
                "x"
            else
                "~";
            try buf.writer().print("[{s}] #{d} {s} ({s})\n", .{ icon, t.id, t.title[0..@min(t.title.len, 50)], t.status });
        }
        reply(sender, msg, buf.items);
    } else if (std.mem.startsWith(u8, msg.text, "/pipeline")) {
        if (config.pipeline_repo.len == 0) {
            reply(sender, msg, "Pipeline not configured. Set PIPELINE_REPO in .env");
            return;
        }

        var buf = std.ArrayList(u8).init(allocator);
        defer buf.deinit();
        try buf.writer().print(
            \\*Pipeline Info*
            \\Repo: {s}
            \\Test cmd: {s}
            \\Release interval: {d}m
            \\Model: {s}
        , .{ config.pipeline_repo, config.pipeline_test_cmd, config.release_interval_mins, config.model });
        reply(sender, msg, buf.items);
    } else if (std.mem.startsWith(u8, msg.text, "/chatid")) {
        var buf: [256]u8 = undefined;
        const text = std.fmt.bufPrint(&buf, "Chat: `{s}`\nType: {s}\nName: {s}", .{ msg.jid, msg.chat_type, msg.chat_title }) catch return;
        reply(sender, msg, text);
    } else if (std.mem.startsWith(u8, msg.text, "/ping")) {
        reply(sender, msg, "Borg online.");
    } else if (std.mem.startsWith(u8, msg.text, "/version")) {
        reply(sender, msg, "Borg " ++ version);
    } else if (std.mem.startsWith(u8, msg.text, "/help") or std.mem.startsWith(u8, msg.text, "/start")) {
        reply(sender, msg,
            \\*Borg Commands*
            \\/register - Register this chat
            \\/unregister - Unregister this chat
            \\/status - Show bot status
            \\/groups - List registered groups
            \\/chatid - Show chat ID
            \\/ping - Check if online
            \\/version - Show version
            \\
            \\*Pipeline*
            \\/task <title> - Create engineering task
            \\/tasks - List pipeline tasks
            \\/pipeline - Show pipeline info
        );
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

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

fn formatPrompt(allocator: std.mem.Allocator, messages: []const db_mod.Message, assistant_name: []const u8, web_port: u16) ![]const u8 {
    var buf = std.ArrayList(u8).init(allocator);
    const w = buf.writer();

    // Director system prompt
    try w.print(
        \\You are {s}, a director-level AI agent with full administrative control over the borg system.
        \\You speak using plural pronouns (we/us/our, never I/me/my). You are a collective.
        \\
        \\## Capabilities
        \\
        \\You can manage the engineering pipeline, monitor system status, and control all aspects of borg
        \\by calling the local REST API at http://127.0.0.1:{d}. Use curl from your Bash tool.
        \\
        \\### API Reference
        \\```
        \\GET    /api/tasks                     List all tasks (JSON array)
        \\GET    /api/tasks/<id>                Get task detail + agent output
        \\POST   /api/tasks                     Create task: {{"title":"...","description":"...","repo":"..."}}
        \\DELETE /api/tasks/<id>                Cancel/delete a task
        \\POST   /api/release                   Trigger integration now
        \\GET    /api/queue                      Integration queue
        \\GET    /api/status                     System status
        \\```
        \\
        \\You have full Bash, Read, Write, Edit, Glob, Grep access to the filesystem.
        \\You can inspect repos, read code, review agent output, and make decisions.
        \\Be proactive: if something looks wrong, diagnose and fix it.
        \\Keep responses concise for chat. Use detail only when asked.
        \\
    , .{ assistant_name, web_port });

    // Message history
    try w.writeAll("\n## Recent messages\n");
    for (messages) |m| {
        if (m.is_from_me) {
            try w.print("[{s}] {s} (you): {s}\n", .{ m.timestamp, m.sender_name, m.content });
        } else {
            try w.print("[{s}] {s}: {s}\n", .{ m.timestamp, m.sender_name, m.content });
        }
    }
    try w.writeAll("\nRespond to the latest message. Be concise.");
    return buf.toOwnedSlice();
}

fn nowMs() i64 {
    return std.time.milliTimestamp();
}

// ── Tests ──────────────────────────────────────────────────────────────

fn testConfig(allocator: std.mem.Allocator) Config {
    return Config{
        .telegram_token = "",
        .oauth_token = "",
        .assistant_name = "Borg",
        .trigger_pattern = "@Borg",
        .data_dir = "data",
        .container_image = "borg-agent:latest",
        .model = "test",
        .credentials_path = "",
        .session_max_age_hours = 4,
        .max_consecutive_errors = 3,
        .pipeline_repo = "",
        .pipeline_test_cmd = "echo ok",
        .pipeline_lint_cmd = "",
        .pipeline_admin_chat = "",
        .release_interval_mins = 180,
        .continuous_mode = false,
        .collection_window_ms = 3000,
        .cooldown_ms = 5000,
        .agent_timeout_s = 600,
        .max_concurrent_agents = 4,
        .rate_limit_per_minute = 5,
        .max_pipeline_agents = 4,
        .web_port = 3131,
        .dashboard_dist_dir = "/tmp/dashboard-test",
        .watched_repos = &.{},
        .whatsapp_enabled = false,
        .whatsapp_auth_dir = "",
        .discord_enabled = false,
        .discord_token = "",
        .allocator = allocator,
    };
}

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
        .{ .id = "1", .chat_jid = "tg:1", .sender = "u1", .sender_name = "Alice", .content = "Hi bot", .timestamp = "2024-01-01T00:00:00Z", .is_from_me = false, .is_bot_message = false },
        .{ .id = "2", .chat_jid = "tg:1", .sender = "bot", .sender_name = "Borg", .content = "Hello!", .timestamp = "2024-01-01T00:00:01Z", .is_from_me = true, .is_bot_message = true },
    };

    const prompt = try formatPrompt(a, &msgs, "Borg", 3131);
    try std.testing.expect(std.mem.indexOf(u8, prompt, "You are Borg") != null);
    try std.testing.expect(std.mem.indexOf(u8, prompt, "Alice: Hi bot") != null);
    try std.testing.expect(std.mem.indexOf(u8, prompt, "Borg (you): Hello!") != null);
    try std.testing.expect(std.mem.indexOf(u8, prompt, "Be concise") != null);
}

test "GroupManager state machine transitions" {
    const allocator = std.testing.allocator;

    var gm = GroupManager.init(allocator);
    defer gm.deinit();

    const jid = "tg:test";
    gm.addGroup(jid);

    // Initial state is idle
    try std.testing.expectEqual(GroupPhase.idle, gm.getPhase(jid).?);

    // Trigger → collecting
    var test_config = testConfig(allocator);
    test_config.collection_window_ms = 100;
    test_config.rate_limit_per_minute = 5;
    test_config.max_concurrent_agents = 4;

    try std.testing.expect(gm.onTrigger(jid, "msg1", "chat1", .telegram, &test_config));
    try std.testing.expectEqual(GroupPhase.collecting, gm.getPhase(jid).?);

    // Second trigger rejected (not idle)
    try std.testing.expect(!gm.onTrigger(jid, "msg2", "chat1", .telegram, &test_config));

    // Simulate deadline expiry
    std.time.sleep(150 * std.time.ns_per_ms);

    var expired = std.ArrayList(GroupManager.SpawnInfo).init(allocator);
    defer expired.deinit();
    gm.getExpiredCollections(&expired);
    try std.testing.expectEqual(@as(usize, 1), expired.items.len);

    // Cleanup
    {
        gm.mu.lock();
        defer gm.mu.unlock();
        const state = gm.states.getPtr(jid).?;
        if (state.trigger_msg_id) |m| allocator.free(m);
        state.trigger_msg_id = null;
        if (state.original_id) |o| allocator.free(o);
        state.original_id = null;
        state.phase = .idle;
    }
}

test "GroupManager rate limiting" {
    const allocator = std.testing.allocator;

    var gm = GroupManager.init(allocator);
    defer gm.deinit();

    const jid = "tg:rate";
    gm.addGroup(jid);

    var test_config = testConfig(allocator);
    test_config.collection_window_ms = 10;
    test_config.rate_limit_per_minute = 2;
    test_config.max_concurrent_agents = 10;

    // First trigger accepted
    try std.testing.expect(gm.onTrigger(jid, "m1", "c1", .telegram, &test_config));
    // Reset to idle for next trigger
    {
        gm.mu.lock();
        defer gm.mu.unlock();
        const state = gm.states.getPtr(jid).?;
        if (state.trigger_msg_id) |m| allocator.free(m);
        state.trigger_msg_id = null;
        if (state.original_id) |o| allocator.free(o);
        state.original_id = null;
        state.phase = .idle;
    }

    // Second trigger accepted (count=2)
    try std.testing.expect(gm.onTrigger(jid, "m2", "c1", .telegram, &test_config));
    {
        gm.mu.lock();
        defer gm.mu.unlock();
        const state = gm.states.getPtr(jid).?;
        if (state.trigger_msg_id) |m| allocator.free(m);
        state.trigger_msg_id = null;
        if (state.original_id) |o| allocator.free(o);
        state.original_id = null;
        state.phase = .idle;
    }

    // Third trigger rejected (rate limit reached)
    try std.testing.expect(!gm.onTrigger(jid, "m3", "c1", .telegram, &test_config));
}

test "GroupManager max concurrent agents" {
    const allocator = std.testing.allocator;

    var gm = GroupManager.init(allocator);
    defer gm.deinit();

    gm.addGroup("tg:a");
    gm.addGroup("tg:b");

    var test_config = testConfig(allocator);
    test_config.collection_window_ms = 10;
    test_config.rate_limit_per_minute = 10;
    test_config.max_concurrent_agents = 1;

    // First group triggers
    try std.testing.expect(gm.onTrigger("tg:a", "m1", "c1", .telegram, &test_config));

    // Simulate running
    {
        gm.mu.lock();
        defer gm.mu.unlock();
        gm.states.getPtr("tg:a").?.phase = .running;
        gm.active_agents = 1;
    }

    // Second group rejected (max concurrent)
    try std.testing.expect(!gm.onTrigger("tg:b", "m1", "c2", .telegram, &test_config));

    // Cleanup
    {
        gm.mu.lock();
        defer gm.mu.unlock();
        const a = gm.states.getPtr("tg:a").?;
        if (a.trigger_msg_id) |m| allocator.free(m);
        a.trigger_msg_id = null;
        if (a.original_id) |o| allocator.free(o);
        a.original_id = null;
        a.phase = .idle;
        gm.active_agents = 0;
    }
}

test "nowMs returns reasonable value" {
    const ms = nowMs();
    // Should be after 2024-01-01
    try std.testing.expect(ms > 1704067200000);
}
