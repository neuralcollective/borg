// Tests for spec #33: Add tests for db.zig pipeline task query functions
//
// Covers: getNextPipelineTask, getActivePipelineTasks, getPipelineTask,
//         updateTaskError, setTaskSessionId
//
// All query allocations use an ArenaAllocator so string cleanup is handled
// automatically — no need to call freePipelineTask in these tests.
//
// To include in the build, add to the test block in db.zig:
//   _ = @import("db_pipeline_query_test.zig");

const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;
const PipelineTask = db_mod.PipelineTask;

// =============================================================================
// AC1 — getNextPipelineTask returns null on empty DB
// =============================================================================

test "AC1: getNextPipelineTask returns null on empty DB" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const task = try db.getNextPipelineTask(arena.allocator());
    try std.testing.expect(task == null);
}

// =============================================================================
// AC2 — getNextPipelineTask returns the sole active task
// =============================================================================

test "AC2: getNextPipelineTask returns the sole backlog task" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createPipelineTask("My Task", "desc", "/repo", "tg:1", "tg:1");

    const task = try db.getNextPipelineTask(arena.allocator());
    try std.testing.expect(task != null);
    try std.testing.expectEqualStrings("My Task", task.?.title);
    try std.testing.expectEqualStrings("backlog", task.?.status);
}

// =============================================================================
// AC3 — getNextPipelineTask priority ordering
//
// Priority: rebase(0) > retry(1) > impl(2) > qa_fix(3)/qa(3) > spec(4) > backlog(5)
// =============================================================================

test "AC3: getNextPipelineTask priority: rebase > retry > impl > spec > backlog" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id_rebase = try db.createPipelineTask("T-rebase",  "d", "/repo", "", "");
    const id_retry  = try db.createPipelineTask("T-retry",   "d", "/repo", "", "");
    const id_impl   = try db.createPipelineTask("T-impl",    "d", "/repo", "", "");
    const id_qa_fix = try db.createPipelineTask("T-qa_fix",  "d", "/repo", "", "");
    const id_qa     = try db.createPipelineTask("T-qa",      "d", "/repo", "", "");
    const id_spec   = try db.createPipelineTask("T-spec",    "d", "/repo", "", "");
    const id_back   = try db.createPipelineTask("T-backlog", "d", "/repo", "", "");

    try db.updateTaskStatus(id_rebase, "rebase");
    try db.updateTaskStatus(id_retry,  "retry");
    try db.updateTaskStatus(id_impl,   "impl");
    try db.updateTaskStatus(id_qa_fix, "qa_fix");
    try db.updateTaskStatus(id_qa,     "qa");
    try db.updateTaskStatus(id_spec,   "spec");
    _ = id_back; // stays backlog

    // Highest priority: rebase
    {
        const t = (try db.getNextPipelineTask(arena.allocator())).?;
        try std.testing.expectEqualStrings("rebase", t.status);
    }
    try db.updateTaskStatus(id_rebase, "done");

    // Next: retry
    {
        const t = (try db.getNextPipelineTask(arena.allocator())).?;
        try std.testing.expectEqualStrings("retry", t.status);
    }
    try db.updateTaskStatus(id_retry, "done");

    // Next: impl
    {
        const t = (try db.getNextPipelineTask(arena.allocator())).?;
        try std.testing.expectEqualStrings("impl", t.status);
    }
    try db.updateTaskStatus(id_impl, "done");

    // Next: qa_fix or qa (same weight — either is correct)
    {
        const t = (try db.getNextPipelineTask(arena.allocator())).?;
        const s = t.status;
        const ok = std.mem.eql(u8, s, "qa_fix") or std.mem.eql(u8, s, "qa");
        try std.testing.expect(ok);
    }
    try db.updateTaskStatus(id_qa_fix, "done");
    try db.updateTaskStatus(id_qa,     "done");

    // Next: spec
    {
        const t = (try db.getNextPipelineTask(arena.allocator())).?;
        try std.testing.expectEqualStrings("spec", t.status);
    }
    try db.updateTaskStatus(id_spec, "done");

    // Last remaining: backlog
    {
        const t = (try db.getNextPipelineTask(arena.allocator())).?;
        try std.testing.expectEqualStrings("backlog", t.status);
    }
}

