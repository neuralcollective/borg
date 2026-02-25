// Tests for spec #43: Test db.updateTaskStatus() valid and invalid transitions
//
// Covers: updateTaskStatus — forward status progression, error on nonexistent
//         task ID, field isolation, and idempotency.
//
// All allocations use an ArenaAllocator so string cleanup is handled
// automatically — no need to call freePipelineTask in these tests.
//
// To include in the build, add to the test block in src/db.zig:
//   _ = @import("db_update_task_status_test.zig");

const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;

// =============================================================================
// AC1 — Single valid transition persists the new status
// =============================================================================

test "AC1: updateTaskStatus backlog->spec persists new status" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Task A", "desc", "/repo", "tg:1", "tg:1");

    // Sanity: newly created task starts at backlog
    const before = (try db.getPipelineTask(arena.allocator(), id)).?;
    try std.testing.expectEqualStrings("backlog", before.status);

    try db.updateTaskStatus(id, "spec");

    const after = (try db.getPipelineTask(arena.allocator(), id)).?;
    try std.testing.expectEqualStrings("spec", after.status);
}

// =============================================================================
// AC2 — Full forward sequence advances without error
//
// Exercises every phase in the canonical pipeline order:
//   backlog → spec → qa → impl → done → release
// =============================================================================

test "AC2: full pipeline status sequence backlog->spec->qa->impl->done->release" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Pipeline Task", "d", "/repo", "", "");

    const sequence = [_][]const u8{ "spec", "qa", "impl", "done", "release" };
    for (sequence) |phase| {
        try db.updateTaskStatus(id, phase);
        const t = (try db.getPipelineTask(arena.allocator(), id)).?;
        try std.testing.expectEqualStrings(phase, t.status);
    }
}

// =============================================================================
// AC3 — Nonexistent task ID returns error.TaskNotFound
// =============================================================================

test "AC3: updateTaskStatus with nonexistent ID returns error.TaskNotFound" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    // Empty database — ID 9999 does not exist
    try std.testing.expectError(error.TaskNotFound, db.updateTaskStatus(9999, "spec"));
}

// =============================================================================
// AC4 — Status update does not mutate other task fields
// =============================================================================

test "AC4: updateTaskStatus only changes status, not branch/error/session/attempt" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask(
        "Isolation Task",
        "a description",
        "/path/to/repo",
        "tg:creator",
        "tg:notify",
    );

    // Set all mutable fields to known values before the status update
    try db.updateTaskBranch(id, "feature/my-branch");
    try db.updateTaskError(id, "previous error text");
    try db.setTaskSessionId(id, "sess-preserve-me");
    try db.incrementTaskAttempt(id);

    // Now advance the status
    try db.updateTaskStatus(id, "impl");

    const t = (try db.getPipelineTask(arena.allocator(), id)).?;

    try std.testing.expectEqualStrings("impl",                 t.status);
    try std.testing.expectEqualStrings("Isolation Task",       t.title);
    try std.testing.expectEqualStrings("a description",        t.description);
    try std.testing.expectEqualStrings("/path/to/repo",        t.repo_path);
    try std.testing.expectEqualStrings("feature/my-branch",   t.branch);
    try std.testing.expectEqualStrings("previous error text",  t.last_error);
    try std.testing.expectEqualStrings("sess-preserve-me",     t.session_id);
    try std.testing.expectEqual(@as(i64, 1),                   t.attempt);
    try std.testing.expectEqualStrings("tg:creator",           t.created_by);
    try std.testing.expectEqualStrings("tg:notify",            t.notify_chat);
}

// =============================================================================
// AC5 — Status update affects only the targeted task
// =============================================================================

test "AC5: updateTaskStatus only mutates the targeted task, not others" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("Task 1", "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("Task 2", "d", "/repo", "", "");

    try db.updateTaskStatus(id1, "spec");

    const t1 = (try db.getPipelineTask(arena.allocator(), id1)).?;
    const t2 = (try db.getPipelineTask(arena.allocator(), id2)).?;

    try std.testing.expectEqualStrings("spec",    t1.status);
    try std.testing.expectEqualStrings("backlog", t2.status);
}

// =============================================================================
// AC6 — Idempotent same-status update succeeds (not a false TaskNotFound)
//
// sqlite3_changes() returns 1 even when the new value equals the old value,
// so a same-status write must not trigger error.TaskNotFound.
// =============================================================================

test "AC6: updateTaskStatus with same status is idempotent and succeeds" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Idempotent Task", "d", "/repo", "", "");

    // First write: advance to spec
    try db.updateTaskStatus(id, "spec");

    // Second write: same status — must not error
    try db.updateTaskStatus(id, "spec");

    const t = (try db.getPipelineTask(arena.allocator(), id)).?;
    try std.testing.expectEqualStrings("spec", t.status);
}

// =============================================================================
// E1 — Zero task ID returns error.TaskNotFound
// =============================================================================

test "E1: updateTaskStatus with ID=0 returns error.TaskNotFound" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    // SQLite AUTOINCREMENT never issues row ID 0
    try std.testing.expectError(error.TaskNotFound, db.updateTaskStatus(0, "spec"));
}

// =============================================================================
// E2 — Negative task ID returns error.TaskNotFound
// =============================================================================

test "E2: updateTaskStatus with negative ID returns error.TaskNotFound" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try std.testing.expectError(error.TaskNotFound, db.updateTaskStatus(-1, "spec"));
}

// =============================================================================
// E3 — Update after deletePipelineTask returns error.TaskNotFound
// =============================================================================

test "E3: updateTaskStatus after delete returns error.TaskNotFound" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("To Be Deleted", "d", "/repo", "", "");
    try db.deletePipelineTask(id);

    try std.testing.expectError(error.TaskNotFound, db.updateTaskStatus(id, "spec"));
}

// =============================================================================
// E4 — All active and terminal statuses round-trip correctly
// =============================================================================

test "E4: all active pipeline statuses round-trip through updateTaskStatus" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const active_statuses = [_][]const u8{
        "backlog", "spec", "qa", "qa_fix", "impl", "retry", "rebase",
    };
    const terminal_statuses = [_][]const u8{
        "done", "merged", "failed", "release",
    };

    // Use a single task and write each status in turn
    const id = try db.createPipelineTask("Status Roundtrip", "d", "/repo", "", "");

    for (active_statuses) |s| {
        try db.updateTaskStatus(id, s);
        const t = (try db.getPipelineTask(arena.allocator(), id)).?;
        try std.testing.expectEqualStrings(s, t.status);
    }

    for (terminal_statuses) |s| {
        try db.updateTaskStatus(id, s);
        const t = (try db.getPipelineTask(arena.allocator(), id)).?;
        try std.testing.expectEqualStrings(s, t.status);
    }
}

// =============================================================================
// E5 — Arbitrary status string is stored verbatim (no validation)
// =============================================================================

test "E5: updateTaskStatus stores an arbitrary string verbatim" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Verbatim Task", "d", "/repo", "", "");

    try db.updateTaskStatus(id, "custom_phase");

    const t = (try db.getPipelineTask(arena.allocator(), id)).?;
    try std.testing.expectEqualStrings("custom_phase", t.status);
}
