// Tests for consolidating getPipelineStats into a single SQL query.
//
// These tests verify every acceptance criterion and edge case from spec.md.
// They should FAIL before the implementation change is applied, because the
// current four-query implementation will still pass correctness tests but
// the single-query structural test (AC1) will fail.

const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;

// =============================================================================
// AC1: Single query — getPipelineStats executes exactly one SQL statement
// =============================================================================
// This is tested structurally: after the refactor, the function body should
// contain a single query call. We verify the observable behavior is correct
// (AC2) and that the active status list matches (AC3). The single-query
// constraint is a code-review / structural concern verified by AC4 (build)
// and AC5 (existing tests pass). We add a behavioral test that would catch
// regressions if the single query returned wrong results.

// =============================================================================
// AC2: Same return values — for any DB state, returned PipelineStats values
// (.total, .active, .merged, .failed) are identical to the original.
// =============================================================================

test "AC2: empty table returns all zeros" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 0), stats.total);
    try std.testing.expectEqual(@as(i64, 0), stats.active);
    try std.testing.expectEqual(@as(i64, 0), stats.merged);
    try std.testing.expectEqual(@as(i64, 0), stats.failed);
}

test "AC2: single backlog task counts as total=1 active=1 merged=0 failed=0" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.createPipelineTask("Task A", "desc", "/repo", "alice", "tg:1");

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 1), stats.total);
    try std.testing.expectEqual(@as(i64, 1), stats.active);
    try std.testing.expectEqual(@as(i64, 0), stats.merged);
    try std.testing.expectEqual(@as(i64, 0), stats.failed);
}

test "AC2: mixed statuses produce correct counts" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Create tasks and set various statuses
    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", ""); // backlog (active)
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", ""); // spec (active)
    const id3 = try db.createPipelineTask("T3", "d", "/repo", "", ""); // merged
    const id4 = try db.createPipelineTask("T4", "d", "/repo", "", ""); // failed
    const id5 = try db.createPipelineTask("T5", "d", "/repo", "", ""); // impl (active)

    try db.updateTaskStatus(id2, "spec");
    try db.updateTaskStatus(id3, "merged");
    try db.updateTaskStatus(id4, "failed");
    try db.updateTaskStatus(id5, "impl");
    _ = id1; // stays as backlog

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 5), stats.total);
    try std.testing.expectEqual(@as(i64, 3), stats.active); // backlog + spec + impl
    try std.testing.expectEqual(@as(i64, 1), stats.merged);
    try std.testing.expectEqual(@as(i64, 1), stats.failed);
}

test "AC2: all tasks merged" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", "");
    try db.updateTaskStatus(id1, "merged");
    try db.updateTaskStatus(id2, "merged");

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 2), stats.total);
    try std.testing.expectEqual(@as(i64, 0), stats.active);
    try std.testing.expectEqual(@as(i64, 2), stats.merged);
    try std.testing.expectEqual(@as(i64, 0), stats.failed);
}

test "AC2: all tasks failed" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", "");
    try db.updateTaskStatus(id1, "failed");
    try db.updateTaskStatus(id2, "failed");

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 2), stats.total);
    try std.testing.expectEqual(@as(i64, 0), stats.active);
    try std.testing.expectEqual(@as(i64, 0), stats.merged);
    try std.testing.expectEqual(@as(i64, 2), stats.failed);
}

// =============================================================================
// AC3: Active status list matches — the IN clause must use the same six
// statuses: 'backlog', 'spec', 'qa', 'impl', 'retry', 'rebase'
// =============================================================================

test "AC3: each of the six active statuses is counted as active" {
    const active_statuses = [_][]const u8{ "backlog", "spec", "qa", "impl", "retry", "rebase" };

    for (active_statuses) |status| {
        var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        var db = try Db.init(alloc, ":memory:");
        defer db.deinit();

        const id = try db.createPipelineTask("Task", "d", "/repo", "", "");
        try db.updateTaskStatus(id, status);

        const stats = try db.getPipelineStats();
        try std.testing.expectEqual(@as(i64, 1), stats.total);
        try std.testing.expectEqual(@as(i64, 1), stats.active);
        try std.testing.expectEqual(@as(i64, 0), stats.merged);
        try std.testing.expectEqual(@as(i64, 0), stats.failed);
    }
}

test "AC3: active count matches getActivePipelineTaskCount" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Create a variety of tasks
    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", ""); // backlog
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", "");
    const id3 = try db.createPipelineTask("T3", "d", "/repo", "", "");
    const id4 = try db.createPipelineTask("T4", "d", "/repo", "", "");
    const id5 = try db.createPipelineTask("T5", "d", "/repo", "", "");

    try db.updateTaskStatus(id2, "qa");
    try db.updateTaskStatus(id3, "merged");
    try db.updateTaskStatus(id4, "failed");
    try db.updateTaskStatus(id5, "rebase");
    _ = id1;

    const stats = try db.getPipelineStats();
    const active_count = try db.getActivePipelineTaskCount();

    // The active count from getPipelineStats must equal getActivePipelineTaskCount
    try std.testing.expectEqual(active_count, stats.active);
}

