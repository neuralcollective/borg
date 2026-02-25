const std = @import("std");

pub const RepoConfig = struct {
    path: []const u8,
    test_cmd: []const u8,
    is_self: bool, // true for primary repo (triggers self-update)
};

pub const Config = struct {
    telegram_token: []const u8,
    oauth_token: []const u8,
    assistant_name: []const u8,
    trigger_pattern: []const u8,
    data_dir: []const u8,
    container_image: []const u8,
    model: []const u8,
    credentials_path: []const u8,
    session_max_age_hours: i64,
    max_consecutive_errors: u32,
    // Pipeline config
    pipeline_repo: []const u8,
    pipeline_test_cmd: []const u8,
    pipeline_lint_cmd: []const u8,
    pipeline_admin_chat: []const u8,
    release_interval_mins: u32,
    continuous_mode: bool,
    // Agent lifecycle
    collection_window_ms: i64,
    cooldown_ms: i64,
    agent_timeout_s: i64,
    max_concurrent_agents: u32,
    rate_limit_per_minute: u32,
    max_pipeline_agents: u32,
    // Web dashboard
    web_port: u16,
    dashboard_dist_dir: []const u8,
    // Multi-repo
    watched_repos: []RepoConfig,
    // WhatsApp config
    whatsapp_enabled: bool,
    whatsapp_auth_dir: []const u8,
    // Discord config
    discord_enabled: bool,
    discord_token: []const u8,
    allocator: std.mem.Allocator,

    pub fn load(allocator: std.mem.Allocator) !Config {
        const env_content = std.fs.cwd().readFileAlloc(allocator, ".env", 8192) catch "";

        // Try reading OAuth token from Claude credentials file (it rotates)
        const home = std.posix.getenv("HOME") orelse "/home/shulgin";
        const creds_path = try std.fmt.allocPrint(allocator, "{s}/.claude/.credentials.json", .{home});
        const oauth = readOAuthToken(allocator, creds_path) orelse
            getEnv(allocator, env_content, "CLAUDE_CODE_OAUTH_TOKEN") orelse "";

        const release_mins_str = getEnv(allocator, env_content, "RELEASE_INTERVAL_MINS") orelse "180";
        const release_mins = std.fmt.parseInt(u32, release_mins_str, 10) catch 180;

        const collection_ms_str = getEnv(allocator, env_content, "COLLECTION_WINDOW_MS") orelse "3000";
        const cooldown_ms_str = getEnv(allocator, env_content, "COOLDOWN_MS") orelse "5000";
        const timeout_s_str = getEnv(allocator, env_content, "AGENT_TIMEOUT_S") orelse "600";
        const max_agents_str = getEnv(allocator, env_content, "MAX_CONCURRENT_AGENTS") orelse "4";
        const rate_limit_str = getEnv(allocator, env_content, "RATE_LIMIT_PER_MINUTE") orelse "5";
        const max_pipeline_agents_str = getEnv(allocator, env_content, "MAX_PIPELINE_AGENTS") orelse "4";
        const web_port_str = getEnv(allocator, env_content, "WEB_PORT") orelse "3131";

        var config = Config{
            .telegram_token = getEnv(allocator, env_content, "TELEGRAM_BOT_TOKEN") orelse "",
            .oauth_token = oauth,
            .assistant_name = getEnv(allocator, env_content, "ASSISTANT_NAME") orelse "Borg",
            .trigger_pattern = getEnv(allocator, env_content, "TRIGGER_PATTERN") orelse "@Borg",
            .data_dir = getEnv(allocator, env_content, "DATA_DIR") orelse "data",
            .container_image = getEnv(allocator, env_content, "CONTAINER_IMAGE") orelse "borg-agent:latest",
            .model = getEnv(allocator, env_content, "CLAUDE_MODEL") orelse "claude-sonnet-4-6",
            .credentials_path = creds_path,
            .session_max_age_hours = 4,
            .max_consecutive_errors = 3,
            .pipeline_repo = getEnv(allocator, env_content, "PIPELINE_REPO") orelse "",
            .pipeline_test_cmd = getEnv(allocator, env_content, "PIPELINE_TEST_CMD") orelse "zig build test",
            .pipeline_lint_cmd = getEnv(allocator, env_content, "PIPELINE_LINT_CMD") orelse "",
            .pipeline_admin_chat = getEnv(allocator, env_content, "PIPELINE_ADMIN_CHAT") orelse "",
            .release_interval_mins = release_mins,
            .continuous_mode = std.mem.eql(u8, getEnv(allocator, env_content, "CONTINUOUS_MODE") orelse "false", "true"),
            .collection_window_ms = std.fmt.parseInt(i64, collection_ms_str, 10) catch 3000,
            .cooldown_ms = std.fmt.parseInt(i64, cooldown_ms_str, 10) catch 5000,
            .agent_timeout_s = std.fmt.parseInt(i64, timeout_s_str, 10) catch 600,
            .max_concurrent_agents = std.fmt.parseInt(u32, max_agents_str, 10) catch 4,
            .rate_limit_per_minute = std.fmt.parseInt(u32, rate_limit_str, 10) catch 5,
            .max_pipeline_agents = std.fmt.parseInt(u32, max_pipeline_agents_str, 10) catch 2,
            .web_port = std.fmt.parseInt(u16, web_port_str, 10) catch 3131,
            .dashboard_dist_dir = getEnv(allocator, env_content, "DASHBOARD_DIST_DIR") orelse try std.fmt.allocPrint(allocator, "{s}/dashboard/dist", .{getEnv(allocator, env_content, "PIPELINE_REPO") orelse "."}),
            .watched_repos = &.{},
            .whatsapp_enabled = std.mem.eql(u8, getEnv(allocator, env_content, "WHATSAPP_ENABLED") orelse "false", "true"),
            .whatsapp_auth_dir = getEnv(allocator, env_content, "WHATSAPP_AUTH_DIR") orelse "whatsapp/auth",
            .discord_enabled = std.mem.eql(u8, getEnv(allocator, env_content, "DISCORD_ENABLED") orelse "false", "true"),
            .discord_token = getEnv(allocator, env_content, "DISCORD_TOKEN") orelse "",
            .allocator = allocator,
        };

        // Build watched_repos list
        config.watched_repos = try parseWatchedRepos(allocator, env_content, config.pipeline_repo, config.pipeline_test_cmd);

        return config;
    }

    pub fn getTestCmdForRepo(self: *Config, repo_path: []const u8) []const u8 {
        for (self.watched_repos) |rc| {
            if (std.mem.eql(u8, rc.path, repo_path)) return rc.test_cmd;
        }
        return self.pipeline_test_cmd;
    }

    /// Re-read OAuth token from credentials file (handles token rotation)
    pub fn refreshOAuthToken(self: *Config) void {
        if (readOAuthToken(self.allocator, self.credentials_path)) |new_token| {
            self.oauth_token = new_token;
        }
    }
};

