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
    allocator: std.mem.Allocator,

    pub fn load(allocator: std.mem.Allocator) !Config {
        const env_content = std.fs.cwd().readFileAlloc(allocator, ".env", 8192) catch "";

        // Try reading OAuth token from Claude credentials file (it rotates)
        const home = std.posix.getenv("HOME") orelse "/home/shulgin";
        const creds_path = try std.fmt.allocPrint(allocator, "{s}/.claude/.credentials.json", .{home});
        const oauth = readOAuthToken(allocator, creds_path) orelse
            getEnv(allocator, env_content, "CLAUDE_CODE_OAUTH_TOKEN") orelse "";

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
