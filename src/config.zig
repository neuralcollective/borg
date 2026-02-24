const std = @import("std");

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
    // Agent lifecycle
    collection_window_ms: i64,
    cooldown_ms: i64,
    agent_timeout_s: i64,
    max_concurrent_agents: u32,
    rate_limit_per_minute: u32,
    // WhatsApp config
    whatsapp_enabled: bool,
    whatsapp_auth_dir: []const u8,
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

        return Config{
            .telegram_token = getEnv(allocator, env_content, "TELEGRAM_BOT_TOKEN") orelse "",
            .oauth_token = oauth,
            .assistant_name = getEnv(allocator, env_content, "ASSISTANT_NAME") orelse "Borg",
            .trigger_pattern = getEnv(allocator, env_content, "TRIGGER_PATTERN") orelse "@Borg",
            .data_dir = getEnv(allocator, env_content, "DATA_DIR") orelse "data",
            .container_image = getEnv(allocator, env_content, "CONTAINER_IMAGE") orelse "borg-agent:latest",
            .model = getEnv(allocator, env_content, "CLAUDE_MODEL") orelse "claude-opus-4-6",
            .credentials_path = creds_path,
            .session_max_age_hours = 4,
            .max_consecutive_errors = 3,
            .pipeline_repo = getEnv(allocator, env_content, "PIPELINE_REPO") orelse "",
            .pipeline_test_cmd = getEnv(allocator, env_content, "PIPELINE_TEST_CMD") orelse "zig build test",
            .pipeline_lint_cmd = getEnv(allocator, env_content, "PIPELINE_LINT_CMD") orelse "",
            .pipeline_admin_chat = getEnv(allocator, env_content, "PIPELINE_ADMIN_CHAT") orelse "",
            .release_interval_mins = release_mins,
            .collection_window_ms = std.fmt.parseInt(i64, collection_ms_str, 10) catch 3000,
            .cooldown_ms = std.fmt.parseInt(i64, cooldown_ms_str, 10) catch 5000,
            .agent_timeout_s = std.fmt.parseInt(i64, timeout_s_str, 10) catch 600,
            .max_concurrent_agents = std.fmt.parseInt(u32, max_agents_str, 10) catch 4,
            .rate_limit_per_minute = std.fmt.parseInt(u32, rate_limit_str, 10) catch 5,
            .whatsapp_enabled = std.mem.eql(u8, getEnv(allocator, env_content, "WHATSAPP_ENABLED") orelse "false", "true"),
            .whatsapp_auth_dir = getEnv(allocator, env_content, "WHATSAPP_AUTH_DIR") orelse "whatsapp/auth",
            .allocator = allocator,
        };
    }

    /// Re-read OAuth token from credentials file (handles token rotation)
    pub fn refreshOAuthToken(self: *Config) void {
        if (readOAuthToken(self.allocator, self.credentials_path)) |new_token| {
            self.oauth_token = new_token;
        }
    }
};

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
