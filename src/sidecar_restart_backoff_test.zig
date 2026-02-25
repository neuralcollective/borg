// Tests for the sidecar exponential-backoff restart logic.
//
// All tests target the NEW API described in spec.md §3.  They will FAIL until
// the implementation is in place because:
//   - Sidecar.init currently only accepts 2 args (allocator, assistant_name).
//   - SIDECAR_RESTART_BASE_MS / SIDECAR_RESTART_MAX_MS / SIDECAR_STABLE_THRESHOLD_MS
//     do not yet exist in sidecar.zig.
//   - The backoff fields (restart_delay_ms, next_restart_at_ms, restart_count,
//     last_start_ms) do not yet exist on Sidecar.
//   - The credential fields (discord_token, wa_auth_dir, wa_disabled) do not exist.
//   - tickRestart and formatExitReason do not yet exist.
//
// To include these tests in the build, add inside src/sidecar.zig:
//   test { _ = @import("sidecar_restart_backoff_test.zig"); }
// and make formatExitReason pub.

const std = @import("std");
const sidecar = @import("sidecar.zig");
const Sidecar = sidecar.Sidecar;

// Helper: create a Sidecar using the new 5-arg init with empty credentials.
fn testSidecar(alloc: std.mem.Allocator) Sidecar {
    return Sidecar.init(alloc, "TestBot", "", "", false);
}

// Spawn a trivial long-lived process and assign it as the sidecar's child.
// The child has all I/O closed so it does not interact with the test.
// Caller must ensure s.deinit() is called to reap it.
fn attachLiveChild(s: *Sidecar, alloc: std.mem.Allocator) !void {
    var child = std.process.Child.init(&.{ "sleep", "999" }, alloc);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Close;
    child.stderr_behavior = .Close;
    try child.spawn();
    s.child = child;
    s.last_start_ms = std.time.milliTimestamp();
}

// Spawn a process that exits immediately with code 1 ("false").
// Waits 100 ms so tryWait() is guaranteed to see the exit.
fn attachDeadChild(s: *Sidecar, alloc: std.mem.Allocator) !void {
    var child = std.process.Child.init(&.{"false"}, alloc);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Close;
    child.stderr_behavior = .Close;
    try child.spawn();
    s.child = child;
    s.last_start_ms = std.time.milliTimestamp();
    std.time.sleep(100 * std.time.ns_per_ms);
}

// ── §5 Required test cases ──────────────────────────────────────────────

// AC1: After init, restart_delay_ms equals SIDECAR_RESTART_BASE_MS (5000).
test "AC1: init sets restart_delay_ms to SIDECAR_RESTART_BASE_MS" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();
    try std.testing.expectEqual(sidecar.SIDECAR_RESTART_BASE_MS, s.restart_delay_ms);
}

// AC2: Doubling from 5000 produces the sequence
//      5000 → 10000 → 20000 → 40000 → 80000 → 160000 → 300000 (capped at 7th step).
test "AC2: backoff doubling produces correct sequence capped at max" {
    var delay: i64 = sidecar.SIDECAR_RESTART_BASE_MS;
    const expected = [_]i64{ 10_000, 20_000, 40_000, 80_000, 160_000, 300_000 };
    for (expected) |exp| {
        delay = @min(delay * 2, sidecar.SIDECAR_RESTART_MAX_MS);
        try std.testing.expectEqual(exp, delay);
    }
}

// AC3: SIDECAR_RESTART_MAX_MS cap: doubling 160000 yields exactly 300000, not 320000.
test "AC3: doubling 160000 yields 300000 not 320000" {
    const uncapped: i64 = 160_000 * 2;
    const capped = @min(uncapped, sidecar.SIDECAR_RESTART_MAX_MS);
    try std.testing.expectEqual(@as(i64, 300_000), capped);
    try std.testing.expect(capped != 320_000);
}

// AC4: restart_count increments by 1 for each simulated crash.
// Uses "false" (exits with code 1) as a disposable child process.
test "AC4: restart_count increments when tickRestart detects a crashed child" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();
    try std.testing.expectEqual(@as(u32, 0), s.restart_count);

    try attachDeadChild(&s, std.testing.allocator);
    // next_restart_at_ms is 0 after init → tickRestart would attempt a re-spawn
    // immediately after detecting the exit.  Push it far into the future so the
    // test only exercises the crash-detection half of the function.
    s.next_restart_at_ms = std.time.milliTimestamp() + 999_999_999;

    s.tickRestart();

    try std.testing.expectEqual(@as(u32, 1), s.restart_count);
}

