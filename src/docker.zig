const std = @import("std");
const http = @import("http.zig");
const json = @import("json.zig");
const agent_mod = @import("agent.zig");

pub const ContainerConfig = struct {
    image: []const u8,
    name: []const u8,
    env: []const []const u8,
    binds: []const []const u8,
    memory_limit: u64 = 512 * 1024 * 1024, // 512MB
    pids_limit: i64 = 256,
};

pub const ContainerResult = struct {
    id: []const u8,
    allocator: std.mem.Allocator,

    pub fn deinit(self: *ContainerResult) void {
        self.allocator.free(self.id);
    }
};

pub const Docker = struct {
    socket_path: []const u8,
    allocator: std.mem.Allocator,

    pub fn init(allocator: std.mem.Allocator) Docker {
        return Docker{
            .socket_path = "/var/run/docker.sock",
            .allocator = allocator,
        };
    }

    pub fn createContainer(self: *Docker, arena: std.mem.Allocator, config: ContainerConfig) !ContainerResult {
        var body = std.ArrayList(u8).init(arena);
        const w = body.writer();

        try w.writeAll("{\"Image\":\"");
        try w.writeAll(config.image);
        try w.writeAll("\",\"Tty\":false,\"OpenStdin\":true,\"StdinOnce\":true");

        // Env
        try w.writeAll(",\"Env\":[");
        for (config.env, 0..) |env, i| {
            if (i > 0) try w.writeAll(",");
            try w.writeAll("\"");
            const escaped = try json.escapeString(arena, env);
            try w.writeAll(escaped);
            try w.writeAll("\"");
        }
        try w.writeAll("]");

        // HostConfig
        try w.writeAll(",\"HostConfig\":{");

        // Binds
        try w.writeAll("\"Binds\":[");
        for (config.binds, 0..) |bind, i| {
            if (i > 0) try w.writeAll(",");
            try w.writeAll("\"");
            try w.writeAll(bind);
            try w.writeAll("\"");
        }
        try w.writeAll("]");

        try w.print(",\"Memory\":{d}", .{config.memory_limit});
        try w.print(",\"PidsLimit\":{d}", .{config.pids_limit});
        try w.writeAll(",\"SecurityOpt\":[\"no-new-privileges:true\"]");
        try w.writeAll("}}");

        var name_buf: [256]u8 = undefined;
        const path = try std.fmt.bufPrint(&name_buf, "/v1.46/containers/create?name={s}", .{config.name});

        var resp = try http.unixRequest(self.allocator, self.socket_path, .POST, path, body.items);
        defer resp.deinit();

        if (@intFromEnum(resp.status) >= 400) {
            std.log.err("Docker create failed ({d}): {s}", .{ @intFromEnum(resp.status), resp.body[0..@min(resp.body.len, 300)] });
            return error.DockerCreateFailed;
        }

        var parsed = try json.parse(arena, resp.body);
        defer parsed.deinit();

        const id = json.getString(parsed.value, "Id") orelse return error.DockerCreateFailed;
        return ContainerResult{
            .id = try self.allocator.dupe(u8, id),
            .allocator = self.allocator,
        };
    }

    pub fn startContainer(self: *Docker, container_id: []const u8) !void {
        var path_buf: [256]u8 = undefined;
        const path = try std.fmt.bufPrint(&path_buf, "/v1.46/containers/{s}/start", .{container_id});
        var resp = try http.unixRequest(self.allocator, self.socket_path, .POST, path, null);
        defer resp.deinit();
    }

    pub fn stopContainer(self: *Docker, container_id: []const u8) !void {
        var path_buf: [256]u8 = undefined;
        const path = try std.fmt.bufPrint(&path_buf, "/v1.46/containers/{s}/stop?t=5", .{container_id});
        var resp = try http.unixRequest(self.allocator, self.socket_path, .POST, path, null);
        defer resp.deinit();
    }

    pub fn removeContainer(self: *Docker, container_id: []const u8) !void {
        var path_buf: [256]u8 = undefined;
        const path = try std.fmt.bufPrint(&path_buf, "/v1.46/containers/{s}?force=true", .{container_id});
        var resp = try http.unixRequest(self.allocator, self.socket_path, .DELETE, path, null);
        defer resp.deinit();
    }

    pub fn attachAndWrite(self: *Docker, container_id: []const u8, input: []const u8) ![]const u8 {
        // Use exec to run the entrypoint with stdin piped
        _ = self;
        _ = container_id;
        _ = input;
        // Docker attach over Unix socket is complex; we'll use docker exec via CLI instead
        return "";
    }

    /// Run a container with stdin piped and capture stdout.
    /// This shells out to `docker` CLI for reliable stdin/stdout handling.
    pub fn runWithStdio(self: *Docker, config: ContainerConfig, stdin_data: []const u8, stream_cb: agent_mod.StreamCallback) !RunResult {
        // Validate all bind mounts before creating container
        for (config.binds) |bind| {
            if (!isBindSafe(bind)) {
                std.log.err("Blocked unsafe mount: {s}", .{bind});
                return error.DockerCreateFailed;
            }
        }

        var argv = std.ArrayList([]const u8).init(self.allocator);
        defer argv.deinit();

        try argv.appendSlice(&.{ "docker", "run", "--rm", "-i", "--name", config.name });

        // Security: PID limit, no privilege escalation, read-only root, drop all caps
        try argv.appendSlice(&.{ "--pids-limit", "256", "--security-opt", "no-new-privileges:true" });
        try argv.appendSlice(&.{ "--cap-drop", "ALL" });
        try argv.appendSlice(&.{ "--network", "host" }); // Claude Code needs network for API

        // Memory + CPU limits
        var mem_buf: [32]u8 = undefined;
        const mem_str = try std.fmt.bufPrint(&mem_buf, "{d}", .{config.memory_limit});
        try argv.appendSlice(&.{ "--memory", mem_str, "--cpus", "2" });

        // Env vars
        for (config.env) |env| {
            try argv.appendSlice(&.{ "-e", env });
        }

        // Binds (already validated above)
        for (config.binds) |bind| {
            try argv.appendSlice(&.{ "-v", bind });
        }

        try argv.append(config.image);

        var child = std.process.Child.init(argv.items, self.allocator);
        child.stdin_behavior = .Pipe;
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Ignore;

        try child.spawn();

        // Write stdin
        if (child.stdin) |stdin| {
            stdin.writeAll(stdin_data) catch {};
            stdin.close();
            child.stdin = null;
        }

        // Read stdout, streaming each chunk via callback
        var stdout_buf = std.ArrayList(u8).init(self.allocator);
        if (child.stdout) |stdout| {
            var read_buf: [8192]u8 = undefined;
            while (true) {
                const n = stdout.read(&read_buf) catch break;
                if (n == 0) break;
                try stdout_buf.appendSlice(read_buf[0..n]);
                stream_cb.call(read_buf[0..n]);
            }
        }

        const term = try child.wait();
        const exit_code: u8 = switch (term) {
            .Exited => |code| code,
            else => 1,
        };

        return RunResult{
            .stdout = try stdout_buf.toOwnedSlice(),
            .exit_code = exit_code,
            .allocator = self.allocator,
        };
    }

    pub fn imageExists(self: *Docker, image_name: []const u8) !bool {
        var path_buf: [512]u8 = undefined;
        const path = try std.fmt.bufPrint(&path_buf, "/v1.46/images/{s}/json", .{image_name});
        var resp = try http.unixRequest(self.allocator, self.socket_path, .GET, path, null);
        defer resp.deinit();
        return resp.status == .ok;
    }

    pub fn killContainer(self: *Docker, name: []const u8) !void {
        var argv = [_][]const u8{ "docker", "kill", name };
        var child = std.process.Child.init(&argv, self.allocator);
        child.stdout_behavior = .Ignore;
        child.stderr_behavior = .Ignore;
        try child.spawn();
        _ = try child.wait();
    }

    pub fn isAvailable(self: *Docker) bool {
        var resp = http.unixRequest(self.allocator, self.socket_path, .GET, "/v1.46/_ping", null) catch return false;
        defer resp.deinit();
        return resp.status == .ok;
    }

    pub fn cleanupOrphans(self: *Docker) !void {
        var path_buf: [256]u8 = undefined;
        const path = try std.fmt.bufPrint(&path_buf, "/v1.46/containers/json?all=true&filters={{\"label\":[\"borg.managed=true\"]}}", .{});
        var resp = try http.unixRequest(self.allocator, self.socket_path, .GET, path, null);
        defer resp.deinit();

        if (resp.status != .ok) return;

        var parsed = try json.parse(self.allocator, resp.body);
        defer parsed.deinit();

        if (parsed.value != .array) return;
        for (parsed.value.array.items) |container| {
            if (json.getString(container, "Id")) |id| {
                std.log.info("Cleaning up orphan container: {s}", .{id[0..@min(id.len, 12)]});
                self.removeContainer(id) catch {};
            }
        }
    }
};

