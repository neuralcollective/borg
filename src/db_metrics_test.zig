// Tests for phase metrics tracking: duration_ms, success, cost_usd columns in
// task_outputs, and the getPhaseMetrics / setOutputSuccess DB functions.
//
// Covers every acceptance criterion and edge case from spec.md (Task #56).
// These tests FAIL until the implementation adds:
//   - duration_ms, success, cost_usd columns to task_outputs
//   - storeTaskOutputFull new signature returning !i64
//   - setOutputSuccess function
//   - getPhaseMetrics function / PhaseMetrics struct
//
// To include in the build, add to the test block in src/db.zig:
//   _ = @import("db_metrics_test.zig");

const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;

// =============================================================================
// Helpers
// =============================================================================

fn freeOutputs(alloc: std.mem.Allocator, outputs: []Db.TaskOutput) void {
    for (outputs) |o| {
        alloc.free(o.phase);
        alloc.free(o.output);
        alloc.free(o.raw_stream);
        alloc.free(o.created_at);
    }
    alloc.free(outputs);
}

fn freeMetrics(alloc: std.mem.Allocator, metrics: []Db.PhaseMetrics) void {
    for (metrics) |m| {
        alloc.free(m.phase);
    }
    alloc.free(metrics);
}

// Find a PhaseMetrics entry by phase name; returns null if not found.
fn findPhase(metrics: []Db.PhaseMetrics, phase: []const u8) ?Db.PhaseMetrics {
    for (metrics) |m| {
        if (std.mem.eql(u8, m.phase, phase)) return m;
    }
    return null;
}

// =============================================================================
// AC1 — Schema: new columns exist and default correctly
// =============================================================================

test "AC1: storeTaskOutput rows default duration_ms to 0" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // storeTaskOutput (old 4-param function) must still work and default new fields
    try db.storeTaskOutput(1, "spec", "output", 0);

    const outputs = try db.getTaskOutputs(alloc, 1);
    defer freeOutputs(alloc, outputs);

    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(i64, 0), outputs[0].duration_ms);
}

test "AC1: storeTaskOutput rows default success to true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(1, "spec", "output", 0);

    const outputs = try db.getTaskOutputs(alloc, 1);
    defer freeOutputs(alloc, outputs);

    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expect(outputs[0].success == true);
}

test "AC1: storeTaskOutput rows default cost_usd to 0.0" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(1, "spec", "output", 0);

    const outputs = try db.getTaskOutputs(alloc, 1);
    defer freeOutputs(alloc, outputs);

    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(f64, 0.0), outputs[0].cost_usd);
}

// =============================================================================
// AC3 — Success flag round-trip via storeTaskOutputFull
// =============================================================================

test "AC3: storeTaskOutputFull stores success=true and getTaskOutputs returns it" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(1, "impl", "output", "raw", 0, 5000, true, 0.0);

    const outputs = try db.getTaskOutputs(alloc, 1);
    defer freeOutputs(alloc, outputs);

    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expect(outputs[0].success == true);
}

test "AC3: storeTaskOutputFull stores success=false and getTaskOutputs returns it" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(1, "impl", "output", "raw", 0, 5000, false, 0.0);

    const outputs = try db.getTaskOutputs(alloc, 1);
    defer freeOutputs(alloc, outputs);

    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expect(outputs[0].success == false);
}

test "AC3: setOutputSuccess changes success from true to false" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const output_id = try db.storeTaskOutputFull(1, "impl", "output", "raw", 0, 5000, true, 0.0);

    // Initially success = true
    {
        const outputs = try db.getTaskOutputs(alloc, 1);
        defer freeOutputs(alloc, outputs);
        try std.testing.expect(outputs[0].success == true);
    }

    // After setOutputSuccess(false), success = false
    try db.setOutputSuccess(output_id, false);

    const outputs = try db.getTaskOutputs(alloc, 1);
    defer freeOutputs(alloc, outputs);
    try std.testing.expect(outputs[0].success == false);
}

test "AC3: setOutputSuccess changes success from false to true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const output_id = try db.storeTaskOutputFull(1, "impl", "output", "raw", 0, 5000, false, 0.0);
    try db.setOutputSuccess(output_id, true);

    const outputs = try db.getTaskOutputs(alloc, 1);
    defer freeOutputs(alloc, outputs);
    try std.testing.expect(outputs[0].success == true);
}

