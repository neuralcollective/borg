// Tests for spec #55: SQLite-backed run history log with queryable stats
//
// Covers acceptance criteria from spec.md:
//   AC1  — Schema migration: run_history table and indexes exist after Db.init
//   AC2  — logRunStart round-trip: fields match on freshly-inserted running row
//   AC3  — logRunFinish updates row: status, bytes_out, finished_at, duration_s
//   AC4  — getRecentRuns ordering: newest-first (started_at DESC, id DESC)
//   AC5  — getRecentRuns status filter: exact match, no cross-contamination
//   AC6  — getRecentRuns limit respected
//   AC7  — getRecentRuns empty DB returns zero-length slice
//   AC8  — getRunStats counts: total, done, failed, running
//   AC9  — getRunStats avg_duration_s: average over completed rows only
//   AC10 — getRunStats empty DB returns all-zero RunStats
//   AC15 — RunHistoryEntry.deinit frees all heap-allocated string fields
//   Edge — logRunFinish on unknown id is a no-op
//   Edge — getRecentRuns with unrecognised status filter returns zero rows
//   Edge — tie-breaking: same started_at rows ordered by id DESC
//   Edge — orphaned running rows returned by getRecentRuns and counted by getRunStats
//   Edge — very long error_msg is stored and retrieved correctly
//
// To include in the build, add to the test block in src/db.zig:
//   _ = @import("db_run_history_test.zig");

const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;
const RunHistoryEntry = db_mod.RunHistoryEntry;
const RunStats = db_mod.RunStats;

// =============================================================================
// Helpers
// =============================================================================

/// Free every heap-allocated string field of a RunHistoryEntry slice, then
/// free the outer slice.  Pass the same allocator that was given to
/// getRecentRuns.
fn freeEntries(alloc: std.mem.Allocator, entries: []RunHistoryEntry) void {
    for (entries) |e| e.deinit(alloc);
    alloc.free(entries);
}

/// Force started_at for the most-recently inserted run_history row.
fn forceStartedAt(db: *Db, row_id: i64, ts: []const u8) !void {
    try db.sqlite_db.execute(
        "UPDATE run_history SET started_at = ?1 WHERE id = ?2",
        .{ ts, row_id },
    );
}

// =============================================================================
// AC1 — Schema migration: table and indexes exist after Db.init
// =============================================================================

test "AC1: run_history table exists on fresh in-memory DB" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    // Querying the table must not error
    var rows = try db.sqlite_db.query(
        arena.allocator(),
        "SELECT COUNT(*) FROM run_history",
        .{},
    );
    defer rows.deinit();
    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
}

test "AC1: idx_run_history_started index exists" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    var rows = try db.sqlite_db.query(
        arena.allocator(),
        "SELECT name FROM sqlite_master WHERE type='index' AND name='idx_run_history_started'",
        .{},
    );
    defer rows.deinit();
    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
}

test "AC1: idx_run_history_status index exists" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    var rows = try db.sqlite_db.query(
        arena.allocator(),
        "SELECT name FROM sqlite_master WHERE type='index' AND name='idx_run_history_status'",
        .{},
    );
    defer rows.deinit();
    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
}

// =============================================================================
// AC2 — logRunStart round-trip: fields on freshly-inserted running row
// =============================================================================

test "AC2: logRunStart returns a positive id" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.logRunStart(0, "impl", "/repo");
    try std.testing.expect(id > 0);
}

test "AC2: logRunStart ids are strictly increasing" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id1 = try db.logRunStart(0, "spec", "/r");
    const id2 = try db.logRunStart(0, "qa",   "/r");
    try std.testing.expect(id2 > id1);
}

test "AC2: logRunStart row has status 'running'" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.logRunStart(0, "impl", "/repo");
    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(usize, 1), entries.len);
    try std.testing.expectEqualStrings("running", entries[0].status);
}

test "AC2: logRunStart row has correct phase" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.logRunStart(0, "impl", "/repo");
    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqualStrings("impl", entries[0].phase);
}

test "AC2: logRunStart row has correct repo_path" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.logRunStart(0, "impl", "/repo");
    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqualStrings("/repo", entries[0].repo_path);
}

test "AC2: logRunStart row has bytes_out == 0" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.logRunStart(0, "impl", "/repo");
    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(i64, 0), entries[0].bytes_out);
}

