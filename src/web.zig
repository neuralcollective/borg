const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;
const json_mod = @import("json.zig");
const Config = @import("config.zig").Config;

const LogEntry = struct {
    timestamp: i64,
    level: [8]u8,
    level_len: u8,
    message: [512]u8,
    message_len: u16,
    active: bool,
};

const LOG_RING_SIZE = 500;

pub const WebServer = struct {
    allocator: std.mem.Allocator,
    db: *Db,
    config: *Config,
    running: std.atomic.Value(bool),
    port: u16,

    // Log ring buffer
    log_ring: [LOG_RING_SIZE]LogEntry,
    log_head: usize,
    log_count: usize,
    log_mu: std.Thread.Mutex,

    // SSE clients
    sse_clients: std.ArrayList(std.net.Stream),
    sse_mu: std.Thread.Mutex,

    start_time: i64,

    pub fn init(allocator: std.mem.Allocator, db: *Db, config: *Config, port: u16) WebServer {
        return .{
            .allocator = allocator,
            .db = db,
            .config = config,
            .running = std.atomic.Value(bool).init(true),
            .port = port,
            .log_ring = [_]LogEntry{.{ .timestamp = 0, .level = undefined, .level_len = 0, .message = undefined, .message_len = 0, .active = false }} ** LOG_RING_SIZE,
            .log_head = 0,
            .log_count = 0,
            .log_mu = .{},
            .sse_clients = std.ArrayList(std.net.Stream).init(allocator),
            .sse_mu = .{},
            .start_time = std.time.timestamp(),
        };
    }

    pub fn pushLog(self: *WebServer, level: []const u8, message: []const u8) void {
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

        self.broadcastSse(level, message);
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
        const addr = std.net.Address.parseIp4("127.0.0.1", self.port) catch {
            std.log.err("Web: invalid address", .{});
            return;
        };

        var server = addr.listen(.{
            .reuse_address = true,
        }) catch |err| {
            std.log.err("Web: listen failed on port {d}: {}", .{ self.port, err });
            return;
        };
        defer server.deinit();

        std.log.info("Web dashboard: http://127.0.0.1:{d}", .{self.port});

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

        const request = buf[0..n];
        const path = parsePath(request);

        if (std.mem.eql(u8, path, "/")) {
            self.serveDashboard(stream);
        } else if (std.mem.eql(u8, path, "/api/logs")) {
            self.serveSse(stream);
            return; // Don't close — SSE keeps connection open
        } else if (std.mem.eql(u8, path, "/api/tasks")) {
            self.serveTasksJson(stream);
        } else if (std.mem.eql(u8, path, "/api/queue")) {
            self.serveQueueJson(stream);
        } else if (std.mem.eql(u8, path, "/api/status")) {
            self.serveStatusJson(stream);
        } else {
            self.serve404(stream);
        }
        stream.close();
    }

    pub fn parsePath(request: []const u8) []const u8 {
        // "GET /path HTTP/1.1\r\n..."
        if (std.mem.indexOf(u8, request, " ")) |start| {
            const rest = request[start + 1 ..];
            if (std.mem.indexOf(u8, rest, " ")) |end| {
                return rest[0..end];
            }
        }
        return "/";
    }

    fn serveDashboard(self: *WebServer, stream: std.net.Stream) void {
        _ = self;
        const html = DASHBOARD_HTML;
        var header_buf: [256]u8 = undefined;
        const header = std.fmt.bufPrint(&header_buf, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {d}\r\nConnection: close\r\n\r\n", .{html.len}) catch return;
        stream.writeAll(header) catch return;
        stream.writeAll(html) catch return;
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
            w.print("{{\"id\":{d},\"title\":\"{s}\",\"description\":\"{s}\",\"status\":\"{s}\",\"branch\":\"{s}\",\"attempt\":{d},\"max_attempts\":{d},\"created_by\":\"{s}\",\"created_at\":\"{s}\"}}", .{
                t.id,
                title,
                desc,
                t.status,
                t.branch,
                t.attempt,
                t.max_attempts,
                t.created_by,
                t.created_at,
            }) catch return;
        }

        w.writeAll("]") catch return;
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
            w.print("{{\"id\":{d},\"task_id\":{d},\"branch\":\"{s}\",\"status\":\"{s}\",\"queued_at\":\"{s}\"}}", .{
                q.id,
                q.task_id,
                q.branch,
                q.status,
                q.queued_at,
            }) catch return;
        }

        w.writeAll("]") catch return;
        self.sendJson(stream, buf.items);
    }

    fn serveStatusJson(self: *WebServer, stream: std.net.Stream) void {
        const now = std.time.timestamp();
        const uptime = now - self.start_time;

        var buf: [1024]u8 = undefined;
        const body = std.fmt.bufPrint(&buf, "{{\"uptime_s\":{d},\"model\":\"{s}\",\"pipeline_repo\":\"{s}\",\"release_interval_mins\":{d},\"test_cmd\":\"{s}\"}}", .{
            uptime,
            self.config.model,
            self.config.pipeline_repo,
            self.config.release_interval_mins,
            self.config.pipeline_test_cmd,
        }) catch return;

        self.sendJson(stream, body);
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

const DASHBOARD_HTML =
    \\<!DOCTYPE html>
    \\<html><head><meta charset="utf-8"><title>Borg Dashboard</title>
    \\<style>
    \\* { margin: 0; padding: 0; box-sizing: border-box; }
    \\body { background: #0a0a0f; color: #c8c8d0; font-family: 'JetBrains Mono', 'Fira Code', monospace; font-size: 13px; }
    \\header { background: #12121a; border-bottom: 1px solid #2a2a3a; padding: 12px 20px; display: flex; justify-content: space-between; align-items: center; }
    \\header h1 { font-size: 16px; color: #7aa2f7; font-weight: 600; }
    \\header .status { color: #9ece6a; font-size: 12px; }
    \\.container { display: grid; grid-template-columns: 1fr 1fr; grid-template-rows: 1fr 1fr; height: calc(100vh - 48px); gap: 1px; background: #1a1a2a; }
    \\.panel { background: #0a0a0f; display: flex; flex-direction: column; overflow: hidden; }
    \\.panel-header { background: #12121a; padding: 8px 14px; font-size: 11px; text-transform: uppercase; letter-spacing: 1px; color: #565680; border-bottom: 1px solid #1a1a2a; flex-shrink: 0; display: flex; justify-content: space-between; }
    \\.panel-body { flex: 1; overflow-y: auto; padding: 8px 14px; }
    \\.log-line { padding: 2px 0; white-space: pre-wrap; word-break: break-all; font-size: 12px; }
    \\.log-info { color: #7aa2f7; }
    \\.log-warn { color: #e0af68; }
    \\.log-err { color: #f7768e; }
    \\.task-row { padding: 6px 0; border-bottom: 1px solid #1a1a2a; display: flex; gap: 10px; align-items: center; }
    \\.task-id { color: #565680; width: 30px; }
    \\.task-title { flex: 1; color: #c8c8d0; }
    \\.badge { padding: 2px 8px; border-radius: 3px; font-size: 10px; text-transform: uppercase; }
    \\.badge-backlog { background: #1a1a3a; color: #565680; }
    \\.badge-spec, .badge-qa { background: #1a2a3a; color: #7aa2f7; }
    \\.badge-impl, .badge-retry { background: #2a2a1a; color: #e0af68; }
    \\.badge-done { background: #1a2a1a; color: #9ece6a; }
    \\.badge-merged { background: #0a2a0a; color: #73daca; }
    \\.badge-rebase { background: #2a1a2a; color: #bb9af7; }
    \\.badge-failed { background: #2a1a1a; color: #f7768e; }
    \\.queue-row { padding: 6px 0; border-bottom: 1px solid #1a1a2a; }
    \\.stat { padding: 8px 0; display: flex; justify-content: space-between; border-bottom: 1px solid #1a1a2a; }
    \\.stat-label { color: #565680; }
    \\.stat-value { color: #c8c8d0; }
    \\.countdown { color: #e0af68; }
    \\::-webkit-scrollbar { width: 6px; }
    \\::-webkit-scrollbar-track { background: #0a0a0f; }
    \\::-webkit-scrollbar-thumb { background: #2a2a3a; border-radius: 3px; }
    \\</style></head><body>
    \\<header>
    \\  <h1>BORG</h1>
    \\  <span class="status" id="conn-status">connecting...</span>
    \\</header>
    \\<div class="container">
    \\  <div class="panel">
    \\    <div class="panel-header"><span>Live Logs</span><span id="log-count">0</span></div>
    \\    <div class="panel-body" id="logs"></div>
    \\  </div>
    \\  <div class="panel">
    \\    <div class="panel-header"><span>Pipeline Tasks</span><span id="task-count">0</span></div>
    \\    <div class="panel-body" id="tasks"></div>
    \\  </div>
    \\  <div class="panel">
    \\    <div class="panel-header"><span>Integration Queue</span><span id="queue-count">0</span></div>
    \\    <div class="panel-body" id="queue"></div>
    \\  </div>
    \\  <div class="panel">
    \\    <div class="panel-header"><span>System Status</span></div>
    \\    <div class="panel-body" id="status"></div>
    \\  </div>
    \\</div>
    \\<script>
    \\const $ = id => document.getElementById(id);
    \\let logCount = 0;
    \\
    \\// SSE for live logs
    \\const es = new EventSource('/api/logs');
    \\es.onopen = () => { $('conn-status').textContent = 'connected'; $('conn-status').style.color = '#9ece6a'; };
    \\es.onerror = () => { $('conn-status').textContent = 'disconnected'; $('conn-status').style.color = '#f7768e'; };
    \\es.onmessage = e => {
    \\  try {
    \\    const d = JSON.parse(e.data);
    \\    const el = document.createElement('div');
    \\    const cls = d.level === 'err' ? 'log-err' : d.level === 'warn' ? 'log-warn' : 'log-info';
    \\    el.className = 'log-line ' + cls;
    \\    const ts = new Date(d.ts * 1000).toLocaleTimeString();
    \\    el.textContent = ts + ' [' + d.level + '] ' + d.message;
    \\    const logs = $('logs');
    \\    logs.appendChild(el);
    \\    logCount++;
    \\    $('log-count').textContent = logCount;
    \\    if (logs.children.length > 500) logs.removeChild(logs.firstChild);
    \\    logs.scrollTop = logs.scrollHeight;
    \\  } catch(_) {}
    \\};
    \\
    \\function badge(status) {
    \\  return '<span class="badge badge-' + status + '">' + status + '</span>';
    \\}
    \\
    \\async function refreshTasks() {
    \\  try {
    \\    const r = await fetch('/api/tasks');
    \\    const tasks = await r.json();
    \\    $('task-count').textContent = tasks.length;
    \\    $('tasks').innerHTML = tasks.map(t =>
    \\      '<div class="task-row">' +
    \\        '<span class="task-id">#' + t.id + '</span>' +
    \\        badge(t.status) +
    \\        '<span class="task-title">' + t.title + '</span>' +
    \\        (t.branch ? '<span style="color:#565680;font-size:11px">' + t.branch + '</span>' : '') +
    \\      '</div>'
    \\    ).join('');
    \\  } catch(_) {}
    \\}
    \\
    \\async function refreshQueue() {
    \\  try {
    \\    const r = await fetch('/api/queue');
    \\    const queue = await r.json();
    \\    $('queue-count').textContent = queue.length;
    \\    if (queue.length === 0) {
    \\      $('queue').innerHTML = '<div style="color:#565680;padding:20px">No branches in queue</div>';
    \\      return;
    \\    }
    \\    $('queue').innerHTML = queue.map(q =>
    \\      '<div class="queue-row">' +
    \\        badge(q.status) +
    \\        ' <span style="color:#c8c8d0">' + q.branch + '</span>' +
    \\        ' <span style="color:#565680;font-size:11px">task #' + q.task_id + ' &middot; ' + q.queued_at + '</span>' +
    \\      '</div>'
    \\    ).join('');
    \\  } catch(_) {}
    \\}
    \\
    \\async function refreshStatus() {
    \\  try {
    \\    const r = await fetch('/api/status');
    \\    const s = await r.json();
    \\    const h = Math.floor(s.uptime_s / 3600);
    \\    const m = Math.floor((s.uptime_s % 3600) / 60);
    \\    $('status').innerHTML =
    \\      '<div class="stat"><span class="stat-label">Uptime</span><span class="stat-value">' + h + 'h ' + m + 'm</span></div>' +
    \\      '<div class="stat"><span class="stat-label">Model</span><span class="stat-value">' + s.model + '</span></div>' +
    \\      '<div class="stat"><span class="stat-label">Pipeline Repo</span><span class="stat-value">' + s.pipeline_repo + '</span></div>' +
    \\      '<div class="stat"><span class="stat-label">Release Interval</span><span class="stat-value">' + s.release_interval_mins + ' min</span></div>' +
    \\      '<div class="stat"><span class="stat-label">Test Command</span><span class="stat-value">' + s.test_cmd + '</span></div>';
    \\  } catch(_) {}
    \\}
    \\
    \\refreshTasks(); refreshQueue(); refreshStatus();
    \\setInterval(refreshTasks, 5000);
    \\setInterval(refreshQueue, 5000);
    \\setInterval(refreshStatus, 10000);
    \\</script></body></html>
;

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
