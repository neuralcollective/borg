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
    const watched = getEnv(allocator, env_content, "WATCHED_REPOS") orelse "";
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

// ── getEnv Tests ───────────────────────────────────────────────────────

// AC1: Basic KEY=VALUE parsing via getEnv
test "getEnv basic KEY=VALUE parsing returns correct value" {
    const alloc = std.testing.allocator;
    const result = getEnv(alloc, "MY_KEY=my_value", "MY_KEY");
    defer if (result) |v| alloc.free(v);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("my_value", result.?);
}

test "getEnv returns null for nonexistent key" {
    const alloc = std.testing.allocator;
    const result = getEnv(alloc, "MY_KEY=my_value", "_BORG_TEST_MISSING_KEY_42");
    defer if (result) |v| alloc.free(v);
    try std.testing.expect(result == null);
}

test "getEnv returned slice is allocator-owned and freeable" {
    const alloc = std.testing.allocator;
    // std.testing.allocator detects leaks; if we can free without error, ownership is correct
    const result = getEnv(alloc, "OWNED=test_value", "OWNED");
    try std.testing.expect(result != null);
    alloc.free(result.?);
}

// AC2: Skipping # comment lines
test "getEnv skips comment lines" {
    const alloc = std.testing.allocator;
    const env =
        \\# this is a comment
        \\VISIBLE=found
        \\# SECRET=hidden
    ;

    const v1 = getEnv(alloc, env, "VISIBLE");
    defer if (v1) |v| alloc.free(v);
    try std.testing.expect(v1 != null);
    try std.testing.expectEqualStrings("found", v1.?);

    // Key that only appears inside a comment must not be parseable
    const v2 = getEnv(alloc, env, "SECRET");
    defer if (v2) |v| alloc.free(v);
    try std.testing.expect(v2 == null);
}

test "getEnv does not parse commented-out key-value pairs" {
    const alloc = std.testing.allocator;
    const env = "# SECRET=hidden";

    const result = getEnv(alloc, env, "SECRET");
    defer if (result) |v| alloc.free(v);
    try std.testing.expect(result == null);
}

// AC3: Skipping blank lines
test "getEnv skips blank and whitespace-only lines" {
    const alloc = std.testing.allocator;
    const env =
        \\FIRST=one
        \\
        \\
        \\SECOND=two
    ;

    const v1 = getEnv(alloc, env, "FIRST");
    defer if (v1) |v| alloc.free(v);
    try std.testing.expect(v1 != null);
    try std.testing.expectEqualStrings("one", v1.?);

    const v2 = getEnv(alloc, env, "SECOND");
    defer if (v2) |v| alloc.free(v);
    try std.testing.expect(v2 != null);
    try std.testing.expectEqualStrings("two", v2.?);
}

// AC4: Values containing = characters
test "getEnv handles values containing equals signs" {
    const alloc = std.testing.allocator;
    const result = getEnv(alloc, "TOKEN=abc=def=ghi", "TOKEN");
    defer if (result) |v| alloc.free(v);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("abc=def=ghi", result.?);
}

// AC5: Process environment fallback
test "getEnv falls back to process environment when key absent from env_content" {
    const alloc = std.testing.allocator;
    // PATH is virtually always set in process environment
    const result = getEnv(alloc, "", "PATH");
    defer if (result) |v| alloc.free(v);

    // Verify it returns the same value as std.posix.getenv
    const expected = std.posix.getenv("PATH");
    if (expected) |exp| {
        try std.testing.expect(result != null);
        try std.testing.expectEqualStrings(exp, result.?);
    }
}

test "getEnv env_content value takes precedence over process environment" {
    const alloc = std.testing.allocator;
    // PATH exists in process env; override it via env_content
    const result = getEnv(alloc, "PATH=/custom/override/path", "PATH");
    defer if (result) |v| alloc.free(v);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("/custom/override/path", result.?);
}

// ── Edge Case Tests ────────────────────────────────────────────────────

// Edge case 1: Value with leading/trailing =
test "getEnv handles value with leading and trailing equals" {
    const alloc = std.testing.allocator;
    const result = getEnv(alloc, "KEY==value=", "KEY");
    defer if (result) |v| alloc.free(v);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("=value=", result.?);
}

// Edge case 2: Key with empty value
test "getEnv returns empty string for key with empty value" {
    const alloc = std.testing.allocator;
    const result = getEnv(alloc, "EMPTY_VAL=", "EMPTY_VAL");
    defer if (result) |v| alloc.free(v);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("", result.?);
}

// Edge case 3: Whitespace around key and =
test "getEnv trims whitespace around key and value" {
    const alloc = std.testing.allocator;
    const result = getEnv(alloc, "  MY_KEY  =  my_value  ", "MY_KEY");
    defer if (result) |v| alloc.free(v);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("my_value", result.?);
}

// Edge case 4: Quoted values containing =
test "getEnv strips quotes and preserves equals in quoted value" {
    const alloc = std.testing.allocator;
    const result = getEnv(alloc, "QUOTED=\"abc=def\"", "QUOTED");
    defer if (result) |v| alloc.free(v);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("abc=def", result.?);
}

// Edge case 5: Process env fallback returns allocator-owned memory
test "getEnv process env fallback returns freeable allocator-owned memory" {
    const alloc = std.testing.allocator;
    // PATH is virtually always set; fetch via fallback (empty env_content)
    const result = getEnv(alloc, "", "PATH");
    if (result) |v| {
        // If this doesn't panic/leak, std.testing.allocator confirms ownership
        alloc.free(v);
    }
}

// Edge case 6: Multiple entries with independent lookup
test "getEnv handles multiple entries with independent lookups" {
    const alloc = std.testing.allocator;
    const env =
        \\HOST=localhost
        \\PORT=8080
        \\DB_NAME=mydb
    ;

    const v1 = getEnv(alloc, env, "HOST");
    defer if (v1) |v| alloc.free(v);
    try std.testing.expect(v1 != null);
    try std.testing.expectEqualStrings("localhost", v1.?);

    const v2 = getEnv(alloc, env, "PORT");
    defer if (v2) |v| alloc.free(v);
    try std.testing.expect(v2 != null);
    try std.testing.expectEqualStrings("8080", v2.?);

    const v3 = getEnv(alloc, env, "DB_NAME");
    defer if (v3) |v| alloc.free(v);
    try std.testing.expect(v3 != null);
    try std.testing.expectEqualStrings("mydb", v3.?);
}

// Combined: comments, blanks, and valid entries
test "getEnv parses correctly with mixed comments blanks and entries" {
    const alloc = std.testing.allocator;
    const env =
        \\# Database settings
        \\DB_HOST=localhost
        \\
        \\# DB_PASSWORD=secret
        \\DB_PORT=5432
        \\
    ;

    const v1 = getEnv(alloc, env, "DB_HOST");
    defer if (v1) |v| alloc.free(v);
    try std.testing.expect(v1 != null);
    try std.testing.expectEqualStrings("localhost", v1.?);

    const v2 = getEnv(alloc, env, "DB_PORT");
    defer if (v2) |v| alloc.free(v);
    try std.testing.expect(v2 != null);
    try std.testing.expectEqualStrings("5432", v2.?);

    // DB_PASSWORD only appears in a comment
    const v3 = getEnv(alloc, env, "DB_PASSWORD");
    defer if (v3) |v| alloc.free(v);
    try std.testing.expect(v3 == null);
}
