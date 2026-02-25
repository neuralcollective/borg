// Tests for the HOME fallback fix in Config.load (config.zig:70).
//
// These tests verify that:
//   - The hardcoded "/home/shulgin" literal is gone from config.zig (AC1).
//   - The fallback when HOME is unset is "/root" (AC2).
//   - An explicit HOME value is honoured by the credentials path (AC3).
//   - refreshOAuthToken is unaffected by the change (AC4).
//   - Edge cases for empty HOME and missing credentials file are handled (EC).
//
// To include in the build, add to config.zig inside a comptime block:
//   comptime { _ = @import("config_home_fallback_test.zig"); }
//
// AC1 and AC2 use @embedFile to inspect the source directly, so they fail
// immediately (before any implementation work) without requiring compilation
// of unimplemented APIs.
//
// AC3, AC4, and the edge-case tests use Config.initFromContent, which does not
// exist yet.  Those tests FAIL TO COMPILE until initFromContent is added to
// config.zig, satisfying the "tests fail before implementation" requirement.

const std = @import("std");
const config_mod = @import("config.zig");
const Config = config_mod.Config;

// =============================================================================
// AC1: The literal "/home/shulgin" is absent from config.zig
//
// Reads the source file at compile time and asserts the developer-specific
// path is not present.  This test FAILS before the fix is applied.
// =============================================================================

test "AC1: config.zig does not contain the hardcoded /home/shulgin path" {
    const src = @embedFile("config.zig");
    const found = std.mem.indexOf(u8, src, "/home/shulgin") != null;
    // If this fails: replace `orelse "/home/shulgin"` with `orelse "/root"` on
    // the line that reads `const home = std.posix.getenv("HOME") orelse ...`
    try std.testing.expect(!found);
}

// =============================================================================
// AC2: The fallback value is "/root"
//
// Reads the source file at compile time and asserts that the orelse fallback
// for the HOME lookup is specifically "/root".  This test FAILS before the fix.
// =============================================================================

test "AC2: config.zig uses /root as the HOME environment fallback" {
    const src = @embedFile("config.zig");
    // The corrected line must read: orelse "/root"
    const has_root_fallback = std.mem.indexOf(u8, src, "orelse \"/root\"") != null;
    try std.testing.expect(has_root_fallback);
}

// =============================================================================
// Structural: path formula is correct for /root
//
// Verifies that the credentials path formula produces the expected string for
// the fallback home directory, independently of the config loading path.
// This test compiles and passes even before the fix, documenting the expected
// formula that the fixed code must satisfy.
// =============================================================================

test "credentials path formula produces correct path for /root" {
    const alloc = std.testing.allocator;

    const path = try std.fmt.allocPrint(alloc, "{s}/.claude/.credentials.json", .{"/root"});
    defer alloc.free(path);

    try std.testing.expectEqualStrings("/root/.claude/.credentials.json", path);
}

test "credentials path formula produces correct path for an arbitrary HOME" {
    const alloc = std.testing.allocator;

    const path = try std.fmt.allocPrint(alloc, "{s}/.claude/.credentials.json", .{"/home/user"});
    defer alloc.free(path);

    try std.testing.expectEqualStrings("/home/user/.claude/.credentials.json", path);
}

// =============================================================================
// AC3: When HOME is set in the process environment, initFromContent derives
//      credentials_path as "$HOME/.claude/.credentials.json".
//
// This test FAILS TO COMPILE until Config.initFromContent is added to
// config.zig (signature: pub fn initFromContent(allocator, env_content) !Config).
// Once that function exists the test exercises the runtime path.
// =============================================================================

test "AC3: credentials_path uses the actual HOME env when HOME is set" {
    const home = std.posix.getenv("HOME") orelse {
        // HOME is not set in this process — skip rather than produce a
        // misleading failure; the source-level tests (AC1/AC2) cover this case.
        return;
    };
    if (home.len == 0) return; // empty HOME is a separate edge case

    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    // Use sentinel env content so no config keys interfere.
    const cfg = try Config.initFromContent(alloc, "BORG_HOME_FALLBACK_TEST=1");

    const expected = try std.fmt.allocPrint(alloc, "{s}/.claude/.credentials.json", .{home});

    try std.testing.expectEqualStrings(expected, cfg.credentials_path);
}