fn parseWatchedRepos(allocator: std.mem.Allocator, env_content: []const u8, primary_repo: []const u8, primary_test_cmd: []const u8) ![]RepoConfig {
    var repos = std.ArrayList(RepoConfig).init(allocator);

    // Primary repo always first
    if (primary_repo.len > 0) {
        try repos.append(.{ .path = primary_repo, .test_cmd = primary_test_cmd, .is_self = true });
    }

    // Parse WATCHED_REPOS: pipe-delimited, each entry is path:test_cmd
    const watched_opt = getEnv(allocator, env_content, "WATCHED_REPOS");
    defer if (watched_opt) |w| allocator.free(w);
    const watched = watched_opt orelse "";
    if (watched.len > 0) {
        var entries = std.mem.splitScalar(u8, watched, '|');
        while (entries.next()) |entry| {
            const trimmed = std.mem.trim(u8, entry, &[_]u8{ ' ', '\t' });
            if (trimmed.len == 0) continue;

            // Skip if same as primary
            if (std.mem.indexOf(u8, trimmed, ":")) |colon| {
                const path = std.mem.trim(u8, trimmed[0..colon], &[_]u8{ ' ', '\t' });
                const cmd = std.mem.trim(u8, trimmed[colon + 1 ..], &[_]u8{ ' ', '\t' });
                if (path.len == 0) continue;
                if (std.mem.eql(u8, path, primary_repo)) continue;
                try repos.append(.{
                    .path = try allocator.dupe(u8, path),
                    .test_cmd = if (cmd.len > 0) try allocator.dupe(u8, cmd) else "make test",
                    .is_self = false,
                });
            } else {
                if (std.mem.eql(u8, trimmed, primary_repo)) continue;
                try repos.append(.{
                    .path = try allocator.dupe(u8, trimmed),
                    .test_cmd = "make test",
                    .is_self = false,
                });
            }
        }
    }

    return repos.toOwnedSlice();
}

