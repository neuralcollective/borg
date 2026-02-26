// Tests for Task #84: Log warning on env var integer parse failure.
//
// Two layers of coverage:
//
//   1. Unit tests (AC-1…AC-7+) for the new `parseEnvInt` helper.
//      These tests FAIL TO COMPILE until config.zig gains:
//        pub fn parseEnvInt(comptime T: type, str: []const u8,
//                           var_name: []const u8, default: T) T
//
//   2. Integration tests (AC-I8…) via Config.initFromContent that verify
//      every numeric parse-site applies the correct fallback value.
//      Most integration tests already pass with the silent-fallback
//      implementation; they serve as regression tests post-refactor.
//
// Warning emission (criterion 13):
//   Warnings are written to stderr by borgLogFn (src/main.zig).
//   Programmatic assertion is not possible without overriding std_options
//   in the root module, so warnings are verified manually:
//     zig build test 2>&1 | grep warn
//   Expected lines for each invalid-value test have the form:
//     warn: env <VAR>: invalid value '<bad>', using default <N>
//
// Isolation note: tests that assert default values assume the corresponding
// env vars (e.g. PIPELINE_TICK_S, WEB_PORT) are NOT set in the process
// environment of the test runner.  This is the standard assumption for CI.
//
// To wire into the build, add at the bottom of src/config.zig:
//   test { _ = @import("config_parse_warn_test.zig"); }

const std = @import("std");
const config_mod = @import("config.zig");
const Config = config_mod.Config;

// This declaration FAILS TO COMPILE until config.zig exposes parseEnvInt as pub.
// That compile failure is the intended initial "red" state for tests AC-1…AC-7.
const parseEnvInt = config_mod.parseEnvInt;

// ═══════════════════════════════════════════════════════════════════════════════
// Unit tests for parseEnvInt
// ═══════════════════════════════════════════════════════════════════════════════

// AC-1: Valid integer string → parsed value returned, no fallback.
test "AC-1: parseEnvInt returns parsed value for valid integer" {
    const result = parseEnvInt(u32, "10", "FOO", 5);
    try std.testing.expectEqual(@as(u32, 10), result);
}

// AC-2: Non-numeric string → default returned.
// Warning expected on stderr: env FOO: invalid value 'abc', using default 5
test "AC-2: parseEnvInt returns default for non-numeric string" {
    const result = parseEnvInt(u32, "abc", "FOO", 5);
    try std.testing.expectEqual(@as(u32, 5), result);
}

// AC-3: Typographic digit–letter mix → default returned.
// Matches the motivating example from the task description.
// Warning expected on stderr: env PIPELINE_TICK_S: invalid value '3o', using default 30
test "AC-3: parseEnvInt returns default for mixed digit-letter typo '3o'" {
    const result = parseEnvInt(u64, "3o", "PIPELINE_TICK_S", 30);
    try std.testing.expectEqual(@as(u64, 30), result);
}

// AC-4: Zero is a valid integer, not the default.
// No warning should be emitted.
test "AC-4: parseEnvInt returns 0 for '0', not the default 5" {
    const result = parseEnvInt(u32, "0", "FOO", 5);
    try std.testing.expectEqual(@as(u32, 0), result);
}

// AC-5: Value that overflows the target type → default returned.
// Warning expected on stderr: env WEB_PORT: invalid value '99999', using default 3131
test "AC-5: parseEnvInt returns default when value overflows u16" {
    const result = parseEnvInt(u16, "99999", "WEB_PORT", 3131);
    try std.testing.expectEqual(@as(u16, 3131), result);
}

// AC-6: Negative value for an unsigned type → default returned.
// Warning expected on stderr: env FOO: invalid value '-1', using default 5
test "AC-6: parseEnvInt returns default for negative value in unsigned type" {
    const result = parseEnvInt(u32, "-1", "FOO", 5);
    try std.testing.expectEqual(@as(u32, 5), result);
}

// AC-7: Empty string is unparseable → default returned.
// Warning expected on stderr: env FOO: invalid value '', using default 5
test "AC-7: parseEnvInt returns default for empty string" {
    const result = parseEnvInt(u32, "", "FOO", 5);
    try std.testing.expectEqual(@as(u32, 5), result);
}

// AC-1a: Maximum u16 value (65535) is accepted.
test "AC-1a: parseEnvInt accepts maximum u16 value 65535" {
    const result = parseEnvInt(u16, "65535", "WEB_PORT", 3131);
    try std.testing.expectEqual(@as(u16, 65535), result);
}

