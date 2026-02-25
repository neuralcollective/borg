// Tests for Config.initFromContent – an env-content-driven constructor extracted
// from Config.load() by the implementation step.
//
// ALL tests here fail to compile until config.zig gains:
//   pub fn initFromContent(allocator: std.mem.Allocator, env_content: []const u8) !Config
//
// To wire this file into the build, add to config.zig:
//   test { _ = @import("config_init_test.zig"); }
//
// Test isolation: no process-environment variables are mutated.  All env values
// are supplied through the env_content string; keys absent from that string may
// fall back to the process environment, so tests that assert defaults use key
// names that are never set in a normal CI environment.

const std = @import("std");
const config_mod = @import("config.zig");
const Config = config_mod.Config;

// ── Boolean fields ─────────────────────────────────────────────────────────

// AC-B1: PIPELINE_AUTO_MERGE=false → primary repo auto_merge=false
test "AC-B1: PIPELINE_AUTO_MERGE=false sets primary repo auto_merge=false" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_REPO=/primary
        \\PIPELINE_TEST_CMD=zig build test
        \\PIPELINE_AUTO_MERGE=false
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expect(cfg.watched_repos.len == 1);
    try std.testing.expectEqualStrings("/primary", cfg.watched_repos[0].path);
    try std.testing.expect(cfg.watched_repos[0].is_self == true);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == false);
}

// AC-B2: PIPELINE_AUTO_MERGE absent → primary repo auto_merge=true (default)
test "AC-B2: PIPELINE_AUTO_MERGE absent defaults to auto_merge=true on primary" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_REPO=/primary
        \\PIPELINE_TEST_CMD=zig build test
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expect(cfg.watched_repos.len == 1);
    try std.testing.expectEqualStrings("/primary", cfg.watched_repos[0].path);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == true);
}

// AC-B3: PIPELINE_AUTO_MERGE=true (explicit) → primary repo auto_merge=true
test "AC-B3: PIPELINE_AUTO_MERGE=true (explicit) preserves auto_merge=true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_REPO=/primary
        \\PIPELINE_TEST_CMD=zig build test
        \\PIPELINE_AUTO_MERGE=true
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expect(cfg.watched_repos.len == 1);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == true);
}

// AC-B4: CONTINUOUS_MODE=true → config.continuous_mode=true
test "AC-B4: CONTINUOUS_MODE=true sets continuous_mode=true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\CONTINUOUS_MODE=true
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expect(cfg.continuous_mode == true);
}

// AC-B5: CONTINUOUS_MODE absent → config.continuous_mode=false
test "AC-B5: CONTINUOUS_MODE absent defaults to false" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    // Use a distinct dummy key so the parser has non-empty content but
    // CONTINUOUS_MODE is genuinely absent from env_content.
    const env =
        \\BORG_INIT_TEST_DUMMY=1
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expect(cfg.continuous_mode == false);
}

// AC-B6: WHATSAPP_ENABLED=true → config.whatsapp_enabled=true
test "AC-B6: WHATSAPP_ENABLED=true sets whatsapp_enabled=true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\WHATSAPP_ENABLED=true
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expect(cfg.whatsapp_enabled == true);
}

// AC-B7: DISCORD_ENABLED=true → config.discord_enabled=true
test "AC-B7: DISCORD_ENABLED=true sets discord_enabled=true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\DISCORD_ENABLED=true
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expect(cfg.discord_enabled == true);
}

// ── Numeric fields ─────────────────────────────────────────────────────────

// AC-N1: PIPELINE_MAX_BACKLOG=10 → pipeline_max_backlog=10
test "AC-N1: PIPELINE_MAX_BACKLOG=10 sets pipeline_max_backlog=10" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_MAX_BACKLOG=10
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(u32, 10), cfg.pipeline_max_backlog);
}

// AC-N2: PIPELINE_MAX_BACKLOG absent → pipeline_max_backlog=5 (default)
test "AC-N2: PIPELINE_MAX_BACKLOG absent defaults to 5" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\BORG_INIT_TEST_DUMMY=1
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(u32, 5), cfg.pipeline_max_backlog);
}

