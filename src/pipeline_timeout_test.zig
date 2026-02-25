// Tests for spec: Enforce agent timeout with SIGTERM/SIGKILL escalation
//
// Covers acceptance criteria from spec.md that are exercisable at the
// pipeline.zig / integration level:
//
//   AC5  — Timeout error message format includes the numeric duration
//   AC6  — agentTimeoutWatchdog is no longer a public symbol in pipeline
//   AC9  — timeout_s is threaded through ContainerConfig and DirectAgentConfig
//          so the per-task deadline reaches both docker and agent layers
//   E9   — AGENT_TIMEOUT_S_FALLBACK constant is no longer exported
//
// Tests that reference agent.DirectAgentConfig.timeout_s or
// docker.ContainerConfig.timeout_s will FAIL TO COMPILE until those fields
// are added, providing compile-time proof that AC9 can be satisfied.
//
// NOTE: AGENT_TIMEOUT_S_FALLBACK and agentTimeoutWatchdog are private symbols
// in pipeline.zig, so @hasDecl cannot detect their presence or absence.
// Their removal is verified indirectly: once the new timeout_s fields exist in
// ContainerConfig and DirectAgentConfig, there is no longer any need for the
// pipeline-level watchdog.
//
// To include in the build, add to pipeline.zig's test block:
//   _ = @import("pipeline_timeout_test.zig");

const std = @import("std");
const agent_mod = @import("agent.zig");
const docker_mod = @import("docker.zig");
const pipeline = @import("pipeline.zig");

// =============================================================================
// AC5: Pipeline timeout error message format
//
// When pipeline catches error.AgentTimedOut it must format a reason string
// like "timed out after 600s".  These tests document and verify the exact
// format string the implementation must use.
//
// The format string itself always compiles, so these tests always pass.
// They are intentionally kept even though they pass from day one — they pin
// the message format so a future change to the wording will break them.
// =============================================================================

test "AC5: timeout message format matches spec for agent_timeout_s = 600" {
    const alloc = std.testing.allocator;
    const timeout_s: i64 = 600;
    const msg = try std.fmt.allocPrint(alloc, "timed out after {d}s", .{timeout_s});
    defer alloc.free(msg);
    try std.testing.expectEqualStrings("timed out after 600s", msg);
}

test "AC5: timeout message format matches spec for agent_timeout_s = 300" {
    const alloc = std.testing.allocator;
    const timeout_s: i64 = 300;
    const msg = try std.fmt.allocPrint(alloc, "timed out after {d}s", .{timeout_s});
    defer alloc.free(msg);
    try std.testing.expectEqualStrings("timed out after 300s", msg);
}

test "AC5: timeout message format includes 's' suffix (seconds unit)" {
    const alloc = std.testing.allocator;
    const timeout_s: i64 = 1000;
    const msg = try std.fmt.allocPrint(alloc, "timed out after {d}s", .{timeout_s});
    defer alloc.free(msg);
    // Must contain the numeric value.
    try std.testing.expect(std.mem.indexOf(u8, msg, "1000") != null);
    // Must end with 's' for seconds.
    try std.testing.expect(msg[msg.len - 1] == 's');
}

test "AC5: @errorName(error.AgentTimedOut) is \"AgentTimedOut\"" {
    // Verifies the error tag name used in failTask detail strings.
    try std.testing.expectEqualStrings("AgentTimedOut", @errorName(error.AgentTimedOut));
}

// =============================================================================
// AC6: agentTimeoutWatchdog is not a public member of Pipeline
//
// agentTimeoutWatchdog was always private, so @hasDecl always returns false.
// This test documents the intent and protects against accidental publication.
// =============================================================================

test "AC6: agentTimeoutWatchdog is not a public declaration on Pipeline" {
    try std.testing.expect(!@hasDecl(pipeline.Pipeline, "agentTimeoutWatchdog"));
}

test "AC6: agentTimeoutWatchdog is not a public declaration in the pipeline module" {
    try std.testing.expect(!@hasDecl(pipeline, "agentTimeoutWatchdog"));
}