/// Read from .env file content, falling back to process environment.
/// .env values are NOT loaded into process.env (security: keeps secrets off child processes).
fn getEnv(allocator: std.mem.Allocator, env_content: []const u8, key: []const u8) ?[]const u8 {
    // Check .env file first
    if (findEnvValue(allocator, env_content, key)) |val| return val;
    // Fall back to process environment
    const val = std.posix.getenv(key) orelse return null;
    return allocator.dupe(u8, val) catch null;
}

fn readOAuthToken(allocator: std.mem.Allocator, path: []const u8) ?[]const u8 {
    const content = std.fs.cwd().readFileAlloc(allocator, path, 65536) catch return null;
    defer allocator.free(content);
    const json = @import("json.zig");
    var parsed = json.parse(allocator, content) catch return null;
    defer parsed.deinit();
    if (json.getObject(parsed.value, "claudeAiOauth")) |oauth_obj| {
        if (json.getString(oauth_obj, "accessToken")) |token| {
            return allocator.dupe(u8, token) catch null;
        }
    }
    return null;
}

fn findEnvValue(allocator: std.mem.Allocator, content: []const u8, key: []const u8) ?[]const u8 {
    var lines = std.mem.splitScalar(u8, content, '\n');
    while (lines.next()) |line| {
        const trimmed = std.mem.trim(u8, line, &[_]u8{ ' ', '\t', '\r' });
        if (trimmed.len == 0 or trimmed[0] == '#') continue;

        if (std.mem.indexOf(u8, trimmed, "=")) |eq_pos| {
            const k = std.mem.trim(u8, trimmed[0..eq_pos], &[_]u8{ ' ', '\t' });
            if (std.mem.eql(u8, k, key)) {
                var v = std.mem.trim(u8, trimmed[eq_pos + 1 ..], &[_]u8{ ' ', '\t' });
                // Strip quotes
                if (v.len >= 2 and (v[0] == '"' or v[0] == '\'')) {
                    if (v[v.len - 1] == v[0]) {
                        v = v[1 .. v.len - 1];
                    }
                }
                return allocator.dupe(u8, v) catch null;
            }
        }
    }
    return null;
}

// ── Tests ──────────────────────────────────────────────────────────────

test "findEnvValue basic parsing" {
    const alloc = std.testing.allocator;
    const env =
        \\KEY1=value1
        \\KEY2 = value2
        \\KEY3=
    ;

    const v1 = findEnvValue(alloc, env, "KEY1");
    defer if (v1) |v| alloc.free(v);
    try std.testing.expectEqualStrings("value1", v1.?);

    const v2 = findEnvValue(alloc, env, "KEY2");
    defer if (v2) |v| alloc.free(v);
    try std.testing.expectEqualStrings("value2", v2.?);

    const v3 = findEnvValue(alloc, env, "KEY3");
    defer if (v3) |v| alloc.free(v);
    try std.testing.expectEqualStrings("", v3.?);

    try std.testing.expect(findEnvValue(alloc, env, "MISSING") == null);
}

test "findEnvValue strips matching quotes" {
    const alloc = std.testing.allocator;
    const env =
        \\A="hello world"
        \\B='single quoted'
        \\C="mismatched'
    ;

    const v1 = findEnvValue(alloc, env, "A");
    defer if (v1) |v| alloc.free(v);
    try std.testing.expectEqualStrings("hello world", v1.?);

    const v2 = findEnvValue(alloc, env, "B");
    defer if (v2) |v| alloc.free(v);
    try std.testing.expectEqualStrings("single quoted", v2.?);

    // Mismatched quotes are preserved
    const v3 = findEnvValue(alloc, env, "C");
    defer if (v3) |v| alloc.free(v);
    try std.testing.expectEqualStrings("\"mismatched'", v3.?);
}

