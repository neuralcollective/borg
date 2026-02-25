// Tests for Task #8: Fix Docker Container Name Collision for Concurrent Agents
//
// Verifies that Pipeline promotes the container sequence counter from a
// function-local anonymous struct (`const seq = struct { ... }`) to an
// explicit `container_seq: std.atomic.Value(u32)` field, and that the
// container name format is simplified from "borg-{s}-{d}-{d}" (persona +
// timestamp + seq) to "borg-{s}-{d}" (persona + counter only).
//
// These tests FAIL before the implementation because:
//   - `container_seq` field does not yet exist on Pipeline
//   - The old 3-segment format string is still present in spawnAgent
//   - The function-local `seq` anonymous struct still exists in spawnAgent

const std = @import("std");
const pipeline_mod = @import("pipeline.zig");
const Pipeline = pipeline_mod.Pipeline;

// @embedFile gives us the raw source text of pipeline.zig at compile time.
// This lets us verify private function internals (format strings, removed
// constructs) that are not accessible through @typeInfo or @hasDecl.
const pipeline_src = @embedFile("pipeline.zig");

// =============================================================================
// AC1: Pipeline struct declares container_seq: std.atomic.Value(u32)
// =============================================================================

test "AC1: Pipeline struct has container_seq field of type std.atomic.Value(u32)" {
    // The field declaration must appear verbatim in the Pipeline struct body.
    // Before implementation this fails because the field does not exist yet.
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "container_seq: std.atomic.Value(u32)") != null,
    );
}

test "AC1: container_seq field is accessible via @typeInfo on Pipeline struct" {
    // Confirm the field name is present in the compiled type information.
    const info = @typeInfo(Pipeline);
    var found = false;
    for (info.@"struct".fields) |f| {
        if (std.mem.eql(u8, f.name, "container_seq")) {
            found = true;
            break;
        }
    }
    try std.testing.expect(found);
}

// =============================================================================
// AC2: Pipeline.init initializes container_seq to 0
// =============================================================================

test "AC2: Pipeline.init contains .container_seq = std.atomic.Value(u32).init(0)" {
    // The return literal in init() must set container_seq to 0.
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, ".container_seq = std.atomic.Value(u32).init(0)") != null,
    );
}

// =============================================================================
// AC3: spawnAgent uses self.container_seq.fetchAdd — no function-local seq struct
// =============================================================================

test "AC3: spawnAgent calls self.container_seq.fetchAdd(1, .monotonic)" {
    // The new implementation must delegate to the struct-level atomic counter.
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "self.container_seq.fetchAdd(1, .monotonic)") != null,
    );
}

test "AC3: function-local seq anonymous struct is removed from spawnAgent" {
    // The old pattern was:
    //   const seq = struct {
    //       var counter = std.atomic.Value(u32).init(0);
    //   };
    // After the fix, this block must not exist anywhere in pipeline.zig.
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "const seq = struct {") == null,
    );
}

test "AC3: old seq.counter.fetchAdd call is removed" {
    // The old usage: const n = seq.counter.fetchAdd(1, .monotonic);
    // After the fix, this should be replaced by self.container_seq.fetchAdd.
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "seq.counter.fetchAdd") == null,
    );
}

// =============================================================================
// AC4: std.time.timestamp() is no longer part of the container name format
// =============================================================================

test "AC4: old three-segment container name format is removed" {
    // The old format "borg-{s}-{d}-{d}" embedded a timestamp as the second
    // {d} specifier. This must no longer appear in pipeline.zig.
    //
    // Search for the closing quote after the second {d} to avoid matching
    // the new two-segment format as a substring.
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "\"borg-{s}-{d}-{d}\"") == null,
    );
}

test "AC4: new two-segment container name format is present" {
    // The replacement format "borg-{s}-{d}" (persona + counter only) must
    // appear in the source.  The closing quote ensures we match the complete
    // format string, not a prefix of a longer one.
    try std.testing.expect(
        std.mem.indexOf(u8, pipeline_src, "\"borg-{s}-{d}\"") != null,
    );
}

// =============================================================================
// AC5: Generated container names match pattern borg-{persona}-{n}
// =============================================================================

test "AC5: format string produces borg-manager-0 for counter=0" {
    var buf: [128]u8 = undefined;
    const name = try std.fmt.bufPrint(&buf, "borg-{s}-{d}", .{ "manager", @as(u32, 0) });
    try std.testing.expectEqualStrings("borg-manager-0", name);
}

test "AC5: format string produces borg-qa-7 for counter=7" {
    var buf: [128]u8 = undefined;
    const name = try std.fmt.bufPrint(&buf, "borg-{s}-{d}", .{ "qa", @as(u32, 7) });
    try std.testing.expectEqualStrings("borg-qa-7", name);
}

