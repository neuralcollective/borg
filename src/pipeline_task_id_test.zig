// Tests for: Add task_id parameter to spawnAgent for container naming
//
// Verifies that spawnAgent accepts a task_id: i64 parameter, that the
// Docker container name format includes the task ID, and that all five
// call sites pass the correct task ID value.
//
// These tests should FAIL before the implementation is applied because
// the source code does not yet contain the updated signatures/formats.

const std = @import("std");
const pipeline_mod = @import("pipeline.zig");
const Pipeline = pipeline_mod.Pipeline;

// We use @embedFile to inspect the source at comptime. This lets us verify
// private function signatures and format strings that aren't accessible via
// @hasDecl or @typeInfo (since spawnAgent is not pub).
const pipeline_src = @embedFile("pipeline.zig");

// =============================================================================
// AC3: spawnAgent has task_id: i64 as its second parameter (after self)
// =============================================================================

test "AC3: spawnAgent signature includes task_id: i64 parameter" {
    // The new signature should be:
    //   fn spawnAgent(self: *Pipeline, task_id: i64, persona: AgentPersona, ...)
    // Before implementation, this will fail because the old signature is:
    //   fn spawnAgent(self: *Pipeline, persona: AgentPersona, ...)
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "fn spawnAgent(self: *Pipeline, task_id: i64,") != null,
    );
}

test "AC3: task_id parameter appears before persona parameter" {
    // Verify ordering: task_id comes before persona in the parameter list.
    const task_id_pos = std.mem.indexOf(u8, pipeline_src, "task_id: i64") orelse {
        try std.testing.expect(false); // task_id not found at all
        return;
    };
    const persona_pos = std.mem.indexOf(u8, pipeline_src, "persona: AgentPersona") orelse {
        try std.testing.expect(false); // persona not found at all
        return;
    };
    try std.testing.expect(task_id_pos < persona_pos);
}

// =============================================================================
// AC4: Container name format includes the task ID
// =============================================================================

test "AC4: container name format string includes task ID segment" {
    // The new format should be "borg-{s}-t{d}-{d}-{d}" instead of "borg-{s}-{d}-{d}"
    // The 't' prefix before the task_id distinguishes it from timestamp/sequence.
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "borg-{s}-t{d}-{d}-{d}") != null,
    );
}

test "AC4: old container name format without task ID is removed" {
    // The old format "borg-{s}-{d}-{d}" (3 segments) should no longer exist.
    // After implementation, only "borg-{s}-t{d}-{d}-{d}" (4 segments) should remain.
    // We check that the old format is NOT present.
    //
    // Note: We search for the full bufPrint call pattern to avoid matching substrings
    // of the new format or unrelated code.
    const old_pattern = "\"borg-{s}-{d}-{d}\"";
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, old_pattern) == null,
    );
}

test "AC4: container name format produces expected output" {
    // Verify that the new format string produces names like "borg-manager-t19-1700000000-0"
    var name_buf: [128]u8 = undefined;
    const name = try std.fmt.bufPrint(&name_buf, "borg-{s}-t{d}-{d}-{d}", .{
        "manager", @as(i64, 19), @as(i64, 1700000000), @as(u32, 0),
    });
    try std.testing.expectEqualStrings("borg-manager-t19-1700000000-0", name);
}

test "AC4: container name format with qa persona" {
    var name_buf: [128]u8 = undefined;
    const name = try std.fmt.bufPrint(&name_buf, "borg-{s}-t{d}-{d}-{d}", .{
        "qa", @as(i64, 42), @as(i64, 1700000000), @as(u32, 3),
    });
    try std.testing.expectEqualStrings("borg-qa-t42-1700000000-3", name);
}

test "AC4: container name format with worker persona" {
    var name_buf: [128]u8 = undefined;
    const name = try std.fmt.bufPrint(&name_buf, "borg-{s}-t{d}-{d}-{d}", .{
        "worker", @as(i64, 100), @as(i64, 1700000000), @as(u32, 1),
    });
    try std.testing.expectEqualStrings("borg-worker-t100-1700000000-1", name);
}

