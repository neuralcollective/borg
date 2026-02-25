// Tests for the memory leak fix in Config.refreshOAuthToken.
//
// These tests verify that refreshOAuthToken properly frees the previous
// heap-allocated oauth_token before replacing it, and does not attempt
// to free non-heap-allocated string literals.
//
// To include in the build, add to config.zig:
//   test { _ = @import("config_test.zig"); }
//
// All tests below should FAIL before the fix is applied because the
// Config struct lacks the oauth_token_owned field (compile error).

const std = @import("std");
const config_mod = @import("config.zig");
const Config = config_mod.Config;

/// Create a minimal Config suitable for unit testing.
fn testConfig(allocator: std.mem.Allocator, credentials_path: []const u8, oauth_token: []const u8, owned: bool) Config {
    return Config{
        .telegram_token = "",
        .oauth_token = oauth_token,
        .oauth_token_owned = owned,
        .assistant_name = "",
        .trigger_pattern = "",
        .data_dir = "",
        .container_image = "",
        .model = "",
        .credentials_path = credentials_path,
        .session_max_age_hours = 0,
        .max_consecutive_errors = 0,
        .pipeline_repo = "",
        .pipeline_test_cmd = "",
        .pipeline_lint_cmd = "",
        .pipeline_admin_chat = "",
        .release_interval_mins = 0,
        .continuous_mode = false,
        .chat_collection_window_ms = 0,
        .chat_cooldown_ms = 0,
        .agent_timeout_s = 0,
        .max_chat_agents = 0,
        .chat_rate_limit = 0,
        .pipeline_max_agents = 0,
        .web_port = 0,
        .dashboard_dist_dir = "",
        .watched_repos = &.{},
        .whatsapp_enabled = false,
        .whatsapp_auth_dir = "",
        .discord_enabled = false,
        .discord_token = "",
        .allocator = allocator,
    };
}

/// Write a minimal Claude credentials JSON file with the given access token.
fn writeTempCredentials(path: []const u8, token: []const u8) !void {
    const file = try std.fs.cwd().createFile(path, .{});
    defer file.close();
    try file.writer().print(
        \\{{"claudeAiOauth":{{"accessToken":"{s}"}}}}
    , .{token});
}

fn deleteTempFile(path: []const u8) void {
    std.fs.cwd().deleteFile(path) catch {};
}

// =============================================================================
// AC1: Old token is freed
// After refreshOAuthToken is called with a new token available, the previous
// oauth_token memory is freed. Verifiable by std.testing.allocator leak check.
// =============================================================================

test "AC1: refreshOAuthToken frees the previous heap-allocated token" {
    const alloc = std.testing.allocator;
    const creds_path = "/tmp/borg_test_ac1.json";
    defer deleteTempFile(creds_path);
    try writeTempCredentials(creds_path, "fresh-token-ac1");

    // Allocate an initial token on the heap
    const old_token = try alloc.dupe(u8, "old-heap-token");
    // Do NOT defer free — refreshOAuthToken must free it

    var cfg = testConfig(alloc, creds_path, old_token, true);

    cfg.refreshOAuthToken();

    // New token should be assigned
    try std.testing.expectEqualStrings("fresh-token-ac1", cfg.oauth_token);
    try std.testing.expect(cfg.oauth_token_owned == true);

    // Clean up the new token (allocated by readOAuthToken)
    alloc.free(@constCast(cfg.oauth_token));
    // If refreshOAuthToken did NOT free old_token, std.testing.allocator
    // will report a leak and fail this test.
}

// =============================================================================
// AC2: No double-free on literal
// When oauth_token is "" (string literal, not heap-allocated) and
// oauth_token_owned=false, refreshOAuthToken must not free the literal.
// =============================================================================