// =============================================================================
// AC4 — getNextPipelineTask excludes non-active statuses
// =============================================================================

test "AC4: getNextPipelineTask excludes done, merged, failed, test" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T-done",   "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("T-merged", "d", "/repo", "", "");
    const id3 = try db.createPipelineTask("T-failed", "d", "/repo", "", "");
    const id4 = try db.createPipelineTask("T-test",   "d", "/repo", "", "");

    try db.updateTaskStatus(id1, "done");
    try db.updateTaskStatus(id2, "merged");
    try db.updateTaskStatus(id3, "failed");
    try db.updateTaskStatus(id4, "test");

    const task = try db.getNextPipelineTask(arena.allocator());
    try std.testing.expect(task == null);
}

// =============================================================================
// AC5 — getNextPipelineTask tie-breaks by insertion order (oldest first)
// =============================================================================

test "AC5: getNextPipelineTask returns the earlier-inserted task when priorities are equal" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("First",  "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("Second", "d", "/repo", "", "");
    _ = id2;

    const task = (try db.getNextPipelineTask(arena.allocator())).?;
    try std.testing.expectEqual(id1, task.id);
    try std.testing.expectEqualStrings("First", task.title);
}

// =============================================================================
// AC6 — getActivePipelineTasks returns empty slice on empty DB
// =============================================================================

test "AC6: getActivePipelineTasks returns empty slice on empty DB" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const tasks = try db.getActivePipelineTasks(arena.allocator(), 20);
    try std.testing.expectEqual(@as(usize, 0), tasks.len);
}

// =============================================================================
// AC7 — getActivePipelineTasks returns all active tasks in priority order
// =============================================================================

test "AC7: getActivePipelineTasks returns tasks in priority order (impl > spec > backlog)" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id_back = try db.createPipelineTask("T-backlog", "d", "/repo", "", "");
    const id_impl = try db.createPipelineTask("T-impl",    "d", "/repo", "", "");
    const id_spec = try db.createPipelineTask("T-spec",    "d", "/repo", "", "");

    _ = id_back; // stays backlog
    try db.updateTaskStatus(id_impl, "impl");
    try db.updateTaskStatus(id_spec, "spec");

    const tasks = try db.getActivePipelineTasks(arena.allocator(), 20);
    try std.testing.expectEqual(@as(usize, 3), tasks.len);
    try std.testing.expectEqualStrings("impl",    tasks[0].status);
    try std.testing.expectEqualStrings("spec",    tasks[1].status);
    try std.testing.expectEqualStrings("backlog", tasks[2].status);
}

// =============================================================================
// AC8 — getActivePipelineTasks excludes non-active statuses
// =============================================================================

test "AC8: getActivePipelineTasks excludes merged tasks" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createPipelineTask("B1", "d", "/repo", "", "");
    _ = try db.createPipelineTask("B2", "d", "/repo", "", "");
    const id_m = try db.createPipelineTask("M1", "d", "/repo", "", "");
    try db.updateTaskStatus(id_m, "merged");

    const tasks = try db.getActivePipelineTasks(arena.allocator(), 20);
    try std.testing.expectEqual(@as(usize, 2), tasks.len);
    for (tasks) |t| {
        try std.testing.expectEqualStrings("backlog", t.status);
    }
}

// =============================================================================
// AC9 — getActivePipelineTasks respects the limit parameter
// =============================================================================

test "AC9: getActivePipelineTasks respects limit=2 from 5 tasks" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    for (0..5) |_| {
        _ = try db.createPipelineTask("T", "d", "/repo", "", "");
    }

    const limited = try db.getActivePipelineTasks(arena.allocator(), 2);
    try std.testing.expectEqual(@as(usize, 2), limited.len);
}

test "AC9: getActivePipelineTasks with limit=0 returns empty slice" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createPipelineTask("T", "d", "/repo", "", "");

    const tasks = try db.getActivePipelineTasks(arena.allocator(), 0);
    try std.testing.expectEqual(@as(usize, 0), tasks.len);
}