test "findEnvValue skips comments and blank lines" {
    const alloc = std.testing.allocator;
    const env =
        \\# comment
        \\
        \\  # indented comment
        \\REAL=value
    ;

    try std.testing.expect(findEnvValue(alloc, env, "#") == null);

    const v = findEnvValue(alloc, env, "REAL");
    defer if (v) |val| alloc.free(val);
    try std.testing.expectEqualStrings("value", v.?);
}

// ── parseWatchedRepos tests ────────────────────────────────────────────

test "parseWatchedRepos empty env no primary" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const repos = try parseWatchedRepos(alloc, "", "", "");
    try std.testing.expect(repos.len == 0);
}

test "parseWatchedRepos empty env with primary" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const repos = try parseWatchedRepos(alloc, "", "/home/project", "zig build test");
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/home/project", repos[0].path);
    try std.testing.expectEqualStrings("zig build test", repos[0].test_cmd);
    try std.testing.expect(repos[0].is_self);
}

test "parseWatchedRepos single path no cmd" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const repos = try parseWatchedRepos(alloc, "WATCHED_REPOS=/repo/a", "/primary", "zig test");
    try std.testing.expect(repos.len == 2);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expect(repos[0].is_self);
    try std.testing.expectEqualStrings("/repo/a", repos[1].path);
    try std.testing.expectEqualStrings("make test", repos[1].test_cmd);
    try std.testing.expect(!repos[1].is_self);
}

test "parseWatchedRepos single path with cmd" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const repos = try parseWatchedRepos(alloc, "WATCHED_REPOS=/repo/a:npm test", "/primary", "zig test");
    try std.testing.expect(repos.len == 2);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expect(repos[0].is_self);
    try std.testing.expectEqualStrings("/repo/a", repos[1].path);
    try std.testing.expectEqualStrings("npm test", repos[1].test_cmd);
    try std.testing.expect(!repos[1].is_self);
}

test "parseWatchedRepos pipe-delimited multiple repos" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const repos = try parseWatchedRepos(alloc, "WATCHED_REPOS=/repo/a:cmd1|/repo/b:cmd2", "/primary", "zig test");
    try std.testing.expect(repos.len == 3);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expect(repos[0].is_self);
    try std.testing.expectEqualStrings("/repo/a", repos[1].path);
    try std.testing.expectEqualStrings("cmd1", repos[1].test_cmd);
    try std.testing.expect(!repos[1].is_self);
    try std.testing.expectEqualStrings("/repo/b", repos[2].path);
    try std.testing.expectEqualStrings("cmd2", repos[2].test_cmd);
    try std.testing.expect(!repos[2].is_self);
}

test "parseWatchedRepos duplicate of primary is skipped" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const repos = try parseWatchedRepos(alloc, "WATCHED_REPOS=/main:other_cmd|/second:test", "/main", "zig test");
    try std.testing.expect(repos.len == 2);
    try std.testing.expectEqualStrings("/main", repos[0].path);
    try std.testing.expect(repos[0].is_self);
    try std.testing.expectEqualStrings("/second", repos[1].path);
    try std.testing.expectEqualStrings("test", repos[1].test_cmd);
    try std.testing.expect(!repos[1].is_self);
}

test "parseWatchedRepos whitespace trimming" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const repos = try parseWatchedRepos(alloc, "WATCHED_REPOS= /repo/a : cmd1 | /repo/b ", "/primary", "zig test");
    try std.testing.expect(repos.len == 3);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expect(repos[0].is_self);
    try std.testing.expectEqualStrings("/repo/a", repos[1].path);
    try std.testing.expectEqualStrings("cmd1", repos[1].test_cmd);
    try std.testing.expect(!repos[1].is_self);
    try std.testing.expectEqualStrings("/repo/b", repos[2].path);
    try std.testing.expectEqualStrings("make test", repos[2].test_cmd);
    try std.testing.expect(!repos[2].is_self);
}

test "parseWatchedRepos empty entries between pipes skipped" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const repos = try parseWatchedRepos(alloc, "WATCHED_REPOS=/repo/a||/repo/b", "/primary", "zig test");
    try std.testing.expect(repos.len == 3);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expect(repos[0].is_self);
    try std.testing.expectEqualStrings("/repo/a", repos[1].path);
    try std.testing.expectEqualStrings("/repo/b", repos[2].path);
}