// AC-N3: CONTAINER_MEMORY_MB=2048 → container_memory_mb=2048
test "AC-N3: CONTAINER_MEMORY_MB=2048 sets container_memory_mb=2048" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\CONTAINER_MEMORY_MB=2048
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(u64, 2048), cfg.container_memory_mb);
}

// AC-N4: WEB_PORT=8080 → web_port=8080
test "AC-N4: WEB_PORT=8080 sets web_port=8080" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\WEB_PORT=8080
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(u16, 8080), cfg.web_port);
}

// AC-N5: WEB_PORT absent → web_port=3131 (default)
test "AC-N5: WEB_PORT absent defaults to 3131" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\BORG_INIT_TEST_DUMMY=1
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(u16, 3131), cfg.web_port);
}

// AC-N6: PIPELINE_TICK_S=60 → pipeline_tick_s=60
test "AC-N6: PIPELINE_TICK_S=60 sets pipeline_tick_s=60" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_TICK_S=60
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(u64, 60), cfg.pipeline_tick_s);
}

// AC-N7: PIPELINE_SEED_COOLDOWN_S=7200 → pipeline_seed_cooldown_s=7200
test "AC-N7: PIPELINE_SEED_COOLDOWN_S=7200 sets pipeline_seed_cooldown_s=7200" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_SEED_COOLDOWN_S=7200
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(i64, 7200), cfg.pipeline_seed_cooldown_s);
}

// AC-N8: Invalid numeric (PIPELINE_MAX_BACKLOG=abc) falls back to default 5
test "AC-N8: invalid PIPELINE_MAX_BACKLOG value falls back to default 5" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_MAX_BACKLOG=abc
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(u32, 5), cfg.pipeline_max_backlog);
}

// ── WATCHED_REPOS end-to-end ───────────────────────────────────────────────

// AC-W1: !manual suffix sets auto_merge=false and strips suffix from test_cmd
test "AC-W1: WATCHED_REPOS !manual suffix disables auto_merge and strips suffix" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_REPO=/other
        \\PIPELINE_TEST_CMD=zig test
        \\WATCHED_REPOS=/repo/a:npm test!manual
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    // Should have primary + 1 watched
    try std.testing.expect(cfg.watched_repos.len == 2);
    const watched = cfg.watched_repos[1];
    try std.testing.expectEqualStrings("/repo/a", watched.path);
    try std.testing.expectEqualStrings("npm test", watched.test_cmd);
    try std.testing.expect(watched.auto_merge == false);
    try std.testing.expect(watched.is_self == false);
}

// AC-W2: No !manual → auto_merge=true, test_cmd preserved
test "AC-W2: WATCHED_REPOS without !manual has auto_merge=true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_REPO=/other
        \\PIPELINE_TEST_CMD=zig test
        \\WATCHED_REPOS=/repo/b:go test ./...
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expect(cfg.watched_repos.len == 2);
    const watched = cfg.watched_repos[1];
    try std.testing.expectEqualStrings("/repo/b", watched.path);
    try std.testing.expectEqualStrings("go test ./...", watched.test_cmd);
    try std.testing.expect(watched.auto_merge == true);
}

// AC-W3: PIPELINE_AUTO_MERGE=false + mixed !manual entries
test "AC-W3: PIPELINE_AUTO_MERGE=false with mixed WATCHED_REPOS !manual entries" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_REPO=/primary
        \\PIPELINE_TEST_CMD=zig test
        \\PIPELINE_AUTO_MERGE=false
        \\WATCHED_REPOS=/second:cmd2!manual|/third:cmd3
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    // primary + second + third = 3
    try std.testing.expect(cfg.watched_repos.len == 3);

    // Primary: auto_merge=false (from PIPELINE_AUTO_MERGE=false)
    try std.testing.expectEqualStrings("/primary", cfg.watched_repos[0].path);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == false);
    try std.testing.expect(cfg.watched_repos[0].is_self == true);

    // Second: auto_merge=false (from !manual)
    try std.testing.expectEqualStrings("/second", cfg.watched_repos[1].path);
    try std.testing.expectEqualStrings("cmd2", cfg.watched_repos[1].test_cmd);
    try std.testing.expect(cfg.watched_repos[1].auto_merge == false);

    // Third: auto_merge=true (no !manual)
    try std.testing.expectEqualStrings("/third", cfg.watched_repos[2].path);
    try std.testing.expectEqualStrings("cmd3", cfg.watched_repos[2].test_cmd);
    try std.testing.expect(cfg.watched_repos[2].auto_merge == true);
}