test "AC3: setOutputSuccess does not affect other rows" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id1 = try db.storeTaskOutputFull(1, "impl", "out1", "raw1", 0, 5000, true, 0.0);
    _ = try db.storeTaskOutputFull(1, "rebase", "out2", "raw2", 0, 3000, true, 0.0);

    // Only update the first row
    try db.setOutputSuccess(id1, false);

    const outputs = try db.getTaskOutputs(alloc, 1);
    defer freeOutputs(alloc, outputs);

    try std.testing.expectEqual(@as(usize, 2), outputs.len);
    // One row has success=false, the other has success=true
    var false_count: usize = 0;
    var true_count: usize = 0;
    for (outputs) |o| {
        if (o.success) true_count += 1 else false_count += 1;
    }
    try std.testing.expectEqual(@as(usize, 1), false_count);
    try std.testing.expectEqual(@as(usize, 1), true_count);
}

// =============================================================================
// AC4 — Success flag: spec phase failure (success=false stored at store time)
// =============================================================================

test "AC4: spec phase stored with success=false is retrievable" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(5, "spec", "", "", 0, 1000, false, 0.0);

    const outputs = try db.getTaskOutputs(alloc, 5);
    defer freeOutputs(alloc, outputs);

    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expect(outputs[0].success == false);
}

// =============================================================================
// AC5 — Success flag: qa phase failure via setOutputSuccess
// =============================================================================

test "AC5: qa phase initially stored as success=true, updated to false on commit failure" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const output_id = try db.storeTaskOutputFull(3, "qa", "agent output", "raw", 0, 2000, true, 0.0);
    // Commit fails → update to false
    try db.setOutputSuccess(output_id, false);

    const outputs = try db.getTaskOutputs(alloc, 3);
    defer freeOutputs(alloc, outputs);

    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expect(outputs[0].success == false);
}

// =============================================================================
// AC6 — Cost captured: cost_usd stored and retrieved via storeTaskOutputFull
// =============================================================================

test "AC6: storeTaskOutputFull stores cost_usd and getTaskOutputs returns it" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(1, "impl", "output", "raw", 0, 5000, true, 0.125);

    const outputs = try db.getTaskOutputs(alloc, 1);
    defer freeOutputs(alloc, outputs);

    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    // Use approximate equality for floating-point (0.125 is exact in IEEE 754)
    try std.testing.expectApproxEqAbs(@as(f64, 0.125), outputs[0].cost_usd, 1e-9);
}

test "AC6: cost_usd zero is stored and retrieved correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(1, "spec", "output", "raw", 0, 1000, true, 0.0);

    const outputs = try db.getTaskOutputs(alloc, 1);
    defer freeOutputs(alloc, outputs);

    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(f64, 0.0), outputs[0].cost_usd);
}

test "AC6: cost_usd large value stored correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(2, "rebase", "out", "raw", 0, 60000, true, 1.5);

    const outputs = try db.getTaskOutputs(alloc, 2);
    defer freeOutputs(alloc, outputs);

    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectApproxEqAbs(@as(f64, 1.5), outputs[0].cost_usd, 1e-9);
}

// =============================================================================
// Duration round-trip via storeTaskOutputFull
// =============================================================================

test "duration_ms stored and retrieved correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(1, "spec", "output", "raw", 0, 12345, true, 0.0);

    const outputs = try db.getTaskOutputs(alloc, 1);
    defer freeOutputs(alloc, outputs);

    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(i64, 12345), outputs[0].duration_ms);
}

test "duration_ms zero is stored and retrieved correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(1, "spec", "output", "raw", 0, 0, true, 0.0);

    const outputs = try db.getTaskOutputs(alloc, 1);
    defer freeOutputs(alloc, outputs);

    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(i64, 0), outputs[0].duration_ms);
}

test "storeTaskOutputFull returns the inserted row id" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id1 = try db.storeTaskOutputFull(1, "spec", "out1", "raw1", 0, 1000, true, 0.0);
    const id2 = try db.storeTaskOutputFull(1, "qa", "out2", "raw2", 0, 2000, true, 0.0);

    // IDs must be positive and distinct
    try std.testing.expect(id1 > 0);
    try std.testing.expect(id2 > 0);
    try std.testing.expect(id1 != id2);
}

// =============================================================================
// AC7 — getPhaseMetrics: empty table returns empty slice
// =============================================================================

test "AC7: getPhaseMetrics on empty task_outputs returns empty slice" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    try std.testing.expectEqual(@as(usize, 0), metrics.len);
}