// =============================================================================
// AC5: All five call sites pass a task ID
// =============================================================================

test "AC5: seedRepo passes 0 as task_id (no task context)" {
    // seedRepo has no PipelineTask, so it should pass 0 as the sentinel.
    // Pattern: self.spawnAgent(0, .manager, ...)
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "self.spawnAgent(0, .manager,") != null,
    );
}

test "AC5: runSpecPhase passes task.id to spawnAgent" {
    // In runSpecPhase, the call should be: self.spawnAgent(task.id, .manager, ...)
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "self.spawnAgent(task.id, .manager,") != null,
    );
}

test "AC5: runQaPhase passes task.id to spawnAgent" {
    // In runQaPhase, the call should be: self.spawnAgent(task.id, .qa, ...)
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "self.spawnAgent(task.id, .qa,") != null,
    );
}

test "AC5: runImplPhase passes task.id to spawnAgent" {
    // In runImplPhase, the call should be: self.spawnAgent(task.id, .worker, ...)
    // There may be multiple .worker calls; we need at least one with task.id
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "self.spawnAgent(task.id, .worker,") != null,
    );
}

test "AC5: runRebasePhase passes task.id to spawnAgent" {
    // runRebasePhase also uses .worker persona with task.id.
    // Since runImplPhase also uses .worker, we verify there are at least TWO
    // occurrences of "self.spawnAgent(task.id, .worker," in the source.
    const pattern = "self.spawnAgent(task.id, .worker,";
    const first = std.mem.indexOf(u8, pipeline_src, pattern) orelse {
        try std.testing.expect(false); // not found at all
        return;
    };
    // Search for a second occurrence after the first
    const rest = pipeline_src[first + pattern.len ..];
    try std.testing.expect(
        std.mem.indexOf(u8, rest, pattern) != null,
    );
}

test "AC5: no spawnAgent calls without task_id parameter" {
    // After the change, there should be no calls matching the OLD pattern:
    //   self.spawnAgent(.manager,  or  self.spawnAgent(.worker,  or  self.spawnAgent(.qa,
    // All calls must now have a task_id before the persona.
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "self.spawnAgent(.manager,") == null,
    );
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "self.spawnAgent(.worker,") == null,
    );
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "self.spawnAgent(.qa,") == null,
    );
}

// =============================================================================
// AC6: No other behavioral changes — Pipeline public API is intact
// =============================================================================

test "AC6: Pipeline struct still has expected public fields" {
    // Verify that the Pipeline struct's public shape hasn't changed.
    const info = @typeInfo(Pipeline);
    const fields = info.@"struct".fields;

    const expected_fields = [_][]const u8{
        "allocator",
        "db",
        "docker",
        "tg",
        "config",
        "running",
        "active_agents",
    };

    for (expected_fields) |expected| {
        var found = false;
        for (fields) |f| {
            if (std.mem.eql(u8, f.name, expected)) {
                found = true;
                break;
            }
        }
        try std.testing.expect(found);
    }
}

test "AC6: AgentPersona enum has expected variants" {
    // The persona enum should still have manager, qa, worker.
    try std.testing.expectEqualStrings("manager", @tagName(pipeline_mod.AgentPersona.manager));
    try std.testing.expectEqualStrings("qa", @tagName(pipeline_mod.AgentPersona.qa));
    try std.testing.expectEqualStrings("worker", @tagName(pipeline_mod.AgentPersona.worker));
}

// =============================================================================
// Edge Case 1: seedRepo sentinel value (task_id = 0)
// =============================================================================