// =============================================================================
// AC10 — getActivePipelineTasks covers all seven active statuses
// =============================================================================

test "AC10: getActivePipelineTasks returns all 7 active statuses with rebase first" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id_back   = try db.createPipelineTask("T-backlog", "d", "/repo", "", "");
    const id_spec   = try db.createPipelineTask("T-spec",    "d", "/repo", "", "");
    const id_qa     = try db.createPipelineTask("T-qa",      "d", "/repo", "", "");
    const id_qa_fix = try db.createPipelineTask("T-qa_fix",  "d", "/repo", "", "");
    const id_impl   = try db.createPipelineTask("T-impl",    "d", "/repo", "", "");
    const id_retry  = try db.createPipelineTask("T-retry",   "d", "/repo", "", "");
    const id_rebase = try db.createPipelineTask("T-rebase",  "d", "/repo", "", "");

    _ = id_back; // stays backlog
    try db.updateTaskStatus(id_spec,   "spec");
    try db.updateTaskStatus(id_qa,     "qa");
    try db.updateTaskStatus(id_qa_fix, "qa_fix");
    try db.updateTaskStatus(id_impl,   "impl");
    try db.updateTaskStatus(id_retry,  "retry");
    try db.updateTaskStatus(id_rebase, "rebase");

    const tasks = try db.getActivePipelineTasks(arena.allocator(), 20);
    try std.testing.expectEqual(@as(usize, 7), tasks.len);
    try std.testing.expectEqualStrings("rebase", tasks[0].status);
}

// =============================================================================
// AC11 — getPipelineTask returns null for a nonexistent ID
// =============================================================================

test "AC11: getPipelineTask returns null for nonexistent ID" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const task = try db.getPipelineTask(arena.allocator(), 9999);
    try std.testing.expect(task == null);
}

// =============================================================================
// AC12 — getPipelineTask returns the correct task by ID
// =============================================================================

test "AC12: getPipelineTask returns the correct task by ID, not another" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createPipelineTask("First Task",  "d1", "/repo", "", "");
    const id2 = try db.createPipelineTask("Second Task", "d2", "/repo", "", "");

    const task = (try db.getPipelineTask(arena.allocator(), id2)).?;
    try std.testing.expectEqual(id2, task.id);
    try std.testing.expectEqualStrings("Second Task", task.title);
}

// =============================================================================
// AC13 — getPipelineTask maps all fields correctly
// =============================================================================

test "AC13: getPipelineTask maps all 13 fields correctly after mutations" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask(
        "Full Field Task",
        "A full description",
        "/path/to/repo",
        "tg:creator",
        "tg:notify",
    );

    try db.updateTaskStatus(id, "impl");
    try db.updateTaskBranch(id, "task-33-branch");
    try db.updateTaskError(id, "previous error");
    try db.setTaskSessionId(id, "sess-xyz");
    try db.incrementTaskAttempt(id);

    const t = (try db.getPipelineTask(arena.allocator(), id)).?;

    try std.testing.expectEqual(id,                           t.id);
    try std.testing.expectEqualStrings("Full Field Task",     t.title);
    try std.testing.expectEqualStrings("A full description",  t.description);
    try std.testing.expectEqualStrings("/path/to/repo",       t.repo_path);
    try std.testing.expectEqualStrings("task-33-branch",      t.branch);
    try std.testing.expectEqualStrings("impl",                t.status);
    try std.testing.expectEqual(@as(i64, 1),                  t.attempt);
    try std.testing.expectEqual(@as(i64, 5),                  t.max_attempts);
    try std.testing.expectEqualStrings("previous error",      t.last_error);
    try std.testing.expectEqualStrings("tg:creator",          t.created_by);
    try std.testing.expectEqualStrings("tg:notify",           t.notify_chat);
    try std.testing.expectEqualStrings("sess-xyz",            t.session_id);
    // created_at is DB-generated; verify it is non-empty
    try std.testing.expect(t.created_at.len > 0);
}

// =============================================================================
// AC14 — updateTaskError stores error string, leaves other fields unchanged
// =============================================================================

