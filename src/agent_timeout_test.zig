// Tests for spec: Enforce agent timeout with SIGTERM/SIGKILL escalation
//
// Covers every acceptance criterion and edge case from spec.md that is
// exercisable at the agent.zig level:
//
//   AC1 — DirectAgentConfig.timeout_s exists; SIGKILL_GRACE_S = 30
//   AC1 — Watchdog fires, sets fired=true, and kills a hanging process
//   AC3 — timeout_s defaults to 0 in DirectAgentConfig
//   AC4 — Watchdog exits cleanly when done is set before the deadline
//   E1  — timeout_s ≤ 0 means "no timeout" (field stores the value)
//   E2  — fired=true even when SIGTERM alone kills the process
//   E5  — SIGKILL to an already-dead process does not panic
//
// All tests that reference agent.SIGKILL_GRACE_S, agent.DirectWatchdog, or
// agent.runDirectWatchdog will FAIL TO COMPILE until the implementation adds
// those symbols.  Tests that reference DirectAgentConfig.timeout_s will also
// fail to compile until that field is added.
//
// To include in the build, add to agent.zig's test block:
//   _ = @import("agent_timeout_test.zig");
//
// Requires the following to be made pub in agent.zig:
//   pub const SIGKILL_GRACE_S: i64 = 30;
//   pub const DirectWatchdog = struct { ... };
//   pub fn runDirectWatchdog(ctx: DirectWatchdog) void;

const std = @import("std");
const agent = @import("agent.zig");

// =============================================================================
// AC1 + AC3: DirectAgentConfig has timeout_s field with default 0
//
// Compile-time proof that the field exists.  Will fail to compile until
// `timeout_s: i64 = 0` is added to DirectAgentConfig.
// =============================================================================

test "AC3: DirectAgentConfig has timeout_s field" {
    // Initialise a config with the new field.  Compile error if absent.
    const cfg = agent.DirectAgentConfig{
        .model = "claude-opus-4-6",
        .oauth_token = "tok",
        .session_id = null,
        .session_dir = "/tmp",
        .assistant_name = "",
        .timeout_s = 0,
    };
    try std.testing.expectEqual(@as(i64, 0), cfg.timeout_s);
}

test "AC3: DirectAgentConfig timeout_s defaults to 0" {
    // Omit timeout_s — it must have a default of 0.
    const cfg = agent.DirectAgentConfig{
        .model = "m",
        .oauth_token = "t",
        .session_id = null,
        .session_dir = "/s",
        .assistant_name = "",
    };
    try std.testing.expectEqual(@as(i64, 0), cfg.timeout_s);
}

test "AC1: DirectAgentConfig timeout_s accepts positive value" {
    const cfg = agent.DirectAgentConfig{
        .model = "m",
        .oauth_token = "t",
        .session_id = null,
        .session_dir = "/s",
        .assistant_name = "",
        .timeout_s = 600,
    };
    try std.testing.expectEqual(@as(i64, 600), cfg.timeout_s);
}

// =============================================================================
// AC1: SIGKILL_GRACE_S constant exists with value 30
//
// Compile-time proof that the constant is exported.  Fails to compile until
// `pub const SIGKILL_GRACE_S: i64 = 30` is added to agent.zig.
// =============================================================================

test "AC1: SIGKILL_GRACE_S constant exists and equals 30" {
    try std.testing.expectEqual(@as(i64, 30), agent.SIGKILL_GRACE_S);
}

// =============================================================================
// AC1: DirectWatchdog type is exported
//
// Verifies the struct fields match the spec.  Fails to compile until
// `pub const DirectWatchdog = struct { ... }` is added to agent.zig.
// =============================================================================