test "Edge1: task_id=0 produces unambiguous container name with t0" {
    // seedRepo passes 0 as task_id. The container name should contain "t0"
    // which is unambiguous since real task IDs start at 1.
    var name_buf: [128]u8 = undefined;
    const name = try std.fmt.bufPrint(&name_buf, "borg-{s}-t{d}-{d}-{d}", .{
        "manager", @as(i64, 0), @as(i64, 1700000000), @as(u32, 0),
    });
    try std.testing.expectEqualStrings("borg-manager-t0-1700000000-0", name);
    // Verify the "t0" segment is present and distinguishable
    try std.testing.expect(std.mem.indexOf(u8, name, "-t0-") != null);
}

// =============================================================================
// Edge Case 2: Container name buffer length (128 bytes)
// =============================================================================

test "Edge2: worst-case container name fits in 128-byte buffer" {
    // Worst case: longest persona ("manager"=7), max i64 (19 digits),
    // large timestamp (10 digits), max u32 sequence (10 digits).
    // "borg-manager-t9223372036854775807-9999999999-4294967295" = 55 chars
    var name_buf: [128]u8 = undefined;
    const name = std.fmt.bufPrint(&name_buf, "borg-{s}-t{d}-{d}-{d}", .{
        "manager", std.math.maxInt(i64), @as(i64, 9999999999), std.math.maxInt(u32),
    });
    // Should not return an error (buffer large enough)
    try std.testing.expect(name != error.NoSpaceLeft);
    const result = try name;
    // Verify it fits
    try std.testing.expect(result.len <= 128);
}

test "Edge2: typical container name is well within buffer" {
    // Typical case: task_id < 10000, normal timestamp, low sequence
    var name_buf: [128]u8 = undefined;
    const name = try std.fmt.bufPrint(&name_buf, "borg-{s}-t{d}-{d}-{d}", .{
        "worker", @as(i64, 42), @as(i64, 1700000000), @as(u32, 0),
    });
    // "borg-worker-t42-1700000000-0" = 28 chars, well under 128
    try std.testing.expect(name.len < 64);
}

// =============================================================================
// Edge Case 3: Negative task IDs (should not occur but i64 allows them)
// =============================================================================

test "Edge3: negative task_id produces valid container name" {
    // SQLite ROWIDs are always positive (1+), so negative values should not
    // occur. But since the type is i64, we verify the format doesn't break.
    var name_buf: [128]u8 = undefined;
    const name = try std.fmt.bufPrint(&name_buf, "borg-{s}-t{d}-{d}-{d}", .{
        "qa", @as(i64, -1), @as(i64, 1700000000), @as(u32, 0),
    });
    // Should produce "borg-qa-t-1-1700000000-0" — valid string, no crash
    try std.testing.expectEqualStrings("borg-qa-t-1-1700000000-0", name);
}

test "Edge3: large negative task_id still fits in buffer" {
    var name_buf: [128]u8 = undefined;
    const name = std.fmt.bufPrint(&name_buf, "borg-{s}-t{d}-{d}-{d}", .{
        "manager", std.math.minInt(i64), @as(i64, 9999999999), std.math.maxInt(u32),
    });
    // minInt(i64) = -9223372036854775808 (20 chars with minus sign)
    // Total: "borg-manager-t-9223372036854775808-9999999999-4294967295" = 56 chars
    try std.testing.expect(name != error.NoSpaceLeft);
    const result = try name;
    try std.testing.expect(result.len <= 128);
}

// =============================================================================
// Edge Case 4: Log message consistency — container name in logs
// =============================================================================

test "Edge4: log format string still references container_name" {
    // The log line at ~line 1293 should still log the container name,
    // which now includes the task ID automatically.
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "Spawning {s} agent: {s}") != null,
    );
}

// =============================================================================
// AC1: Compilation check — implicit
// If this file compiles (as part of `zig build test`), AC1 is satisfied.
// The @import("pipeline.zig") at the top ensures pipeline.zig also compiles.
// =============================================================================

// =============================================================================
// AC2: Tests pass — implicit
// If all tests in this file and existing tests pass, AC2 is satisfied.
// =============================================================================