// AC-W4: WATCHED_REPOS absent + PIPELINE_REPO set → one entry, is_self=true
test "AC-W4: WATCHED_REPOS absent with PIPELINE_REPO yields single primary entry" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_REPO=/primary
        \\PIPELINE_TEST_CMD=zig test
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(usize, 1), cfg.watched_repos.len);
    try std.testing.expectEqualStrings("/primary", cfg.watched_repos[0].path);
    try std.testing.expect(cfg.watched_repos[0].is_self == true);
}

// AC-W5: PIPELINE_REPO absent + WATCHED_REPOS=/a:cmd → no primary prepended
test "AC-W5: empty PIPELINE_REPO with WATCHED_REPOS yields only watched entry" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\WATCHED_REPOS=/a:cmd
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(usize, 1), cfg.watched_repos.len);
    try std.testing.expectEqualStrings("/a", cfg.watched_repos[0].path);
    try std.testing.expectEqualStrings("cmd", cfg.watched_repos[0].test_cmd);
    try std.testing.expect(cfg.watched_repos[0].is_self == false);
}

// ── Defaults for missing keys ──────────────────────────────────────────────

// AC-D1: empty env-content → documented defaults for all tuning fields
test "AC-D1: empty env-content produces documented default values" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    // Use a key that is certainly absent from any CI environment to ensure
    // the numeric/bool keys under test are not present in env_content.
    const env =
        \\BORG_INIT_TEST_DUMMY=1
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(u32, 5), cfg.pipeline_max_backlog);
    try std.testing.expectEqual(@as(u64, 1024), cfg.container_memory_mb);
    try std.testing.expectEqual(@as(u16, 3131), cfg.web_port);
    try std.testing.expectEqual(@as(u64, 30), cfg.pipeline_tick_s);
    try std.testing.expectEqual(@as(i64, 3600), cfg.pipeline_seed_cooldown_s);
    try std.testing.expectEqual(@as(i64, 300), cfg.remote_check_interval_s);
    try std.testing.expect(cfg.continuous_mode == false);
    try std.testing.expect(cfg.whatsapp_enabled == false);
    try std.testing.expect(cfg.discord_enabled == false);
}

// ── Edge cases ─────────────────────────────────────────────────────────────

// EC-1: Case-sensitive booleans – only exact "false" disables auto_merge;
//       only exact "true" enables continuous_mode / whatsapp / discord.
test "EC-1a: PIPELINE_AUTO_MERGE=True (not exact false) keeps auto_merge=true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_REPO=/primary
        \\PIPELINE_TEST_CMD=zig test
        \\PIPELINE_AUTO_MERGE=True
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expect(cfg.watched_repos.len == 1);
    // "True" != "false" so auto_merge must be true
    try std.testing.expect(cfg.watched_repos[0].auto_merge == true);
}

test "EC-1b: PIPELINE_AUTO_MERGE=FALSE (uppercase) keeps auto_merge=true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_REPO=/primary
        \\PIPELINE_TEST_CMD=zig test
        \\PIPELINE_AUTO_MERGE=FALSE
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expect(cfg.watched_repos.len == 1);
    // "FALSE" != "false" so auto_merge must be true
    try std.testing.expect(cfg.watched_repos[0].auto_merge == true);
}

test "EC-1c: PIPELINE_AUTO_MERGE=1 keeps auto_merge=true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_REPO=/primary
        \\PIPELINE_TEST_CMD=zig test
        \\PIPELINE_AUTO_MERGE=1
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expect(cfg.watched_repos.len == 1);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == true);
}

