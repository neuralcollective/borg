// Tests for db.zig task output storage and retrieval.
//
// Covers storeTaskOutput(), storeTaskOutputFull(), and getTaskOutputs().
// Every acceptance criterion and edge case from spec.md (Task #34) is tested.
//
// To include in the build, add to the test block in src/db.zig:
//   _ = @import("db_task_output_test.zig");

const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;

// =============================================================================
// Helpers
// =============================================================================

/// Force the created_at column of a specific task_output row to a fixed
/// timestamp.  Used to make ordering tests deterministic: SQLite's
/// datetime('now') has 1-second resolution, so multiple inserts in the same
/// second share identical created_at values and their relative ORDER BY
/// position is unspecified.
fn forceCreatedAt(db: *Db, row_id: i64, ts: []const u8) !void {
    try db.sqlite_db.execute(
        "UPDATE task_outputs SET created_at = ?1 WHERE id = ?2",
        .{ ts, row_id },
    );
}

/// Free every heap-allocated string field of each TaskOutput, then free the
/// outer slice.  Pass the same allocator that was given to getTaskOutputs.
fn freeOutputs(alloc: std.mem.Allocator, outputs: []Db.TaskOutput) void {
    for (outputs) |o| {
        alloc.free(o.phase);
        alloc.free(o.output);
        alloc.free(o.raw_stream);
        alloc.free(o.created_at);
    }
    alloc.free(outputs);
}

// =============================================================================
// AC1 — Empty result for unknown task_id
// =============================================================================

test "AC1: getTaskOutputs on unknown task_id returns zero-length slice" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const outputs = try db.getTaskOutputs(alloc, 9999);
    try std.testing.expectEqual(@as(usize, 0), outputs.len);
}

test "AC1: getTaskOutputs on fresh DB with no rows returns zero-length slice" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // No rows inserted at all
    const outputs = try db.getTaskOutputs(alloc, 1);
    try std.testing.expectEqual(@as(usize, 0), outputs.len);
}

// =============================================================================
// AC2 — storeTaskOutput round-trip: single row
// =============================================================================

test "AC2: storeTaskOutput stores phase correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(1, "spec", "hello", 0);

    const outputs = try db.getTaskOutputs(alloc, 1);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqualStrings("spec", outputs[0].phase);
}

test "AC2: storeTaskOutput stores output correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(1, "spec", "hello", 0);

    const outputs = try db.getTaskOutputs(alloc, 1);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqualStrings("hello", outputs[0].output);
}

test "AC2: storeTaskOutput raw_stream defaults to empty string" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(1, "spec", "hello", 0);

    const outputs = try db.getTaskOutputs(alloc, 1);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqualStrings("", outputs[0].raw_stream);
}

test "AC2: storeTaskOutput exit_code zero is stored correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(1, "spec", "hello", 0);

    const outputs = try db.getTaskOutputs(alloc, 1);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(i64, 0), outputs[0].exit_code);
}

test "AC2: storeTaskOutput created_at is non-empty" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(1, "spec", "hello", 0);

    const outputs = try db.getTaskOutputs(alloc, 1);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expect(outputs[0].created_at.len > 0);
}

// =============================================================================
// AC3 — storeTaskOutputFull round-trip: single row
// =============================================================================

test "AC3: storeTaskOutputFull stores phase correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutputFull(2, "impl", "out", "ndjson-data", 1);

    const outputs = try db.getTaskOutputs(alloc, 2);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqualStrings("impl", outputs[0].phase);
}

test "AC3: storeTaskOutputFull stores output correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutputFull(2, "impl", "out", "ndjson-data", 1);

    const outputs = try db.getTaskOutputs(alloc, 2);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqualStrings("out", outputs[0].output);
}

test "AC3: storeTaskOutputFull stores raw_stream correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutputFull(2, "impl", "out", "ndjson-data", 1);

    const outputs = try db.getTaskOutputs(alloc, 2);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqualStrings("ndjson-data", outputs[0].raw_stream);
}

test "AC3: storeTaskOutputFull stores exit_code correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutputFull(2, "impl", "out", "ndjson-data", 1);

    const outputs = try db.getTaskOutputs(alloc, 2);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(i64, 1), outputs[0].exit_code);
}

// =============================================================================
// AC4 — Multiple chunks returned in ascending timestamp order
// =============================================================================