// ── parseWatchedRepos edge-case tests ──────────────────────────────────

test "parseWatchedRepos only whitespace and pipes with primary" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const repos = try parseWatchedRepos(alloc, "WATCHED_REPOS=| | |", "/primary", "zig test");
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expect(repos[0].is_self);
}

test "parseWatchedRepos only whitespace and pipes no primary" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const repos = try parseWatchedRepos(alloc, "WATCHED_REPOS=| | |", "", "");
    try std.testing.expect(repos.len == 0);
}

test "parseWatchedRepos colon with empty path skipped" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const repos = try parseWatchedRepos(alloc, "WATCHED_REPOS=:some_cmd", "/primary", "zig test");
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expect(repos[0].is_self);
}

test "parseWatchedRepos colon with empty cmd uses default" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const repos = try parseWatchedRepos(alloc, "WATCHED_REPOS=/repo/a:", "/primary", "zig test");
    try std.testing.expect(repos.len == 2);
    try std.testing.expectEqualStrings("/repo/a", repos[1].path);
    try std.testing.expectEqualStrings("make test", repos[1].test_cmd);
    try std.testing.expect(!repos[1].is_self);
}

test "parseWatchedRepos path without colon matching primary skipped" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const repos = try parseWatchedRepos(alloc, "WATCHED_REPOS=/primary", "/primary", "zig test");
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expect(repos[0].is_self);
}

// ── parseWatchedRepos Tests (explicit allocator) ───────────────────────

test "parseWatchedRepos: primary repo is first" {
    const alloc = std.testing.allocator;
    const env =
        \\WATCHED_REPOS=/other:cmd_other
    ;
    const repos = try parseWatchedRepos(alloc, env, "/primary", "test_cmd");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len >= 1);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expectEqualStrings("test_cmd", repos[0].test_cmd);
    try std.testing.expect(repos[0].is_self == true);
}

test "parseWatchedRepos: empty primary repo omitted" {
    const alloc = std.testing.allocator;
    const env =
        \\WATCHED_REPOS=/a:cmd_a
    ;
    const repos = try parseWatchedRepos(alloc, env, "", "");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/a", repos[0].path);
    try std.testing.expectEqualStrings("cmd_a", repos[0].test_cmd);
    try std.testing.expect(repos[0].is_self == false);
}

test "parseWatchedRepos: multiple pipe-delimited repos" {
    const alloc = std.testing.allocator;
    const env =
        \\WATCHED_REPOS=/a:cmd_a|/b:cmd_b
    ;
    const repos = try parseWatchedRepos(alloc, env, "/primary", "pcmd");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 3);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expect(repos[0].is_self == true);
    try std.testing.expectEqualStrings("/a", repos[1].path);
    try std.testing.expectEqualStrings("cmd_a", repos[1].test_cmd);
    try std.testing.expectEqualStrings("/b", repos[2].path);
    try std.testing.expectEqualStrings("cmd_b", repos[2].test_cmd);
}

test "parseWatchedRepos: entry without colon uses default cmd" {
    const alloc = std.testing.allocator;
    const env =
        \\WATCHED_REPOS=/repo/path
    ;
    const repos = try parseWatchedRepos(alloc, env, "", "");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/repo/path", repos[0].path);
    try std.testing.expectEqualStrings("make test", repos[0].test_cmd);
}

test "parseWatchedRepos: entry with colon but empty cmd uses default" {
    const alloc = std.testing.allocator;
    const env =
        \\WATCHED_REPOS=/repo/path:
    ;
    const repos = try parseWatchedRepos(alloc, env, "", "");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/repo/path", repos[0].path);
    try std.testing.expectEqualStrings("make test", repos[0].test_cmd);
}

test "parseWatchedRepos: duplicate primary is skipped" {
    const alloc = std.testing.allocator;
    const env =
        \\WATCHED_REPOS=/primary:other_cmd|/other:cmd
    ;
    const repos = try parseWatchedRepos(alloc, env, "/primary", "pcmd");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 2);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expect(repos[0].is_self == true);
    try std.testing.expectEqualStrings("/other", repos[1].path);
    try std.testing.expectEqualStrings("cmd", repos[1].test_cmd);
}