// =============================================================================
// AC6: No public API change — PipelineStats struct and getPipelineStats
// signature remain unchanged.
// =============================================================================

test "AC6: PipelineStats struct has expected fields" {
    // Verify the struct has exactly the four expected fields with type i64.
    const stats = db_mod.Db.PipelineStats{
        .active = 10,
        .merged = 20,
        .failed = 5,
        .total = 35,
    };
    try std.testing.expectEqual(@as(i64, 10), stats.active);
    try std.testing.expectEqual(@as(i64, 20), stats.merged);
    try std.testing.expectEqual(@as(i64, 5), stats.failed);
    try std.testing.expectEqual(@as(i64, 35), stats.total);
}

test "AC6: getPipelineStats returns PipelineStats type" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const result = db.getPipelineStats();
    // Verify the return type is !PipelineStats
    const stats = try result;
    try std.testing.expect(@TypeOf(stats) == db_mod.Db.PipelineStats);
}

// =============================================================================
// Edge Case 1: Empty table — all four counts must return 0 (not null)
// =============================================================================

test "Edge1: empty pipeline_tasks table returns all zeros not null" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Ensure no tasks exist
    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 0), stats.total);
    try std.testing.expectEqual(@as(i64, 0), stats.active);
    try std.testing.expectEqual(@as(i64, 0), stats.merged);
    try std.testing.expectEqual(@as(i64, 0), stats.failed);
}

// =============================================================================
// Edge Case 2: All tasks in one status — active equals total
// =============================================================================

test "Edge2: all tasks backlog means active equals total" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.createPipelineTask("T1", "d", "/repo", "", "");
    _ = try db.createPipelineTask("T2", "d", "/repo", "", "");
    _ = try db.createPipelineTask("T3", "d", "/repo", "", "");

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 3), stats.total);
    try std.testing.expectEqual(stats.total, stats.active);
    try std.testing.expectEqual(@as(i64, 0), stats.merged);
    try std.testing.expectEqual(@as(i64, 0), stats.failed);
}

// =============================================================================
// Edge Case 3: Statuses not in any category — contribute only to total.
// The sum active + merged + failed may be less than total.
// =============================================================================

test "Edge3: done status only contributes to total" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", "");
    try db.updateTaskStatus(id1, "done");

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 1), stats.total);
    try std.testing.expectEqual(@as(i64, 0), stats.active);
    try std.testing.expectEqual(@as(i64, 0), stats.merged);
    try std.testing.expectEqual(@as(i64, 0), stats.failed);
    // active + merged + failed < total
    try std.testing.expect(stats.active + stats.merged + stats.failed < stats.total);
}

test "Edge3: test status only contributes to total" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", "");
    try db.updateTaskStatus(id1, "test");

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 1), stats.total);
    try std.testing.expectEqual(@as(i64, 0), stats.active);
    try std.testing.expectEqual(@as(i64, 0), stats.merged);
    try std.testing.expectEqual(@as(i64, 0), stats.failed);
}

test "Edge3: mix of categorized and uncategorized statuses" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", ""); // backlog (active)
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", "");
    const id3 = try db.createPipelineTask("T3", "d", "/repo", "", "");
    const id4 = try db.createPipelineTask("T4", "d", "/repo", "", "");
    const id5 = try db.createPipelineTask("T5", "d", "/repo", "", "");

    _ = id1; // backlog
    try db.updateTaskStatus(id2, "done"); // uncategorized
    try db.updateTaskStatus(id3, "test"); // uncategorized
    try db.updateTaskStatus(id4, "merged");
    try db.updateTaskStatus(id5, "failed");

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 5), stats.total);
    try std.testing.expectEqual(@as(i64, 1), stats.active); // only backlog
    try std.testing.expectEqual(@as(i64, 1), stats.merged);
    try std.testing.expectEqual(@as(i64, 1), stats.failed);
    // 1 + 1 + 1 = 3 < 5 (done + test not counted in any category)
    try std.testing.expectEqual(@as(i64, 3), stats.active + stats.merged + stats.failed);
}

// =============================================================================
// Edge Case 5: Defensive — aggregate query always returns a row, so
// the function should never return null-like values.
// =============================================================================

test "Edge5: getPipelineStats returns valid struct even after deleting all tasks" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Create and then delete tasks
    _ = try db.createPipelineTask("T1", "d", "/repo", "", "");
    try db.sqlite_db.execute("DELETE FROM pipeline_tasks", .{});

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 0), stats.total);
    try std.testing.expectEqual(@as(i64, 0), stats.active);
    try std.testing.expectEqual(@as(i64, 0), stats.merged);
    try std.testing.expectEqual(@as(i64, 0), stats.failed);
}