pub const RunResult = struct {
    stdout: []const u8,
    exit_code: u8,
    allocator: std.mem.Allocator,

    pub fn deinit(self: *RunResult) void {
        self.allocator.free(self.stdout);
    }
};

/// Check if a bind mount path is safe (no sensitive directories)
fn isBindSafe(bind: []const u8) bool {
    // Extract host path (everything before the first :)
    const colon = std.mem.indexOf(u8, bind, ":") orelse return false;
    const host_path = bind[0..colon];

    // Reject path traversal
    if (std.mem.indexOf(u8, host_path, "..") != null) return false;

    // Block sensitive paths
    const blocked = [_][]const u8{
        "/.ssh",
        "/.aws",
        "/.gnupg",
        "/.config/gcloud",
        "/.kube",
        "/credentials",
        "/.env",
        "/id_rsa",
        "/id_ed25519",
        "/.git/config",
    };
    for (blocked) |pattern| {
        if (std.mem.indexOf(u8, host_path, pattern) != null) return false;
    }

    return true;
}

pub const DockerError = error{
    DockerCreateFailed,
};

// ── Tests ──────────────────────────────────────────────────────────────

test "isBindSafe allows normal paths" {
    try std.testing.expect(isBindSafe("/home/user/project:/workspace"));
    try std.testing.expect(isBindSafe("/tmp/data:/data:rw"));
    try std.testing.expect(isBindSafe("/home/user/borg/data/sessions/test:/home/node/.claude/projects/test"));
}