test "parseWatchedRepos: empty entries are skipped" {
    const alloc = std.testing.allocator;
    const env =
        \\WATCHED_REPOS=||/a:cmd||
    ;
    const repos = try parseWatchedRepos(alloc, env, "", "");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/a", repos[0].path);
    try std.testing.expectEqualStrings("cmd", repos[0].test_cmd);
}

test "parseWatchedRepos: whitespace-only entries are skipped" {
    const alloc = std.testing.allocator;
    const env = "WATCHED_REPOS=  | \t |/a:cmd";
    const repos = try parseWatchedRepos(alloc, env, "", "");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/a", repos[0].path);
    try std.testing.expectEqualStrings("cmd", repos[0].test_cmd);
}

test "parseWatchedRepos: leading and trailing whitespace is trimmed" {
    const alloc = std.testing.allocator;
    const env = "WATCHED_REPOS=  /path : cmd  ";
    const repos = try parseWatchedRepos(alloc, env, "", "");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/path", repos[0].path);
    try std.testing.expectEqualStrings("cmd", repos[0].test_cmd);
}

test "parseWatchedRepos: entry with empty path after colon is skipped" {
    const alloc = std.testing.allocator;
    const env =
        \\WATCHED_REPOS=:cmd
    ;
    const repos = try parseWatchedRepos(alloc, env, "", "");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 0);
}

test "parseWatchedRepos: no WATCHED_REPOS in env" {
    const alloc = std.testing.allocator;
    const env =
        \\OTHER_KEY=value
    ;
    const repos = try parseWatchedRepos(alloc, env, "/primary", "pcmd");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expectEqualStrings("pcmd", repos[0].test_cmd);
    try std.testing.expect(repos[0].is_self == true);
}

test "parseWatchedRepos: watched entries have is_self false" {
    const alloc = std.testing.allocator;
    const env =
        \\WATCHED_REPOS=/a:x|/b:y|/c
    ;
    const repos = try parseWatchedRepos(alloc, env, "/primary", "pcmd");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 4);
    try std.testing.expect(repos[0].is_self == true);
    for (repos[1..]) |r| {
        try std.testing.expect(r.is_self == false);
    }
}

// ── parseWatchedRepos Edge Cases ───────────────────────────────────────

test "parseWatchedRepos: empty env and empty primary" {
    const alloc = std.testing.allocator;
    const repos = try parseWatchedRepos(alloc, "", "", "");
    defer alloc.free(repos);
    try std.testing.expect(repos.len == 0);
}

test "parseWatchedRepos: WATCHED_REPOS is empty string" {
    const alloc = std.testing.allocator;
    const env =
        \\WATCHED_REPOS=
    ;
    const repos = try parseWatchedRepos(alloc, env, "/primary", "pcmd");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expect(repos[0].is_self == true);
}

test "parseWatchedRepos: single entry without delimiter" {
    const alloc = std.testing.allocator;
    const env =
        \\WATCHED_REPOS=/single:test
    ;
    const repos = try parseWatchedRepos(alloc, env, "", "");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/single", repos[0].path);
    try std.testing.expectEqualStrings("test", repos[0].test_cmd);
}

test "parseWatchedRepos: duplicate primary without colon" {
    const alloc = std.testing.allocator;
    const env =
        \\WATCHED_REPOS=/primary
    ;
    const repos = try parseWatchedRepos(alloc, env, "/primary", "pcmd");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expect(repos[0].is_self == true);
}

test "parseWatchedRepos: duplicate primary with colon" {
    const alloc = std.testing.allocator;
    const env =
        \\WATCHED_REPOS=/primary:other_cmd
    ;
    const repos = try parseWatchedRepos(alloc, env, "/primary", "pcmd");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expectEqualStrings("pcmd", repos[0].test_cmd);
    try std.testing.expect(repos[0].is_self == true);
}

test "parseWatchedRepos: colon in command portion" {
    const alloc = std.testing.allocator;
    const env = "WATCHED_REPOS=/repo:make -C /path test";
    const repos = try parseWatchedRepos(alloc, env, "", "");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/repo", repos[0].path);
    try std.testing.expectEqualStrings("make -C /path test", repos[0].test_cmd);
}