// AC-1b: i64 overflow with a very large negative number → default returned.
// -9999999999999999999 underflows i64 (min is -9223372036854775808).
// Warning expected on stderr.
test "AC-1b: parseEnvInt returns default for i64 underflow" {
    const result = parseEnvInt(i64, "-9999999999999999999", "X", 300);
    try std.testing.expectEqual(@as(i64, 300), result);
}

// AC-1c: Valid positive i64 value is parsed correctly.
test "AC-1c: parseEnvInt parses valid positive i64 correctly" {
    const result = parseEnvInt(i64, "7200", "PIPELINE_SEED_COOLDOWN_S", 3600);
    try std.testing.expectEqual(@as(i64, 7200), result);
}

// AC-1d: Leading whitespace makes the string unparseable (parseInt is strict).
// Warning expected on stderr.
test "AC-1d: parseEnvInt returns default for string with leading space" {
    const result = parseEnvInt(u32, " 10", "FOO", 5);
    try std.testing.expectEqual(@as(u32, 5), result);
}

// AC-1e: Correct variable name and default appear in warning for named variable.
// This is a behavioural proxy: if parseEnvInt(u32, "bad", "MY_VAR", 42) returns 42,
// we know the fallback path ran (and per implementation the warning includes "MY_VAR",
// "bad", and "42" on stderr).
test "AC-1e: parseEnvInt returns named default confirming fallback path executed" {
    const result = parseEnvInt(u32, "bad", "MY_VAR", 42);
    try std.testing.expectEqual(@as(u32, 42), result);
}

// AC-1f: u64 default value is returned correctly (no sign extension issues).
test "AC-1f: parseEnvInt returns u64 default for invalid input" {
    const result = parseEnvInt(u64, "nope", "PIPELINE_TICK_S", 30);
    try std.testing.expectEqual(@as(u64, 30), result);
}

// AC-1g: i64 negative valid value is accepted.
test "AC-1g: parseEnvInt accepts negative i64 value within range" {
    const result = parseEnvInt(i64, "-300", "SOME_SIGNED_VAR", 0);
    try std.testing.expectEqual(@as(i64, -300), result);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Integration tests via Config.initFromContent
//
// Each test passes an invalid value for one numeric env var and asserts the
// correct fallback default is used.  The fallback values match the `catch`
// constants in Config.initFromContent (not necessarily the `orelse` default
// strings, which may differ for a few variables — see spec §3 note).
// ═══════════════════════════════════════════════════════════════════════════════

// AC-I8: PIPELINE_TICK_S typo → default 30.
// Warning on stderr: env PIPELINE_TICK_S: invalid value '3o', using default 30
test "AC-I8: PIPELINE_TICK_S=3o falls back to default 30" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "PIPELINE_TICK_S=3o");
    try std.testing.expectEqual(@as(u64, 30), cfg.pipeline_tick_s);
}

// AC-I9: WEB_PORT non-numeric → default 3131.
// Warning on stderr: env WEB_PORT: invalid value 'notaport', using default 3131
test "AC-I9: WEB_PORT=notaport falls back to default 3131" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "WEB_PORT=notaport");
    try std.testing.expectEqual(@as(u16, 3131), cfg.web_port);
}

// AC-I10: PIPELINE_MAX_BACKLOG non-numeric → default 5.
// Mirrors existing AC-N8 in config_env_test.zig; must continue passing.
// Warning on stderr: env PIPELINE_MAX_BACKLOG: invalid value 'abc', using default 5
test "AC-I10: PIPELINE_MAX_BACKLOG=abc falls back to default 5" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "PIPELINE_MAX_BACKLOG=abc");
    try std.testing.expectEqual(@as(u32, 5), cfg.pipeline_max_backlog);
}

// AC-I11: PIPELINE_MAX_BACKLOG=0 is valid (zero ≠ default 5).
// Mirrors existing EC-2 in config_env_test.zig; must continue passing.
// No warning should be emitted.
test "AC-I11: PIPELINE_MAX_BACKLOG=0 is valid and returns 0, not the default 5" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "PIPELINE_MAX_BACKLOG=0");
    try std.testing.expectEqual(@as(u32, 0), cfg.pipeline_max_backlog);
}

// AC-I12: CONTAINER_MEMORY_MB non-numeric → default 1024.
// Warning on stderr: env CONTAINER_MEMORY_MB: invalid value 'bad', using default 1024
test "AC-I12: CONTAINER_MEMORY_MB=bad falls back to default 1024" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "CONTAINER_MEMORY_MB=bad");
    try std.testing.expectEqual(@as(u64, 1024), cfg.container_memory_mb);
}

// ─── Remaining parse sites ────────────────────────────────────────────────────

// RELEASE_INTERVAL_MINS non-numeric → default 180 (the catch constant).
test "AC-I13: RELEASE_INTERVAL_MINS=bad falls back to default 180" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "RELEASE_INTERVAL_MINS=bad");
    try std.testing.expectEqual(@as(u32, 180), cfg.release_interval_mins);
}