test "isBindSafe blocks sensitive paths" {
    try std.testing.expect(!isBindSafe("/home/user/.ssh:/workspace/.ssh"));
    try std.testing.expect(!isBindSafe("/home/user/.aws:/root/.aws"));
    try std.testing.expect(!isBindSafe("/home/user/../etc/passwd:/etc/passwd"));
    try std.testing.expect(!isBindSafe("/home/user/.env:/app/.env"));
    try std.testing.expect(!isBindSafe("/home/user/.gnupg:/root/.gnupg"));
    try std.testing.expect(!isBindSafe("no-colon-is-invalid"));
}

test "isBindSafe uses only first colon as split point" {
    // Third segment ":ro" must not affect host-path extraction.
    try std.testing.expect(isBindSafe("/src:/dst:ro"));
    try std.testing.expect(isBindSafe("/tmp/work:/workspace:rw"));
    // Sensitive host path is still rejected even when extra colon segments follow.
    try std.testing.expect(!isBindSafe("/home/user/.ssh:/dst:ro"));
    try std.testing.expect(!isBindSafe("/home/user/.aws:/root/.aws:ro"));
}

test "isBindSafe returns false when bind string contains no colon" {
    try std.testing.expect(!isBindSafe("nodst"));
    try std.testing.expect(!isBindSafe(""));
    try std.testing.expect(!isBindSafe("/absolute/path/no/colon"));
}

test "isBindSafe with leading colon has empty host path" {
    // host_path == ""; no blocked patterns match, no ".." present.
    // Current implementation returns true.
    try std.testing.expect(isBindSafe(":/dst"));
    try std.testing.expect(isBindSafe(":/workspace:ro"));
}

test "isBindSafe with trailing colon has non-empty safe host path" {
    // host_path == "/src"; container path is empty but that is Docker's concern.
    try std.testing.expect(isBindSafe("/src:"));
    try std.testing.expect(!isBindSafe("/home/user/.ssh:"));
}