test "AC7: getPhaseMetrics with only excluded phases returns empty slice" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // These phases are NOT in the filter
    try db.storeTaskOutput(0, "seed", "s", 0);
    try db.storeTaskOutput(0, "seed_proposals", "s", 0);
    try db.storeTaskOutput(1, "spec_diff", "d", 0);
    try db.storeTaskOutput(1, "qa_diff", "d", 0);
    try db.storeTaskOutput(1, "impl_diff", "d", 0);
    try db.storeTaskOutput(1, "test", "t", 1);

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    try std.testing.expectEqual(@as(usize, 0), metrics.len);
}

// =============================================================================
// AC8 — getPhaseMetrics: aggregation correctness
// =============================================================================

test "AC8: 3 spec rows with known values produce correct aggregation" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // 3 spec rows: 2 success, 1 failure; durations 10000/20000/30000
    _ = try db.storeTaskOutputFull(1, "spec", "o1", "r1", 0, 10000, true, 0.0);
    _ = try db.storeTaskOutputFull(2, "spec", "o2", "r2", 0, 20000, true, 0.0);
    _ = try db.storeTaskOutputFull(3, "spec", "o3", "r3", 0, 30000, false, 0.0);

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    const spec = findPhase(metrics, "spec") orelse {
        try std.testing.expect(false); // must find spec entry
        return;
    };

    try std.testing.expectEqual(@as(i64, 3), spec.attempt_count);
    try std.testing.expectEqual(@as(i64, 2), spec.success_count);
    // AVG(10000, 20000, 30000) = 20000.0
    try std.testing.expectApproxEqAbs(@as(f64, 20000.0), spec.mean_duration_ms, 1e-6);
}

test "AC8: getPhaseMetrics success_count zero when all fail" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(1, "impl", "o1", "r1", 0, 5000, false, 0.0);
    _ = try db.storeTaskOutputFull(2, "impl", "o2", "r2", 0, 7000, false, 0.0);

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    const impl = findPhase(metrics, "impl") orelse {
        try std.testing.expect(false);
        return;
    };

    try std.testing.expectEqual(@as(i64, 2), impl.attempt_count);
    try std.testing.expectEqual(@as(i64, 0), impl.success_count);
}

test "AC8: getPhaseMetrics success_count equals attempt_count when all succeed" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(1, "qa", "o1", "r1", 0, 3000, true, 0.0);
    _ = try db.storeTaskOutputFull(2, "qa", "o2", "r2", 0, 4000, true, 0.0);
    _ = try db.storeTaskOutputFull(3, "qa", "o3", "r3", 0, 5000, true, 0.0);

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    const qa = findPhase(metrics, "qa") orelse {
        try std.testing.expect(false);
        return;
    };

    try std.testing.expectEqual(@as(i64, 3), qa.attempt_count);
    try std.testing.expectEqual(@as(i64, 3), qa.success_count);
}

test "AC8: getPhaseMetrics total_cost_usd sums all rows" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(1, "impl", "o1", "r1", 0, 5000, true, 0.5);
    _ = try db.storeTaskOutputFull(2, "impl", "o2", "r2", 0, 6000, true, 0.25);
    _ = try db.storeTaskOutputFull(3, "impl", "o3", "r3", 0, 7000, true, 0.25);

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    const impl = findPhase(metrics, "impl") orelse {
        try std.testing.expect(false);
        return;
    };

    // 0.5 + 0.25 + 0.25 = 1.0 (exact in IEEE 754)
    try std.testing.expectApproxEqAbs(@as(f64, 1.0), impl.total_cost_usd, 1e-9);
}

// =============================================================================
// Edge case — NULLIF: rows with duration_ms=0 excluded from mean
// =============================================================================

test "Edge: rows with duration_ms=0 are excluded from mean calculation" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Two rows with duration=0 (pre-migration defaults), one with 10000
    _ = try db.storeTaskOutputFull(1, "spec", "o1", "r1", 0, 0, true, 0.0);
    _ = try db.storeTaskOutputFull(2, "spec", "o2", "r2", 0, 0, true, 0.0);
    _ = try db.storeTaskOutputFull(3, "spec", "o3", "r3", 0, 10000, true, 0.0);

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    const spec = findPhase(metrics, "spec") orelse {
        try std.testing.expect(false);
        return;
    };

    // NULLIF excludes zeros; AVG over only [10000] = 10000.0, not AVG([0,0,10000])=3333.3
    try std.testing.expectApproxEqAbs(@as(f64, 10000.0), spec.mean_duration_ms, 1e-6);
    // attempt_count still includes all 3 rows
    try std.testing.expectEqual(@as(i64, 3), spec.attempt_count);
}