// AC5: When last_start_ms is set to now - SIDECAR_STABLE_THRESHOLD_MS,
//      backoff resets to SIDECAR_RESTART_BASE_MS.
test "AC5: running >= SIDECAR_STABLE_THRESHOLD_MS resets restart_delay_ms" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();
    try attachLiveChild(&s, std.testing.allocator);

    s.restart_delay_ms = 40_000;
    s.last_start_ms = std.time.milliTimestamp() - sidecar.SIDECAR_STABLE_THRESHOLD_MS;

    s.tickRestart();

    try std.testing.expectEqual(sidecar.SIDECAR_RESTART_BASE_MS, s.restart_delay_ms);
}

// AC6: When last_start_ms is set to now - (SIDECAR_STABLE_THRESHOLD_MS - 1),
//      backoff is NOT reset.
test "AC6: running < SIDECAR_STABLE_THRESHOLD_MS does not reset restart_delay_ms" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();
    try attachLiveChild(&s, std.testing.allocator);

    s.restart_delay_ms = 40_000;
    s.last_start_ms = std.time.milliTimestamp() - (sidecar.SIDECAR_STABLE_THRESHOLD_MS - 1);

    s.tickRestart();

    try std.testing.expectEqual(@as(i64, 40_000), s.restart_delay_ms);
}

// AC7: formatExitReason with .Exited = 0 produces a string containing "0".
// Requires formatExitReason to be pub in sidecar.zig.
test "AC7: formatExitReason with Exited(0) produces string containing 0" {
    var buf: [128]u8 = undefined;
    const term: std.process.Child.Term = .{ .Exited = 0 };
    const result = Sidecar.formatExitReason(&buf, term);
    try std.testing.expect(std.mem.indexOf(u8, result, "0") != null);
}

// AC8: formatExitReason with .Signal = 9 produces a string containing "9".
test "AC8: formatExitReason with Signal(9) produces string containing 9" {
    var buf: [128]u8 = undefined;
    const term: std.process.Child.Term = .{ .Signal = 9 };
    const result = Sidecar.formatExitReason(&buf, term);
    try std.testing.expect(std.mem.indexOf(u8, result, "9") != null);
}

// EC1: Doubling when already at SIDECAR_RESTART_MAX_MS keeps value at
//      SIDECAR_RESTART_MAX_MS.
test "EC1: doubling SIDECAR_RESTART_MAX_MS stays at SIDECAR_RESTART_MAX_MS" {
    const result = @min(sidecar.SIDECAR_RESTART_MAX_MS * 2, sidecar.SIDECAR_RESTART_MAX_MS);
    try std.testing.expectEqual(sidecar.SIDECAR_RESTART_MAX_MS, result);
}

// EC2: next_restart_at_ms is set to now + restart_delay_ms (before doubling) after a crash.
test "EC2: next_restart_at_ms is set to now + restart_delay_ms after crash" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();
    try attachDeadChild(&s, std.testing.allocator);

    s.restart_delay_ms = sidecar.SIDECAR_RESTART_BASE_MS;
    // Prevent immediate re-spawn so we only test the crash-detection path.
    s.next_restart_at_ms = std.time.milliTimestamp() + 999_999_999;

    const before = std.time.milliTimestamp();
    s.tickRestart();
    const after = std.time.milliTimestamp();

    // Child must be reaped.
    try std.testing.expect(s.child == null);

    // next_restart_at_ms must be in [before + BASE_MS, after + BASE_MS + 500ms tolerance].
    try std.testing.expect(s.next_restart_at_ms >= before + sidecar.SIDECAR_RESTART_BASE_MS);
    try std.testing.expect(s.next_restart_at_ms <= after + sidecar.SIDECAR_RESTART_BASE_MS + 500);
}

// ── §6 Acceptance criteria – state-observable tests ────────────────────

// AC criterion 3: first restart attempt fires no sooner than 5 s after crash.
// The scheduled next_restart_at_ms must be at least BASE_MS ms in the future.
test "AC criterion 3: next_restart_at_ms is at least BASE_MS after the current time" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();
    try attachDeadChild(&s, std.testing.allocator);
    s.restart_delay_ms = sidecar.SIDECAR_RESTART_BASE_MS;
    s.next_restart_at_ms = std.time.milliTimestamp() + 999_999_999;

    const call_time = std.time.milliTimestamp();
    s.tickRestart();

    try std.testing.expect(s.child == null);
    // The scheduled restart is strictly in the future relative to when tickRestart ran.
    try std.testing.expect(s.next_restart_at_ms > call_time);
    // And it is at least BASE_MS after the call.
    try std.testing.expect(s.next_restart_at_ms >= call_time + sidecar.SIDECAR_RESTART_BASE_MS);
}