test "AC4: getTaskOutputs returns three rows ordered by created_at ASC" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const task_id: i64 = 3;

    try db.storeTaskOutput(task_id, "first", "a", 0);
    const id1 = db.sqlite_db.lastInsertRowId();
    try forceCreatedAt(&db, id1, "2026-01-01 00:00:01");

    try db.storeTaskOutput(task_id, "second", "b", 0);
    const id2 = db.sqlite_db.lastInsertRowId();
    try forceCreatedAt(&db, id2, "2026-01-01 00:00:02");

    try db.storeTaskOutput(task_id, "third", "c", 0);
    const id3 = db.sqlite_db.lastInsertRowId();
    try forceCreatedAt(&db, id3, "2026-01-01 00:00:03");

    const outputs = try db.getTaskOutputs(alloc, task_id);
    try std.testing.expectEqual(@as(usize, 3), outputs.len);
    try std.testing.expectEqualStrings("first", outputs[0].phase);
    try std.testing.expectEqualStrings("second", outputs[1].phase);
    try std.testing.expectEqualStrings("third", outputs[2].phase);
}

test "AC4: output content is preserved in order" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const task_id: i64 = 4;

    try db.storeTaskOutput(task_id, "p1", "content-one", 0);
    const id1 = db.sqlite_db.lastInsertRowId();
    try forceCreatedAt(&db, id1, "2026-01-01 00:00:01");

    try db.storeTaskOutput(task_id, "p2", "content-two", 0);
    const id2 = db.sqlite_db.lastInsertRowId();
    try forceCreatedAt(&db, id2, "2026-01-01 00:00:02");

    const outputs = try db.getTaskOutputs(alloc, task_id);
    try std.testing.expectEqual(@as(usize, 2), outputs.len);
    try std.testing.expectEqualStrings("content-one", outputs[0].output);
    try std.testing.expectEqualStrings("content-two", outputs[1].output);
}

test "AC4: reverse timestamp insertion returns earliest first" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const task_id: i64 = 5;

    // Insert in reverse chronological order, then verify ASC retrieval
    try db.storeTaskOutput(task_id, "later", "y", 0);
    const id1 = db.sqlite_db.lastInsertRowId();
    try forceCreatedAt(&db, id1, "2026-01-01 00:00:02");

    try db.storeTaskOutput(task_id, "earlier", "x", 0);
    const id2 = db.sqlite_db.lastInsertRowId();
    try forceCreatedAt(&db, id2, "2026-01-01 00:00:01");

    const outputs = try db.getTaskOutputs(alloc, task_id);
    try std.testing.expectEqual(@as(usize, 2), outputs.len);
    try std.testing.expectEqualStrings("earlier", outputs[0].phase);
    try std.testing.expectEqualStrings("later", outputs[1].phase);
}

// =============================================================================
// AC5 — Task isolation: outputs are scoped to their task_id
// =============================================================================

test "AC5: outputs for task A and task B have correct counts" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const task_a: i64 = 10;
    const task_b: i64 = 20;

    try db.storeTaskOutput(task_a, "spec", "a1", 0);
    try db.storeTaskOutput(task_a, "impl", "a2", 0);
    try db.storeTaskOutput(task_b, "spec", "b1", 0);
    try db.storeTaskOutput(task_b, "impl", "b2", 0);
    try db.storeTaskOutput(task_b, "qa", "b3", 0);

    const outputs_a = try db.getTaskOutputs(alloc, task_a);
    try std.testing.expectEqual(@as(usize, 2), outputs_a.len);

    const outputs_b = try db.getTaskOutputs(alloc, task_b);
    try std.testing.expectEqual(@as(usize, 3), outputs_b.len);
}

test "AC5: task A outputs contain only task A content" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const task_a: i64 = 11;
    const task_b: i64 = 12;

    try db.storeTaskOutput(task_a, "only-a", "a-content", 0);
    try db.storeTaskOutput(task_b, "only-b", "b-content", 0);

    const outputs_a = try db.getTaskOutputs(alloc, task_a);
    try std.testing.expectEqual(@as(usize, 1), outputs_a.len);
    try std.testing.expectEqualStrings("only-a", outputs_a[0].phase);
    try std.testing.expectEqualStrings("a-content", outputs_a[0].output);
}

