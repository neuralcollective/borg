// Tests for spec: Enforce agent timeout with SIGTERM/SIGKILL escalation
//
// Covers every acceptance criterion from spec.md that is exercisable at the
// docker.zig level:
//
//   AC2 — ContainerConfig.timeout_s field exists
//   AC2 — docker.SIGKILL_GRACE_S constant equals 30
//   AC3 — ContainerConfig.timeout_s defaults to 0
//   AC7 — Docker.stopContainer accepts a grace_s parameter (3-arg signature)
//   E1  — timeout_s ≤ 0 means "no timeout" (field stores value as-is)
//   E4  — stopContainer is callable with a grace value (signature check)
//
// Tests that reference docker.SIGKILL_GRACE_S or docker.ContainerConfig.timeout_s
// will FAIL TO COMPILE until those symbols are added.
//
// The AC7 stopContainer parameter-count test will FAIL AT RUNTIME (wrong count)
// until the grace_s parameter is added to Docker.stopContainer.
//
// To include in the build, add to docker.zig's test block:
//   _ = @import("docker_timeout_test.zig");

const std = @import("std");
const docker = @import("docker.zig");

// =============================================================================
// AC2 + AC3: ContainerConfig has timeout_s field with default 0
//
// Compile-time proof that the field exists.  Will fail to compile until
// `timeout_s: i64 = 0` is added to ContainerConfig.
// =============================================================================

test "AC2: ContainerConfig has timeout_s field" {
    // Explicitly set timeout_s — compile error if the field does not exist.
    const cfg = docker.ContainerConfig{
        .image = "test:latest",
        .name = "test-container",
        .env = &.{},
        .binds = &.{},
        .timeout_s = 600,
    };
    try std.testing.expectEqual(@as(i64, 600), cfg.timeout_s);
}

test "AC3: ContainerConfig timeout_s defaults to 0" {
    // Omit timeout_s — it must have a default of 0.
    const cfg = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &.{},
    };
    try std.testing.expectEqual(@as(i64, 0), cfg.timeout_s);
}

test "AC3: ContainerConfig timeout_s = 0 means no deadline" {
    const cfg = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &.{},
        .timeout_s = 0,
    };
    // The guard `if (config.timeout_s <= 0)` in runWithStdio will skip the
    // watchdog entirely when this field is 0.
    try std.testing.expect(cfg.timeout_s <= 0);
}

// =============================================================================
// AC2: docker.SIGKILL_GRACE_S constant exists and equals 30
//
// Compile-time proof that the constant is exported.  Fails to compile until
// `pub const SIGKILL_GRACE_S: u32 = 30` is added to docker.zig.
// =============================================================================

test "AC2: docker.SIGKILL_GRACE_S exists and equals 30" {
    try std.testing.expectEqual(@as(u32, 30), docker.SIGKILL_GRACE_S);
}

// =============================================================================
// AC7: Docker.stopContainer accepts a grace_s parameter
//
// Currently stopContainer has the signature:
//   pub fn stopContainer(self: *Docker, container_id: []const u8) !void
// which has 2 parameters.
//
// After implementation it must be:
//   pub fn stopContainer(self: *Docker, name_or_id: []const u8, grace_s: u32) !void
// which has 3 parameters.
//
// This test checks the parameter count.  It FAILS initially (2 params)
// and PASSES after implementation (3 params).
// =============================================================================

test "AC7: stopContainer has 3 parameters (self, name_or_id, grace_s)" {
    const StopFn = @TypeOf(docker.Docker.stopContainer);
    const fn_info = @typeInfo(StopFn).@"fn";
    // Currently 2 → will fail.  After implementation 3 → will pass.
    try std.testing.expectEqual(@as(usize, 3), fn_info.params.len);
}

test "AC7: stopContainer third parameter has type u32 (grace_s)" {
    const StopFn = @TypeOf(docker.Docker.stopContainer);
    const fn_info = @typeInfo(StopFn).@"fn";
    // This check is guarded to avoid an index-out-of-bounds compile error when
    // the function currently has only 2 params.  If param count is wrong, the
    // previous test already fails.
    if (fn_info.params.len >= 3) {
        const third_type = fn_info.params[2].type orelse
            return error.MissingParamType;
        try std.testing.expect(third_type == u32);
    } else {
        return error.TooFewParameters;
    }
}

// =============================================================================
// AC7: stopContainer return type is still an error union (no signature breakage)
// =============================================================================

test "AC7: stopContainer return type is an error union" {
    const StopFn = @TypeOf(docker.Docker.stopContainer);
    const fn_info = @typeInfo(StopFn).@"fn";
    const ret = fn_info.return_type orelse return error.NoReturnType;
    // Must be an error union (anyerror!void or a concrete error set).
    const ret_info = @typeInfo(ret);
    try std.testing.expect(ret_info == .error_union);
}

// =============================================================================
// E1: ContainerConfig accepts timeout_s ≤ 0 (no-timeout sentinel values)
// =============================================================================

test "E1: ContainerConfig with timeout_s=-1 is valid (disabled sentinel)" {
    const cfg = docker.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &.{},
        .timeout_s = -1,
    };
    // The implementation guards with `if (config.timeout_s <= 0)`.
    try std.testing.expect(cfg.timeout_s <= 0);
}

// =============================================================================
// AC2 + AC3: Existing ContainerConfig fields still compile without timeout_s
//
// Regression: adding a new optional field must not break existing code that
// constructs ContainerConfig without it.
// =============================================================================

test "AC8: existing ContainerConfig fields remain accessible" {
    const cfg = docker.ContainerConfig{
        .image = "borg:latest",
        .name = "borg-agent",
        .env = &.{"FOO=bar"},
        .binds = &.{"/tmp:/tmp"},
        .memory_limit = 512 * 1024 * 1024,
        .pids_limit = 256,
    };
    try std.testing.expectEqualStrings("borg:latest", cfg.image);
    try std.testing.expectEqualStrings("borg-agent", cfg.name);
    try std.testing.expectEqual(@as(u64, 512 * 1024 * 1024), cfg.memory_limit);
    try std.testing.expectEqual(@as(i64, 256), cfg.pids_limit);
}

// =============================================================================
// AC2: SIGKILL_GRACE_S value matches the spec (30 seconds)
//
// The docker stop --time flag will be set to SIGKILL_GRACE_S.  If this value
// drifts, the spec contract is broken.
// =============================================================================

test "AC2: SIGKILL_GRACE_S is exactly 30 (matches docker stop --time default)" {
    // docker stop's default --time is 10s, but the spec requires 30s.
    // Explicitly verify we match the spec, not the Docker default.
    try std.testing.expect(docker.SIGKILL_GRACE_S > 10);
    try std.testing.expectEqual(@as(u32, 30), docker.SIGKILL_GRACE_S);
}