// AC criterion 7: discord_connected and wa_connected are false immediately after
// a crash is detected, before the next connected event.
test "AC criterion 7: connected flags cleared to false when crash detected" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();

    // Simulate a previously connected state.
    s.discord_connected = true;
    s.wa_connected = true;

    try attachDeadChild(&s, std.testing.allocator);
    // Prevent re-spawn so the test only exercises crash detection.
    s.next_restart_at_ms = std.time.milliTimestamp() + 999_999_999;

    s.tickRestart();

    try std.testing.expect(!s.discord_connected);
    try std.testing.expect(!s.wa_connected);
}

// AC criterion 8 / E7: start() clears stdout_buf before spawning the new child
// so stale partial lines from the dead process are not re-parsed.
test "AC criterion 8 / E7: start clears stdout_buf before spawning" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();

    // Pre-load stale bytes that must not survive a restart.
    try s.stdout_buf.appendSlice("{\"source\":\"discord\",\"event\":\"partial");
    try std.testing.expect(s.stdout_buf.items.len > 0);

    // start() may succeed or fail depending on whether bun is in PATH.
    // Either way, stdout_buf must be cleared before (or at) the spawn attempt.
    _ = s.start();

    try std.testing.expectEqual(@as(usize, 0), s.stdout_buf.items.len);
}

// AC criterion 9 (invariant check for init): once a Sidecar is initialised it
// retains a valid struct address; it is never set to null by start() failure.
// Tested by verifying that the struct is fully usable after start() returns an error.
test "AC criterion 9: Sidecar struct remains valid and usable after start failure" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();

    // Populate backoff state to confirm struct is not zeroed on start() error.
    s.restart_delay_ms = 20_000;
    s.restart_count = 3;

    _ = s.start(); // Ignore success or failure.

    // Backoff state must survive regardless of start() outcome.
    // (If start() succeeded, child is set; if it failed, fields are unchanged or updated per spec.)
    // The struct must still be accessible and well-formed.
    try std.testing.expect(s.restart_delay_ms >= sidecar.SIDECAR_RESTART_BASE_MS);
}

// ── §7 Edge cases ──────────────────────────────────────────────────────

// E1: Backoff climbs to 300 s and stays there on repeated crashes.
test "E1: backoff reaches SIDECAR_RESTART_MAX_MS and stays there" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();

    // Simulate enough crashes to overflow past the cap.
    // The sequence is 5000→10000→20000→40000→80000→160000→300000→300000→...
    // Seven doublings reach 300000; confirm it stays there.
    const crash_rounds = 10;
    for (0..crash_rounds) |_| {
        try attachDeadChild(&s, std.testing.allocator);
        // Keep next_restart_at_ms far future so no re-spawn occurs during the loop.
        s.next_restart_at_ms = std.time.milliTimestamp() + 999_999_999;
        s.tickRestart();
    }

    try std.testing.expectEqual(sidecar.SIDECAR_RESTART_MAX_MS, s.restart_delay_ms);
    try std.testing.expectEqual(@as(u32, crash_rounds), s.restart_count);
}

// E4: Process killed by SIGKILL (signal 9) is detected and triggers backoff restart.
test "E4: process killed by SIGKILL triggers restart with backoff" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();
    try attachLiveChild(&s, std.testing.allocator);

    const pid = s.child.?.id;
    s.restart_delay_ms = sidecar.SIDECAR_RESTART_BASE_MS;
    // Push restart window into future so detection is tested without a re-spawn.
    s.next_restart_at_ms = std.time.milliTimestamp() + 999_999_999;

    // Kill the child with SIGKILL.
    std.posix.kill(pid, std.posix.SIG.KILL) catch {};
    std.time.sleep(100 * std.time.ns_per_ms);

    s.tickRestart();

    // Crash must be detected: child reaped, count incremented, delay doubled.
    try std.testing.expect(s.child == null);
    try std.testing.expectEqual(@as(u32, 1), s.restart_count);
    try std.testing.expectEqual(sidecar.SIDECAR_RESTART_BASE_MS * 2, s.restart_delay_ms);
}