test "AC5: task B outputs contain only task B content" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const task_a: i64 = 13;
    const task_b: i64 = 14;

    try db.storeTaskOutput(task_a, "only-a", "a-content", 0);
    try db.storeTaskOutput(task_b, "only-b", "b-content", 0);

    const outputs_b = try db.getTaskOutputs(alloc, task_b);
    try std.testing.expectEqual(@as(usize, 1), outputs_b.len);
    try std.testing.expectEqualStrings("only-b", outputs_b[0].phase);
    try std.testing.expectEqualStrings("b-content", outputs_b[0].output);
}

// =============================================================================
// AC6 — output is truncated at 32 000 bytes
// =============================================================================

test "AC6: output of 40000 bytes is truncated to 32000 on retrieval" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const big_output = [_]u8{'x'} ** 40_000;
    try db.storeTaskOutput(15, "qa", &big_output, 0);

    const outputs = try db.getTaskOutputs(alloc, 15);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(usize, 32_000), outputs[0].output.len);
}

test "AC6: truncated output retains the first 32000 bytes unchanged" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const big_output = [_]u8{'z'} ** 40_000;
    try db.storeTaskOutput(16, "qa", &big_output, 0);

    const outputs = try db.getTaskOutputs(alloc, 16);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    for (outputs[0].output) |byte| {
        try std.testing.expectEqual(@as(u8, 'z'), byte);
    }
}

test "AC6: raw_stream is not affected by output truncation" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const big_output = [_]u8{'x'} ** 40_000;
    try db.storeTaskOutput(17, "qa", &big_output, 0);

    const outputs = try db.getTaskOutputs(alloc, 17);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqualStrings("", outputs[0].raw_stream);
}

// =============================================================================
// AC7 — Non-zero exit code is preserved
// =============================================================================

test "AC7: exit_code 42 stored via storeTaskOutput is retrieved correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(18, "qa", "", 42);

    const outputs = try db.getTaskOutputs(alloc, 18);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(i64, 42), outputs[0].exit_code);
}

test "AC7: exit_code 1 stored via storeTaskOutputFull is retrieved correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutputFull(19, "impl", "x", "raw", 1);

    const outputs = try db.getTaskOutputs(alloc, 19);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(i64, 1), outputs[0].exit_code);
}

test "AC7: exit_code 127 (command not found) is preserved" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(20, "impl", "", 127);

    const outputs = try db.getTaskOutputs(alloc, 20);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(i64, 127), outputs[0].exit_code);
}

// =============================================================================
// AC8 — storeTaskOutputFull raw_stream is not truncated
// =============================================================================

test "AC8: raw_stream of 50000 bytes is stored and retrieved in full" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const big_raw = [_]u8{'r'} ** 50_000;
    try db.storeTaskOutputFull(21, "impl", "small", &big_raw, 0);

    const outputs = try db.getTaskOutputs(alloc, 21);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(usize, 50_000), outputs[0].raw_stream.len);
    try std.testing.expectEqualStrings("small", outputs[0].output);
}

test "AC8: truncation of output does not shorten raw_stream" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const big_output = [_]u8{'o'} ** 40_000;
    const big_raw = [_]u8{'r'} ** 50_000;
    try db.storeTaskOutputFull(22, "impl", &big_output, &big_raw, 0);

    const outputs = try db.getTaskOutputs(alloc, 22);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(usize, 32_000), outputs[0].output.len);
    try std.testing.expectEqual(@as(usize, 50_000), outputs[0].raw_stream.len);
}

test "AC8: raw_stream bytes are preserved correctly after round-trip" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const big_raw = [_]u8{'q'} ** 50_000;
    try db.storeTaskOutputFull(23, "impl", "ok", &big_raw, 0);

    const outputs = try db.getTaskOutputs(alloc, 23);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    for (outputs[0].raw_stream) |byte| {
        try std.testing.expectEqual(@as(u8, 'q'), byte);
    }
}

// =============================================================================
// AC9 — Memory: no leak when freeing returned slice
// =============================================================================

test "AC9: getTaskOutputs strings freed individually without leak" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const db_alloc = arena.allocator();

    var db = try Db.init(db_alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(30, "spec", "hello", 0);
    try db.storeTaskOutputFull(30, "impl", "world", "ndjson", 1);

    // Use std.testing.allocator for output strings so leaks are detected
    const outputs = try db.getTaskOutputs(std.testing.allocator, 30);
    defer freeOutputs(std.testing.allocator, outputs);

    try std.testing.expectEqual(@as(usize, 2), outputs.len);
}