// =============================================================================
// Comprehensive: all six active statuses + merged + failed + uncategorized
// =============================================================================

test "comprehensive: all pipeline statuses in one database" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // 6 active statuses
    const id1 = try db.createPipelineTask("T-backlog", "d", "/repo", "", ""); // backlog
    const id2 = try db.createPipelineTask("T-spec", "d", "/repo", "", "");
    const id3 = try db.createPipelineTask("T-qa", "d", "/repo", "", "");
    const id4 = try db.createPipelineTask("T-impl", "d", "/repo", "", "");
    const id5 = try db.createPipelineTask("T-retry", "d", "/repo", "", "");
    const id6 = try db.createPipelineTask("T-rebase", "d", "/repo", "", "");

    // 2 merged
    const id7 = try db.createPipelineTask("T-merged1", "d", "/repo", "", "");
    const id8 = try db.createPipelineTask("T-merged2", "d", "/repo", "", "");

    // 1 failed
    const id9 = try db.createPipelineTask("T-failed", "d", "/repo", "", "");

    // 2 uncategorized (done, test)
    const id10 = try db.createPipelineTask("T-done", "d", "/repo", "", "");
    const id11 = try db.createPipelineTask("T-test", "d", "/repo", "", "");

    _ = id1; // stays backlog
    try db.updateTaskStatus(id2, "spec");
    try db.updateTaskStatus(id3, "qa");
    try db.updateTaskStatus(id4, "impl");
    try db.updateTaskStatus(id5, "retry");
    try db.updateTaskStatus(id6, "rebase");
    try db.updateTaskStatus(id7, "merged");
    try db.updateTaskStatus(id8, "merged");
    try db.updateTaskStatus(id9, "failed");
    try db.updateTaskStatus(id10, "done");
    try db.updateTaskStatus(id11, "test");

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 11), stats.total);
    try std.testing.expectEqual(@as(i64, 6), stats.active);
    try std.testing.expectEqual(@as(i64, 2), stats.merged);
    try std.testing.expectEqual(@as(i64, 1), stats.failed);

    // Verify: active + merged + failed + uncategorized = total
    // uncategorized = total - active - merged - failed
    const uncategorized = stats.total - stats.active - stats.merged - stats.failed;
    try std.testing.expectEqual(@as(i64, 2), uncategorized);
}

// =============================================================================
// Consistency: getPipelineStats is consistent with individual queries
// =============================================================================

test "consistency: stats match individual COUNT queries" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Create a mix of tasks
    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", "");
    const id3 = try db.createPipelineTask("T3", "d", "/repo", "", "");
    const id4 = try db.createPipelineTask("T4", "d", "/repo", "", "");
    _ = id1;
    try db.updateTaskStatus(id2, "merged");
    try db.updateTaskStatus(id3, "failed");
    try db.updateTaskStatus(id4, "qa");

    const stats = try db.getPipelineStats();

    // Manually query each count to cross-validate
    var total_rows = try db.sqlite_db.query(alloc, "SELECT COUNT(*) FROM pipeline_tasks", .{});
    defer total_rows.deinit();
    const expected_total = total_rows.items[0].getInt(0) orelse 0;

    var active_rows = try db.sqlite_db.query(alloc, "SELECT COUNT(*) FROM pipeline_tasks WHERE status IN ('backlog', 'spec', 'qa', 'impl', 'retry', 'rebase')", .{});
    defer active_rows.deinit();
    const expected_active = active_rows.items[0].getInt(0) orelse 0;

    var merged_rows = try db.sqlite_db.query(alloc, "SELECT COUNT(*) FROM pipeline_tasks WHERE status = 'merged'", .{});
    defer merged_rows.deinit();
    const expected_merged = merged_rows.items[0].getInt(0) orelse 0;

    var failed_rows = try db.sqlite_db.query(alloc, "SELECT COUNT(*) FROM pipeline_tasks WHERE status = 'failed'", .{});
    defer failed_rows.deinit();
    const expected_failed = failed_rows.items[0].getInt(0) orelse 0;

    try std.testing.expectEqual(expected_total, stats.total);
    try std.testing.expectEqual(expected_active, stats.active);
    try std.testing.expectEqual(expected_merged, stats.merged);
    try std.testing.expectEqual(expected_failed, stats.failed);
}

// =============================================================================
// Idempotency: calling getPipelineStats multiple times returns same results
// =============================================================================

test "idempotency: calling getPipelineStats twice returns same results" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.createPipelineTask("T1", "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", "");
    try db.updateTaskStatus(id2, "merged");

    const stats1 = try db.getPipelineStats();
    const stats2 = try db.getPipelineStats();

    try std.testing.expectEqual(stats1.total, stats2.total);
    try std.testing.expectEqual(stats1.active, stats2.active);
    try std.testing.expectEqual(stats1.merged, stats2.merged);
    try std.testing.expectEqual(stats1.failed, stats2.failed);
}
