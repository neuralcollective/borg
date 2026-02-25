// Tests for Config.initFromContent — end-to-end env-variable parsing.
//
// These tests verify that the parsing logic inside Config.load() (extracted
// into Config.initFromContent) correctly handles:
//   - Boolean fields (PIPELINE_AUTO_MERGE, CONTINUOUS_MODE, WHATSAPP_ENABLED, DISCORD_ENABLED)
//   - Numeric fields with defaults (MAX_BACKLOG_SIZE, CONTAINER_MEMORY_MB, WEB_PORT, …)
//   - WATCHED_REPOS pipe/colon parsing end-to-end, including !manual suffix
//   - Defaults when keys are absent
//   - Edge cases: case-sensitive booleans, zero values, whitespace, duplicates
//
// All tests should FAIL to compile until Config.initFromContent is added to
// src/config.zig.
//
// To include in the build, add to config.zig:
//   test { _ = @import("config_env_test.zig"); }
//
// NOTE: Tests that check "default" values assume the corresponding environment
// variables (e.g. MAX_BACKLOG_SIZE, TICK_INTERVAL_S) are not set in the process
// environment of the test runner. This is the standard assumption for a
// development or CI environment that is not actively running borg.

const std = @import("std");
const config_mod = @import("config.zig");
const Config = config_mod.Config;
const RepoConfig = config_mod.RepoConfig;

// ── Helper ────────────────────────────────────────────────────────────────────

/// Find the first RepoConfig in repos whose path equals `path`. Returns null
/// if no such entry exists.
fn findRepo(repos: []const RepoConfig, path: []const u8) ?RepoConfig {
    for (repos) |r| {
        if (std.mem.eql(u8, r.path, path)) return r;
    }
    return null;
}

// ── AC-B: Boolean field tests ─────────────────────────────────────────────────

// AC-B1: PIPELINE_AUTO_MERGE=false → primary repo auto_merge == false
test "AC-B1: PIPELINE_AUTO_MERGE=false disables primary auto_merge" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const env =
        \\PIPELINE_REPO=/primary
        \\PIPELINE_TEST_CMD=make
        \\PIPELINE_AUTO_MERGE=false
    ;
    const cfg = try Config.initFromContent(alloc, env);

    try std.testing.expect(cfg.watched_repos.len >= 1);
    try std.testing.expectEqualStrings("/primary", cfg.watched_repos[0].path);
    try std.testing.expect(cfg.watched_repos[0].is_self == true);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == false);
}

// AC-B2: PIPELINE_AUTO_MERGE absent → primary auto_merge == true (default)
test "AC-B2: PIPELINE_AUTO_MERGE absent defaults to true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const env =
        \\PIPELINE_REPO=/primary
        \\PIPELINE_TEST_CMD=make
    ;
    const cfg = try Config.initFromContent(alloc, env);

    try std.testing.expect(cfg.watched_repos.len >= 1);
    try std.testing.expectEqualStrings("/primary", cfg.watched_repos[0].path);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == true);
}

// AC-B3: PIPELINE_AUTO_MERGE=true (explicit) → primary auto_merge == true
test "AC-B3: PIPELINE_AUTO_MERGE=true explicit keeps auto_merge true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const env =
        \\PIPELINE_REPO=/primary
        \\PIPELINE_TEST_CMD=make
        \\PIPELINE_AUTO_MERGE=true
    ;
    const cfg = try Config.initFromContent(alloc, env);

    try std.testing.expect(cfg.watched_repos.len >= 1);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == true);
}

// AC-B4: CONTINUOUS_MODE=true → config.continuous_mode == true
test "AC-B4: CONTINUOUS_MODE=true sets continuous_mode true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "CONTINUOUS_MODE=true");
    try std.testing.expect(cfg.continuous_mode == true);
}

// AC-B5: CONTINUOUS_MODE absent → config.continuous_mode == false
test "AC-B5: CONTINUOUS_MODE absent defaults to false" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "BORG_UNRELATED=x");
    try std.testing.expect(cfg.continuous_mode == false);
}

// AC-B6: WHATSAPP_ENABLED=true → config.whatsapp_enabled == true
test "AC-B6: WHATSAPP_ENABLED=true sets whatsapp_enabled true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "WHATSAPP_ENABLED=true");
    try std.testing.expect(cfg.whatsapp_enabled == true);
}

// AC-B7: DISCORD_ENABLED=true → config.discord_enabled == true
test "AC-B7: DISCORD_ENABLED=true sets discord_enabled true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "DISCORD_ENABLED=true");
    try std.testing.expect(cfg.discord_enabled == true);
}

// ── AC-N: Numeric field tests ─────────────────────────────────────────────────