test "AC9: freeing empty result slice does not leak" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const db_alloc = arena.allocator();

    var db = try Db.init(db_alloc, ":memory:");
    defer db.deinit();

    const outputs = try db.getTaskOutputs(std.testing.allocator, 9998);
    defer freeOutputs(std.testing.allocator, outputs);

    try std.testing.expectEqual(@as(usize, 0), outputs.len);
}

test "AC9: all four string fields are individually freeable without double-free" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const db_alloc = arena.allocator();

    var db = try Db.init(db_alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutputFull(31, "phase-val", "output-val", "raw-val", 7);

    const outputs = try db.getTaskOutputs(std.testing.allocator, 31);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);

    // Free each field individually — std.testing.allocator detects any leak
    // or double-free
    std.testing.allocator.free(outputs[0].phase);
    std.testing.allocator.free(outputs[0].output);
    std.testing.allocator.free(outputs[0].raw_stream);
    std.testing.allocator.free(outputs[0].created_at);
    std.testing.allocator.free(outputs);
}

test "AC9: multiple rows can each be freed without leak" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const db_alloc = arena.allocator();

    var db = try Db.init(db_alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(32, "s1", "o1", 0);
    try db.storeTaskOutput(32, "s2", "o2", 1);
    try db.storeTaskOutput(32, "s3", "o3", 2);

    const outputs = try db.getTaskOutputs(std.testing.allocator, 32);
    defer freeOutputs(std.testing.allocator, outputs);

    try std.testing.expectEqual(@as(usize, 3), outputs.len);
}

// =============================================================================
// AC10 — TaskOutput struct shape is unchanged
// =============================================================================

test "AC10: TaskOutput has exactly 6 fields" {
    const info = @typeInfo(Db.TaskOutput);
    try std.testing.expectEqual(@as(usize, 6), info.@"struct".fields.len);
}

test "AC10: TaskOutput field id has type i64" {
    const info = @typeInfo(Db.TaskOutput);
    var found = false;
    for (info.@"struct".fields) |f| {
        if (std.mem.eql(u8, f.name, "id")) {
            found = true;
            try std.testing.expect(f.type == i64);
        }
    }
    try std.testing.expect(found);
}

test "AC10: TaskOutput field exit_code has type i64" {
    const info = @typeInfo(Db.TaskOutput);
    var found = false;
    for (info.@"struct".fields) |f| {
        if (std.mem.eql(u8, f.name, "exit_code")) {
            found = true;
            try std.testing.expect(f.type == i64);
        }
    }
    try std.testing.expect(found);
}

test "AC10: TaskOutput string fields have type []const u8" {
    const info = @typeInfo(Db.TaskOutput);
    const string_fields = [_][]const u8{ "phase", "output", "raw_stream", "created_at" };
    for (string_fields) |name| {
        var found = false;
        for (info.@"struct".fields) |f| {
            if (std.mem.eql(u8, f.name, name)) {
                found = true;
                try std.testing.expect(f.type == []const u8);
            }
        }
        try std.testing.expect(found);
    }
}

test "AC10: TaskOutput can be instantiated with all six fields" {
    const to = Db.TaskOutput{
        .id = 1,
        .phase = "spec",
        .output = "out",
        .raw_stream = "raw",
        .exit_code = 0,
        .created_at = "2026-01-01 00:00:00",
    };
    try std.testing.expectEqual(@as(i64, 1), to.id);
    try std.testing.expectEqualStrings("spec", to.phase);
    try std.testing.expectEqualStrings("out", to.output);
    try std.testing.expectEqualStrings("raw", to.raw_stream);
    try std.testing.expectEqual(@as(i64, 0), to.exit_code);
    try std.testing.expectEqualStrings("2026-01-01 00:00:00", to.created_at);
}

// =============================================================================
// AC11 — Mixed storeTaskOutput and storeTaskOutputFull in the same task
// =============================================================================