test "AC2: logRunStart row has empty error_msg" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.logRunStart(0, "impl", "/repo");
    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqualStrings("", entries[0].error_msg);
}

test "AC2: logRunStart row has empty finished_at" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.logRunStart(0, "impl", "/repo");
    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqualStrings("", entries[0].finished_at);
}

test "AC2: logRunStart row has duration_s == 0" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.logRunStart(0, "impl", "/repo");
    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(i64, 0), entries[0].duration_s);
}

test "AC2: logRunStart stores task_id correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.logRunStart(42, "spec", "/r");
    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(i64, 42), entries[0].task_id);
}

test "AC2: logRunStart row has non-empty started_at" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.logRunStart(0, "impl", "/repo");
    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expect(entries[0].started_at.len > 0);
}

// =============================================================================
// AC3 — logRunFinish updates row
// =============================================================================

test "AC3: logRunFinish sets status to 'done'" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.logRunStart(0, "impl", "/repo");
    try db.logRunFinish(id, "done", 4096, "");

    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqualStrings("done", entries[0].status);
}

test "AC3: logRunFinish stores bytes_out" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.logRunStart(0, "impl", "/repo");
    try db.logRunFinish(id, "done", 4096, "");

    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(i64, 4096), entries[0].bytes_out);
}

test "AC3: logRunFinish sets non-empty finished_at" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.logRunStart(0, "impl", "/repo");
    try db.logRunFinish(id, "done", 4096, "");

    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expect(entries[0].finished_at.len > 0);
}

test "AC3: logRunFinish sets duration_s >= 0" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.logRunStart(0, "impl", "/repo");
    try db.logRunFinish(id, "done", 4096, "");

    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expect(entries[0].duration_s >= 0);
}

test "AC3: logRunFinish stores error_msg" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.logRunStart(0, "qa", "/repo");
    try db.logRunFinish(id, "failed", 512, "test exit 1");

    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqualStrings("test exit 1", entries[0].error_msg);
}

test "AC3: logRunFinish with 'failed' status stores correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.logRunStart(0, "qa", "/repo");
    try db.logRunFinish(id, "failed", 512, "test exit 1");

    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqualStrings("failed", entries[0].status);
}

// =============================================================================
// AC4 — getRecentRuns ordering: newest-first
// =============================================================================

test "AC4: getRecentRuns returns three rows newest-first" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id1 = try db.logRunStart(0, "spec", "/r");
    try forceStartedAt(&db, id1, "2026-01-01 00:00:01");
    const id2 = try db.logRunStart(0, "qa",   "/r");
    try forceStartedAt(&db, id2, "2026-01-01 00:00:02");
    const id3 = try db.logRunStart(0, "impl", "/r");
    try forceStartedAt(&db, id3, "2026-01-01 00:00:03");

    // Finish all so we can use phase as the ordering witness
    try db.logRunFinish(id1, "done", 0, "");
    try db.logRunFinish(id2, "done", 0, "");
    try db.logRunFinish(id3, "done", 0, "");

    const entries = try db.getRecentRuns(arena.allocator(), 10, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(usize, 3), entries.len);
    // Newest (00:03 / impl) must come first
    try std.testing.expectEqualStrings("impl", entries[0].phase);
    try std.testing.expectEqualStrings("qa",   entries[1].phase);
    try std.testing.expectEqualStrings("spec", entries[2].phase);
}

// =============================================================================
// AC5 — getRecentRuns status filter
// =============================================================================

test "AC5: getRecentRuns with status='done' returns only done rows" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const done_id = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(done_id, "done", 100, "");
    const fail_id = try db.logRunStart(0, "qa", "/r");
    try db.logRunFinish(fail_id, "failed", 50, "boom");

    const entries = try db.getRecentRuns(arena.allocator(), 10, "done");
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(usize, 1), entries.len);
    try std.testing.expectEqualStrings("done", entries[0].status);
}

test "AC5: getRecentRuns with status='failed' returns only failed rows" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const done_id = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(done_id, "done", 100, "");
    const fail_id = try db.logRunStart(0, "qa", "/r");
    try db.logRunFinish(fail_id, "failed", 50, "boom");

    const entries = try db.getRecentRuns(arena.allocator(), 10, "failed");
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(usize, 1), entries.len);
    try std.testing.expectEqualStrings("failed", entries[0].status);
}