test "AC14: updateTaskError stores error string without touching status or attempt" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("T", "d", "/repo", "", "");
    try db.updateTaskError(id, "subprocess failed: exit 1");

    const t = (try db.getPipelineTask(arena.allocator(), id)).?;
    try std.testing.expectEqualStrings("subprocess failed: exit 1", t.last_error);
    try std.testing.expectEqualStrings("backlog", t.status);
    try std.testing.expectEqual(@as(i64, 0), t.attempt);
}

// =============================================================================
// AC15 — updateTaskError can clear the error to an empty string
// =============================================================================

test "AC15: updateTaskError clears error when set to empty string" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("T", "d", "/repo", "", "");
    try db.updateTaskError(id, "some error");
    try db.updateTaskError(id, "");

    const t = (try db.getPipelineTask(arena.allocator(), id)).?;
    try std.testing.expectEqualStrings("", t.last_error);
}

// =============================================================================
// AC16 — updateTaskError only affects the targeted task
// =============================================================================

test "AC16: updateTaskError does not affect other tasks" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", "");

    try db.updateTaskError(id1, "error on task 1");

    const t2 = (try db.getPipelineTask(arena.allocator(), id2)).?;
    try std.testing.expectEqualStrings("", t2.last_error);
}

// =============================================================================
// AC17 — setTaskSessionId stores session ID, leaves other fields unchanged
// =============================================================================

test "AC17: setTaskSessionId stores session ID without touching status" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("T", "d", "/repo", "", "");
    try db.setTaskSessionId(id, "sess-abc123");

    const t = (try db.getPipelineTask(arena.allocator(), id)).?;
    try std.testing.expectEqualStrings("sess-abc123", t.session_id);
    try std.testing.expectEqualStrings("backlog", t.status);
    try std.testing.expectEqual(@as(i64, 0), t.attempt);
}

// =============================================================================
// AC18 — setTaskSessionId overwrites an existing session ID
// =============================================================================

test "AC18: setTaskSessionId overwrites previous session ID" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("T", "d", "/repo", "", "");
    try db.setTaskSessionId(id, "sess-v1");
    try db.setTaskSessionId(id, "sess-v2");

    const t = (try db.getPipelineTask(arena.allocator(), id)).?;
    try std.testing.expectEqualStrings("sess-v2", t.session_id);
}

// =============================================================================
// AC19 — setTaskSessionId only affects the targeted task
// =============================================================================

test "AC19: setTaskSessionId does not affect other tasks" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", "");

    try db.setTaskSessionId(id1, "sess-for-task-1");

    const t2 = (try db.getPipelineTask(arena.allocator(), id2)).?;
    try std.testing.expectEqualStrings("", t2.session_id);
}

// =============================================================================
// E1 — getNextPipelineTask when all tasks are terminal
// =============================================================================

test "E1: getNextPipelineTask returns null when all tasks are done/merged/failed" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", "");
    const id3 = try db.createPipelineTask("T3", "d", "/repo", "", "");

    try db.updateTaskStatus(id1, "done");
    try db.updateTaskStatus(id2, "merged");
    try db.updateTaskStatus(id3, "failed");

    const task = try db.getNextPipelineTask(arena.allocator());
    try std.testing.expect(task == null);
}

// =============================================================================
// E2 — getActivePipelineTasks limit=1 matches getNextPipelineTask
// =============================================================================

test "E2: getActivePipelineTasks(limit=1) returns same task as getNextPipelineTask" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createPipelineTask("T-backlog", "d", "/repo", "", "");
    const id_impl = try db.createPipelineTask("T-impl", "d", "/repo", "", "");
    try db.updateTaskStatus(id_impl, "impl");

    const next   = (try db.getNextPipelineTask(arena.allocator())).?;
    const active = try db.getActivePipelineTasks(arena.allocator(), 1);

    try std.testing.expectEqual(@as(usize, 1), active.len);
    try std.testing.expectEqual(next.id, active[0].id);
    try std.testing.expectEqualStrings(next.status, active[0].status);
}