test "AC2: refreshOAuthToken does not free string literal when oauth_token_owned is false" {
    const alloc = std.testing.allocator;
    const creds_path = "/tmp/borg_test_ac2.json";
    defer deleteTempFile(creds_path);
    try writeTempCredentials(creds_path, "new-token-ac2");

    // Initial token is a string literal — NOT heap-allocated
    var cfg = testConfig(alloc, creds_path, "", false);

    // Must NOT attempt to free the empty string literal
    cfg.refreshOAuthToken();

    try std.testing.expectEqualStrings("new-token-ac2", cfg.oauth_token);
    try std.testing.expect(cfg.oauth_token_owned == true);

    // Clean up the new heap-allocated token
    alloc.free(@constCast(cfg.oauth_token));
}

// =============================================================================
// AC3: No use-after-free
// The old token pointer is not accessed after being freed. The assignment
// self.oauth_token = new_token happens after the free. Verified by checking
// the new token is readable and correct after refresh.
// =============================================================================

test "AC3: new token value is correct after refresh (no use-after-free)" {
    const alloc = std.testing.allocator;
    const creds_path = "/tmp/borg_test_ac3.json";
    defer deleteTempFile(creds_path);
    try writeTempCredentials(creds_path, "correct-new-token");

    const old_token = try alloc.dupe(u8, "will-be-freed");

    var cfg = testConfig(alloc, creds_path, old_token, true);

    cfg.refreshOAuthToken();

    // Verify new token is readable and correct (not corrupted)
    try std.testing.expectEqualStrings("correct-new-token", cfg.oauth_token);

    alloc.free(@constCast(cfg.oauth_token));
}

// =============================================================================
// AC4: No-op when token unchanged
// If readOAuthToken returns null (credentials file missing or unreadable),
// oauth_token is not modified and no free occurs.
// =============================================================================

test "AC4: refreshOAuthToken is no-op when credentials file is missing" {
    const alloc = std.testing.allocator;

    const original_token = try alloc.dupe(u8, "keep-this-token");

    var cfg = testConfig(alloc, "/tmp/borg_test_nonexistent_file.json", original_token, true);

    cfg.refreshOAuthToken();

    // Token should remain unchanged
    try std.testing.expectEqualStrings("keep-this-token", cfg.oauth_token);
    try std.testing.expect(cfg.oauth_token_owned == true);

    // Clean up — we still own the token since refresh was a no-op
    alloc.free(@constCast(cfg.oauth_token));
}

test "AC4b: no-op refresh with literal token and missing file" {
    const alloc = std.testing.allocator;

    // Literal token, file missing — must not free literal, must not change token
    var cfg = testConfig(alloc, "/tmp/borg_test_nonexistent2.json", "", false);

    cfg.refreshOAuthToken();

    try std.testing.expectEqualStrings("", cfg.oauth_token);
    try std.testing.expect(cfg.oauth_token_owned == false);
}

// =============================================================================
// AC5: Full refresh cycle with std.testing.allocator detects no leaks
// Construct Config with heap-allocated token, call refresh multiple times,
// verify no leak reported.
// =============================================================================

test "AC5: full refresh cycle with std.testing.allocator detects no leaks" {
    const alloc = std.testing.allocator;
    const creds_path = "/tmp/borg_test_ac5.json";
    defer deleteTempFile(creds_path);
    try writeTempCredentials(creds_path, "token-round1");

    // Start with a heap-allocated token
    const initial = try alloc.dupe(u8, "initial-token");

    var cfg = testConfig(alloc, creds_path, initial, true);

    // First refresh: frees "initial-token", sets "token-round1"
    cfg.refreshOAuthToken();
    try std.testing.expectEqualStrings("token-round1", cfg.oauth_token);

    // Write a new token for second refresh
    try writeTempCredentials(creds_path, "token-round2");

    // Second refresh: frees "token-round1", sets "token-round2"
    cfg.refreshOAuthToken();
    try std.testing.expectEqualStrings("token-round2", cfg.oauth_token);

    // Clean up final token
    alloc.free(@constCast(cfg.oauth_token));
    // std.testing.allocator ensures all intermediate allocations are freed.
}