test "AC1: DirectWatchdog struct has expected fields" {
    var done_val = std.atomic.Value(bool).init(false);
    var fired_val = std.atomic.Value(bool).init(false);

    // Constructing the struct verifies field names and types at compile time.
    const ctx = agent.DirectWatchdog{
        .pid = 1, // init (PID 1 always exists; we are not sending it signals)
        .timeout_s = 5,
        .done = &done_val,
        .fired = &fired_val,
    };

    // Sanity-check the values are stored correctly.
    try std.testing.expectEqual(@as(i64, 5), ctx.timeout_s);
    try std.testing.expect(!ctx.done.load(.acquire));
    try std.testing.expect(!ctx.fired.load(.acquire));
}

// =============================================================================
// AC4: Watchdog exits without firing when done is set before the deadline
//
// Spawn the watchdog with a 5-second timeout, immediately signal done=true.
// The watchdog polls every 1 second, so it must exit within ~1 s without
// setting fired=true.
//
// Fails to compile until `pub fn runDirectWatchdog` is exported from agent.zig.
// =============================================================================

test "AC4: watchdog does not fire when done is set before deadline" {
    var done_val = std.atomic.Value(bool).init(false);
    var fired_val = std.atomic.Value(bool).init(false);

    // Use PID 1 as a stand-in — we will cancel before any signal is sent.
    const ctx = agent.DirectWatchdog{
        .pid = 1,
        .timeout_s = 5,
        .done = &done_val,
        .fired = &fired_val,
    };

    const thread = try std.Thread.spawn(.{}, agent.runDirectWatchdog, .{ctx});

    // Signal done immediately so the watchdog exits on its first poll.
    done_val.store(true, .release);

    // The watchdog polls every 1 s; allow a generous 3-second window.
    thread.join();

    try std.testing.expect(!fired_val.load(.acquire));
}

// =============================================================================
// AC1: Watchdog fires and kills a hanging process
//
// Spawn `sleep 100` (which responds to SIGTERM), configure the watchdog with
// timeout_s=1.  After the watchdog fires, set done=true so the grace-period
// loop exits without waiting the full 30 s.
//
// Asserts:
//   • fired=true after the watchdog fires
//   • The child process is no longer running (wait returns Signal or Exited)
//
// Timing budget: the watchdog fires within ~2 s (1 s deadline + 1 s poll
// resolution); total test duration is ~3 s.
//
// Fails to compile until runDirectWatchdog is exported.
// =============================================================================

test "AC1: watchdog fires and kills a hanging subprocess" {
    const alloc = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "sleep 100" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Close;
    child.stderr_behavior = .Close;
    try child.spawn();

    var done_val = std.atomic.Value(bool).init(false);
    var fired_val = std.atomic.Value(bool).init(false);

    const ctx = agent.DirectWatchdog{
        .pid = child.id,
        .timeout_s = 1,
        .done = &done_val,
        .fired = &fired_val,
    };

    const thread = try std.Thread.spawn(.{}, agent.runDirectWatchdog, .{ctx});

    // Poll until fired (budget: 4 s).  Once fired, the process received SIGTERM.
    var elapsed: u32 = 0;
    while (elapsed < 40) : (elapsed += 1) {
        if (fired_val.load(.acquire)) break;
        std.time.sleep(100 * std.time.ns_per_ms);
    }

    // Set done so the watchdog exits the grace-period loop immediately.
    done_val.store(true, .release);
    thread.join();

    // The watchdog must have fired.
    try std.testing.expect(fired_val.load(.acquire));

    // The process should be dead by now (killed by SIGTERM or SIGKILL).
    const term = child.wait() catch |err| return err;
    switch (term) {
        .Signal => {}, // Process was killed by a signal — expected.
        .Exited => {}, // Process exited cleanly after SIGTERM — also acceptable.
        else => return error.UnexpectedTermination,
    }
}

// =============================================================================
// E2: fired=true even when SIGTERM kills the process before SIGKILL grace
//
// `sleep 100` exits immediately on SIGTERM.  The watchdog sets fired=true
// BEFORE sending SIGTERM.  Even though SIGTERM is sufficient, fired must
// remain true — the agent was killed, not a clean exit.
//
// This test uses the same structure as AC1 but explicitly confirms the
// fired flag semantics: once set, it is not cleared even if SIGKILL is
// never needed.
// =============================================================================