// AC-N1: MAX_BACKLOG_SIZE=10 → config.max_backlog_size == 10
test "AC-N1: MAX_BACKLOG_SIZE=10 is parsed correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "MAX_BACKLOG_SIZE=10");
    try std.testing.expectEqual(@as(u32, 10), cfg.max_backlog_size);
}

// AC-N2: MAX_BACKLOG_SIZE absent → config.max_backlog_size == 5 (default)
test "AC-N2: MAX_BACKLOG_SIZE absent defaults to 5" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "BORG_UNRELATED=x");
    try std.testing.expectEqual(@as(u32, 5), cfg.max_backlog_size);
}

// AC-N3: CONTAINER_MEMORY_MB=2048 → config.container_memory_mb == 2048
test "AC-N3: CONTAINER_MEMORY_MB=2048 is parsed correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "CONTAINER_MEMORY_MB=2048");
    try std.testing.expectEqual(@as(u64, 2048), cfg.container_memory_mb);
}

// AC-N4: WEB_PORT=8080 → config.web_port == 8080
test "AC-N4: WEB_PORT=8080 is parsed correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "WEB_PORT=8080");
    try std.testing.expectEqual(@as(u16, 8080), cfg.web_port);
}

// AC-N5: WEB_PORT absent → config.web_port == 3131 (default)
test "AC-N5: WEB_PORT absent defaults to 3131" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "BORG_UNRELATED=x");
    try std.testing.expectEqual(@as(u16, 3131), cfg.web_port);
}

// AC-N6: TICK_INTERVAL_S=60 → config.tick_interval_s == 60
test "AC-N6: TICK_INTERVAL_S=60 is parsed correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "TICK_INTERVAL_S=60");
    try std.testing.expectEqual(@as(u64, 60), cfg.tick_interval_s);
}

// AC-N7: SEED_COOLDOWN_S=7200 → config.seed_cooldown_s == 7200
test "AC-N7: SEED_COOLDOWN_S=7200 is parsed correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "SEED_COOLDOWN_S=7200");
    try std.testing.expectEqual(@as(i64, 7200), cfg.seed_cooldown_s);
}

// AC-N8: MAX_BACKLOG_SIZE=abc (non-numeric) → falls back to default 5
test "AC-N8: non-numeric MAX_BACKLOG_SIZE falls back to default 5" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "MAX_BACKLOG_SIZE=abc");
    try std.testing.expectEqual(@as(u32, 5), cfg.max_backlog_size);
}

// ── AC-W: WATCHED_REPOS integration tests ────────────────────────────────────

// AC-W1: WATCHED_REPOS=/repo/a:npm test!manual → auto_merge==false, test_cmd=="npm test"
test "AC-W1: WATCHED_REPOS !manual disables auto_merge and strips suffix from test_cmd" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const env =
        \\PIPELINE_REPO=/different
        \\WATCHED_REPOS=/repo/a:npm test!manual
    ;
    const cfg = try Config.initFromContent(alloc, env);

    try std.testing.expect(cfg.watched_repos.len >= 2);
    const entry = findRepo(cfg.watched_repos, "/repo/a");
    try std.testing.expect(entry != null);
    try std.testing.expect(entry.?.auto_merge == false);
    try std.testing.expectEqualStrings("npm test", entry.?.test_cmd);
}

// AC-W2: WATCHED_REPOS=/repo/b:go test ./... (no !manual) → auto_merge==true
test "AC-W2: WATCHED_REPOS without !manual has auto_merge true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const env =
        \\PIPELINE_REPO=/different
        \\WATCHED_REPOS=/repo/b:go test ./...
    ;
    const cfg = try Config.initFromContent(alloc, env);

    try std.testing.expect(cfg.watched_repos.len >= 2);
    const entry = findRepo(cfg.watched_repos, "/repo/b");
    try std.testing.expect(entry != null);
    try std.testing.expect(entry.?.auto_merge == true);
    try std.testing.expectEqualStrings("go test ./...", entry.?.test_cmd);
}

// AC-W3: PIPELINE_AUTO_MERGE=false + mixed !manual entries
test "AC-W3: PIPELINE_AUTO_MERGE=false and mixed !manual entries combine correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const env =
        \\PIPELINE_REPO=/main
        \\PIPELINE_TEST_CMD=make
        \\PIPELINE_AUTO_MERGE=false
        \\WATCHED_REPOS=/a:cmd!manual|/b:cmd2
    ;
    const cfg = try Config.initFromContent(alloc, env);

    try std.testing.expectEqual(@as(usize, 3), cfg.watched_repos.len);
    // Primary: PIPELINE_AUTO_MERGE=false → auto_merge==false
    try std.testing.expectEqualStrings("/main", cfg.watched_repos[0].path);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == false);
    try std.testing.expect(cfg.watched_repos[0].is_self == true);
    // /a: !manual → auto_merge==false, cmd stripped
    try std.testing.expectEqualStrings("/a", cfg.watched_repos[1].path);
    try std.testing.expect(cfg.watched_repos[1].auto_merge == false);
    try std.testing.expectEqualStrings("cmd", cfg.watched_repos[1].test_cmd);
    // /b: no suffix → auto_merge==true
    try std.testing.expectEqualStrings("/b", cfg.watched_repos[2].path);
    try std.testing.expect(cfg.watched_repos[2].auto_merge == true);
    try std.testing.expectEqualStrings("cmd2", cfg.watched_repos[2].test_cmd);
}