test "AC5: format string produces borg-worker-255 for counter=255" {
    var buf: [128]u8 = undefined;
    const name = try std.fmt.bufPrint(&buf, "borg-{s}-{d}", .{ "worker", @as(u32, 255) });
    try std.testing.expectEqualStrings("borg-worker-255", name);
}

test "AC5: all three AgentPersona tag names produce valid container names" {
    var buf: [128]u8 = undefined;

    const manager_name = try std.fmt.bufPrint(&buf, "borg-{s}-{d}", .{
        @tagName(pipeline_mod.AgentPersona.manager), @as(u32, 0),
    });
    try std.testing.expect(std.mem.startsWith(u8, manager_name, "borg-manager-"));

    const qa_name = try std.fmt.bufPrint(&buf, "borg-{s}-{d}", .{
        @tagName(pipeline_mod.AgentPersona.qa), @as(u32, 1),
    });
    try std.testing.expect(std.mem.startsWith(u8, qa_name, "borg-qa-"));

    const worker_name = try std.fmt.bufPrint(&buf, "borg-{s}-{d}", .{
        @tagName(pipeline_mod.AgentPersona.worker), @as(u32, 2),
    });
    try std.testing.expect(std.mem.startsWith(u8, worker_name, "borg-worker-"));
}

// =============================================================================
// AC6: Two successive calls within the same second produce distinct names
// =============================================================================

test "AC6: two successive counter increments produce distinct container names" {
    // Simulate two back-to-back spawnAgent calls: they obtain n=0 and n=1
    // from fetchAdd, so even if both calls happen within the same second the
    // names differ.
    var counter = std.atomic.Value(u32).init(0);

    var buf1: [128]u8 = undefined;
    var buf2: [128]u8 = undefined;

    const n1 = counter.fetchAdd(1, .monotonic); // returns 0, counter → 1
    const n2 = counter.fetchAdd(1, .monotonic); // returns 1, counter → 2

    const name1 = try std.fmt.bufPrint(&buf1, "borg-{s}-{d}", .{ "worker", n1 });
    const name2 = try std.fmt.bufPrint(&buf2, "borg-{s}-{d}", .{ "worker", n2 });

    try std.testing.expect(!std.mem.eql(u8, name1, name2));
    try std.testing.expectEqualStrings("borg-worker-0", name1);
    try std.testing.expectEqualStrings("borg-worker-1", name2);
}

test "AC6: names differ even when many calls are made rapidly" {
    var counter = std.atomic.Value(u32).init(0);
    const N = 16;
    var names: [N][128]u8 = undefined;
    var lens: [N]usize = undefined;

    for (0..N) |i| {
        const n = counter.fetchAdd(1, .monotonic);
        const s = try std.fmt.bufPrint(&names[i], "borg-{s}-{d}", .{ "manager", n });
        lens[i] = s.len;
    }

    // Every name must be unique
    for (0..N) |i| {
        for (i + 1..N) |j| {
            const a = names[i][0..lens[i]];
            const b = names[j][0..lens[j]];
            try std.testing.expect(!std.mem.eql(u8, a, b));
        }
    }
}

// =============================================================================
// AC7: Unit test asserts two container names from incremented container_seq
//      are not equal (this IS the unit test required by AC7)
// =============================================================================

test "AC7: two container names produced by incrementing container_seq are not equal" {
    // This is the explicit assertion required by acceptance criterion 7.
    var seq = std.atomic.Value(u32).init(0);

    var buf_a: [128]u8 = undefined;
    var buf_b: [128]u8 = undefined;

    const a = seq.fetchAdd(1, .monotonic); // 0
    const b = seq.fetchAdd(1, .monotonic); // 1

    const name_a = try std.fmt.bufPrint(&buf_a, "borg-{s}-{d}", .{ "manager", a });
    const name_b = try std.fmt.bufPrint(&buf_b, "borg-{s}-{d}", .{ "manager", b });

    // Core assertion from AC7
    try std.testing.expect(!std.mem.eql(u8, name_a, name_b));

    // Also verify exact values for determinism
    try std.testing.expectEqualStrings("borg-manager-0", name_a);
    try std.testing.expectEqualStrings("borg-manager-1", name_b);
}

// =============================================================================
// Edge Case: Counter wrap-around at u32 max (4_294_967_295)
// =============================================================================

test "Edge: fetchAdd wraps from u32 max to 0 without panic" {
    // No special handling is needed; wrapping is the expected u32 behaviour.
    // A process lifetime would need to spawn 4 billion containers to reach
    // this (not a realistic workload).
    var counter = std.atomic.Value(u32).init(std.math.maxInt(u32));

    const before = counter.fetchAdd(1, .monotonic); // reads max, wraps to 0
    const after = counter.load(.monotonic);

    try std.testing.expectEqual(std.math.maxInt(u32), before);
    try std.testing.expectEqual(@as(u32, 0), after);
}