test "AC11: mixed store functions produce two rows" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const task_id: i64 = 40;
    try db.storeTaskOutput(task_id, "spec", "spec-out", 0);
    const id1 = db.sqlite_db.lastInsertRowId();
    try forceCreatedAt(&db, id1, "2026-01-01 00:00:01");

    try db.storeTaskOutputFull(task_id, "impl", "impl-out", "impl-raw", 0);
    const id2 = db.sqlite_db.lastInsertRowId();
    try forceCreatedAt(&db, id2, "2026-01-01 00:00:02");

    const outputs = try db.getTaskOutputs(alloc, task_id);
    try std.testing.expectEqual(@as(usize, 2), outputs.len);
}

test "AC11: storeTaskOutput row has empty raw_stream" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const task_id: i64 = 41;
    try db.storeTaskOutput(task_id, "spec", "spec-out", 0);
    const id1 = db.sqlite_db.lastInsertRowId();
    try forceCreatedAt(&db, id1, "2026-01-01 00:00:01");

    try db.storeTaskOutputFull(task_id, "impl", "impl-out", "impl-raw", 0);
    const id2 = db.sqlite_db.lastInsertRowId();
    try forceCreatedAt(&db, id2, "2026-01-01 00:00:02");

    const outputs = try db.getTaskOutputs(alloc, task_id);
    try std.testing.expectEqual(@as(usize, 2), outputs.len);
    try std.testing.expectEqualStrings("", outputs[0].raw_stream);
}

test "AC11: storeTaskOutputFull row has supplied raw_stream" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const task_id: i64 = 42;
    try db.storeTaskOutput(task_id, "spec", "spec-out", 0);
    const id1 = db.sqlite_db.lastInsertRowId();
    try forceCreatedAt(&db, id1, "2026-01-01 00:00:01");

    try db.storeTaskOutputFull(task_id, "impl", "impl-out", "impl-raw", 2);
    const id2 = db.sqlite_db.lastInsertRowId();
    try forceCreatedAt(&db, id2, "2026-01-01 00:00:02");

    const outputs = try db.getTaskOutputs(alloc, task_id);
    try std.testing.expectEqual(@as(usize, 2), outputs.len);
    try std.testing.expectEqualStrings("impl-raw", outputs[1].raw_stream);
    try std.testing.expectEqual(@as(i64, 0), outputs[0].exit_code);
    try std.testing.expectEqual(@as(i64, 2), outputs[1].exit_code);
}

test "AC11: mixed store phase names are preserved" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const task_id: i64 = 43;
    try db.storeTaskOutput(task_id, "spec", "a", 0);
    const id1 = db.sqlite_db.lastInsertRowId();
    try forceCreatedAt(&db, id1, "2026-01-01 00:00:01");

    try db.storeTaskOutputFull(task_id, "impl", "b", "raw-b", 0);
    const id2 = db.sqlite_db.lastInsertRowId();
    try forceCreatedAt(&db, id2, "2026-01-01 00:00:02");

    const outputs = try db.getTaskOutputs(alloc, task_id);
    try std.testing.expectEqual(@as(usize, 2), outputs.len);
    try std.testing.expectEqualStrings("spec", outputs[0].phase);
    try std.testing.expectEqualStrings("impl", outputs[1].phase);
}

// =============================================================================
// E1 — getTaskOutputs on task_id with no outputs returns empty
// =============================================================================

test "E1: task with outputs stored, different task_id returns empty" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Store output for task 100, query task 999
    try db.storeTaskOutput(100, "spec", "other", 0);

    const outputs = try db.getTaskOutputs(alloc, 999);
    try std.testing.expectEqual(@as(usize, 0), outputs.len);
}

// =============================================================================
// E2 — Multiple outputs inserted within same second: all rows present
// =============================================================================

test "E2: three inserts in same second all appear in result" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const task_id: i64 = 50;
    // All three share datetime('now') — potentially the same second
    try db.storeTaskOutput(task_id, "a", "1", 0);
    try db.storeTaskOutput(task_id, "b", "2", 0);
    try db.storeTaskOutput(task_id, "c", "3", 0);

    const outputs = try db.getTaskOutputs(alloc, task_id);
    try std.testing.expectEqual(@as(usize, 3), outputs.len);
}

// =============================================================================
// E3 — output exactly at 32 000-byte boundary: stored verbatim
// =============================================================================

test "E3: output of exactly 32000 bytes is not truncated" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const exact = [_]u8{'e'} ** 32_000;
    try db.storeTaskOutput(60, "spec", &exact, 0);

    const outputs = try db.getTaskOutputs(alloc, 60);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(usize, 32_000), outputs[0].output.len);
}