// AC-W4: WATCHED_REPOS absent, PIPELINE_REPO=/primary → one entry, is_self==true
test "AC-W4: WATCHED_REPOS absent with primary gives single is_self entry" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const env =
        \\PIPELINE_REPO=/primary
        \\PIPELINE_TEST_CMD=make
    ;
    const cfg = try Config.initFromContent(alloc, env);

    try std.testing.expectEqual(@as(usize, 1), cfg.watched_repos.len);
    try std.testing.expectEqualStrings("/primary", cfg.watched_repos[0].path);
    try std.testing.expect(cfg.watched_repos[0].is_self == true);
}

// AC-W5: PIPELINE_REPO absent (empty), WATCHED_REPOS=/a:cmd → no primary prepended
test "AC-W5: absent PIPELINE_REPO means no primary prepended to watched_repos" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "WATCHED_REPOS=/a:cmd");

    try std.testing.expectEqual(@as(usize, 1), cfg.watched_repos.len);
    try std.testing.expectEqualStrings("/a", cfg.watched_repos[0].path);
    try std.testing.expectEqualStrings("cmd", cfg.watched_repos[0].test_cmd);
    try std.testing.expect(cfg.watched_repos[0].is_self == false);
}

// ── AC-D: Default values for missing keys ────────────────────────────────────

// AC-D1: sentinel env (no relevant keys) → documented defaults
// Assumes MAX_BACKLOG_SIZE, CONTAINER_MEMORY_MB, WEB_PORT, TICK_INTERVAL_S,
// SEED_COOLDOWN_S, REMOTE_CHECK_INTERVAL_S, CONTINUOUS_MODE, WHATSAPP_ENABLED,
// and DISCORD_ENABLED are NOT set in the process environment.
test "AC-D1: sentinel env_content yields all documented defaults" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    // Sentinel key that cannot match any borg config variable
    const cfg = try Config.initFromContent(alloc, "BORG_TEST_SENTINEL_AC_D1=1");

    try std.testing.expectEqual(@as(u32, 5), cfg.max_backlog_size);
    try std.testing.expectEqual(@as(u64, 1024), cfg.container_memory_mb);
    try std.testing.expectEqual(@as(u16, 3131), cfg.web_port);
    try std.testing.expectEqual(@as(u64, 30), cfg.tick_interval_s);
    try std.testing.expectEqual(@as(i64, 3600), cfg.seed_cooldown_s);
    try std.testing.expectEqual(@as(i64, 300), cfg.remote_check_interval_s);
    try std.testing.expect(cfg.continuous_mode == false);
    try std.testing.expect(cfg.whatsapp_enabled == false);
    try std.testing.expect(cfg.discord_enabled == false);
}

// ── EC: Edge case tests ───────────────────────────────────────────────────────

// EC-1a: PIPELINE_AUTO_MERGE=True (mixed case) → NOT treated as "false"; auto_merge==true
test "EC-1a: PIPELINE_AUTO_MERGE=True mixed-case is not false; auto_merge stays true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const env =
        \\PIPELINE_REPO=/p
        \\PIPELINE_AUTO_MERGE=True
    ;
    const cfg = try Config.initFromContent(alloc, env);

    try std.testing.expect(cfg.watched_repos.len >= 1);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == true);
}

// EC-1b: PIPELINE_AUTO_MERGE=TRUE → NOT treated as "false"; auto_merge==true
test "EC-1b: PIPELINE_AUTO_MERGE=TRUE all-caps is not false; auto_merge stays true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const env =
        \\PIPELINE_REPO=/p
        \\PIPELINE_AUTO_MERGE=TRUE
    ;
    const cfg = try Config.initFromContent(alloc, env);

    try std.testing.expect(cfg.watched_repos.len >= 1);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == true);
}

// EC-1c: PIPELINE_AUTO_MERGE=1 → NOT treated as "false"; auto_merge==true
test "EC-1c: PIPELINE_AUTO_MERGE=1 is not false; auto_merge stays true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const env =
        \\PIPELINE_REPO=/p
        \\PIPELINE_AUTO_MERGE=1
    ;
    const cfg = try Config.initFromContent(alloc, env);

    try std.testing.expect(cfg.watched_repos.len >= 1);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == true);
}