test "Edge: all rows have duration_ms=0 → mean_duration_ms is 0.0" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(1, "rebase", "o1", "r1", 0, 0, true, 0.0);
    _ = try db.storeTaskOutputFull(2, "rebase", "o2", "r2", 0, 0, true, 0.0);

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    const rebase = findPhase(metrics, "rebase") orelse {
        try std.testing.expect(false);
        return;
    };

    // COALESCE(AVG(NULLIF(0,0), NULLIF(0,0)), 0.0) = COALESCE(NULL, 0.0) = 0.0
    try std.testing.expectEqual(@as(f64, 0.0), rebase.mean_duration_ms);
}

// =============================================================================
// Edge case — Phase filter: excluded phases do not appear in results
// =============================================================================

test "Edge: spec_diff phase excluded from getPhaseMetrics" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(1, "spec_diff", "diff", 0);
    _ = try db.storeTaskOutputFull(1, "spec", "output", "raw", 0, 5000, true, 0.0);

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    // spec_diff must NOT appear; spec must appear
    for (metrics) |m| {
        try std.testing.expect(!std.mem.eql(u8, m.phase, "spec_diff"));
    }
    try std.testing.expect(findPhase(metrics, "spec") != null);
}

test "Edge: impl_diff phase excluded from getPhaseMetrics" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(1, "impl_diff", "diff", 0);

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    for (metrics) |m| {
        try std.testing.expect(!std.mem.eql(u8, m.phase, "impl_diff"));
    }
}

test "Edge: test phase excluded from getPhaseMetrics" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(1, "test", "test output", 1);

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    for (metrics) |m| {
        try std.testing.expect(!std.mem.eql(u8, m.phase, "test"));
    }
}

test "Edge: seed phase excluded from getPhaseMetrics" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(0, "seed", "s", "raw", 0, 0, true, 0.0);

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    for (metrics) |m| {
        try std.testing.expect(!std.mem.eql(u8, m.phase, "seed"));
    }
}

// =============================================================================
// Edge case — qa_fix phase IS included
// =============================================================================

test "Edge: qa_fix phase is included in getPhaseMetrics" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(1, "qa_fix", "output", "raw", 0, 4000, true, 0.0);

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    const qa_fix = findPhase(metrics, "qa_fix") orelse {
        try std.testing.expect(false); // qa_fix must be present
        return;
    };
    try std.testing.expectEqual(@as(i64, 1), qa_fix.attempt_count);
}

// =============================================================================
// Edge case — All five tracked phases present simultaneously
// =============================================================================

test "Edge: all five tracked phases returned when present" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(1, "spec", "o", "r", 0, 1000, true, 0.0);
    _ = try db.storeTaskOutputFull(1, "qa", "o", "r", 0, 2000, true, 0.0);
    _ = try db.storeTaskOutputFull(1, "qa_fix", "o", "r", 0, 1500, false, 0.0);
    _ = try db.storeTaskOutputFull(1, "impl", "o", "r", 0, 8000, true, 0.0);
    _ = try db.storeTaskOutputFull(1, "rebase", "o", "r", 0, 3000, true, 0.0);

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    // All five phases must be present
    try std.testing.expect(findPhase(metrics, "spec") != null);
    try std.testing.expect(findPhase(metrics, "qa") != null);
    try std.testing.expect(findPhase(metrics, "qa_fix") != null);
    try std.testing.expect(findPhase(metrics, "impl") != null);
    try std.testing.expect(findPhase(metrics, "rebase") != null);

    // No extra phases from excluded categories
    for (metrics) |m| {
        const is_valid = std.mem.eql(u8, m.phase, "spec") or
            std.mem.eql(u8, m.phase, "qa") or
            std.mem.eql(u8, m.phase, "qa_fix") or
            std.mem.eql(u8, m.phase, "impl") or
            std.mem.eql(u8, m.phase, "rebase");
        try std.testing.expect(is_valid);
    }
}

// =============================================================================
// Edge case — Multiple tasks aggregated per phase
// =============================================================================

test "Edge: getPhaseMetrics aggregates across multiple task IDs per phase" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Three different tasks each with an impl phase
    _ = try db.storeTaskOutputFull(1, "impl", "o1", "r1", 0, 4000, true, 0.25);
    _ = try db.storeTaskOutputFull(2, "impl", "o2", "r2", 0, 8000, false, 0.5);
    _ = try db.storeTaskOutputFull(3, "impl", "o3", "r3", 0, 12000, true, 0.25);

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    const impl = findPhase(metrics, "impl") orelse {
        try std.testing.expect(false);
        return;
    };

    try std.testing.expectEqual(@as(i64, 3), impl.attempt_count);
    try std.testing.expectEqual(@as(i64, 2), impl.success_count);
    // AVG(4000, 8000, 12000) = 8000.0
    try std.testing.expectApproxEqAbs(@as(f64, 8000.0), impl.mean_duration_ms, 1e-6);
    // 0.25 + 0.5 + 0.25 = 1.0
    try std.testing.expectApproxEqAbs(@as(f64, 1.0), impl.total_cost_usd, 1e-9);
}