// CHAT_COLLECTION_WINDOW_MS non-numeric → default 3000.
test "AC-I14: CHAT_COLLECTION_WINDOW_MS=bad falls back to default 3000" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "CHAT_COLLECTION_WINDOW_MS=bad");
    try std.testing.expectEqual(@as(i64, 3000), cfg.chat_collection_window_ms);
}

// CHAT_COOLDOWN_MS non-numeric → default 5000.
test "AC-I15: CHAT_COOLDOWN_MS=bad falls back to default 5000" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "CHAT_COOLDOWN_MS=bad");
    try std.testing.expectEqual(@as(i64, 5000), cfg.chat_cooldown_ms);
}

// AGENT_TIMEOUT_S non-numeric → default 600 (the `catch 600` constant, not the
// `orelse "1000"` string default — see spec §3 note on mismatched defaults).
test "AC-I16: AGENT_TIMEOUT_S=bad falls back to catch-default 600" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "AGENT_TIMEOUT_S=bad");
    try std.testing.expectEqual(@as(i64, 600), cfg.agent_timeout_s);
}

// MAX_CHAT_AGENTS non-numeric → default 4.
test "AC-I17: MAX_CHAT_AGENTS=bad falls back to default 4" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "MAX_CHAT_AGENTS=bad");
    try std.testing.expectEqual(@as(u32, 4), cfg.max_chat_agents);
}

// CHAT_RATE_LIMIT non-numeric → default 5.
test "AC-I18: CHAT_RATE_LIMIT=bad falls back to default 5" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "CHAT_RATE_LIMIT=bad");
    try std.testing.expectEqual(@as(u32, 5), cfg.chat_rate_limit);
}

// PIPELINE_MAX_AGENTS non-numeric → default 2 (the `catch 2` constant, not
// the `orelse "4"` string default).
test "AC-I19: PIPELINE_MAX_AGENTS=bad falls back to catch-default 2" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "PIPELINE_MAX_AGENTS=bad");
    try std.testing.expectEqual(@as(u32, 2), cfg.pipeline_max_agents);
}

// PIPELINE_SEED_COOLDOWN_S non-numeric → default 3600.
test "AC-I20: PIPELINE_SEED_COOLDOWN_S=bad falls back to default 3600" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "PIPELINE_SEED_COOLDOWN_S=bad");
    try std.testing.expectEqual(@as(i64, 3600), cfg.pipeline_seed_cooldown_s);
}

// PIPELINE_PROPOSAL_THRESHOLD non-numeric → default 8.
test "AC-I21: PIPELINE_PROPOSAL_THRESHOLD=bad falls back to default 8" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "PIPELINE_PROPOSAL_THRESHOLD=bad");
    try std.testing.expectEqual(@as(i64, 8), cfg.proposal_promote_threshold);
}

// REMOTE_CHECK_INTERVAL_S non-numeric → default 300.
test "AC-I22: REMOTE_CHECK_INTERVAL_S=bad falls back to default 300" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "REMOTE_CHECK_INTERVAL_S=bad");
    try std.testing.expectEqual(@as(i64, 300), cfg.remote_check_interval_s);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Edge case integration tests
// ═══════════════════════════════════════════════════════════════════════════════

// EC-absent: absent env var uses orelse default string which parses successfully.
// No warning should be emitted (verify manually: grep for "PIPELINE_TICK_S" in stderr).
// Assumes PIPELINE_TICK_S is unset in the process environment (standard CI assumption).
test "EC-absent: absent PIPELINE_TICK_S gives default 30 without triggering fallback" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "BORG_TEST_SENTINEL_EC_ABSENT=1");
    try std.testing.expectEqual(@as(u64, 30), cfg.pipeline_tick_s);
}

// EC-overflow-u16: port 65536 overflows u16, falls back to 3131.
test "EC-overflow-u16: WEB_PORT=65536 overflows u16 and falls back to 3131" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "WEB_PORT=65536");
    try std.testing.expectEqual(@as(u16, 3131), cfg.web_port);
}

// EC-negative-unsigned: negative value for u32 field falls back to default.
test "EC-negative-unsigned: PIPELINE_MAX_BACKLOG=-1 falls back to default 5" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "PIPELINE_MAX_BACKLOG=-1");
    try std.testing.expectEqual(@as(u32, 5), cfg.pipeline_max_backlog);
}

// EC-zero-memory: CONTAINER_MEMORY_MB=0 is valid (unusual but parseable).
// No warning should be emitted.
test "EC-zero-memory: CONTAINER_MEMORY_MB=0 parses to 0, not the default 1024" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "CONTAINER_MEMORY_MB=0");
    try std.testing.expectEqual(@as(u64, 0), cfg.container_memory_mb);
}