// E5: Process exits with code 0 (clean exit) is treated the same as any other
// unexpected exit — backoff restart fires.
test "E5: clean exit (code 0) triggers backoff restart same as non-zero exit" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();

    // "true" exits with code 0 on every POSIX system.
    var child = std.process.Child.init(&.{"true"}, std.testing.allocator);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Close;
    child.stderr_behavior = .Close;
    try child.spawn();
    s.child = child;
    s.last_start_ms = std.time.milliTimestamp();
    std.time.sleep(100 * std.time.ns_per_ms);

    s.restart_delay_ms = sidecar.SIDECAR_RESTART_BASE_MS;
    s.next_restart_at_ms = std.time.milliTimestamp() + 999_999_999;

    s.tickRestart();

    // Crash was detected even though exit code was 0.
    try std.testing.expect(s.child == null);
    try std.testing.expectEqual(@as(u32, 1), s.restart_count);
}

// E8: next_restart_at_ms uses the delay value BEFORE it is doubled.
// Verifies that the scheduling window is set with the old delay (e.g. 10 s),
// not the already-doubled value (e.g. 20 s).
test "E8: next_restart_at_ms uses pre-doubled delay value" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();
    try attachDeadChild(&s, std.testing.allocator);

    const original_delay: i64 = 10_000;
    s.restart_delay_ms = original_delay;
    s.next_restart_at_ms = std.time.milliTimestamp() + 999_999_999;

    const before = std.time.milliTimestamp();
    s.tickRestart();
    const after = std.time.milliTimestamp();

    // restart_delay_ms must be doubled now.
    try std.testing.expectEqual(original_delay * 2, s.restart_delay_ms);

    // next_restart_at_ms must use the OLD delay (10_000), not the new one (20_000).
    try std.testing.expect(s.next_restart_at_ms >= before + original_delay);
    try std.testing.expect(s.next_restart_at_ms < before + original_delay * 2);
    _ = after;
}

// E9 boundary: sidecar runs for exactly SIDECAR_STABLE_THRESHOLD_MS → backoff resets.
// (The >= comparison must include the boundary.)
test "E9: exactly at stable threshold resets backoff (>= boundary)" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();
    try attachLiveChild(&s, std.testing.allocator);

    s.restart_delay_ms = 80_000;
    s.last_start_ms = std.time.milliTimestamp() - sidecar.SIDECAR_STABLE_THRESHOLD_MS;

    s.tickRestart();

    try std.testing.expectEqual(sidecar.SIDECAR_RESTART_BASE_MS, s.restart_delay_ms);
}

// E10: sidecar runs for SIDECAR_STABLE_THRESHOLD_MS - 1 ms → backoff NOT reset.
test "E10: one ms below stable threshold does not reset backoff" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();
    try attachLiveChild(&s, std.testing.allocator);

    const elevated: i64 = 80_000;
    s.restart_delay_ms = elevated;
    s.last_start_ms = std.time.milliTimestamp() - (sidecar.SIDECAR_STABLE_THRESHOLD_MS - 1);

    s.tickRestart();

    try std.testing.expectEqual(elevated, s.restart_delay_ms);
}

// ── tickRestart no-child scheduling path ───────────────────────────────

// With child == null and window NOT expired, tickRestart does nothing.
test "tickRestart: unexpired window with no child does not attempt restart" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();

    s.next_restart_at_ms = std.time.milliTimestamp() + 999_999_000;
    s.restart_count = 0;

    s.tickRestart();

    // No restart was attempted.
    try std.testing.expect(s.child == null);
    try std.testing.expectEqual(@as(u32, 0), s.restart_count);
}

// With child == null and window expired (next_restart_at_ms == 0), tickRestart
// calls start().  We accept either outcome (bun available or not) and verify
// that the function made a genuine attempt.
test "tickRestart: expired window with no child attempts start" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();

    s.next_restart_at_ms = 0; // epoch 0 → always in the past
    s.restart_count = 0;

    s.tickRestart();

    // Either start() succeeded (child was spawned) or it failed and restart_count
    // was incremented.  In both cases the "attempted" condition is true.
    const attempted = (s.child != null) or (s.restart_count > 0);
    try std.testing.expect(attempted);
}

// ── §3.6 formatExitReason – full Term variant coverage ─────────────────

// Exited(0): result contains the digit "0".
test "formatExitReason Exited(0) contains 0" {
    var buf: [128]u8 = undefined;
    const result = Sidecar.formatExitReason(&buf, .{ .Exited = 0 });
    try std.testing.expect(std.mem.indexOf(u8, result, "0") != null);
}

// Exited(1): result contains "1".
test "formatExitReason Exited(1) contains 1" {
    var buf: [128]u8 = undefined;
    const result = Sidecar.formatExitReason(&buf, .{ .Exited = 1 });
    try std.testing.expect(std.mem.indexOf(u8, result, "1") != null);
}