// =============================================================================
// Edge Case 1: Initial token is empty literal ""
// First call to refreshOAuthToken must not attempt to free the empty string.
// Handled by oauth_token_owned = false at init.
// =============================================================================

test "Edge1: first refresh with empty literal token does not free literal" {
    const alloc = std.testing.allocator;
    const creds_path = "/tmp/borg_test_edge1.json";
    defer deleteTempFile(creds_path);
    try writeTempCredentials(creds_path, "first-real-token");

    // Empty string literal — NOT heap allocated
    var cfg = testConfig(alloc, creds_path, "", false);

    cfg.refreshOAuthToken();

    try std.testing.expectEqualStrings("first-real-token", cfg.oauth_token);
    try std.testing.expect(cfg.oauth_token_owned == true);

    alloc.free(@constCast(cfg.oauth_token));
}

// =============================================================================
// Edge Case 2: Initial token from heap allocation (getEnv/readOAuthToken)
// oauth_token_owned must be true so the first refresh frees it.
// =============================================================================

test "Edge2: heap-allocated initial token is freed on first refresh" {
    const alloc = std.testing.allocator;
    const creds_path = "/tmp/borg_test_edge2.json";
    defer deleteTempFile(creds_path);
    try writeTempCredentials(creds_path, "replacement-token");

    const heap_token = try alloc.dupe(u8, "heap-initial-from-getenv");

    var cfg = testConfig(alloc, creds_path, heap_token, true);

    cfg.refreshOAuthToken();

    try std.testing.expectEqualStrings("replacement-token", cfg.oauth_token);

    alloc.free(@constCast(cfg.oauth_token));
    // Leak detection ensures heap_token was freed by refreshOAuthToken.
}

// =============================================================================
// Edge Case 3: Credentials file missing or invalid
// readOAuthToken returns null. refreshOAuthToken must be a no-op.
// =============================================================================

test "Edge3: invalid credentials JSON causes no-op refresh" {
    const alloc = std.testing.allocator;
    const creds_path = "/tmp/borg_test_edge3.json";
    defer deleteTempFile(creds_path);

    // Write invalid JSON
    {
        const file = try std.fs.cwd().createFile(creds_path, .{});
        defer file.close();
        try file.writeAll("this is not json");
    }

    const token = try alloc.dupe(u8, "should-not-change");

    var cfg = testConfig(alloc, creds_path, token, true);

    cfg.refreshOAuthToken();

    try std.testing.expectEqualStrings("should-not-change", cfg.oauth_token);

    alloc.free(@constCast(cfg.oauth_token));
}

test "Edge3b: credentials JSON missing accessToken field causes no-op refresh" {
    const alloc = std.testing.allocator;
    const creds_path = "/tmp/borg_test_edge3b.json";
    defer deleteTempFile(creds_path);

    // Valid JSON but missing the expected token field
    {
        const file = try std.fs.cwd().createFile(creds_path, .{});
        defer file.close();
        try file.writeAll(
            \\{"claudeAiOauth":{"other":"field"}}
        );
    }

    const token = try alloc.dupe(u8, "unchanged-token");

    var cfg = testConfig(alloc, creds_path, token, true);

    cfg.refreshOAuthToken();

    try std.testing.expectEqualStrings("unchanged-token", cfg.oauth_token);

    alloc.free(@constCast(cfg.oauth_token));
}

// =============================================================================
// Edge Case 4: Credentials file returns same token value
// Even if content is identical, readOAuthToken allocates a new copy each call.
// The old copy must still be freed.
// =============================================================================

test "Edge4: refresh with same token value still frees old allocation" {
    const alloc = std.testing.allocator;
    const creds_path = "/tmp/borg_test_edge4.json";
    defer deleteTempFile(creds_path);
    try writeTempCredentials(creds_path, "same-value");

    const old_token = try alloc.dupe(u8, "same-value");

    var cfg = testConfig(alloc, creds_path, old_token, true);

    cfg.refreshOAuthToken();

    // Content is the same but it must be a fresh allocation
    try std.testing.expectEqualStrings("same-value", cfg.oauth_token);

    alloc.free(@constCast(cfg.oauth_token));
    // old_token must have been freed by refreshOAuthToken — leak detector verifies.
}