test "EC-1d: CONTINUOUS_MODE=True (not exact true) keeps continuous_mode=false" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\CONTINUOUS_MODE=True
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    // Only "true" (lowercase) activates continuous_mode
    try std.testing.expect(cfg.continuous_mode == false);
}

// EC-2: PIPELINE_MAX_BACKLOG=0 is valid, not a parse error → stored as 0
test "EC-2: PIPELINE_MAX_BACKLOG=0 is accepted and stored as 0" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_MAX_BACKLOG=0
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(u32, 0), cfg.pipeline_max_backlog);
}

// EC-3: !manual-only entry (path:!manual) → test_cmd="make test", auto_merge=false
test "EC-3: WATCHED_REPOS path:!manual uses make test default and disables auto_merge" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\WATCHED_REPOS=/repo:!manual
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(usize, 1), cfg.watched_repos.len);
    try std.testing.expectEqualStrings("/repo", cfg.watched_repos[0].path);
    try std.testing.expectEqualStrings("make test", cfg.watched_repos[0].test_cmd);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == false);
}

// EC-4: Whitespace around !manual – space between cmd and !manual is handled
test "EC-4: WATCHED_REPOS with space before !manual strips suffix and whitespace" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    // The value " cmd !manual" has a space between cmd and !manual.
    const env = "WATCHED_REPOS=/repo: cmd !manual";

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(usize, 1), cfg.watched_repos.len);
    try std.testing.expectEqualStrings("/repo", cfg.watched_repos[0].path);
    try std.testing.expectEqualStrings("cmd", cfg.watched_repos[0].test_cmd);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == false);
}

// EC-5: PIPELINE_AUTO_MERGE=false with no WATCHED_REPOS → single entry, no panic
test "EC-5: PIPELINE_AUTO_MERGE=false with WATCHED_REPOS absent does not panic" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_REPO=/primary
        \\PIPELINE_TEST_CMD=zig test
        \\PIPELINE_AUTO_MERGE=false
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(usize, 1), cfg.watched_repos.len);
    try std.testing.expectEqualStrings("/primary", cfg.watched_repos[0].path);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == false);
}

// EC-6: WATCHED_REPOS entry that duplicates PIPELINE_REPO is silently skipped
test "EC-6: WATCHED_REPOS entry matching PIPELINE_REPO is deduplicated" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\PIPELINE_REPO=/primary
        \\PIPELINE_TEST_CMD=zig test
        \\PIPELINE_AUTO_MERGE=true
        \\WATCHED_REPOS=/primary:other_cmd|/second:cmd2
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    // /primary appears only once (as primary), /second is added
    try std.testing.expectEqual(@as(usize, 2), cfg.watched_repos.len);
    try std.testing.expectEqualStrings("/primary", cfg.watched_repos[0].path);
    try std.testing.expect(cfg.watched_repos[0].is_self == true);
    // The duplicate /primary:other_cmd is skipped; primary keeps its original test_cmd
    try std.testing.expectEqualStrings("zig test", cfg.watched_repos[0].test_cmd);
    try std.testing.expectEqualStrings("/second", cfg.watched_repos[1].path);
}

// EC-7: Large CONTAINER_MEMORY_MB value is accepted without error
test "EC-7: large CONTAINER_MEMORY_MB=999999 is accepted" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const env =
        \\CONTAINER_MEMORY_MB=999999
    ;

    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(u64, 999999), cfg.container_memory_mb);
}

// EC-8: Test isolation – verified by design: all env_content strings above are
// self-contained and no std.posix.setenv / std.posix.unsetenv calls appear in
// this file.  This test asserts initFromContent accepts an empty string without
// touching the process environment for the keys under test.
test "EC-8: initFromContent with empty string does not crash and returns a Config" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();

    const cfg = try Config.initFromContent(arena.allocator(), "");
    // Spot-check a couple of fields to confirm a valid Config was returned.
    try std.testing.expectEqual(@as(u32, 5), cfg.pipeline_max_backlog);
    try std.testing.expectEqual(@as(u16, 3131), cfg.web_port);
}