// =============================================================================
// E3 — qa and qa_fix have the same priority weight (both weight 3)
//
// Both appear in the active task list and both are returned.  The relative
// order between them is unspecified when created_at is identical (SQLite's
// datetime('now') has 1-second resolution), so we only assert that both IDs
// are present in the result.
// =============================================================================

test "E3: qa_fix and qa have same weight — both appear in active task list" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id_qa_fix = try db.createPipelineTask("T-qa_fix", "d", "/repo", "", "");
    const id_qa     = try db.createPipelineTask("T-qa",     "d", "/repo", "", "");

    try db.updateTaskStatus(id_qa_fix, "qa_fix");
    try db.updateTaskStatus(id_qa,     "qa");

    const tasks = try db.getActivePipelineTasks(arena.allocator(), 20);
    try std.testing.expectEqual(@as(usize, 2), tasks.len);

    // Both tasks must be present; order between them is unspecified for equal priority+timestamp
    var found_qa_fix = false;
    var found_qa     = false;
    for (tasks) |t| {
        if (t.id == id_qa_fix) found_qa_fix = true;
        if (t.id == id_qa)     found_qa     = true;
    }
    try std.testing.expect(found_qa_fix);
    try std.testing.expect(found_qa);
}

// =============================================================================
// E4 — updateTaskError with a very long string (> 1000 chars)
// =============================================================================

test "E4: updateTaskError stores and retrieves a long error string without truncation" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("T", "d", "/repo", "", "");

    const long_err = "E" ** 2000;
    try db.updateTaskError(id, long_err);

    const t = (try db.getPipelineTask(arena.allocator(), id)).?;
    try std.testing.expectEqual(@as(usize, 2000), t.last_error.len);
    try std.testing.expectEqualStrings(long_err, t.last_error);
}

// =============================================================================
// E5 — getPipelineTask after deletePipelineTask returns null
// =============================================================================

test "E5: getPipelineTask returns null after the task is deleted" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("T", "d", "/repo", "", "");
    try db.deletePipelineTask(id);

    const task = try db.getPipelineTask(arena.allocator(), id);
    try std.testing.expect(task == null);
}

// =============================================================================
// E6 — getActivePipelineTasks with limit larger than count returns all tasks
// =============================================================================

test "E6: getActivePipelineTasks with oversized limit returns all active tasks" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createPipelineTask("T1", "d", "/repo", "", "");
    _ = try db.createPipelineTask("T2", "d", "/repo", "", "");
    _ = try db.createPipelineTask("T3", "d", "/repo", "", "");

    const tasks = try db.getActivePipelineTasks(arena.allocator(), 1000);
    try std.testing.expectEqual(@as(usize, 3), tasks.len);
}

// =============================================================================
// E7 — setTaskSessionId with empty string stores "" not NULL
// =============================================================================

test "E7: setTaskSessionId with empty string stores empty, not NULL" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("T", "d", "/repo", "", "");
    try db.setTaskSessionId(id, "sess-old");
    try db.setTaskSessionId(id, "");

    const t = (try db.getPipelineTask(arena.allocator(), id)).?;
    try std.testing.expectEqualStrings("", t.session_id);
}

// =============================================================================
// E8 — getNextPipelineTask and getActivePipelineTasks agree on the first result
// =============================================================================

test "E8: getNextPipelineTask and getActivePipelineTasks(limit=1) return the same task" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createPipelineTask("T-backlog", "d", "/repo", "", "");
    const id_rebase = try db.createPipelineTask("T-rebase", "d", "/repo", "", "");
    const id_spec   = try db.createPipelineTask("T-spec",   "d", "/repo", "", "");

    try db.updateTaskStatus(id_rebase, "rebase");
    try db.updateTaskStatus(id_spec,   "spec");

    const next   = (try db.getNextPipelineTask(arena.allocator())).?;
    const active = try db.getActivePipelineTasks(arena.allocator(), 1);

    try std.testing.expectEqual(@as(usize, 1), active.len);
    try std.testing.expectEqual(id_rebase, next.id);
    try std.testing.expectEqual(id_rebase, active[0].id);
    try std.testing.expectEqualStrings("rebase", next.status);
    try std.testing.expectEqualStrings("rebase", active[0].status);
}