test "AC5: getRecentRuns with status='running' returns only running rows" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.logRunStart(0, "spec", "/r");  // stays running
    const done_id = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(done_id, "done", 0, "");

    const entries = try db.getRecentRuns(arena.allocator(), 10, "running");
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(usize, 1), entries.len);
    try std.testing.expectEqualStrings("running", entries[0].status);
}

test "AC5: getRecentRuns with null filter returns all statuses" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.logRunStart(0, "spec", "/r");
    const done_id = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(done_id, "done", 0, "");
    const fail_id = try db.logRunStart(0, "qa", "/r");
    try db.logRunFinish(fail_id, "failed", 0, "");

    const entries = try db.getRecentRuns(arena.allocator(), 10, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(usize, 3), entries.len);
}

// =============================================================================
// AC6 — getRecentRuns limit respected
// =============================================================================

test "AC6: getRecentRuns limit=3 returns exactly 3 of 5 rows" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    var i: usize = 0;
    while (i < 5) : (i += 1) {
        _ = try db.logRunStart(0, "impl", "/r");
    }

    const entries = try db.getRecentRuns(arena.allocator(), 3, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(usize, 3), entries.len);
}

test "AC6: getRecentRuns limit=1 returns exactly 1 row" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.logRunStart(0, "impl", "/r");
    _ = try db.logRunStart(0, "qa",   "/r");

    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(usize, 1), entries.len);
}

// =============================================================================
// AC7 — getRecentRuns on empty DB
// =============================================================================

test "AC7: getRecentRuns on empty DB returns zero-length slice" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const entries = try db.getRecentRuns(arena.allocator(), 20, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(usize, 0), entries.len);
}

test "AC7: getRecentRuns with status filter on empty DB returns zero-length slice" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const entries = try db.getRecentRuns(arena.allocator(), 20, "done");
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(usize, 0), entries.len);
}

// =============================================================================
// AC8 — getRunStats counts
// =============================================================================

test "AC8: getRunStats total count is correct" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const d1 = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(d1, "done", 0, "");
    const d2 = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(d2, "done", 0, "");
    const f1 = try db.logRunStart(0, "qa", "/r");
    try db.logRunFinish(f1, "failed", 0, "err");
    _ = try db.logRunStart(0, "spec", "/r"); // running

    const stats = try db.getRunStats(arena.allocator());
    try std.testing.expectEqual(@as(i64, 4), stats.total);
}

test "AC8: getRunStats done count is correct" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const d1 = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(d1, "done", 0, "");
    const d2 = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(d2, "done", 0, "");
    const f1 = try db.logRunStart(0, "qa", "/r");
    try db.logRunFinish(f1, "failed", 0, "err");
    _ = try db.logRunStart(0, "spec", "/r");

    const stats = try db.getRunStats(arena.allocator());
    try std.testing.expectEqual(@as(i64, 2), stats.done);
}

test "AC8: getRunStats failed count is correct" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const d1 = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(d1, "done", 0, "");
    const d2 = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(d2, "done", 0, "");
    const f1 = try db.logRunStart(0, "qa", "/r");
    try db.logRunFinish(f1, "failed", 0, "err");
    _ = try db.logRunStart(0, "spec", "/r");

    const stats = try db.getRunStats(arena.allocator());
    try std.testing.expectEqual(@as(i64, 1), stats.failed);
}

test "AC8: getRunStats running count is correct" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const d1 = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(d1, "done", 0, "");
    const d2 = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(d2, "done", 0, "");
    const f1 = try db.logRunStart(0, "qa", "/r");
    try db.logRunFinish(f1, "failed", 0, "err");
    _ = try db.logRunStart(0, "spec", "/r");

    const stats = try db.getRunStats(arena.allocator());
    try std.testing.expectEqual(@as(i64, 1), stats.running);
}

test "AC8: getRunStats total_bytes_out sums all rows" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id1 = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(id1, "done", 1000, "");
    const id2 = try db.logRunStart(0, "qa", "/r");
    try db.logRunFinish(id2, "done", 2000, "");

    const stats = try db.getRunStats(arena.allocator());
    try std.testing.expectEqual(@as(i64, 3000), stats.total_bytes_out);
}