test "Edge: counter at u32 max still produces a valid name string" {
    var buf: [128]u8 = undefined;
    const name = try std.fmt.bufPrint(&buf, "borg-{s}-{d}", .{
        "worker", std.math.maxInt(u32),
    });
    // "borg-worker-4294967295" — valid string, no crash
    try std.testing.expectEqualStrings("borg-worker-4294967295", name);
}

// =============================================================================
// Edge Case: name_buf overflow — longest possible name fits in 128 bytes
// =============================================================================

test "Edge: longest container name (worker + u32 max) fits in 128-byte buffer" {
    // "borg-worker-4294967295" is 22 bytes — well within the 128-byte buffer.
    var buf: [128]u8 = undefined;
    const name = try std.fmt.bufPrint(&buf, "borg-{s}-{d}", .{
        "worker", std.math.maxInt(u32),
    });
    try std.testing.expect(name.len < 128);
    try std.testing.expect(name.len == 22);
}

test "Edge: longest manager name (manager + u32 max) fits in 128-byte buffer" {
    // "borg-manager-4294967295" is 23 bytes.
    var buf: [128]u8 = undefined;
    const name = try std.fmt.bufPrint(&buf, "borg-{s}-{d}", .{
        "manager", std.math.maxInt(u32),
    });
    try std.testing.expect(name.len < 128);
    try std.testing.expectEqualStrings("borg-manager-4294967295", name);
}

// =============================================================================
// Edge Case: Multiple Pipeline instances each start counter at 0
// =============================================================================

test "Edge: two independent atomic counters each produce 0 as their first value" {
    // Each Pipeline instance initialises its own container_seq to 0.
    // This is simulated here with two independent atomic values mirroring
    // what Pipeline.init() does.
    var seq_a = std.atomic.Value(u32).init(0);
    var seq_b = std.atomic.Value(u32).init(0);

    // Advance seq_a several times
    _ = seq_a.fetchAdd(1, .monotonic); // 0
    _ = seq_a.fetchAdd(1, .monotonic); // 1
    _ = seq_a.fetchAdd(1, .monotonic); // 2

    // seq_b is completely independent; its first fetchAdd still returns 0
    const b_first = seq_b.fetchAdd(1, .monotonic);
    try std.testing.expectEqual(@as(u32, 0), b_first);
}

test "Edge: counter starts at 0, giving the first name suffix of 0" {
    var seq = std.atomic.Value(u32).init(0);
    const first = seq.fetchAdd(1, .monotonic);
    try std.testing.expectEqual(@as(u32, 0), first);

    var buf: [128]u8 = undefined;
    const name = try std.fmt.bufPrint(&buf, "borg-{s}-{d}", .{ "qa", first });
    try std.testing.expectEqualStrings("borg-qa-0", name);
}

// =============================================================================
// Edge Case: Concurrent spawnAgent calls — fetchAdd(.monotonic) is atomic
// =============================================================================

test "Edge: concurrent fetchAdd calls produce distinct values" {
    // N threads each call fetchAdd once; all returned values must be unique.
    // This validates that .monotonic atomics prevent duplicate counter reads.
    const N = 8;
    var counter = std.atomic.Value(u32).init(0);
    var results: [N]u32 = undefined;

    const S = struct {
        fn run(ctr: *std.atomic.Value(u32), out: *u32) void {
            out.* = ctr.fetchAdd(1, .monotonic);
        }
    };

    var threads: [N]std.Thread = undefined;
    for (0..N) |i| {
        threads[i] = try std.Thread.spawn(.{}, S.run, .{ &counter, &results[i] });
    }
    for (threads) |t| t.join();

    // Every result must be distinct
    for (0..N) |i| {
        for (i + 1..N) |j| {
            try std.testing.expect(results[i] != results[j]);
        }
    }
    // And all values must be in [0, N)
    for (results) |r| {
        try std.testing.expect(r < N);
    }
}

// =============================================================================
// Structural: no timestamp in container name context
// =============================================================================

test "Structural: spawnAgent name format has exactly 2 format specifiers" {
    // The new bufPrint call takes exactly (persona_str, counter_u32).
    // We verify this by confirming the format string "borg-{s}-{d}" is present
    // and the old format "borg-{s}-{d}-{d}" (with extra timestamp specifier)
    // is absent.
    const new_fmt = "\"borg-{s}-{d}\"";
    const old_fmt = "\"borg-{s}-{d}-{d}\"";

    try std.testing.expect(std.mem.indexOf(u8, pipeline_src, new_fmt) != null);
    try std.testing.expect(std.mem.indexOf(u8, pipeline_src, old_fmt) == null);
}