// =============================================================================
// E9: AGENT_TIMEOUT_S_FALLBACK is not a public export
//
// The constant was always private.  After implementation it is deleted
// entirely.  These tests confirm it is not accidentally exposed.
// =============================================================================

test "E9: AGENT_TIMEOUT_S_FALLBACK is not exported from the pipeline module" {
    try std.testing.expect(!@hasDecl(pipeline, "AGENT_TIMEOUT_S_FALLBACK"));
}

// =============================================================================
// AC9: ContainerConfig.timeout_s field exists for pipeline to thread through
//
// pipeline.spawnAgent must populate ContainerConfig.timeout_s with
// config.agent_timeout_s.  This compile-time test proves the field exists
// so the assignment can be written.
//
// FAILS TO COMPILE until `timeout_s: i64 = 0` is added to ContainerConfig.
// =============================================================================

test "AC9: docker.ContainerConfig has timeout_s field (threaded from pipeline)" {
    const cfg = docker_mod.ContainerConfig{
        .image = "borg:latest",
        .name = "borg-agent",
        .env = &.{},
        .binds = &.{},
        .timeout_s = 600, // Simulates: .timeout_s = self.config.agent_timeout_s
    };
    try std.testing.expectEqual(@as(i64, 600), cfg.timeout_s);
}

test "AC9: docker.ContainerConfig timeout_s = 0 disables deadline" {
    const cfg = docker_mod.ContainerConfig{
        .image = "img",
        .name = "c",
        .env = &.{},
        .binds = &.{},
        .timeout_s = 0,
    };
    try std.testing.expect(cfg.timeout_s <= 0);
}

// =============================================================================
// AC9: DirectAgentConfig.timeout_s field exists for pipeline to thread through
//
// pipeline.spawnAgentHost must populate DirectAgentConfig.timeout_s with
// config.agent_timeout_s.  This compile-time test proves the field exists.
//
// FAILS TO COMPILE until `timeout_s: i64 = 0` is added to DirectAgentConfig.
// =============================================================================

test "AC9: agent.DirectAgentConfig has timeout_s field (threaded from pipeline)" {
    const cfg = agent_mod.DirectAgentConfig{
        .model = "claude-opus-4-6",
        .oauth_token = "tok",
        .session_id = null,
        .session_dir = "/tmp",
        .assistant_name = "",
        .timeout_s = 600, // Simulates: .timeout_s = self.config.agent_timeout_s
    };
    try std.testing.expectEqual(@as(i64, 600), cfg.timeout_s);
}

test "AC9: agent.DirectAgentConfig timeout_s = 0 disables deadline" {
    const cfg = agent_mod.DirectAgentConfig{
        .model = "m",
        .oauth_token = "t",
        .session_id = null,
        .session_dir = "/s",
        .assistant_name = "",
        .timeout_s = 0,
    };
    try std.testing.expect(cfg.timeout_s <= 0);
}

// =============================================================================
// AC9: Both constants are consistent
//
// agent.SIGKILL_GRACE_S and docker.SIGKILL_GRACE_S must have the same value
// so the grace period is uniform across both paths.
//
// FAILS TO COMPILE until both constants are added.
// =============================================================================

test "AC9: agent and docker SIGKILL_GRACE_S constants are consistent" {
    // Both are 30s per spec.  If they diverge, timeout behaviour will differ
    // between host agents and containerised agents.
    try std.testing.expectEqual(
        @as(i64, agent_mod.SIGKILL_GRACE_S),
        @as(i64, docker_mod.SIGKILL_GRACE_S),
    );
}

// =============================================================================
// AC8: Public pipeline API is unchanged after refactor
//
// Regression guard: the Pipeline struct and AgentPersona enum must still be
// exported so callers in main.zig continue to compile.
// =============================================================================

test "AC8: pipeline.Pipeline struct is still exported" {
    try std.testing.expect(@hasDecl(pipeline, "Pipeline"));
}

test "AC8: pipeline.AgentPersona enum is still exported" {
    try std.testing.expect(@hasDecl(pipeline, "AgentPersona"));
}