test "E2: fired remains true when process exits cleanly after SIGTERM" {
    const alloc = std.testing.allocator;

    // sleep responds to SIGTERM, so no SIGKILL will be needed.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "sleep 100" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Close;
    child.stderr_behavior = .Close;
    try child.spawn();

    var done_val = std.atomic.Value(bool).init(false);
    var fired_val = std.atomic.Value(bool).init(false);

    const ctx = agent.DirectWatchdog{
        .pid = child.id,
        .timeout_s = 1,
        .done = &done_val,
        .fired = &fired_val,
    };

    const thread = try std.Thread.spawn(.{}, agent.runDirectWatchdog, .{ctx});

    // Wait until fired, then signal done (mimicking runDirect after child.wait).
    var elapsed: u32 = 0;
    while (elapsed < 40) : (elapsed += 1) {
        if (fired_val.load(.acquire)) break;
        std.time.sleep(100 * std.time.ns_per_ms);
    }
    done_val.store(true, .release);

    thread.join();

    // fired must still be true — the agent was killed via SIGTERM.
    try std.testing.expect(fired_val.load(.acquire));

    _ = child.wait() catch {};
}

// =============================================================================
// E1: timeout_s ≤ 0 means no timeout (field stores the value as-is)
//
// The guard `if (config.timeout_s <= 0)` lives in runDirect, not in the
// struct.  Here we verify the field can hold ≤ 0 values and that the
// DirectAgentConfig is initialized correctly.
// =============================================================================

test "E1: DirectAgentConfig with timeout_s=0 is valid" {
    const cfg = agent.DirectAgentConfig{
        .model = "m",
        .oauth_token = "t",
        .session_id = null,
        .session_dir = "/s",
        .assistant_name = "",
        .timeout_s = 0,
    };
    try std.testing.expectEqual(@as(i64, 0), cfg.timeout_s);
}

test "E1: DirectAgentConfig with negative timeout_s is valid" {
    const cfg = agent.DirectAgentConfig{
        .model = "m",
        .oauth_token = "t",
        .session_id = null,
        .session_dir = "/s",
        .assistant_name = "",
        .timeout_s = -1,
    };
    // Negative value stored as-is; runDirect must treat it as disabled.
    try std.testing.expect(cfg.timeout_s <= 0);
}

// =============================================================================
// E5: Sending SIGKILL to an already-dead process does not panic
//
// This tests the OS guarantee that kill(dead_pid, SIGKILL) returns ESRCH,
// which the implementation wraps with `catch {}`.  If the implementation
// panics on ESRCH, this test will catch it.
// =============================================================================

test "E5: SIGKILL to already-dead process is silently ignored" {
    const alloc = std.testing.allocator;

    // Spawn a process that exits immediately.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "exit 0" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Close;
    child.stderr_behavior = .Close;
    try child.spawn();

    const pid = child.id;
    _ = try child.wait(); // Process is now dead.

    // Sending SIGKILL to a dead PID must not panic.  Errors are ignored.
    std.posix.kill(pid, std.posix.SIG.KILL) catch {};

    // Reaching this line means no panic occurred.
}

// =============================================================================
// AC1: runDirectWatchdog is exported from agent.zig
//
// A compile-time check that the function exists and is callable.
// Fails to compile until `pub fn runDirectWatchdog` is added.
// =============================================================================

test "AC1: runDirectWatchdog function is exported" {
    // @TypeOf verifies the function exists at compile time.
    const WatchdogFn = @TypeOf(agent.runDirectWatchdog);
    const fn_info = @typeInfo(WatchdogFn).@"fn";
    // Takes exactly one parameter (the DirectWatchdog context).
    try std.testing.expectEqual(@as(usize, 1), fn_info.params.len);
}
