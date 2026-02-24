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

        const stats = self.db.getPipelineStats() catch db_mod.Db.PipelineStats{ .active = 0, .merged = 0, .failed = 0, .total = 0 };

        var buf: [2048]u8 = undefined;
        const body = std.fmt.bufPrint(&buf, "{{\"uptime_s\":{d},\"model\":\"{s}\",\"pipeline_repo\":\"{s}\",\"release_interval_mins\":{d},\"test_cmd\":\"{s}\",\"continuous_mode\":{s},\"assistant_name\":\"{s}\",\"active_tasks\":{d},\"merged_tasks\":{d},\"failed_tasks\":{d},\"total_tasks\":{d}}}", .{
            uptime,
            self.config.model,
            self.config.pipeline_repo,
            self.config.release_interval_mins,
            self.config.pipeline_test_cmd,
            if (self.config.continuous_mode) "true" else "false",
            self.config.assistant_name,
            stats.active,
            stats.merged,
            stats.failed,
            stats.total,
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
    \\*{margin:0;padding:0;box-sizing:border-box}
    \\body{background:#0a0a0f;color:#c8c8d0;font-family:'JetBrains Mono','Fira Code',monospace;font-size:13px}
    \\header{background:#12121a;border-bottom:1px solid #2a2a3a;padding:10px 20px;display:flex;align-items:center;gap:16px}
    \\header h1{font-size:15px;color:#7aa2f7;font-weight:700;letter-spacing:1px}
    \\.hdr-sep{width:1px;height:20px;background:#2a2a3a}
    \\.hdr-item{font-size:11px;color:#565680}
    \\.hdr-item span{color:#c8c8d0}
    \\.hdr-mode{font-size:10px;padding:2px 8px;border-radius:3px;text-transform:uppercase;letter-spacing:1px;font-weight:600}
    \\.hdr-mode-cont{background:#1a2a1a;color:#9ece6a}
    \\.hdr-mode-interval{background:#2a2a1a;color:#e0af68}
    \\.hdr-right{margin-left:auto;display:flex;align-items:center;gap:12px}
    \\.hdr-dot{width:7px;height:7px;border-radius:50%;display:inline-block}
    \\.dot-ok{background:#9ece6a}
    \\.dot-err{background:#f7768e}
    \\.dot-warn{background:#e0af68}
    \\.stats-bar{background:#12121a;border-bottom:1px solid #1a1a2a;padding:8px 20px;display:flex;gap:24px}
    \\.stat-pill{display:flex;align-items:center;gap:6px;font-size:11px}
    \\.stat-num{font-size:16px;font-weight:700}
    \\.stat-num-active{color:#7aa2f7}
    \\.stat-num-merged{color:#9ece6a}
    \\.stat-num-failed{color:#f7768e}
    \\.stat-num-total{color:#565680}
    \\.stat-lbl{color:#565680;text-transform:uppercase;letter-spacing:0.5px;font-size:10px}
    \\.main{display:grid;grid-template-columns:1fr 1fr;height:calc(100vh - 84px);gap:1px;background:#1a1a2a}
    \\.panel{background:#0a0a0f;display:flex;flex-direction:column;overflow:hidden}
    \\.panel-full{grid-column:1/-1}
    \\.ph{background:#12121a;padding:7px 14px;font-size:10px;text-transform:uppercase;letter-spacing:1px;color:#565680;border-bottom:1px solid #1a1a2a;flex-shrink:0;display:flex;justify-content:space-between;align-items:center}
    \\.ph-filters{display:flex;gap:4px}
    \\.ph-filter{background:none;border:1px solid #2a2a3a;color:#565680;padding:1px 6px;border-radius:2px;font-size:9px;cursor:pointer;font-family:inherit}
    \\.ph-filter:hover,.ph-filter.active{border-color:#7aa2f7;color:#7aa2f7}
    \\.pb{flex:1;overflow-y:auto;padding:6px 14px}
    \\.log-line{padding:1px 0;white-space:pre-wrap;word-break:break-all;font-size:11px;line-height:1.5}
    \\.log-ts{color:#3a3a5a}
    \\.log-info .log-lvl{color:#7aa2f7}
    \\.log-warn .log-lvl{color:#e0af68}
    \\.log-err .log-lvl{color:#f7768e}
    \\.log-debug .log-lvl{color:#565680}
    \\.task-row{padding:5px 0;border-bottom:1px solid #1a1a2a;display:flex;gap:8px;align-items:center}
    \\.task-row:last-child{border-bottom:none}
    \\.task-active{background:#0d0d18;border-left:2px solid #7aa2f7;padding-left:10px;margin:2px -14px;padding-right:14px}
    \\.task-id{color:#3a3a5a;width:28px;font-size:11px}
    \\.task-title{flex:1;color:#c8c8d0;font-size:12px}
    \\.task-attempt{color:#3a3a5a;font-size:10px}
    \\.badge{padding:1px 6px;border-radius:2px;font-size:9px;text-transform:uppercase;letter-spacing:0.5px;font-weight:600}
    \\.badge-backlog{background:#1a1a2a;color:#565680}
    \\.badge-spec{background:#1a1a3a;color:#7aa2f7}
    \\.badge-qa{background:#1a2a3a;color:#7dcfff}
    \\.badge-impl{background:#2a2a1a;color:#e0af68}
    \\.badge-retry{background:#2a1a1a;color:#f7768e}
    \\.badge-done{background:#1a2a1a;color:#9ece6a}
    \\.badge-merged{background:#0a2a0a;color:#73daca}
    \\.badge-rebase{background:#2a1a2a;color:#bb9af7}
    \\.badge-failed{background:#2a1a1a;color:#f7768e}
    \\.badge-queued{background:#1a1a3a;color:#7aa2f7}
    \\.badge-merging{background:#2a2a1a;color:#e0af68}
    \\.badge-excluded{background:#2a1a1a;color:#f7768e}
    \\.phase-bar{display:flex;gap:2px;margin:8px 0}
    \\.phase-step{flex:1;height:3px;background:#1a1a2a;border-radius:1px}
    \\.phase-step.done{background:#565680}
    \\.phase-step.current{background:#7aa2f7;box-shadow:0 0 6px rgba(122,162,247,0.4)}
    \\.queue-row{padding:5px 0;border-bottom:1px solid #1a1a2a;display:flex;gap:8px;align-items:center;font-size:12px}
    \\.queue-branch{color:#c8c8d0;flex:1}
    \\.queue-meta{color:#3a3a5a;font-size:10px}
    \\.empty{color:#3a3a5a;padding:20px 0;text-align:center;font-size:11px}
    \\.cfg-row{padding:6px 0;display:flex;justify-content:space-between;border-bottom:1px solid #1a1a2a;font-size:12px}
    \\.cfg-k{color:#565680}
    \\.cfg-v{color:#c8c8d0}
    \\::-webkit-scrollbar{width:5px}
    \\::-webkit-scrollbar-track{background:#0a0a0f}
    \\::-webkit-scrollbar-thumb{background:#2a2a3a;border-radius:2px}
    \\</style></head><body>
    \\<header>
    \\  <h1>BORG</h1>
    \\  <div class="hdr-sep"></div>
    \\  <span class="hdr-mode" id="mode-badge">...</span>
    \\  <div class="hdr-sep"></div>
    \\  <span class="hdr-item">uptime <span id="uptime">--</span></span>
    \\  <div class="hdr-sep"></div>
    \\  <span class="hdr-item">model <span id="model">--</span></span>
    \\  <div class="hdr-right">
    \\    <span class="hdr-dot" id="conn-dot"></span>
    \\    <span class="hdr-item" id="conn-status" style="font-size:10px">connecting</span>
    \\  </div>
    \\</header>
    \\<div class="stats-bar">
    \\  <div class="stat-pill"><span class="stat-num stat-num-active" id="s-active">-</span><span class="stat-lbl">active</span></div>
    \\  <div class="stat-pill"><span class="stat-num stat-num-merged" id="s-merged">-</span><span class="stat-lbl">merged</span></div>
    \\  <div class="stat-pill"><span class="stat-num stat-num-failed" id="s-failed">-</span><span class="stat-lbl">failed</span></div>
    \\  <div class="stat-pill"><span class="stat-num stat-num-total" id="s-total">-</span><span class="stat-lbl">total</span></div>
    \\</div>
    \\<div class="main">
    \\  <div class="panel" style="grid-row:1/3">
    \\    <div class="ph"><span>Live Logs</span>
    \\      <div class="ph-filters">
    \\        <button class="ph-filter active" data-lvl="all">all</button>
    \\        <button class="ph-filter" data-lvl="info">info</button>
    \\        <button class="ph-filter" data-lvl="warn">warn</button>
    \\        <button class="ph-filter" data-lvl="err">err</button>
    \\      </div>
    \\    </div>
    \\    <div class="pb" id="logs"></div>
    \\  </div>
    \\  <div class="panel">
    \\    <div class="ph"><span>Pipeline Tasks</span><span id="task-count">0</span></div>
    \\    <div class="pb" id="tasks"></div>
    \\  </div>
    \\  <div class="panel">
    \\    <div class="ph"><span>Integration Queue</span><span id="queue-count">0</span></div>
    \\    <div class="pb" id="queue"></div>
    \\  </div>
    \\</div>
    \\<script>
    \\const $=id=>document.getElementById(id);
    \\let logFilter='all';
    \\let startTs=Date.now();
    \\
    \\document.querySelectorAll('.ph-filter').forEach(b=>{
    \\  b.onclick=()=>{
    \\    document.querySelectorAll('.ph-filter').forEach(x=>x.classList.remove('active'));
    \\    b.classList.add('active');
    \\    logFilter=b.dataset.lvl;
    \\    document.querySelectorAll('.log-line').forEach(l=>{
    \\      l.style.display=(logFilter==='all'||l.dataset.lvl===logFilter)?'':'none';
    \\    });
    \\  };
    \\});
    \\
    \\const es=new EventSource('/api/logs');
    \\es.onopen=()=>{$('conn-dot').className='hdr-dot dot-ok';$('conn-status').textContent='live'};
    \\es.onerror=()=>{$('conn-dot').className='hdr-dot dot-err';$('conn-status').textContent='disconnected'};
    \\es.onmessage=e=>{
    \\  try{
    \\    const d=JSON.parse(e.data);
    \\    const el=document.createElement('div');
    \\    el.className='log-line log-'+d.level;
    \\    el.dataset.lvl=d.level;
    \\    const ts=new Date(d.ts*1000).toLocaleTimeString('en-GB',{hour12:false});
    \\    el.innerHTML='<span class="log-ts">'+ts+'</span> <span class="log-lvl">['+d.level+']</span> '+escHtml(d.message);
    \\    if(logFilter!=='all'&&d.level!==logFilter)el.style.display='none';
    \\    const logs=$('logs');
    \\    logs.appendChild(el);
    \\    if(logs.children.length>500)logs.removeChild(logs.firstChild);
    \\    logs.scrollTop=logs.scrollHeight;
    \\  }catch(_){}
    \\};
    \\
    \\function escHtml(s){return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;')}
    \\function badge(s){return '<span class="badge badge-'+s+'">'+s+'</span>'}
    \\
    \\const PHASES=['backlog','spec','qa','impl','done','merged'];
    \\function phaseBar(status){
    \\  const idx=PHASES.indexOf(status);
    \\  return '<div class="phase-bar">'+PHASES.map((p,i)=>{
    \\    const cls=i<idx?'done':i===idx?'current':'';
    \\    return '<div class="phase-step '+cls+'" title="'+p+'"></div>';
    \\  }).join('')+'</div>';
    \\}
    \\
    \\async function refreshTasks(){
    \\  try{
    \\    const r=await fetch('/api/tasks');
    \\    const tasks=await r.json();
    \\    const active=tasks.filter(t=>['backlog','spec','qa','impl','retry','rebase'].includes(t.status));
    \\    const done=tasks.filter(t=>!['backlog','spec','qa','impl','retry','rebase'].includes(t.status));
    \\    $('task-count').textContent=tasks.length;
    \\    let html='';
    \\    if(active.length>0){
    \\      active.forEach(t=>{
    \\        const isWorking=['spec','qa','impl','retry','rebase'].includes(t.status);
    \\        const effectiveStatus=t.status==='retry'?'impl':t.status==='rebase'?'impl':t.status;
    \\        html+='<div class="task-row'+(isWorking?' task-active':'')+'">';
    \\        html+='<span class="task-id">#'+t.id+'</span>';
    \\        html+=badge(t.status);
    \\        html+='<span class="task-title">'+escHtml(t.title)+'</span>';
    \\        if(t.attempt>0)html+='<span class="task-attempt">'+t.attempt+'/'+t.max_attempts+'</span>';
    \\        html+='</div>';
    \\        if(isWorking)html+=phaseBar(effectiveStatus);
    \\      });
    \\    }
    \\    if(done.length>0){
    \\      done.slice(0,20).forEach(t=>{
    \\        html+='<div class="task-row">';
    \\        html+='<span class="task-id">#'+t.id+'</span>';
    \\        html+=badge(t.status);
    \\        html+='<span class="task-title">'+escHtml(t.title)+'</span>';
    \\        html+='</div>';
    \\      });
    \\    }
    \\    if(tasks.length===0)html='<div class="empty">No pipeline tasks yet</div>';
    \\    $('tasks').innerHTML=html;
    \\  }catch(_){}
    \\}
    \\
    \\async function refreshQueue(){
    \\  try{
    \\    const r=await fetch('/api/queue');
    \\    const q=await r.json();
    \\    $('queue-count').textContent=q.length;
    \\    if(q.length===0){$('queue').innerHTML='<div class="empty">Queue empty</div>';return}
    \\    $('queue').innerHTML=q.map(e=>
    \\      '<div class="queue-row">'+badge(e.status)+
    \\      '<span class="queue-branch">'+escHtml(e.branch)+'</span>'+
    \\      '<span class="queue-meta">#'+e.task_id+'</span></div>'
    \\    ).join('');
    \\  }catch(_){}
    \\}
    \\
    \\async function refreshStatus(){
    \\  try{
    \\    const r=await fetch('/api/status');
    \\    const s=await r.json();
    \\    const h=Math.floor(s.uptime_s/3600);
    \\    const m=Math.floor((s.uptime_s%3600)/60);
    \\    const sec=s.uptime_s%60;
    \\    $('uptime').textContent=h+'h '+m+'m '+sec+'s';
    \\    $('model').textContent=s.model;
    \\    const mb=$('mode-badge');
    \\    if(s.continuous_mode){mb.textContent='continuous';mb.className='hdr-mode hdr-mode-cont'}
    \\    else{mb.textContent='every '+s.release_interval_mins+'m';mb.className='hdr-mode hdr-mode-interval'}
    \\    $('s-active').textContent=s.active_tasks;
    \\    $('s-merged').textContent=s.merged_tasks;
    \\    $('s-failed').textContent=s.failed_tasks;
    \\    $('s-total').textContent=s.total_tasks;
    \\  }catch(_){}
    \\}
    \\
    \\refreshTasks();refreshQueue();refreshStatus();
    \\setInterval(refreshTasks,3000);
    \\setInterval(refreshQueue,3000);
    \\setInterval(refreshStatus,3000);
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
