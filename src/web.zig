const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;
const json_mod = @import("json.zig");
const modes = @import("modes.zig");
const Config = @import("config.zig").Config;
const main = @import("main.zig");

const TaskStream = struct {
    clients: std.ArrayList(std.net.Stream),
    line_buf: std.ArrayList(u8),
    history: std.ArrayList(u8),
};

const LogEntry = struct {
    timestamp: i64,
    level: [8]u8,
    level_len: u8,
    message: [512]u8,
    message_len: u16,
    active: bool,
};

const LOG_RING_SIZE = 500;

pub const WebChatMessage = struct {
    sender_name: []const u8,
    text: []const u8,
    timestamp: i64,
    thread_id: []const u8,
};

pub const WebServer = struct {
    allocator: std.mem.Allocator,
    db: *Db,
    config: *Config,
    running: std.atomic.Value(bool),
    bind_addr: []const u8,
    port: u16,

    // Log ring buffer
    log_ring: [LOG_RING_SIZE]LogEntry,
    log_head: usize,
    log_count: usize,
    log_mu: std.Thread.Mutex,

    // SSE clients (logs)
    sse_clients: std.ArrayList(std.net.Stream),
    sse_mu: std.Thread.Mutex,

    // Chat message queue (web UI → main loop)
    chat_queue: std.ArrayList(WebChatMessage),
    chat_mu: std.Thread.Mutex,

    // Chat SSE clients (main loop → web UI)
    chat_sse_clients: std.ArrayList(std.net.Stream),
    chat_sse_mu: std.Thread.Mutex,

    // Per-task live stream SSE
    task_streams: std.AutoHashMap(i64, TaskStream),
    task_stream_mu: std.Thread.Mutex,

    start_time: i64,
    force_restart_signal: ?*std.atomic.Value(bool),

    pub fn init(allocator: std.mem.Allocator, db: *Db, config: *Config, port: u16, bind_addr: []const u8) WebServer {
        return .{
            .allocator = allocator,
            .db = db,
            .config = config,
            .running = std.atomic.Value(bool).init(true),
            .bind_addr = bind_addr,
            .port = port,
            .log_ring = [_]LogEntry{.{ .timestamp = 0, .level = undefined, .level_len = 0, .message = undefined, .message_len = 0, .active = false }} ** LOG_RING_SIZE,
            .log_head = 0,
            .log_count = 0,
            .log_mu = .{},
            .sse_clients = std.ArrayList(std.net.Stream).init(allocator),
            .sse_mu = .{},
            .chat_queue = std.ArrayList(WebChatMessage).init(allocator),
            .chat_mu = .{},
            .chat_sse_clients = std.ArrayList(std.net.Stream).init(allocator),
            .chat_sse_mu = .{},
            .task_streams = std.AutoHashMap(i64, TaskStream).init(allocator),
            .task_stream_mu = .{},
            .start_time = std.time.timestamp(),
            .force_restart_signal = null,
        };
    }

    pub fn pushLog(self: *WebServer, level: []const u8, message: []const u8) void {
        {
            self.log_mu.lock();
            defer self.log_mu.unlock();

            var entry = &self.log_ring[self.log_head];
            entry.timestamp = std.time.timestamp();
            entry.active = true;

            const llen = @min(level.len, entry.level.len);
            @memcpy(entry.level[0..llen], level[0..llen]);
            entry.level_len = @intCast(llen);

            const mlen = @min(message.len, entry.message.len);
            @memcpy(entry.message[0..mlen], message[0..mlen]);
            entry.message_len = @intCast(mlen);

            self.log_head = (self.log_head + 1) % LOG_RING_SIZE;
            if (self.log_count < LOG_RING_SIZE) self.log_count += 1;
        }

        // Persist warn/error to DB events table
        if (std.mem.eql(u8, level, "warn") or std.mem.eql(u8, level, "err")) {
            self.db.logEvent(level, "system", message, "");
        }

        self.broadcastSse(level, message);
    }

    /// Drain all pending chat messages (called by main loop)
    pub fn drainChatMessages(self: *WebServer) []WebChatMessage {
        self.chat_mu.lock();
        defer self.chat_mu.unlock();
        return self.chat_queue.toOwnedSlice() catch &[_]WebChatMessage{};
    }

    /// Broadcast a chat response to all connected chat SSE clients
    pub fn broadcastChatEvent(self: *WebServer, text: []const u8, thread_id: []const u8) void {
        self.chat_sse_mu.lock();
        defer self.chat_sse_mu.unlock();

        var esc_buf: [8192]u8 = undefined;
        const escaped = jsonEscape(&esc_buf, text[0..@min(text.len, 4000)]);

        var esc_thread: [128]u8 = undefined;
        const thread_esc = jsonEscape(&esc_thread, thread_id);

        var buf: [8192]u8 = undefined;
        const line = std.fmt.bufPrint(&buf, "data: {{\"role\":\"assistant\",\"text\":\"{s}\",\"ts\":{d},\"thread\":\"{s}\"}}\n\n", .{
            escaped,
            std.time.timestamp(),
            thread_esc,
        }) catch return;

        var i: usize = 0;
        while (i < self.chat_sse_clients.items.len) {
            self.chat_sse_clients.items[i].writeAll(line) catch {
                _ = self.chat_sse_clients.swapRemove(i);
                continue;
            };
            i += 1;
        }
    }

    /// Broadcast raw NDJSON chunk to SSE clients watching a task.
    /// Buffers partial lines and sends complete NDJSON lines as SSE events.
    /// Accumulates history so late-joining clients can catch up.
    pub fn broadcastTaskStream(self: *WebServer, task_id: i64, data: []const u8) void {
        self.task_stream_mu.lock();
        defer self.task_stream_mu.unlock();

        const entry = self.task_streams.getPtr(task_id) orelse return;

        entry.line_buf.appendSlice(data) catch return;

        var pos: usize = 0;
        while (std.mem.indexOfScalarPos(u8, entry.line_buf.items, pos, '\n')) |nl| {
            const line = entry.line_buf.items[pos..nl];
            if (line.len > 0) {
                // Accumulate as pre-formatted SSE (cap at 2MB)
                if (entry.history.items.len < 2 * 1024 * 1024) {
                    entry.history.appendSlice("data: ") catch {};
                    entry.history.appendSlice(line) catch {};
                    entry.history.appendSlice("\n\n") catch {};
                }

                // Send to live clients
                var i: usize = 0;
                while (i < entry.clients.items.len) {
                    const ok = blk: {
                        entry.clients.items[i].writeAll("data: ") catch break :blk false;
                        entry.clients.items[i].writeAll(line) catch break :blk false;
                        entry.clients.items[i].writeAll("\n\n") catch break :blk false;
                        break :blk true;
                    };
                    if (!ok) {
                        entry.clients.items[i].close();
                        _ = entry.clients.swapRemove(i);
                        continue;
                    }
                    i += 1;
                }
            }
            pos = nl + 1;
        }

        if (pos > 0) {
            const remaining = entry.line_buf.items.len - pos;
            if (remaining > 0) {
                std.mem.copyForwards(u8, entry.line_buf.items[0..remaining], entry.line_buf.items[pos..]);
            }
            entry.line_buf.shrinkRetainingCapacity(remaining);
        }
    }

    /// Register a task for live streaming (called when agent starts)
    pub fn startTaskStream(self: *WebServer, task_id: i64) void {
        self.task_stream_mu.lock();
        defer self.task_stream_mu.unlock();
        self.task_streams.put(task_id, .{
            .clients = std.ArrayList(std.net.Stream).init(self.allocator),
            .line_buf = std.ArrayList(u8).init(self.allocator),
            .history = std.ArrayList(u8).init(self.allocator),
        }) catch {};
    }

    /// Clean up task stream (called when agent finishes)
    pub fn endTaskStream(self: *WebServer, task_id: i64) void {
        self.task_stream_mu.lock();
        defer self.task_stream_mu.unlock();
        if (self.task_streams.fetchRemove(task_id)) |kv| {
            for (kv.value.clients.items) |client| {
                client.writeAll("data: {\"type\":\"stream_end\"}\n\n") catch {};
                client.close();
            }
            var v = kv.value;
            v.clients.deinit();
            v.line_buf.deinit();
            v.history.deinit();
        }
    }

    fn serveTaskStream(self: *WebServer, stream: std.net.Stream, path: []const u8) void {
        // Extract task_id from /api/tasks/<id>/stream
        const rest = path["/api/tasks/".len..];
        const slash_pos = std.mem.indexOf(u8, rest, "/") orelse return;
        const id_str = rest[0..slash_pos];
        const task_id = std.fmt.parseInt(i64, id_str, 10) catch return;

        stream.writeAll("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n") catch return;

        self.task_stream_mu.lock();
        defer self.task_stream_mu.unlock();

        var entry = self.task_streams.getPtr(task_id) orelse {
            // No active stream — send end event and close
            stream.writeAll("data: {\"type\":\"stream_end\"}\n\n") catch {};
            stream.close();
            return;
        };

        // Replay history so late-joining clients see past events
        if (entry.history.items.len > 0) {
            stream.writeAll(entry.history.items) catch {
                stream.close();
                return;
            };
        }

        entry.clients.append(stream) catch {
            stream.close();
        };
    }

    fn broadcastSse(self: *WebServer, level: []const u8, message: []const u8) void {
        self.sse_mu.lock();
        defer self.sse_mu.unlock();

        var esc_buf: [4096]u8 = undefined;
        const escaped = jsonEscape(&esc_buf, message[0..@min(message.len, 2000)]);

        var buf: [4096]u8 = undefined;
        const line = std.fmt.bufPrint(&buf, "data: {{\"level\":\"{s}\",\"message\":\"{s}\",\"ts\":{d}}}\n\n", .{
            level,
            escaped,
            std.time.timestamp(),
        }) catch return;

        var i: usize = 0;
        while (i < self.sse_clients.items.len) {
            self.sse_clients.items[i].writeAll(line) catch {
                _ = self.sse_clients.swapRemove(i);
                continue;
            };
            i += 1;
        }
    }

    pub fn run(self: *WebServer) void {
        const addr = std.net.Address.parseIp4(self.bind_addr, self.port) catch {
            std.log.err("Web: invalid bind address '{s}'", .{self.bind_addr});
            return;
        };

        var server = addr.listen(.{
            .reuse_address = true,
        }) catch |err| {
            std.log.err("Web: listen failed on {s}:{d}: {}", .{ self.bind_addr, self.port, err });
            return;
        };
        defer server.deinit();

        std.log.info("Web dashboard: http://{s}:{d}", .{ self.bind_addr, self.port });

        while (self.running.load(.acquire)) {
            const conn = server.accept() catch |err| {
                if (err == error.SocketNotListening) break;
                continue;
            };
            self.handleConnection(conn.stream) catch {
                conn.stream.close();
            };
        }
    }

    pub fn stop(self: *WebServer) void {
        self.running.store(false, .release);
        // Connect to ourselves to unblock accept()
        if (std.net.tcpConnectToAddress(std.net.Address.parseIp4("127.0.0.1", self.port) catch return)) |conn| {
            conn.close();
        } else |_| {}
    }

    fn handleConnection(self: *WebServer, stream: std.net.Stream) !void {
        var buf: [4096]u8 = undefined;
        const n = stream.read(&buf) catch {
            stream.close();
            return;
        };
        if (n == 0) {
            stream.close();
            return;
        }

        const initial = buf[0..n];
        const method = parseMethod(initial);
        const path = parsePath(initial);

        // For POST requests, ensure we read the complete body
        var request_alloc: ?[]u8 = null;
        defer if (request_alloc) |ra| self.allocator.free(ra);
        const request = blk: {
            if (!std.mem.eql(u8, method, "POST")) break :blk initial;
            const header_end = std.mem.indexOf(u8, initial, "\r\n\r\n") orelse break :blk initial;
            const headers = initial[0 .. header_end + 4];
            const content_length = parseContentLength(headers) orelse break :blk initial;
            const body_received = n - headers.len;
            if (body_received >= content_length) break :blk initial;

            // Need to read more — allocate buffer for full request
            const total = headers.len + content_length;
            var full = self.allocator.alloc(u8, total) catch break :blk initial;
            @memcpy(full[0..n], initial);
            var filled: usize = n;
            while (filled < total) {
                const r = stream.read(full[filled..total]) catch break;
                if (r == 0) break;
                filled += r;
            }
            request_alloc = full;
            break :blk full[0..filled];
        };

        if (std.mem.eql(u8, path, "/api/logs")) {
            self.serveSse(stream);
            return; // Don't close — SSE keeps connection open
        } else if (std.mem.eql(u8, path, "/api/chat/threads") and std.mem.eql(u8, method, "GET")) {
            self.serveChatThreads(stream);
        } else if (std.mem.eql(u8, path, "/api/chat/events")) {
            self.serveChatSse(stream);
            return; // Don't close — SSE keeps connection open
        } else if (std.mem.startsWith(u8, path, "/api/chat/messages")) {
            self.serveChatMessages(stream, path);
        } else if (std.mem.eql(u8, path, "/api/chat") and std.mem.eql(u8, method, "POST")) {
            self.handleChatPost(stream, request);
        } else if (std.mem.eql(u8, path, "/api/chat") and std.mem.eql(u8, method, "OPTIONS")) {
            self.serveCorsPreflightChat(stream);
        } else if (std.mem.eql(u8, path, "/api/tasks") and std.mem.eql(u8, method, "POST")) {
            self.handleCreateTask(stream, request);
        } else if (std.mem.startsWith(u8, path, "/api/tasks/") and std.mem.endsWith(u8, path, "/stream")) {
            self.serveTaskStream(stream, path);
            return; // SSE keeps connection open
        } else if (std.mem.startsWith(u8, path, "/api/tasks/") and std.mem.endsWith(u8, path, "/retry") and std.mem.eql(u8, method, "POST")) {
            self.handleRetryTask(stream, path);
        } else if (std.mem.startsWith(u8, path, "/api/tasks/") and std.mem.eql(u8, method, "DELETE")) {
            self.handleDeleteTask(stream, path);
        } else if (std.mem.eql(u8, path, "/api/release") and std.mem.eql(u8, method, "POST")) {
            self.handleTriggerRelease(stream);
        } else if (std.mem.eql(u8, path, "/api/tasks")) {
            self.serveTasksJson(stream);
        } else if (std.mem.startsWith(u8, path, "/api/tasks/")) {
            self.serveTaskDetailJson(stream, path);
        } else if (std.mem.eql(u8, path, "/api/queue")) {
            self.serveQueueJson(stream);
        } else if (std.mem.eql(u8, path, "/api/status")) {
            self.serveStatusJson(stream);
        } else if (std.mem.startsWith(u8, path, "/api/events")) {
            self.serveEventsJson(stream, path);
        } else if (std.mem.eql(u8, path, "/api/proposals/triage") and std.mem.eql(u8, method, "POST")) {
            self.handleTriageProposals(stream);
        } else if (std.mem.startsWith(u8, path, "/api/proposals/") and std.mem.eql(u8, method, "POST")) {
            self.handleProposalAction(stream, path);
        } else if (std.mem.startsWith(u8, path, "/api/proposals")) {
            self.serveProposalsJson(stream, path);
        } else if (std.mem.eql(u8, path, "/api/modes")) {
            self.serveModesJson(stream);
        } else if (std.mem.eql(u8, path, "/api/settings") and std.mem.eql(u8, method, "GET")) {
            self.serveSettingsJson(stream);
        } else if (std.mem.eql(u8, path, "/api/settings") and (std.mem.eql(u8, method, "PUT") or std.mem.eql(u8, method, "POST"))) {
            self.handleUpdateSettings(stream, request);
        } else {
            self.serveStatic(stream, path);
        }
        stream.close();
    }

    pub fn parseMethod(request: []const u8) []const u8 {
        if (std.mem.indexOf(u8, request, " ")) |end| {
            return request[0..end];
        }
        return "GET";
    }

    pub fn parsePath(request: []const u8) []const u8 {
        if (std.mem.indexOf(u8, request, " ")) |start| {
            const rest = request[start + 1 ..];
            if (std.mem.indexOf(u8, rest, " ")) |end| {
                return rest[0..end];
            }
        }
        return "/";
    }

    fn parseContentLength(headers: []const u8) ?usize {
        var lines = std.mem.splitSequence(u8, headers, "\r\n");
        while (lines.next()) |line| {
            if (line.len > 16 and (line[0] == 'C' or line[0] == 'c')) {
                const lower_prefix = "content-length: ";
                if (line.len >= lower_prefix.len) {
                    var match = true;
                    for (lower_prefix, 0..) |expected, i| {
                        const actual = if (line[i] >= 'A' and line[i] <= 'Z') line[i] + 32 else line[i];
                        if (actual != expected) {
                            match = false;
                            break;
                        }
                    }
                    if (match) {
                        return std.fmt.parseInt(usize, std.mem.trim(u8, line[lower_prefix.len..], " \t"), 10) catch null;
                    }
                }
            }
        }
        return null;
    }

    fn parseBody(request: []const u8) []const u8 {
        if (std.mem.indexOf(u8, request, "\r\n\r\n")) |pos| {
            return request[pos + 4 ..];
        }
        return "";
    }

    fn handleCreateTask(self: *WebServer, stream: std.net.Stream, request: []const u8) void {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        const body = parseBody(request);
        if (body.len == 0) {
            self.serveJsonResponse(stream, 400, "{\"error\":\"empty body\"}");
            return;
        }

        var parsed = json_mod.parse(alloc, body) catch {
            self.serveJsonResponse(stream, 400, "{\"error\":\"invalid JSON\"}");
            return;
        };
        defer parsed.deinit();

        const title = json_mod.getString(parsed.value, "title") orelse {
            self.serveJsonResponse(stream, 400, "{\"error\":\"missing title\"}");
            return;
        };
        const description = json_mod.getString(parsed.value, "description") orelse title;
        const repo = json_mod.getString(parsed.value, "repo") orelse self.config.pipeline_repo;
        const mode = json_mod.getString(parsed.value, "mode") orelse "swe";

        const task_id = self.db.createPipelineTask(title, description, repo, "director", "", mode) catch {
            self.serve500(stream);
            return;
        };

        var buf: [128]u8 = undefined;
        const resp = std.fmt.bufPrint(&buf, "{{\"id\":{d},\"status\":\"created\"}}", .{task_id}) catch return;
        self.serveJsonResponse(stream, 201, resp);
        std.log.info("Director created task #{d}: {s}", .{ task_id, title });
    }

    fn handleDeleteTask(self: *WebServer, stream: std.net.Stream, path: []const u8) void {
        const id_str = path["/api/tasks/".len..];
        const task_id = std.fmt.parseInt(i64, id_str, 10) catch {
            self.serveJsonResponse(stream, 400, "{\"error\":\"invalid task ID\"}");
            return;
        };

        self.db.updateTaskStatus(task_id, "failed") catch {
            self.serve500(stream);
            return;
        };

        self.serveJsonResponse(stream, 200, "{\"status\":\"deleted\"}");
        std.log.info("Director cancelled task #{d}", .{task_id});
    }

    fn handleRetryTask(self: *WebServer, stream: std.net.Stream, path: []const u8) void {
        // path is /api/tasks/:id/retry
        const suffix = "/retry";
        const inner = path["/api/tasks/".len .. path.len - suffix.len];
        const task_id = std.fmt.parseInt(i64, inner, 10) catch {
            self.serveJsonResponse(stream, 400, "{\"error\":\"invalid task ID\"}");
            return;
        };

        self.db.resetTaskAttempt(task_id) catch {
            self.serve500(stream);
            return;
        };
        self.db.updateTaskStatus(task_id, "backlog") catch {
            self.serve500(stream);
            return;
        };

        self.serveJsonResponse(stream, 200, "{\"status\":\"retrying\"}");
        std.log.info("Director retrying task #{d}", .{task_id});
    }

    fn handleTriggerRelease(self: *WebServer, stream: std.net.Stream) void {
        if (self.force_restart_signal) |sig| {
            sig.store(true, .release);
            std.log.info("Director triggered self-update restart", .{});
        }
        self.serveJsonResponse(stream, 200, "{\"status\":\"restart triggered\"}");
    }

    fn handleChatPost(self: *WebServer, stream: std.net.Stream, request: []const u8) void {
        const body = parseBody(request);
        if (body.len == 0) {
            self.serveJsonResponse(stream, 400, "{\"error\":\"empty body\"}");
            return;
        }

        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        var parsed = json_mod.parse(alloc, body) catch {
            self.serveJsonResponse(stream, 400, "{\"error\":\"invalid JSON\"}");
            return;
        };
        defer parsed.deinit();

        const text = json_mod.getString(parsed.value, "text") orelse {
            self.serveJsonResponse(stream, 400, "{\"error\":\"missing text\"}");
            return;
        };
        const sender_name = json_mod.getString(parsed.value, "sender") orelse "web-user";
        const thread = json_mod.getString(parsed.value, "thread") orelse "web:dashboard";

        const msg = WebChatMessage{
            .sender_name = self.allocator.dupe(u8, sender_name) catch return,
            .text = self.allocator.dupe(u8, text) catch return,
            .timestamp = std.time.timestamp(),
            .thread_id = self.allocator.dupe(u8, thread) catch return,
        };

        self.chat_mu.lock();
        self.chat_queue.append(msg) catch {
            self.chat_mu.unlock();
            self.allocator.free(msg.sender_name);
            self.allocator.free(msg.text);
            self.serve500(stream);
            return;
        };
        self.chat_mu.unlock();

        // Echo back to all chat SSE clients
        {
            self.chat_sse_mu.lock();
            defer self.chat_sse_mu.unlock();

            var esc_buf: [8192]u8 = undefined;
            const escaped = jsonEscape(&esc_buf, text[0..@min(text.len, 4000)]);
            var esc_name: [128]u8 = undefined;
            const name_esc = jsonEscape(&esc_name, sender_name);

            var esc_thread: [128]u8 = undefined;
            const thread_esc = jsonEscape(&esc_thread, thread);

            var buf: [8192]u8 = undefined;
            const line = std.fmt.bufPrint(&buf, "data: {{\"role\":\"user\",\"sender\":\"{s}\",\"text\":\"{s}\",\"ts\":{d},\"thread\":\"{s}\"}}\n\n", .{
                name_esc,
                escaped,
                std.time.timestamp(),
                thread_esc,
            }) catch return;

            var i: usize = 0;
            while (i < self.chat_sse_clients.items.len) {
                self.chat_sse_clients.items[i].writeAll(line) catch {
                    _ = self.chat_sse_clients.swapRemove(i);
                    continue;
                };
                i += 1;
            }
        }

        self.serveJsonResponse(stream, 200, "{\"status\":\"sent\"}");
    }

    fn serveChatThreads(self: *WebServer, stream: std.net.Stream) void {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        var rows = self.db.sqlite_db.query(
            alloc,
            "SELECT chat_jid, MAX(timestamp) as last_ts, COUNT(*) as msg_count FROM messages WHERE chat_jid LIKE 'web:%' GROUP BY chat_jid ORDER BY last_ts DESC",
            .{},
        ) catch {
            self.serve500(stream);
            return;
        };
        defer rows.deinit();

        var buf = std.ArrayList(u8).init(alloc);
        const w = buf.writer();
        w.writeAll("[") catch return;

        for (rows.items, 0..) |row, i| {
            if (i > 0) w.writeAll(",") catch return;
            var esc_jid: [128]u8 = undefined;
            const jid = jsonEscape(&esc_jid, row.get(0) orelse "");
            const last_ts = row.get(1) orelse "";
            const count = row.getInt(2) orelse 0;
            w.print("{{\"id\":\"{s}\",\"last_ts\":\"{s}\",\"message_count\":{d}}}", .{
                jid, last_ts, count,
            }) catch return;
        }

        w.writeAll("]") catch return;
        self.sendJson(stream, buf.items);
    }

    fn serveChatMessages(self: *WebServer, stream: std.net.Stream, path: []const u8) void {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        const thread_id = if (std.mem.indexOf(u8, path, "?thread=")) |pos|
            path[pos + "?thread=".len ..]
        else
            "web:dashboard";

        const messages = self.db.getMessagesSince(alloc, thread_id, "") catch {
            self.serve500(stream);
            return;
        };

        var buf = std.ArrayList(u8).init(alloc);
        const w = buf.writer();
        w.writeAll("[") catch return;

        for (messages, 0..) |m, i| {
            if (i > 0) w.writeAll(",") catch return;
            var esc_text: [8192]u8 = undefined;
            var esc_sender: [256]u8 = undefined;
            const text_e = jsonEscape(&esc_text, m.content);
            const sender_e = jsonEscape(&esc_sender, m.sender_name);
            w.print("{{\"role\":\"{s}\",\"sender\":\"{s}\",\"text\":\"{s}\",\"ts\":\"{s}\"}}", .{
                if (m.is_from_me) "assistant" else "user",
                sender_e,
                text_e,
                m.timestamp,
            }) catch return;
        }

        w.writeAll("]") catch return;
        self.sendJson(stream, buf.items);
    }

    fn serveChatSse(self: *WebServer, stream: std.net.Stream) void {
        const header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
        stream.writeAll(header) catch return;

        self.chat_sse_mu.lock();
        self.chat_sse_clients.append(stream) catch {};
        self.chat_sse_mu.unlock();
    }

    fn serveCorsPreflightChat(_: *WebServer, stream: std.net.Stream) void {
        stream.writeAll("HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, GET, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nConnection: close\r\n\r\n") catch return;
    }

    fn serveJsonResponse(_: *WebServer, stream: std.net.Stream, status: u16, body: []const u8) void {
        const status_text = switch (status) {
            200 => "200 OK",
            201 => "201 Created",
            400 => "400 Bad Request",
            404 => "404 Not Found",
            500 => "500 Internal Server Error",
            else => "200 OK",
        };
        var header_buf: [256]u8 = undefined;
        const header = std.fmt.bufPrint(&header_buf, "HTTP/1.1 {s}\r\nContent-Type: application/json\r\nContent-Length: {d}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n", .{ status_text, body.len }) catch return;
        stream.writeAll(header) catch return;
        stream.writeAll(body) catch return;
    }

    fn serveStatic(self: *WebServer, stream: std.net.Stream, path: []const u8) void {
        const dist_dir = self.config.dashboard_dist_dir;
        const cwd = std.fs.cwd();

        const file_rel = if (std.mem.eql(u8, path, "/")) "index.html" else if (path.len > 1) path[1..] else "index.html";

        if (std.mem.indexOf(u8, file_rel, "..") != null) {
            self.serve404(stream);
            return;
        }

        var file_path_buf: [1024]u8 = undefined;
        const full_path = std.fmt.bufPrint(&file_path_buf, "{s}/{s}", .{ dist_dir, file_rel }) catch {
            self.serve404(stream);
            return;
        };

        const file = cwd.openFile(full_path, .{}) catch {
            // SPA fallback: serve index.html for non-asset routes
            var idx_buf: [1024]u8 = undefined;
            const idx_path = std.fmt.bufPrint(&idx_buf, "{s}/index.html", .{dist_dir}) catch {
                self.serve404(stream);
                return;
            };
            const idx = cwd.openFile(idx_path, .{}) catch {
                self.serve404(stream);
                return;
            };
            defer idx.close();
            self.sendFile(stream, idx, "text/html");
            return;
        };
        defer file.close();

        const content_type = guessContentType(full_path);
        self.sendFile(stream, file, content_type);
    }

    fn sendFile(_: *WebServer, stream: std.net.Stream, file: std.fs.File, content_type: []const u8) void {
        const stat = file.stat() catch return;
        const cache = if (std.mem.eql(u8, content_type, "text/html")) "no-cache" else "public, max-age=31536000";
        var header_buf: [512]u8 = undefined;
        const header = std.fmt.bufPrint(&header_buf, "HTTP/1.1 200 OK\r\nContent-Type: {s}\r\nContent-Length: {d}\r\nCache-Control: {s}\r\nConnection: close\r\n\r\n", .{ content_type, stat.size, cache }) catch return;
        stream.writeAll(header) catch return;

        var buf: [16384]u8 = undefined;
        while (true) {
            const n = file.read(&buf) catch return;
            if (n == 0) break;
            stream.writeAll(buf[0..n]) catch return;
        }
    }

    fn serveSse(self: *WebServer, stream: std.net.Stream) void {
        const header = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
        stream.writeAll(header) catch return;

        // Send existing log entries
        {
            self.log_mu.lock();
            defer self.log_mu.unlock();

            if (self.log_count > 0) {
                const start = if (self.log_count >= LOG_RING_SIZE) self.log_head else 0;
                var i: usize = 0;
                while (i < self.log_count) : (i += 1) {
                    const idx = (start + i) % LOG_RING_SIZE;
                    const entry = self.log_ring[idx];
                    if (!entry.active) continue;
                    var buf: [4096]u8 = undefined;
                    const escaped = jsonEscape(&buf, entry.message[0..entry.message_len]);
                    var line_buf: [4096]u8 = undefined;
                    const line = std.fmt.bufPrint(&line_buf, "data: {{\"level\":\"{s}\",\"message\":\"{s}\",\"ts\":{d}}}\n\n", .{
                        entry.level[0..entry.level_len],
                        escaped,
                        entry.timestamp,
                    }) catch continue;
                    stream.writeAll(line) catch return;
                }
            }
        }

        // Register for future events
        self.sse_mu.lock();
        self.sse_clients.append(stream) catch {};
        self.sse_mu.unlock();
    }

    fn serveTasksJson(self: *WebServer, stream: std.net.Stream) void {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        const tasks = self.db.getAllPipelineTasks(alloc, 50) catch {
            self.serve500(stream);
            return;
        };

        var buf = std.ArrayList(u8).init(alloc);
        const w = buf.writer();
        w.writeAll("[") catch return;

        for (tasks, 0..) |t, i| {
            if (i > 0) w.writeAll(",") catch return;
            var esc_title: [512]u8 = undefined;
            var esc_desc: [512]u8 = undefined;
            const title = jsonEscape(&esc_title, t.title);
            const desc = jsonEscape(&esc_desc, t.description);
            var esc_repo: [512]u8 = undefined;
            const repo = jsonEscape(&esc_repo, t.repo_path);
            var esc_err: [512]u8 = undefined;
            const last_err = jsonEscape(&esc_err, t.last_error[0..@min(t.last_error.len, 200)]);
            w.print("{{\"id\":{d},\"title\":\"{s}\",\"description\":\"{s}\",\"status\":\"{s}\",\"branch\":\"{s}\",\"repo_path\":\"{s}\",\"attempt\":{d},\"max_attempts\":{d},\"created_by\":\"{s}\",\"created_at\":\"{s}\",\"last_error\":\"{s}\",\"mode\":\"{s}\"}}", .{
                t.id,
                title,
                desc,
                t.status,
                t.branch,
                repo,
                t.attempt,
                t.max_attempts,
                t.created_by,
                t.created_at,
                last_err,
                t.mode,
            }) catch return;
        }

        w.writeAll("]") catch return;
        self.sendJson(stream, buf.items);
    }

    fn serveTaskDetailJson(self: *WebServer, stream: std.net.Stream, path: []const u8) void {
        const id_str = path["/api/tasks/".len..];
        const task_id = std.fmt.parseInt(i64, id_str, 10) catch {
            self.serve404(stream);
            return;
        };

        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        const task = self.db.getPipelineTask(alloc, task_id) catch {
            self.serve500(stream);
            return;
        } orelse {
            self.serve404(stream);
            return;
        };

        const outputs = self.db.getTaskOutputs(alloc, task_id) catch {
            self.serve500(stream);
            return;
        };

        var buf = std.ArrayList(u8).init(alloc);
        const w = buf.writer();

        var esc_title: [512]u8 = undefined;
        var esc_desc: [2048]u8 = undefined;
        var esc_err: [4096]u8 = undefined;
        const title = jsonEscape(&esc_title, task.title);
        const desc = jsonEscape(&esc_desc, task.description);
        const last_err = jsonEscape(&esc_err, task.last_error);

        var esc_repo: [512]u8 = undefined;
        const repo = jsonEscape(&esc_repo, task.repo_path);
        w.print("{{\"id\":{d},\"title\":\"{s}\",\"description\":\"{s}\",\"status\":\"{s}\",\"branch\":\"{s}\",\"repo_path\":\"{s}\",\"attempt\":{d},\"max_attempts\":{d},\"last_error\":\"{s}\",\"created_by\":\"{s}\",\"created_at\":\"{s}\",\"mode\":\"{s}\",\"outputs\":[", .{
            task.id,
            title,
            desc,
            task.status,
            task.branch,
            repo,
            task.attempt,
            task.max_attempts,
            last_err,
            task.created_by,
            task.created_at,
            task.mode,
        }) catch return;

        for (outputs, 0..) |o, i| {
            if (i > 0) w.writeAll(",") catch return;
            const esc_out = jsonEscapeAlloc(alloc, o.output) catch continue;
            const esc_raw = jsonEscapeAlloc(alloc, o.raw_stream) catch "";
            w.print("{{\"id\":{d},\"phase\":\"{s}\",\"output\":\"{s}\",\"raw_stream\":\"{s}\",\"exit_code\":{d},\"created_at\":\"{s}\"}}", .{
                o.id,
                o.phase,
                esc_out,
                esc_raw,
                o.exit_code,
                o.created_at,
            }) catch return;
        }

        w.writeAll("]}") catch return;
        self.sendJson(stream, buf.items);
    }

    fn serveQueueJson(self: *WebServer, stream: std.net.Stream) void {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        const queued = self.db.getQueuedBranches(alloc) catch {
            self.serve500(stream);
            return;
        };

        var buf = std.ArrayList(u8).init(alloc);
        const w = buf.writer();
        w.writeAll("[") catch return;

        for (queued, 0..) |q, i| {
            if (i > 0) w.writeAll(",") catch return;
            var esc_repo: [512]u8 = undefined;
            const repo = jsonEscape(&esc_repo, q.repo_path);
            w.print("{{\"id\":{d},\"task_id\":{d},\"branch\":\"{s}\",\"repo_path\":\"{s}\",\"status\":\"{s}\",\"queued_at\":\"{s}\"}}", .{
                q.id,
                q.task_id,
                q.branch,
                repo,
                q.status,
                q.queued_at,
            }) catch return;
        }

        w.writeAll("]") catch return;
        self.sendJson(stream, buf.items);
    }

    fn serveModesJson(self: *WebServer, stream: std.net.Stream) void {
        _ = self;
        var buf: [4096]u8 = undefined;
        var fbs = std.io.fixedBufferStream(&buf);
        const w = fbs.writer();
        w.writeAll("[") catch return;
        for (modes.all_modes, 0..) |mode, mi| {
            if (mi > 0) w.writeAll(",") catch return;
            w.print("{{\"name\":\"{s}\",\"label\":\"{s}\",\"phases\":[", .{ mode.name, mode.label }) catch return;
            for (mode.phases, 0..) |phase, pi| {
                if (pi > 0) w.writeAll(",") catch return;
                w.print("{{\"name\":\"{s}\",\"label\":\"{s}\",\"priority\":{d}}}", .{ phase.name, phase.label, phase.priority }) catch return;
            }
            w.writeAll("]}") catch return;
        }
        w.writeAll("]") catch return;
        const headers = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
        stream.writeAll(headers) catch return;
        stream.writeAll(fbs.getWritten()) catch return;
    }

    fn serveProposalsJson(self: *WebServer, stream: std.net.Stream, path: []const u8) void {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        // Parse optional ?status= filter from path
        const status_filter: ?[]const u8 = if (std.mem.indexOf(u8, path, "?status=")) |pos|
            path[pos + "?status=".len ..]
        else
            null;

        const proposals = self.db.getProposals(alloc, status_filter, 100) catch {
            self.serve500(stream);
            return;
        };

        var buf = std.ArrayList(u8).init(alloc);
        const w = buf.writer();
        w.writeAll("[") catch return;

        for (proposals, 0..) |p, i| {
            if (i > 0) w.writeAll(",") catch return;
            var esc_title: [512]u8 = undefined;
            var esc_desc: [4096]u8 = undefined;
            var esc_rat: [4096]u8 = undefined;
            var esc_repo: [512]u8 = undefined;
            var esc_triage: [4096]u8 = undefined;
            const title = jsonEscape(&esc_title, p.title);
            const desc = jsonEscape(&esc_desc, p.description);
            const rat = jsonEscape(&esc_rat, p.rationale);
            const repo = jsonEscape(&esc_repo, p.repo_path);
            const triage_r = jsonEscape(&esc_triage, p.triage_reasoning);
            w.print("{{\"id\":{d},\"repo_path\":\"{s}\",\"title\":\"{s}\",\"description\":\"{s}\",\"rationale\":\"{s}\",\"status\":\"{s}\",\"created_at\":\"{s}\",\"triage_score\":{d},\"triage_impact\":{d},\"triage_feasibility\":{d},\"triage_risk\":{d},\"triage_effort\":{d},\"triage_reasoning\":\"{s}\"}}", .{
                p.id,
                repo,
                title,
                desc,
                rat,
                p.status,
                p.created_at,
                p.triage_score,
                p.triage_impact,
                p.triage_feasibility,
                p.triage_risk,
                p.triage_effort,
                triage_r,
            }) catch return;
        }

        w.writeAll("]") catch return;
        self.sendJson(stream, buf.items);
    }

    fn handleProposalAction(self: *WebServer, stream: std.net.Stream, path: []const u8) void {
        // /api/proposals/:id/approve or /api/proposals/:id/dismiss
        const rest = path["/api/proposals/".len..];
        const slash_pos = std.mem.indexOf(u8, rest, "/") orelse {
            self.serveJsonResponse(stream, 400, "{\"error\":\"missing action\"}");
            return;
        };
        const id_str = rest[0..slash_pos];
        const action = rest[slash_pos + 1 ..];
        const proposal_id = std.fmt.parseInt(i64, id_str, 10) catch {
            self.serveJsonResponse(stream, 400, "{\"error\":\"invalid proposal ID\"}");
            return;
        };

        if (std.mem.eql(u8, action, "approve")) {
            var arena = std.heap.ArenaAllocator.init(self.allocator);
            defer arena.deinit();
            const alloc = arena.allocator();

            const proposal = self.db.getProposal(alloc, proposal_id) catch {
                self.serve500(stream);
                return;
            } orelse {
                self.serveJsonResponse(stream, 404, "{\"error\":\"proposal not found\"}");
                return;
            };

            const task_id = self.db.createPipelineTask(
                proposal.title,
                proposal.description,
                proposal.repo_path,
                "proposal",
                "",
                "swe",
            ) catch {
                self.serve500(stream);
                return;
            };

            self.db.updateProposalStatus(proposal_id, "approved") catch {
                self.serve500(stream);
                return;
            };

            var buf: [128]u8 = undefined;
            const resp = std.fmt.bufPrint(&buf, "{{\"status\":\"approved\",\"task_id\":{d}}}", .{task_id}) catch return;
            self.serveJsonResponse(stream, 200, resp);
            std.log.info("Proposal #{d} approved → task #{d}: {s}", .{ proposal_id, task_id, proposal.title });
        } else if (std.mem.eql(u8, action, "dismiss")) {
            self.db.updateProposalStatus(proposal_id, "dismissed") catch {
                self.serve500(stream);
                return;
            };
            self.serveJsonResponse(stream, 200, "{\"status\":\"dismissed\"}");
            std.log.info("Proposal #{d} dismissed", .{proposal_id});
        } else if (std.mem.eql(u8, action, "reopen")) {
            self.db.updateProposalStatus(proposal_id, "proposed") catch {
                self.serve500(stream);
                return;
            };
            self.serveJsonResponse(stream, 200, "{\"status\":\"proposed\"}");
            std.log.info("Proposal #{d} reopened", .{proposal_id});
        } else {
            self.serveJsonResponse(stream, 400, "{\"error\":\"unknown action\"}");
        }
    }

    fn handleTriageProposals(self: *WebServer, stream: std.net.Stream) void {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        const proposals = self.db.getProposals(alloc, "proposed", 100) catch {
            self.serve500(stream);
            return;
        };
        if (proposals.len == 0) {
            self.serveJsonResponse(stream, 200, "{\"scored\":0}");
            return;
        }

        // Get recent merged tasks for duplicate/already-done detection
        const merged_tasks = self.db.getRecentMergedTasks(alloc, 50) catch &[0]db_mod.PipelineTask{};

        // Build prompt listing all proposals
        var prompt_buf = std.ArrayList(u8).init(alloc);
        const pw = prompt_buf.writer();
        pw.writeAll(
            \\Rate each proposal on 4 dimensions (1-5 scale), and flag proposals
            \\that should be auto-dismissed.
            \\
            \\Dimensions:
            \\- impact: How much value does this deliver? (5 = critical fix/feature, 1 = cosmetic)
            \\- feasibility: How likely is an AI agent to implement this correctly without human help? (5 = trivial, 1 = needs human)
            \\- risk: How likely to break existing functionality? (5 = very risky, 1 = safe)
            \\- effort: How many agent cycles will this need? (5 = massive multi-file, 1 = simple one-file)
            \\
            \\Overall score formula: (impact * 2 + feasibility * 2 - risk - effort) mapped to 1-10 scale.
            \\
            \\Set "dismiss": true if the proposal should be auto-closed for any of these reasons:
            \\- Already implemented (covered by a recently merged task)
            \\- Duplicate of another proposal in this list
            \\- Nonsensical, vague, or not actionable
            \\- Irrelevant to the project
            \\
            \\Reply with ONLY a JSON array, no markdown fences, no commentary:
            \\[{"id": <number>, "impact": <1-5>, "feasibility": <1-5>, "risk": <1-5>, "effort": <1-5>, "score": <1-10>, "reasoning": "<one sentence>", "dismiss": <true|false>}]
            \\
        ) catch return;

        if (merged_tasks.len > 0) {
            pw.writeAll("Recently merged tasks (for duplicate detection):\n") catch return;
            for (merged_tasks) |t| {
                pw.print("- {s}\n", .{t.title}) catch return;
            }
            pw.writeAll("\n") catch return;
        }

        pw.writeAll("Proposals to evaluate:\n\n") catch return;
        for (proposals) |p| {
            pw.print("- ID {d}: {s}\n  Description: {s}\n  Rationale: {s}\n\n", .{
                p.id,
                p.title,
                if (p.description.len > 0) p.description else "(none)",
                if (p.rationale.len > 0) p.rationale else "(none)",
            }) catch return;
        }

        // Spawn claude CLI for scoring
        self.config.refreshOAuthToken();
        var argv = std.ArrayList([]const u8).init(alloc);
        argv.appendSlice(&.{ "claude", "--print", "--model", "haiku", "--permission-mode", "bypassPermissions" }) catch return;
        var child = std.process.Child.init(argv.items, alloc);
        child.stdin_behavior = .Pipe;
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Ignore;

        var env = std.process.getEnvMap(alloc) catch {
            self.serve500(stream);
            return;
        };
        env.put("CLAUDE_CODE_OAUTH_TOKEN", self.config.oauth_token) catch return;
        child.env_map = &env;

        child.spawn() catch {
            self.serve500(stream);
            return;
        };

        if (child.stdin) |stdin| {
            stdin.writeAll(prompt_buf.items) catch {};
            stdin.close();
            child.stdin = null;
        }

        var stdout_buf = std.ArrayList(u8).init(alloc);
        if (child.stdout) |stdout| {
            var read_buf: [8192]u8 = undefined;
            while (true) {
                const n = stdout.read(&read_buf) catch break;
                if (n == 0) break;
                stdout_buf.appendSlice(read_buf[0..n]) catch break;
            }
        }
        _ = child.wait() catch {};

        // Parse JSON array from output (skip any leading non-[ text)
        const output = stdout_buf.items;
        const arr_start = std.mem.indexOf(u8, output, "[") orelse {
            std.log.warn("Triage: no JSON array in output ({d} bytes)", .{output.len});
            self.serveJsonResponse(stream, 500, "{\"error\":\"no scores in output\"}");
            return;
        };
        const arr_end_idx = std.mem.lastIndexOf(u8, output, "]") orelse {
            self.serveJsonResponse(stream, 500, "{\"error\":\"malformed JSON\"}");
            return;
        };
        const json_slice = output[arr_start .. arr_end_idx + 1];

        var parsed = json_mod.parse(alloc, json_slice) catch {
            std.log.warn("Triage: JSON parse failed", .{});
            self.serveJsonResponse(stream, 500, "{\"error\":\"JSON parse failed\"}");
            return;
        };
        defer parsed.deinit();

        const items = switch (parsed.value) {
            .array => |a| a.items,
            else => {
                self.serveJsonResponse(stream, 500, "{\"error\":\"expected JSON array\"}");
                return;
            },
        };

        var scored: u32 = 0;
        var dismissed: u32 = 0;
        for (items) |item| {
            const p_id = json_mod.getInt(item, "id") orelse continue;
            const impact = json_mod.getInt(item, "impact") orelse continue;
            const feasibility = json_mod.getInt(item, "feasibility") orelse continue;
            const risk = json_mod.getInt(item, "risk") orelse continue;
            const effort = json_mod.getInt(item, "effort") orelse continue;
            const score = json_mod.getInt(item, "score") orelse continue;
            const reasoning = json_mod.getString(item, "reasoning") orelse "";
            const should_dismiss = json_mod.getBool(item, "dismiss") orelse false;

            self.db.updateProposalTriage(p_id, score, impact, feasibility, risk, effort, reasoning) catch continue;
            scored += 1;

            if (should_dismiss) {
                self.db.updateProposalStatus(p_id, "auto_dismissed") catch continue;
                dismissed += 1;
                std.log.info("Triage: auto-dismissed proposal #{d}: {s}", .{ p_id, reasoning });
            }
        }

        std.log.info("Triage: scored {d}/{d} proposals, auto-dismissed {d}", .{ scored, proposals.len, dismissed });
        var resp_buf: [128]u8 = undefined;
        const resp = std.fmt.bufPrint(&resp_buf, "{{\"scored\":{d},\"dismissed\":{d}}}", .{ scored, dismissed }) catch return;
        self.serveJsonResponse(stream, 200, resp);
    }

    fn serveEventsJson(self: *WebServer, stream: std.net.Stream, path: []const u8) void {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        // Parse query params: ?category=X&level=Y&since=Z&limit=N
        var category: ?[]const u8 = null;
        var level: ?[]const u8 = null;
        var since_ts: i64 = 0;
        var limit_n: i64 = 200;

        if (std.mem.indexOf(u8, path, "?")) |q_pos| {
            const query = path[q_pos + 1 ..];
            var it = std.mem.splitScalar(u8, query, '&');
            while (it.next()) |param| {
                if (std.mem.indexOf(u8, param, "=")) |eq| {
                    const key = param[0..eq];
                    const val = param[eq + 1 ..];
                    if (std.mem.eql(u8, key, "category")) {
                        category = val;
                    } else if (std.mem.eql(u8, key, "level")) {
                        level = val;
                    } else if (std.mem.eql(u8, key, "since")) {
                        since_ts = std.fmt.parseInt(i64, val, 10) catch 0;
                    } else if (std.mem.eql(u8, key, "limit")) {
                        limit_n = std.fmt.parseInt(i64, val, 10) catch 200;
                    }
                }
            }
        }

        const events = self.db.getEvents(alloc, category, level, since_ts, limit_n) catch {
            self.sendJson(stream, "[]");
            return;
        };

        var buf = std.ArrayList(u8).init(alloc);
        const w = buf.writer();
        w.writeAll("[") catch return;
        for (events, 0..) |ev, i| {
            if (i > 0) w.writeAll(",") catch return;
            var esc_msg: [2048]u8 = undefined;
            var esc_meta: [2048]u8 = undefined;
            w.print("{{\"id\":{d},\"ts\":{d},\"level\":\"{s}\",\"category\":\"{s}\",\"message\":\"{s}\",\"metadata\":\"{s}\"}}", .{
                ev.id,
                ev.ts,
                ev.level,
                ev.category,
                jsonEscape(&esc_msg, ev.message[0..@min(ev.message.len, 1000)]),
                jsonEscape(&esc_meta, ev.metadata[0..@min(ev.metadata.len, 1000)]),
            }) catch return;
        }
        w.writeAll("]") catch return;
        self.sendJson(stream, buf.items);
    }

    fn serveStatusJson(self: *WebServer, stream: std.net.Stream) void {
        const now = std.time.timestamp();
        const uptime = now - self.start_time;

        const stats = self.db.getPipelineStats() catch db_mod.Db.PipelineStats{ .active = 0, .merged = 0, .failed = 0, .total = 0, .dispatched = 0 };

        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        var buf = std.ArrayList(u8).init(alloc);
        const w = buf.writer();

        // Build watched_repos JSON array
        w.print("{{\"version\":\"{s}\",\"uptime_s\":{d},\"model\":\"{s}\",\"watched_repos\":[", .{
            main.version,
            uptime,
            self.config.model,
        }) catch return;

        for (self.config.watched_repos, 0..) |repo, i| {
            if (i > 0) w.writeAll(",") catch return;
            var esc_path: [512]u8 = undefined;
            var esc_cmd: [512]u8 = undefined;
            w.print("{{\"path\":\"{s}\",\"test_cmd\":\"{s}\",\"is_self\":{s},\"auto_merge\":{s},\"mode\":\"{s}\"}}", .{
                jsonEscape(&esc_path, repo.path),
                jsonEscape(&esc_cmd, repo.test_cmd),
                if (repo.is_self) "true" else "false",
                if (repo.auto_merge) "true" else "false",
                repo.mode,
            }) catch return;
        }

        w.print("],\"release_interval_mins\":{d},\"continuous_mode\":{s},\"assistant_name\":\"{s}\",\"active_tasks\":{d},\"merged_tasks\":{d},\"failed_tasks\":{d},\"total_tasks\":{d},\"dispatched_agents\":{d}}}", .{
            self.config.release_interval_mins,
            if (self.config.continuous_mode) "true" else "false",
            self.config.assistant_name,
            stats.active,
            stats.merged,
            stats.failed,
            stats.total,
            stats.dispatched,
        }) catch return;

        self.sendJson(stream, buf.items);
    }

    const mutable_settings = [_]struct { key: []const u8, kind: enum { bool_val, int_val, str_val } }{
        .{ .key = "continuous_mode", .kind = .bool_val },
        .{ .key = "release_interval_mins", .kind = .int_val },
        .{ .key = "pipeline_max_backlog", .kind = .int_val },
        .{ .key = "agent_timeout_s", .kind = .int_val },
        .{ .key = "pipeline_seed_cooldown_s", .kind = .int_val },
        .{ .key = "pipeline_tick_s", .kind = .int_val },
        .{ .key = "model", .kind = .str_val },
        .{ .key = "container_memory_mb", .kind = .int_val },
        .{ .key = "assistant_name", .kind = .str_val },
        .{ .key = "pipeline_max_agents", .kind = .int_val },
    };

    fn serveSettingsJson(self: *WebServer, stream: std.net.Stream) void {
        var buf: [2048]u8 = undefined;
        var fbs = std.io.fixedBufferStream(&buf);
        const w = fbs.writer();
        var esc_model: [128]u8 = undefined;
        var esc_name: [128]u8 = undefined;
        w.print("{{\"continuous_mode\":{s},\"release_interval_mins\":{d},\"pipeline_max_backlog\":{d},\"agent_timeout_s\":{d},\"pipeline_seed_cooldown_s\":{d},\"pipeline_tick_s\":{d},\"model\":\"{s}\",\"container_memory_mb\":{d},\"assistant_name\":\"{s}\",\"pipeline_max_agents\":{d}}}", .{
            if (self.config.continuous_mode) "true" else "false",
            self.config.release_interval_mins,
            self.config.pipeline_max_backlog,
            self.config.agent_timeout_s,
            self.config.pipeline_seed_cooldown_s,
            self.config.pipeline_tick_s,
            jsonEscape(&esc_model, self.config.model),
            self.config.container_memory_mb,
            jsonEscape(&esc_name, self.config.assistant_name),
            self.config.pipeline_max_agents,
        }) catch return;
        self.sendJson(stream, fbs.getWritten());
    }

    fn handleUpdateSettings(self: *WebServer, stream: std.net.Stream, request: []const u8) void {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        const body = parseBody(request);
        if (body.len == 0) {
            self.serveJsonResponse(stream, 400, "{\"error\":\"empty body\"}");
            return;
        }

        var parsed = json_mod.parse(alloc, body) catch {
            self.serveJsonResponse(stream, 400, "{\"error\":\"invalid JSON\"}");
            return;
        };
        defer parsed.deinit();

        var changed: u32 = 0;
        for (mutable_settings) |s| {
            switch (s.kind) {
                .bool_val => {
                    if (json_mod.getBool(parsed.value, s.key)) |val| {
                        const val_str = if (val) "true" else "false";
                        self.db.setState(s.key, val_str) catch continue;
                        self.applySettingToConfig(s.key, val_str);
                        changed += 1;
                    }
                },
                .int_val => {
                    if (json_mod.getInt(parsed.value, s.key)) |val| {
                        var num_buf: [32]u8 = undefined;
                        const val_str = std.fmt.bufPrint(&num_buf, "{d}", .{val}) catch continue;
                        self.db.setState(s.key, val_str) catch continue;
                        self.applySettingToConfig(s.key, val_str);
                        changed += 1;
                    }
                },
                .str_val => {
                    if (json_mod.getString(parsed.value, s.key)) |val| {
                        self.db.setState(s.key, val) catch continue;
                        self.applySettingToConfig(s.key, val);
                        changed += 1;
                    }
                },
            }
        }

        var resp_buf: [64]u8 = undefined;
        const resp = std.fmt.bufPrint(&resp_buf, "{{\"updated\":{d}}}", .{changed}) catch return;
        self.serveJsonResponse(stream, 200, resp);
    }

    fn applySettingToConfig(self: *WebServer, key: []const u8, val: []const u8) void {
        if (std.mem.eql(u8, key, "continuous_mode")) {
            self.config.continuous_mode = std.mem.eql(u8, val, "true");
        } else if (std.mem.eql(u8, key, "release_interval_mins")) {
            self.config.release_interval_mins = std.fmt.parseInt(u32, val, 10) catch return;
        } else if (std.mem.eql(u8, key, "pipeline_max_backlog")) {
            self.config.pipeline_max_backlog = std.fmt.parseInt(u32, val, 10) catch return;
        } else if (std.mem.eql(u8, key, "agent_timeout_s")) {
            self.config.agent_timeout_s = std.fmt.parseInt(i64, val, 10) catch return;
        } else if (std.mem.eql(u8, key, "pipeline_seed_cooldown_s")) {
            self.config.pipeline_seed_cooldown_s = std.fmt.parseInt(i64, val, 10) catch return;
        } else if (std.mem.eql(u8, key, "pipeline_tick_s")) {
            self.config.pipeline_tick_s = std.fmt.parseInt(u64, val, 10) catch return;
        } else if (std.mem.eql(u8, key, "container_memory_mb")) {
            self.config.container_memory_mb = std.fmt.parseInt(u64, val, 10) catch return;
        } else if (std.mem.eql(u8, key, "pipeline_max_agents")) {
            self.config.pipeline_max_agents = std.fmt.parseInt(u32, val, 10) catch return;
        } else if (std.mem.eql(u8, key, "model")) {
            self.config.model = self.config.allocator.dupe(u8, val) catch return;
        } else if (std.mem.eql(u8, key, "assistant_name")) {
            self.config.assistant_name = self.config.allocator.dupe(u8, val) catch return;
        }
    }

    fn sendJson(_: *WebServer, stream: std.net.Stream, body: []const u8) void {
        var header_buf: [256]u8 = undefined;
        const header = std.fmt.bufPrint(&header_buf, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {d}\r\nConnection: close\r\n\r\n", .{body.len}) catch return;
        stream.writeAll(header) catch return;
        stream.writeAll(body) catch return;
    }

    fn serve404(_: *WebServer, stream: std.net.Stream) void {
        stream.writeAll("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n") catch return;
    }

    fn serve500(_: *WebServer, stream: std.net.Stream) void {
        stream.writeAll("HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n") catch return;
    }
};

fn guessContentType(path: []const u8) []const u8 {
    if (std.mem.endsWith(u8, path, ".html")) return "text/html";
    if (std.mem.endsWith(u8, path, ".css")) return "text/css";
    if (std.mem.endsWith(u8, path, ".js")) return "application/javascript";
    if (std.mem.endsWith(u8, path, ".json")) return "application/json";
    if (std.mem.endsWith(u8, path, ".svg")) return "image/svg+xml";
    if (std.mem.endsWith(u8, path, ".png")) return "image/png";
    if (std.mem.endsWith(u8, path, ".ico")) return "image/x-icon";
    return "application/octet-stream";
}

fn jsonEscapeAlloc(allocator: std.mem.Allocator, input: []const u8) ![]const u8 {
    const max_len = 16000;
    const src = if (input.len > max_len) input[0..max_len] else input;
    var out = std.ArrayList(u8).init(allocator);
    for (src) |c| {
        switch (c) {
            '"' => try out.appendSlice("\\\""),
            '\\' => try out.appendSlice("\\\\"),
            '\n' => try out.appendSlice("\\n"),
            '\r' => try out.appendSlice("\\r"),
            '\t' => try out.appendSlice("\\t"),
            else => {
                if (c >= 0x20) {
                    try out.append(c);
                }
            },
        }
    }
    if (input.len > max_len) {
        try out.appendSlice("\\n... (truncated)");
    }
    return out.toOwnedSlice();
}

fn jsonEscape(buf: []u8, input: []const u8) []const u8 {
    var pos: usize = 0;
    for (input) |c| {
        if (pos + 2 >= buf.len) break;
        switch (c) {
            '"' => {
                buf[pos] = '\\';
                buf[pos + 1] = '"';
                pos += 2;
            },
            '\\' => {
                buf[pos] = '\\';
                buf[pos + 1] = '\\';
                pos += 2;
            },
            '\n' => {
                buf[pos] = '\\';
                buf[pos + 1] = 'n';
                pos += 2;
            },
            '\r' => {
                buf[pos] = '\\';
                buf[pos + 1] = 'r';
                pos += 2;
            },
            '\t' => {
                buf[pos] = '\\';
                buf[pos + 1] = 't';
                pos += 2;
            },
            else => {
                if (c >= 0x20) {
                    buf[pos] = c;
                    pos += 1;
                }
            },
        }
    }
    return buf[0..pos];
}

// Dashboard served from dashboard/dist/ — build: cd dashboard && bun run build

// ── Tests ──────────────────────────────────────────────────────────────

test "jsonEscape handles special characters" {
    var buf: [256]u8 = undefined;

    const r1 = jsonEscape(&buf, "hello world");
    try std.testing.expectEqualStrings("hello world", r1);

    const r2 = jsonEscape(&buf, "say \"hello\"");
    try std.testing.expectEqualStrings("say \\\"hello\\\"", r2);

    const r3 = jsonEscape(&buf, "line1\nline2");
    try std.testing.expectEqualStrings("line1\\nline2", r3);

    const r4 = jsonEscape(&buf, "path\\to\\file");
    try std.testing.expectEqualStrings("path\\\\to\\\\file", r4);
}

test "parsePath extracts HTTP path" {
    try std.testing.expectEqualStrings("/api/tasks", WebServer.parsePath("GET /api/tasks HTTP/1.1\r\n"));
    try std.testing.expectEqualStrings("/", WebServer.parsePath("GET / HTTP/1.1\r\n"));
}

test {
    _ = @import("web_sse_leak_test.zig");
}