// Exited(255): result contains "255".
test "formatExitReason Exited(255) contains 255" {
    var buf: [128]u8 = undefined;
    const result = Sidecar.formatExitReason(&buf, .{ .Exited = 255 });
    try std.testing.expect(std.mem.indexOf(u8, result, "255") != null);
}

// Signal(9): result contains "9".
test "formatExitReason Signal(9) contains 9" {
    var buf: [128]u8 = undefined;
    const result = Sidecar.formatExitReason(&buf, .{ .Signal = 9 });
    try std.testing.expect(std.mem.indexOf(u8, result, "9") != null);
}

// Signal(15): result contains "15".
test "formatExitReason Signal(15) contains 15" {
    var buf: [128]u8 = undefined;
    const result = Sidecar.formatExitReason(&buf, .{ .Signal = 15 });
    try std.testing.expect(std.mem.indexOf(u8, result, "15") != null);
}

// Stopped(19): result contains "19".
test "formatExitReason Stopped(19) contains 19" {
    var buf: [128]u8 = undefined;
    const result = Sidecar.formatExitReason(&buf, .{ .Stopped = 19 });
    try std.testing.expect(std.mem.indexOf(u8, result, "19") != null);
}

// Unknown(0xFF): result is non-empty (implementation may represent it in any format).
test "formatExitReason Unknown(0xFF) produces non-empty string" {
    var buf: [128]u8 = undefined;
    const result = Sidecar.formatExitReason(&buf, .{ .Unknown = 0xFF });
    try std.testing.expect(result.len > 0);
}

// ── Constant value assertions ───────────────────────────────────────────

test "constants: SIDECAR_RESTART_BASE_MS is 5000" {
    try std.testing.expectEqual(@as(i64, 5_000), sidecar.SIDECAR_RESTART_BASE_MS);
}

test "constants: SIDECAR_RESTART_MAX_MS is 300000" {
    try std.testing.expectEqual(@as(i64, 300_000), sidecar.SIDECAR_RESTART_MAX_MS);
}

test "constants: SIDECAR_STABLE_THRESHOLD_MS is 60000" {
    try std.testing.expectEqual(@as(i64, 60_000), sidecar.SIDECAR_STABLE_THRESHOLD_MS);
}

// ── init field assertions ───────────────────────────────────────────────

test "init zeroes all backoff counters and timestamps" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();
    try std.testing.expectEqual(@as(u32, 0), s.restart_count);
    try std.testing.expectEqual(@as(i64, 0), s.next_restart_at_ms);
    try std.testing.expectEqual(@as(i64, 0), s.last_start_ms);
}

test "init stores discord_token, wa_auth_dir, wa_disabled" {
    var s = Sidecar.init(std.testing.allocator, "Bot", "tok123", "/auth/dir", true);
    defer s.deinit();
    try std.testing.expectEqualStrings("tok123", s.discord_token);
    try std.testing.expectEqualStrings("/auth/dir", s.wa_auth_dir);
    try std.testing.expect(s.wa_disabled == true);
}

test "init with wa_disabled false stores false" {
    var s = Sidecar.init(std.testing.allocator, "Bot", "", "", false);
    defer s.deinit();
    try std.testing.expect(s.wa_disabled == false);
}

// ── Multi-crash doubling sequence ───────────────────────────────────────

// restart_delay_ms doubles correctly across three consecutive detected crashes.
test "restart_delay_ms doubles across three consecutive crashes" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();

    const expected_delays = [_]i64{
        sidecar.SIDECAR_RESTART_BASE_MS * 2, // after crash 1: 10_000
        sidecar.SIDECAR_RESTART_BASE_MS * 4, // after crash 2: 20_000
        sidecar.SIDECAR_RESTART_BASE_MS * 8, // after crash 3: 40_000
    };

    for (expected_delays) |exp| {
        try attachDeadChild(&s, std.testing.allocator);
        // Keep next_restart_at_ms far in the future to suppress re-spawn.
        s.next_restart_at_ms = std.time.milliTimestamp() + 999_999_999;
        s.tickRestart();
        try std.testing.expectEqual(exp, s.restart_delay_ms);
    }
}

// restart_count matches the number of crashes detected.
test "restart_count equals number of crashes detected" {
    var s = testSidecar(std.testing.allocator);
    defer s.deinit();

    const rounds: u32 = 4;
    for (0..rounds) |_| {
        try attachDeadChild(&s, std.testing.allocator);
        s.next_restart_at_ms = std.time.milliTimestamp() + 999_999_999;
        s.tickRestart();
    }

    try std.testing.expectEqual(rounds, s.restart_count);
}