// =============================================================================
// E4 — output one byte below 32 000: stored verbatim
// =============================================================================

test "E4: output of 31999 bytes is stored verbatim" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const below = [_]u8{'b'} ** 31_999;
    try db.storeTaskOutput(61, "spec", &below, 0);

    const outputs = try db.getTaskOutputs(alloc, 61);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(usize, 31_999), outputs[0].output.len);
}

// =============================================================================
// E5 — Empty strings for phase, output, and raw_stream
// =============================================================================

test "E5: storeTaskOutputFull with all empty strings succeeds" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutputFull(70, "", "", "", 0);

    const outputs = try db.getTaskOutputs(alloc, 70);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqualStrings("", outputs[0].phase);
    try std.testing.expectEqualStrings("", outputs[0].output);
    try std.testing.expectEqualStrings("", outputs[0].raw_stream);
}

test "E5: storeTaskOutput with empty output is stored and retrieved" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(71, "spec", "", 0);

    const outputs = try db.getTaskOutputs(alloc, 71);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqualStrings("", outputs[0].output);
}

// =============================================================================
// E6 — Negative exit_code stored and retrieved correctly
// =============================================================================

test "E6: exit_code -1 is stored and retrieved correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(80, "impl", "", -1);

    const outputs = try db.getTaskOutputs(alloc, 80);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(i64, -1), outputs[0].exit_code);
}

test "E6: exit_code -255 is stored and retrieved correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutput(81, "impl", "", -255);

    const outputs = try db.getTaskOutputs(alloc, 81);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(i64, -255), outputs[0].exit_code);
}

// =============================================================================
// E7 — Two tasks sharing the same phase name remain independent
// =============================================================================

test "E7: tasks with same phase name are independently retrievable" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const task_a: i64 = 90;
    const task_b: i64 = 91;
    try db.storeTaskOutput(task_a, "spec", "output-for-a", 0);
    try db.storeTaskOutput(task_b, "spec", "output-for-b", 0);

    const outputs_a = try db.getTaskOutputs(alloc, task_a);
    const outputs_b = try db.getTaskOutputs(alloc, task_b);

    try std.testing.expectEqual(@as(usize, 1), outputs_a.len);
    try std.testing.expectEqual(@as(usize, 1), outputs_b.len);
    try std.testing.expectEqualStrings("output-for-a", outputs_a[0].output);
    try std.testing.expectEqualStrings("output-for-b", outputs_b[0].output);
}

// =============================================================================
// E8 — getTaskOutputs called twice returns identical results (side-effect-free)
// =============================================================================

test "E8: calling getTaskOutputs twice returns the same data" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const task_id: i64 = 95;
    try db.storeTaskOutput(task_id, "spec", "hello", 5);

    const outputs1 = try db.getTaskOutputs(alloc, task_id);
    const outputs2 = try db.getTaskOutputs(alloc, task_id);

    try std.testing.expectEqual(outputs1.len, outputs2.len);
    try std.testing.expectEqualStrings(outputs1[0].phase, outputs2[0].phase);
    try std.testing.expectEqualStrings(outputs1[0].output, outputs2[0].output);
    try std.testing.expectEqual(outputs1[0].exit_code, outputs2[0].exit_code);
}

// =============================================================================
// E9 — storeTaskOutputFull with empty raw_stream
// =============================================================================

test "E9: storeTaskOutputFull with empty raw_stream retrieves as empty string" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeTaskOutputFull(96, "spec", "output", "", 0);

    const outputs = try db.getTaskOutputs(alloc, 96);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqualStrings("", outputs[0].raw_stream);
    try std.testing.expectEqualStrings("output", outputs[0].output);
}

// =============================================================================
// E10 — Large raw_stream combined with large output: output truncated, raw_stream full
// =============================================================================

test "E10: large output truncated and large raw_stream stored in full" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const big_output = [_]u8{'o'} ** 40_000;
    const big_raw = [_]u8{'r'} ** 50_000;
    try db.storeTaskOutputFull(97, "impl", &big_output, &big_raw, 0);

    const outputs = try db.getTaskOutputs(alloc, 97);
    try std.testing.expectEqual(@as(usize, 1), outputs.len);
    try std.testing.expectEqual(@as(usize, 32_000), outputs[0].output.len);
    try std.testing.expectEqual(@as(usize, 50_000), outputs[0].raw_stream.len);
}