// =============================================================================
// AC4: refreshOAuthToken continues to use credentials_path correctly.
//
// Constructs a minimal Config with credentials_path pointing to a temp file
// that contains a valid OAuth token JSON, calls refreshOAuthToken, and
// verifies the token is updated.  The change to the HOME fallback must not
// break this code path.
//
// This test FAILS TO COMPILE until Config.initFromContent exists (used here
// to verify the full round-trip through a real Config value).
// =============================================================================

test "AC4: refreshOAuthToken reads token from credentials_path after HOME fix" {
    const alloc = std.testing.allocator;

    const creds_path = "/tmp/borg_home_fallback_ac4.json";

    // Write a temporary credentials file.
    {
        const f = try std.fs.cwd().createFile(creds_path, .{});
        defer f.close();
        try f.writeAll(
            \\{"claudeAiOauth":{"accessToken":"ac4-token-from-creds"}}
        );
    }
    defer std.fs.cwd().deleteFile(creds_path) catch {};

    // Build a minimal Config pointing at the temp credentials file.
    // initFromContent is used so we can control env_content.
    var arena = std.heap.ArenaAllocator.init(alloc);
    defer arena.deinit();
    const a = arena.allocator();

    var cfg = try Config.initFromContent(a, "BORG_HOME_FALLBACK_TEST=1");
    // Override credentials_path to the temp file we just wrote.
    cfg.credentials_path = creds_path;

    cfg.refreshOAuthToken();

    try std.testing.expectEqualStrings("ac4-token-from-creds", cfg.oauth_token);
}

// =============================================================================
// Edge case: HOME set to the empty string ("")
//
// std.posix.getenv("HOME") returns "" (non-null) when HOME="", so the fallback
// is NOT triggered and credentials_path becomes "/.claude/.credentials.json".
// This test documents the expected behaviour for this unusual case and verifies
// it is NOT treated as though HOME is absent.
//
// Fails to compile until Config.initFromContent exists.
// =============================================================================

test "EC1: HOME=empty string means fallback is not used; path starts with /" {
    const home_env = std.posix.getenv("HOME");
    if (home_env == null or home_env.?.len != 0) {
        // HOME is either unset or non-empty — can't meaningfully test the empty
        // string edge case without env mutation; skip.
        return;
    }

    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const cfg = try Config.initFromContent(arena.allocator(), "BORG_HOME_FALLBACK_TEST=1");

    // Empty HOME → path is "/.claude/.credentials.json", NOT "/root/..."
    try std.testing.expectEqualStrings("/.claude/.credentials.json", cfg.credentials_path);
}

// =============================================================================
// Edge case: credentials file absent at /root path → oauth falls back to env var
//
// When HOME is unset and the credentials file does not exist at the /root path,
// readOAuthToken returns null and the oauth_token is sourced from
// CLAUDE_CODE_OAUTH_TOKEN in the env content instead.
//
// Fails to compile until Config.initFromContent exists.
// =============================================================================

test "EC2: oauth_token falls back to env var when credentials file absent" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    // Provide the token only via env content; credentials file will not exist.
    const env =
        \\CLAUDE_CODE_OAUTH_TOKEN=env-token-fallback
    ;

    const cfg = try Config.initFromContent(alloc, env);

    // The credentials file at $HOME/.claude/.credentials.json will not exist
    // in this test environment (or will be unreadable), so oauth_token must
    // come from the env var.
    try std.testing.expectEqualStrings("env-token-fallback", cfg.oauth_token);
}

// =============================================================================
// Edge case: credentials_path is always a valid (non-empty) string
//
// After the fix, credentials_path must never be the empty string when
// Config.initFromContent completes successfully, because either HOME is set
// (giving a real path) or the /root fallback is used.
//
// Fails to compile until Config.initFromContent exists.
// =============================================================================

test "EC3: credentials_path is non-empty after initFromContent" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const cfg = try Config.initFromContent(arena.allocator(), "BORG_HOME_FALLBACK_TEST=1");

    try std.testing.expect(cfg.credentials_path.len > 0);
}

// =============================================================================
// Edge case: credentials_path ends with the expected suffix
//
// Regardless of what HOME is (or the /root fallback), the suffix must always
// be "/.claude/.credentials.json".
//
// Fails to compile until Config.initFromContent exists.
// =============================================================================

test "EC4: credentials_path always ends with /.claude/.credentials.json" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const cfg = try Config.initFromContent(arena.allocator(), "BORG_HOME_FALLBACK_TEST=1");

    const suffix = "/.claude/.credentials.json";
    try std.testing.expect(std.mem.endsWith(u8, cfg.credentials_path, suffix));
}