test "parseWatchedRepos: multiple consecutive pipes" {
    const alloc = std.testing.allocator;
    const env =
        \\WATCHED_REPOS=/a:x|||/b:y
    ;
    const repos = try parseWatchedRepos(alloc, env, "", "");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 2);
    try std.testing.expectEqualStrings("/a", repos[0].path);
    try std.testing.expectEqualStrings("x", repos[0].test_cmd);
    try std.testing.expectEqualStrings("/b", repos[1].path);
    try std.testing.expectEqualStrings("y", repos[1].test_cmd);
}

test "parseWatchedRepos: whitespace around path matching primary" {
    const alloc = std.testing.allocator;
    const env = "WATCHED_REPOS=  /primary  ";
    const repos = try parseWatchedRepos(alloc, env, "/primary", "pcmd");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 1);
    try std.testing.expectEqualStrings("/primary", repos[0].path);
    try std.testing.expect(repos[0].is_self == true);
}

test "parseWatchedRepos: entry that is only a colon" {
    const alloc = std.testing.allocator;
    const env =
        \\WATCHED_REPOS=:
    ;
    const repos = try parseWatchedRepos(alloc, env, "", "");
    defer alloc.free(repos);
    defer for (repos) |r| {
        if (!r.is_self) {
            alloc.free(r.path);
            if (!std.mem.eql(u8, r.test_cmd, "make test")) alloc.free(r.test_cmd);
        }
    };
    try std.testing.expect(repos.len == 0);
}

// ── getTestCmdForRepo tests ────────────────────────────────────────────

fn testMinimalConfig(pipeline_test_cmd: []const u8, watched_repos: []RepoConfig) Config {
    return Config{
        .telegram_token = "",
        .oauth_token = "",
        .assistant_name = "",
        .trigger_pattern = "",
        .data_dir = "",
        .container_image = "",
        .model = "",
        .credentials_path = "",
        .session_max_age_hours = 0,
        .max_consecutive_errors = 0,
        .pipeline_repo = "",
        .pipeline_test_cmd = pipeline_test_cmd,
        .pipeline_lint_cmd = "",
        .pipeline_admin_chat = "",
        .release_interval_mins = 0,
        .continuous_mode = false,
        .collection_window_ms = 0,
        .cooldown_ms = 0,
        .agent_timeout_s = 0,
        .max_concurrent_agents = 0,
        .rate_limit_per_minute = 0,
        .max_pipeline_agents = 0,
        .web_port = 0,
        .dashboard_dist_dir = "",
        .watched_repos = watched_repos,
        .whatsapp_enabled = false,
        .whatsapp_auth_dir = "",
        .discord_enabled = false,
        .discord_token = "",
        .allocator = std.testing.allocator,
    };
}

test "getTestCmdForRepo exact match returns repo-specific command" {
    // Include a non-matching entry before the matching one to verify the loop
    // does not short-circuit and that the first exact match wins.
    var repos = [_]RepoConfig{
        .{ .path = "/repos/other", .test_cmd = "go test ./...", .is_self = false },
        .{ .path = "/repos/myapp", .test_cmd = "npm test", .is_self = false },
    };
    var config = testMinimalConfig("zig build test", &repos);
    try std.testing.expectEqualStrings("npm test", config.getTestCmdForRepo("/repos/myapp"));
}

test "getTestCmdForRepo no match returns pipeline_test_cmd default" {
    // A path that starts with the same prefix as an existing entry must not match
    // (std.mem.eql is byte-exact, so "/repos/other" != "/repos/myapp").
    var repos = [_]RepoConfig{
        .{ .path = "/repos/myapp", .test_cmd = "npm test", .is_self = false },
    };
    var config = testMinimalConfig("zig build test", &repos);
    try std.testing.expectEqualStrings("zig build test", config.getTestCmdForRepo("/repos/other"));
}

test "getTestCmdForRepo empty watched_repos returns pipeline_test_cmd default" {
    var repos = [_]RepoConfig{};
    var config = testMinimalConfig("make test", &repos);
    try std.testing.expectEqualStrings("make test", config.getTestCmdForRepo("/any/path"));
}