// =============================================================================
// AC12 — No storeTaskOutput change: old 4-param signature still works
// =============================================================================

test "AC12: storeTaskOutput still accepts 4 parameters without error" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Must compile and succeed with the original 4-param signature
    try db.storeTaskOutput(1, "spec_diff", "diff content", 0);
    try db.storeTaskOutput(1, "impl_diff", "diff content", 0);
    try db.storeTaskOutput(1, "test", "test output", 1);

    const outputs = try db.getTaskOutputs(alloc, 1);
    defer freeOutputs(alloc, outputs);

    try std.testing.expectEqual(@as(usize, 3), outputs.len);
}

// =============================================================================
// PhaseMetrics struct field types
// =============================================================================

test "PhaseMetrics struct has correct field types" {
    // Verify the struct can be initialized with the expected types
    const m = Db.PhaseMetrics{
        .phase = "spec",
        .attempt_count = 5,
        .success_count = 3,
        .mean_duration_ms = 12345.6,
        .total_cost_usd = 0.42,
    };
    try std.testing.expectEqualStrings("spec", m.phase);
    try std.testing.expectEqual(@as(i64, 5), m.attempt_count);
    try std.testing.expectEqual(@as(i64, 3), m.success_count);
    try std.testing.expectApproxEqAbs(@as(f64, 12345.6), m.mean_duration_ms, 1e-6);
    try std.testing.expectApproxEqAbs(@as(f64, 0.42), m.total_cost_usd, 1e-6);
}

test "TaskOutput struct has duration_ms field of type i64" {
    const o = Db.TaskOutput{
        .id = 1,
        .phase = "impl",
        .output = "out",
        .raw_stream = "raw",
        .exit_code = 0,
        .created_at = "2025-01-01",
        .duration_ms = 9876,
        .success = true,
        .cost_usd = 0.25,
    };
    try std.testing.expectEqual(@as(i64, 9876), o.duration_ms);
}

test "TaskOutput struct has success field of type bool" {
    const o_ok = Db.TaskOutput{
        .id = 1,
        .phase = "qa",
        .output = "",
        .raw_stream = "",
        .exit_code = 0,
        .created_at = "",
        .duration_ms = 0,
        .success = true,
        .cost_usd = 0.0,
    };
    const o_fail = Db.TaskOutput{
        .id = 2,
        .phase = "qa",
        .output = "",
        .raw_stream = "",
        .exit_code = 0,
        .created_at = "",
        .duration_ms = 0,
        .success = false,
        .cost_usd = 0.0,
    };
    try std.testing.expect(o_ok.success);
    try std.testing.expect(!o_fail.success);
}

test "TaskOutput struct has cost_usd field of type f64" {
    const o = Db.TaskOutput{
        .id = 1,
        .phase = "rebase",
        .output = "",
        .raw_stream = "",
        .exit_code = 0,
        .created_at = "",
        .duration_ms = 0,
        .success = true,
        .cost_usd = 0.75,
    };
    try std.testing.expectApproxEqAbs(@as(f64, 0.75), o.cost_usd, 1e-9);
}

// =============================================================================
// Edge case — setOutputSuccess on non-existent ID does not error
// =============================================================================

test "Edge: setOutputSuccess on non-existent output_id does not return error" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // ID 9999 does not exist; should not error (UPDATE affects 0 rows)
    try db.setOutputSuccess(9999, false);
}

// =============================================================================
// Edge case — single-row phase has attempt_count=1
// =============================================================================

test "Edge: single spec row produces attempt_count=1 and correct mean" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.storeTaskOutputFull(1, "spec", "output", "raw", 0, 7500, true, 0.0);

    const metrics = try db.getPhaseMetrics(alloc);
    defer freeMetrics(alloc, metrics);

    const spec = findPhase(metrics, "spec") orelse {
        try std.testing.expect(false);
        return;
    };

    try std.testing.expectEqual(@as(i64, 1), spec.attempt_count);
    try std.testing.expectEqual(@as(i64, 1), spec.success_count);
    try std.testing.expectApproxEqAbs(@as(f64, 7500.0), spec.mean_duration_ms, 1e-6);
}