// EC-zero-backlog: PIPELINE_MAX_BACKLOG=0 is valid.
// (Mirrors AC-I11 / EC-2 in config_env_test.zig.)
test "EC-zero-backlog: PIPELINE_MAX_BACKLOG=0 is valid, returns 0" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "PIPELINE_MAX_BACKLOG=0");
    try std.testing.expectEqual(@as(u32, 0), cfg.pipeline_max_backlog);
}

// EC-valid-all-sites: all 14 numeric parse sites accept valid values without
// triggering the fallback path.  No warnings should appear in stderr for this test.
test "EC-valid-all-sites: valid values at all 14 numeric parse sites are accepted" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const env =
        \\RELEASE_INTERVAL_MINS=60
        \\CHAT_COLLECTION_WINDOW_MS=1000
        \\CHAT_COOLDOWN_MS=2000
        \\AGENT_TIMEOUT_S=300
        \\MAX_CHAT_AGENTS=8
        \\CHAT_RATE_LIMIT=10
        \\PIPELINE_MAX_AGENTS=3
        \\WEB_PORT=8080
        \\CONTAINER_MEMORY_MB=512
        \\PIPELINE_MAX_BACKLOG=7
        \\PIPELINE_SEED_COOLDOWN_S=1800
        \\PIPELINE_PROPOSAL_THRESHOLD=5
        \\PIPELINE_TICK_S=15
        \\REMOTE_CHECK_INTERVAL_S=120
    ;
    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(u32, 60), cfg.release_interval_mins);
    try std.testing.expectEqual(@as(i64, 1000), cfg.chat_collection_window_ms);
    try std.testing.expectEqual(@as(i64, 2000), cfg.chat_cooldown_ms);
    try std.testing.expectEqual(@as(i64, 300), cfg.agent_timeout_s);
    try std.testing.expectEqual(@as(u32, 8), cfg.max_chat_agents);
    try std.testing.expectEqual(@as(u32, 10), cfg.chat_rate_limit);
    try std.testing.expectEqual(@as(u32, 3), cfg.pipeline_max_agents);
    try std.testing.expectEqual(@as(u16, 8080), cfg.web_port);
    try std.testing.expectEqual(@as(u64, 512), cfg.container_memory_mb);
    try std.testing.expectEqual(@as(u32, 7), cfg.pipeline_max_backlog);
    try std.testing.expectEqual(@as(i64, 1800), cfg.pipeline_seed_cooldown_s);
    try std.testing.expectEqual(@as(i64, 5), cfg.proposal_promote_threshold);
    try std.testing.expectEqual(@as(u64, 15), cfg.pipeline_tick_s);
    try std.testing.expectEqual(@as(i64, 120), cfg.remote_check_interval_s);
}

// EC-multiple-invalid: multiple invalid numeric fields in a single env_content
// each fall back independently.  Each invalid field should produce its own
// warning line on stderr.
test "EC-multiple-invalid: multiple invalid fields each fall back independently" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const env =
        \\PIPELINE_TICK_S=bad
        \\WEB_PORT=bad
        \\PIPELINE_MAX_BACKLOG=bad
    ;
    const cfg = try Config.initFromContent(arena.allocator(), env);
    try std.testing.expectEqual(@as(u64, 30), cfg.pipeline_tick_s);
    try std.testing.expectEqual(@as(u16, 3131), cfg.web_port);
    try std.testing.expectEqual(@as(u32, 5), cfg.pipeline_max_backlog);
}

// EC-hex-prefix: "0x10" is not a decimal integer and triggers the fallback.
// std.fmt.parseInt(_, _, 10) rejects hex prefixes when base is 10.
test "EC-hex-prefix: PIPELINE_MAX_BACKLOG=0x10 is rejected and falls back to 5" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "PIPELINE_MAX_BACKLOG=0x10");
    try std.testing.expectEqual(@as(u32, 5), cfg.pipeline_max_backlog);
}

// EC-float-value: a floating-point string "3.5" is not a valid decimal integer
// (parseInt rejects the dot) and triggers the fallback with a warning.
// Note: findEnvValue trims leading/trailing spaces from values, so space-padded
// integers like "30 " become "30" and parse successfully — this test uses a
// genuinely unparseable value that survives the trim.
test "EC-float-value: PIPELINE_TICK_S=3.5 is rejected and falls back to 30" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const cfg = try Config.initFromContent(arena.allocator(), "PIPELINE_TICK_S=3.5");
    try std.testing.expectEqual(@as(u64, 30), cfg.pipeline_tick_s);
}