// =============================================================================
// Edge Case 6: Allocator failure / nonexistent file
// readOAuthToken returns null, so refreshOAuthToken is a no-op.
// =============================================================================

test "Edge6: nonexistent credentials file makes refresh a no-op" {
    const alloc = std.testing.allocator;

    const token = try alloc.dupe(u8, "preserved-token");

    var cfg = testConfig(alloc, "/tmp/borg_no_such_file_ever.json", token, true);

    cfg.refreshOAuthToken();

    try std.testing.expectEqualStrings("preserved-token", cfg.oauth_token);
    try std.testing.expect(cfg.oauth_token_owned == true);

    alloc.free(@constCast(cfg.oauth_token));
}

// =============================================================================
// Structural: Config struct has oauth_token_owned field
// =============================================================================

test "Config struct has oauth_token_owned field of type bool" {
    const alloc = std.testing.allocator;
    var cfg = testConfig(alloc, "", "", false);
    try std.testing.expect(@TypeOf(cfg.oauth_token_owned) == bool);
    cfg.oauth_token_owned = true;
    try std.testing.expect(cfg.oauth_token_owned == true);
}

// =============================================================================
// Multiple consecutive refreshes without leak
// Simulates the real-world pattern where refreshOAuthToken is called
// repeatedly (every ~500ms in the main loop).
// =============================================================================

test "multiple consecutive refreshes do not leak memory" {
    const alloc = std.testing.allocator;
    const creds_path = "/tmp/borg_test_multi.json";
    defer deleteTempFile(creds_path);

    // Start with a literal (not owned)
    var cfg = testConfig(alloc, creds_path, "", false);

    // Refresh 5 times with different tokens
    var i: usize = 0;
    while (i < 5) : (i += 1) {
        var buf: [32]u8 = undefined;
        const token_val = std.fmt.bufPrint(&buf, "token-{d}", .{i}) catch unreachable;
        try writeTempCredentials(creds_path, token_val);
        cfg.refreshOAuthToken();
    }

    // Final token should be "token-4"
    try std.testing.expectEqualStrings("token-4", cfg.oauth_token);

    // Free the last token
    if (cfg.oauth_token_owned) {
        alloc.free(@constCast(cfg.oauth_token));
    }
    // All intermediate tokens must have been freed by refreshOAuthToken.
}

// =============================================================================
// Transition from unowned to owned across multiple refreshes
// =============================================================================

test "oauth_token_owned transitions from false to true on first successful refresh" {
    const alloc = std.testing.allocator;
    const creds_path = "/tmp/borg_test_transition.json";
    defer deleteTempFile(creds_path);

    var cfg = testConfig(alloc, creds_path, "", false);

    // Before any refresh: not owned
    try std.testing.expect(cfg.oauth_token_owned == false);

    // Refresh with missing file — should remain unowned
    cfg.refreshOAuthToken();
    try std.testing.expect(cfg.oauth_token_owned == false);
    try std.testing.expectEqualStrings("", cfg.oauth_token);

    // Now create the credentials file
    try writeTempCredentials(creds_path, "first-token");
    cfg.refreshOAuthToken();

    // Should now be owned
    try std.testing.expect(cfg.oauth_token_owned == true);
    try std.testing.expectEqualStrings("first-token", cfg.oauth_token);

    // Second refresh with new token — still owned, old freed
    try writeTempCredentials(creds_path, "second-token");
    cfg.refreshOAuthToken();
    try std.testing.expect(cfg.oauth_token_owned == true);
    try std.testing.expectEqualStrings("second-token", cfg.oauth_token);

    alloc.free(@constCast(cfg.oauth_token));
}