// =============================================================================
// AC9 — getRunStats avg_duration_s over completed rows only
// =============================================================================

test "AC9: avg_duration_s is average of done and failed durations" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id1 = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(id1, "done", 0, "");
    // Overwrite duration_s directly for determinism
    try db.sqlite_db.execute(
        "UPDATE run_history SET duration_s = 10 WHERE id = ?1",
        .{id1},
    );

    const id2 = try db.logRunStart(0, "qa", "/r");
    try db.logRunFinish(id2, "failed", 0, "");
    try db.sqlite_db.execute(
        "UPDATE run_history SET duration_s = 20 WHERE id = ?1",
        .{id2},
    );

    // Add a running row (duration_s should be excluded from average)
    _ = try db.logRunStart(0, "spec", "/r");

    const stats = try db.getRunStats(arena.allocator());
    try std.testing.expectEqual(@as(i64, 15), stats.avg_duration_s);
}

test "AC9: avg_duration_s excludes running rows from average" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    // One done run with duration 100
    const id1 = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(id1, "done", 0, "");
    try db.sqlite_db.execute(
        "UPDATE run_history SET duration_s = 100 WHERE id = ?1",
        .{id1},
    );

    // Two running rows (duration_s = 0, must not skew average)
    _ = try db.logRunStart(0, "qa",   "/r");
    _ = try db.logRunStart(0, "spec", "/r");

    const stats = try db.getRunStats(arena.allocator());
    try std.testing.expectEqual(@as(i64, 100), stats.avg_duration_s);
}

test "AC9: avg_duration_s is 0 when only running rows exist" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.logRunStart(0, "spec", "/r");
    _ = try db.logRunStart(0, "impl", "/r");

    const stats = try db.getRunStats(arena.allocator());
    try std.testing.expectEqual(@as(i64, 0), stats.avg_duration_s);
}

// =============================================================================
// AC10 — getRunStats on empty DB
// =============================================================================

test "AC10: getRunStats on empty DB returns zero total" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const stats = try db.getRunStats(arena.allocator());
    try std.testing.expectEqual(@as(i64, 0), stats.total);
}

test "AC10: getRunStats on empty DB returns zero done" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const stats = try db.getRunStats(arena.allocator());
    try std.testing.expectEqual(@as(i64, 0), stats.done);
}

test "AC10: getRunStats on empty DB returns zero failed" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const stats = try db.getRunStats(arena.allocator());
    try std.testing.expectEqual(@as(i64, 0), stats.failed);
}

test "AC10: getRunStats on empty DB returns zero running" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const stats = try db.getRunStats(arena.allocator());
    try std.testing.expectEqual(@as(i64, 0), stats.running);
}

test "AC10: getRunStats on empty DB returns zero avg_duration_s" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const stats = try db.getRunStats(arena.allocator());
    try std.testing.expectEqual(@as(i64, 0), stats.avg_duration_s);
}

test "AC10: getRunStats on empty DB returns zero total_bytes_out" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const stats = try db.getRunStats(arena.allocator());
    try std.testing.expectEqual(@as(i64, 0), stats.total_bytes_out);
}

// =============================================================================
// AC15 — RunHistoryEntry.deinit frees all heap-allocated string fields
// =============================================================================

test "AC15: deinit frees all string fields without leak" {
    // Use the raw testing allocator (not arena) so the GPA underneath
    // std.testing.allocator can detect any leaked memory.
    const alloc = std.testing.allocator;

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.logRunStart(7, "impl", "/my/repo");
    try db.logRunFinish(id, "done", 1234, "no errors");

    const entries = try db.getRecentRuns(alloc, 1, null);
    // Free explicitly via deinit — no arena safety net
    for (entries) |e| e.deinit(alloc);
    alloc.free(entries);
    // If any field was not freed the GPA-backed std.testing.allocator will
    // report a leak and fail the test.
}

test "AC15: deinit on running row (empty finished_at) does not double-free" {
    const alloc = std.testing.allocator;

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.logRunStart(0, "spec", "/r");

    const entries = try db.getRecentRuns(alloc, 1, null);
    for (entries) |e| e.deinit(alloc);
    alloc.free(entries);
}