// EC-1d: CONTINUOUS_MODE=True (mixed case) → NOT treated as "true"; continuous_mode==false
test "EC-1d: CONTINUOUS_MODE=True mixed-case does not activate (requires exact 'true')" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "CONTINUOUS_MODE=True");
    try std.testing.expect(cfg.continuous_mode == false);
}

// EC-1e: CONTINUOUS_MODE=TRUE → NOT treated as "true"; continuous_mode==false
test "EC-1e: CONTINUOUS_MODE=TRUE all-caps does not activate (requires exact 'true')" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "CONTINUOUS_MODE=TRUE");
    try std.testing.expect(cfg.continuous_mode == false);
}

// EC-2: MAX_BACKLOG_SIZE=0 is a valid value (zero, not default)
test "EC-2: MAX_BACKLOG_SIZE=0 is valid and equals zero, not the default 5" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "MAX_BACKLOG_SIZE=0");
    try std.testing.expectEqual(@as(u32, 0), cfg.max_backlog_size);
}

// EC-3: WATCHED_REPOS=/repo:!manual → test_cmd defaults to "make test", auto_merge==false
test "EC-3: WATCHED_REPOS /repo:!manual uses default cmd and disables auto_merge" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "WATCHED_REPOS=/repo:!manual");

    try std.testing.expect(cfg.watched_repos.len >= 1);
    const entry = findRepo(cfg.watched_repos, "/repo");
    try std.testing.expect(entry != null);
    try std.testing.expectEqualStrings("make test", entry.?.test_cmd);
    try std.testing.expect(entry.?.auto_merge == false);
}

// EC-4: Whitespace around !manual — cmd trims correctly to "cmd", auto_merge==false
test "EC-4: whitespace before !manual in cmd is stripped; result is 'cmd' with auto_merge false" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "WATCHED_REPOS=/repo: cmd !manual");

    try std.testing.expect(cfg.watched_repos.len >= 1);
    const entry = findRepo(cfg.watched_repos, "/repo");
    try std.testing.expect(entry != null);
    try std.testing.expectEqualStrings("cmd", entry.?.test_cmd);
    try std.testing.expect(entry.?.auto_merge == false);
}

// EC-5: PIPELINE_AUTO_MERGE=false with no WATCHED_REPOS → exactly one entry, auto_merge==false
test "EC-5: PIPELINE_AUTO_MERGE=false with no WATCHED_REPOS gives one entry without panic" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const env =
        \\PIPELINE_REPO=/primary
        \\PIPELINE_TEST_CMD=make
        \\PIPELINE_AUTO_MERGE=false
    ;
    const cfg = try Config.initFromContent(alloc, env);

    try std.testing.expectEqual(@as(usize, 1), cfg.watched_repos.len);
    try std.testing.expect(cfg.watched_repos[0].auto_merge == false);
    try std.testing.expect(cfg.watched_repos[0].is_self == true);
}

// EC-6: WATCHED_REPOS duplicate of PIPELINE_REPO is silently skipped
test "EC-6: WATCHED_REPOS duplicate of primary is skipped regardless of !manual flag" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const env =
        \\PIPELINE_REPO=/main
        \\PIPELINE_TEST_CMD=pcmd
        \\WATCHED_REPOS=/main:other_cmd!manual|/other:cmd
    ;
    const cfg = try Config.initFromContent(alloc, env);

    // Must have exactly: primary + /other (no duplicate /main entry)
    try std.testing.expectEqual(@as(usize, 2), cfg.watched_repos.len);
    try std.testing.expectEqualStrings("/main", cfg.watched_repos[0].path);
    try std.testing.expect(cfg.watched_repos[0].is_self == true);
    try std.testing.expectEqualStrings("/other", cfg.watched_repos[1].path);
    // Primary auto_merge must not have been overwritten by the duplicate entry's !manual flag
    try std.testing.expect(cfg.watched_repos[0].auto_merge == true);
}

// EC-7: Large numeric value is accepted without error
test "EC-7: CONTAINER_MEMORY_MB=999999 is accepted" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const cfg = try Config.initFromContent(alloc, "CONTAINER_MEMORY_MB=999999");
    try std.testing.expectEqual(@as(u64, 999999), cfg.container_memory_mb);
}

// EC-8: Test isolation — all env values are supplied via env_content, not process env
// Structural test: supplying a specific value in env_content overrides any process-env value.
test "EC-8: env_content value takes precedence and no process env mutation is needed" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    // Supply an explicit value. Regardless of any process-env setting, the
    // env_content value must win (getEnv checks env_content before process env).
    const cfg = try Config.initFromContent(alloc, "MAX_BACKLOG_SIZE=77");
    try std.testing.expectEqual(@as(u32, 77), cfg.max_backlog_size);
}