// =============================================================================
// Edge — logRunFinish on unknown id is a no-op (no error, zero rows changed)
// =============================================================================

test "Edge: logRunFinish on unknown id returns no error" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    // No row with id 9999 exists
    try db.logRunFinish(9999, "done", 0, "");

    // Table must still be empty
    const entries = try db.getRecentRuns(arena.allocator(), 10, null);
    defer freeEntries(arena.allocator(), entries);
    try std.testing.expectEqual(@as(usize, 0), entries.len);
}

// =============================================================================
// Edge — unrecognised status filter returns zero rows
// =============================================================================

test "Edge: getRecentRuns with unrecognised status returns zero rows" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(id, "done", 0, "");

    const entries = try db.getRecentRuns(arena.allocator(), 10, "bogus_status");
    defer freeEntries(arena.allocator(), entries);
    try std.testing.expectEqual(@as(usize, 0), entries.len);
}

// =============================================================================
// Edge — tie-breaking: rows with identical started_at ordered by id DESC
// =============================================================================

test "Edge: same started_at rows are ordered by id DESC" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const same_ts = "2026-01-15 12:00:00";

    const id1 = try db.logRunStart(0, "spec", "/r");
    try forceStartedAt(&db, id1, same_ts);
    const id2 = try db.logRunStart(0, "qa",   "/r");
    try forceStartedAt(&db, id2, same_ts);
    const id3 = try db.logRunStart(0, "impl", "/r");
    try forceStartedAt(&db, id3, same_ts);

    const entries = try db.getRecentRuns(arena.allocator(), 10, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(usize, 3), entries.len);
    // Highest id (impl, id3) must come first
    try std.testing.expectEqualStrings("impl", entries[0].phase);
    try std.testing.expectEqualStrings("qa",   entries[1].phase);
    try std.testing.expectEqualStrings("spec", entries[2].phase);
}

// =============================================================================
// Edge — orphaned running rows persist and are counted correctly
// =============================================================================

test "Edge: orphaned running rows are returned by getRecentRuns" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    // Simulate crash: logRunStart called, logRunFinish never called
    _ = try db.logRunStart(0, "impl", "/r");

    const entries = try db.getRecentRuns(arena.allocator(), 10, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(usize, 1), entries.len);
    try std.testing.expectEqualStrings("running", entries[0].status);
}

test "Edge: orphaned running rows are counted under running in getRunStats" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.logRunStart(0, "impl", "/r");
    _ = try db.logRunStart(0, "qa",   "/r");

    const stats = try db.getRunStats(arena.allocator());
    try std.testing.expectEqual(@as(i64, 2), stats.running);
    try std.testing.expectEqual(@as(i64, 2), stats.total);
}

// =============================================================================
// Edge — very long error_msg is stored and retrieved correctly
// =============================================================================

test "Edge: very long error_msg is stored and retrieved" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    // Build a 4096-byte error message
    const long_msg = try arena.allocator().alloc(u8, 4096);
    @memset(long_msg, 'x');

    const id = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(id, "error", 0, long_msg);

    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(usize, 1), entries.len);
    try std.testing.expectEqual(@as(usize, 4096), entries[0].error_msg.len);
    try std.testing.expectEqualStrings(long_msg, entries[0].error_msg);
}

// =============================================================================
// Edge — 'error' status stored and retrieved correctly
// =============================================================================

test "Edge: logRunFinish with status='error' is stored correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.logRunStart(0, "impl", "/r");
    try db.logRunFinish(id, "error", 0, "unexpected panic");

    const entries = try db.getRecentRuns(arena.allocator(), 1, "error");
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(@as(usize, 1), entries.len);
    try std.testing.expectEqualStrings("error", entries[0].status);
    try std.testing.expectEqualStrings("unexpected panic", entries[0].error_msg);
}

// =============================================================================
// Edge — getRecentRuns id field matches logRunStart return value
// =============================================================================

test "Edge: RunHistoryEntry.id matches the id returned by logRunStart" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.logRunStart(0, "spec", "/r");

    const entries = try db.getRecentRuns(arena.allocator(), 1, null);
    defer freeEntries(arena.allocator(), entries);

    try std.testing.expectEqual(id, entries[0].id);
}
