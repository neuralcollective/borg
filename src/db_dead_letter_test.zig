// Tests for Task #62: exponential backoff and dead-letter queue — DB layer.
//
// Covers the following acceptance criteria from spec.md:
//   AC2  — setTaskRetryAfter stores a future datetime; task is hidden from
//           getActivePipelineTasks until that datetime has elapsed.
//   AC3  — getActivePipelineTasks (and getNextPipelineTask) only return tasks
//           whose retry_after is empty OR in the past.
//   AC4  — dead_letter status persists with last_error intact.
//   AC5  — dead_letter tasks never appear in getActivePipelineTasks /
//           getNextPipelineTask, and are not counted by getActivePipelineTaskCount.
//   AC6  — requeueDeadLetterTask resets status=backlog, attempt=0, branch='',
//           session_id='', retry_after='', last_error=''.
//   AC7  — requeueDeadLetterTask is a no-op when task is not dead_letter.
//   AC8  — getPipelineStats.failed counts both 'failed' and 'dead_letter'.
//   AC11 — clearAllDispatched does NOT clear retry_after.
//   AC12 — The retry_after column migration is idempotent (execQuiet swallows
//           duplicate-column errors on a second ALTER TABLE).
//
// These tests FAIL until the implementation adds:
//   - "ALTER TABLE pipeline_tasks ADD COLUMN retry_after TEXT DEFAULT ''"
//     to runMigrations in db.zig
//   - pub fn setTaskRetryAfter(self: *Db, task_id: i64, delay_s: i64) !void
//   - pub fn requeueDeadLetterTask(self: *Db, task_id: i64) !void
//   - pub fn getDeadLetterTasks(self: *Db, allocator: std.mem.Allocator, limit: i64) ![]PipelineTask
//   - retry_after guard in getActivePipelineTasks / getNextPipelineTask WHERE clauses
//   - dead_letter counted in getPipelineStats.failed
//
// To include in the build, add to the test block in src/db.zig:
//   _ = @import("db_dead_letter_test.zig");

const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;

// =============================================================================
// AC2 — setTaskRetryAfter schedules a future eligible time
// =============================================================================

test "AC2: setTaskRetryAfter with large delay hides task from getActivePipelineTasks" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Sleeping Task", "desc", "/repo", "", "");
    // 1-hour future delay — task must be invisible to the scheduler
    try db.setTaskRetryAfter(id, 3600);

    const tasks = try db.getActivePipelineTasks(alloc, 10);
    for (tasks) |t| {
        try std.testing.expect(t.id != id);
    }
}

test "AC2: setTaskRetryAfter does not error on a valid task" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    // Should not return an error for any reasonable delay
    try db.setTaskRetryAfter(id, 60);
    try db.setTaskRetryAfter(id, 120);
    try db.setTaskRetryAfter(id, 3600);
}

test "AC2: setTaskRetryAfter only affects the targeted task" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id_sleeping = try db.createPipelineTask("Sleeping", "d", "/repo", "", "");
    const id_ready    = try db.createPipelineTask("Ready",    "d", "/repo", "", "");

    try db.setTaskRetryAfter(id_sleeping, 3600);

    const tasks = try db.getActivePipelineTasks(alloc, 10);

    var found_ready    = false;
    var found_sleeping = false;
    for (tasks) |t| {
        if (t.id == id_ready)    found_ready    = true;
        if (t.id == id_sleeping) found_sleeping = true;
    }
    try std.testing.expect(found_ready);
    try std.testing.expect(!found_sleeping);
}

// =============================================================================
// AC3 — getActivePipelineTasks respects retry_after
// =============================================================================

test "AC3: task with empty retry_after (default) appears in getActivePipelineTasks" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Ready Task", "desc", "/repo", "", "");

    const tasks = try db.getActivePipelineTasks(alloc, 10);

    var found = false;
    for (tasks) |t| {
        if (t.id == id) { found = true; break; }
    }
    try std.testing.expect(found);
}

test "AC3: task with far-future retry_after is excluded from getActivePipelineTasks" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Sleeping Task", "desc", "/repo", "", "");

    // Write a far-future datetime directly to bypass sub-second timing concerns
    try db.sqlite_db.execute(
        "UPDATE pipeline_tasks SET retry_after = '9999-12-31 23:59:59' WHERE id = ?1",
        .{id},
    );

    const tasks = try db.getActivePipelineTasks(alloc, 10);
    for (tasks) |t| {
        try std.testing.expect(t.id != id);
    }
}

test "AC3: task with past retry_after is returned by getActivePipelineTasks" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Past-Due Task", "desc", "/repo", "", "");

    // A date far in the past is always <= datetime('now')
    try db.sqlite_db.execute(
        "UPDATE pipeline_tasks SET retry_after = '2000-01-01 00:00:00' WHERE id = ?1",
        .{id},
    );

    const tasks = try db.getActivePipelineTasks(alloc, 10);

    var found = false;
    for (tasks) |t| {
        if (t.id == id) { found = true; break; }
    }
    try std.testing.expect(found);
}

test "AC3: only the ready task is returned when the other has future retry_after" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id_ready    = try db.createPipelineTask("Ready",    "desc", "/repo", "", "");
    const id_sleeping = try db.createPipelineTask("Sleeping", "desc", "/repo", "", "");

    try db.sqlite_db.execute(
        "UPDATE pipeline_tasks SET retry_after = '9999-12-31 23:59:59' WHERE id = ?1",
        .{id_sleeping},
    );

    const tasks = try db.getActivePipelineTasks(alloc, 10);

    var found_ready    = false;
    var found_sleeping = false;
    for (tasks) |t| {
        if (t.id == id_ready)    found_ready    = true;
        if (t.id == id_sleeping) found_sleeping = true;
    }
    try std.testing.expect(found_ready);
    try std.testing.expect(!found_sleeping);
}

test "AC3: getNextPipelineTask excludes task with future retry_after" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Sleeping Task", "desc", "/repo", "", "");
    try db.sqlite_db.execute(
        "UPDATE pipeline_tasks SET retry_after = '9999-12-31 23:59:59' WHERE id = ?1",
        .{id},
    );

    const next = try db.getNextPipelineTask(alloc);
    // The only task is sleeping → must return null
    if (next) |t| {
        try std.testing.expect(t.id != id);
    }
}

test "AC3: getNextPipelineTask returns task with past retry_after" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Past-Due Task", "desc", "/repo", "", "");
    try db.sqlite_db.execute(
        "UPDATE pipeline_tasks SET retry_after = '2000-01-01 00:00:00' WHERE id = ?1",
        .{id},
    );

    const next = try db.getNextPipelineTask(alloc);
    try std.testing.expect(next != null);
    try std.testing.expectEqual(id, next.?.id);
}

test "AC3: limit is still respected when some tasks are sleeping" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // 3 ready, 2 sleeping
    for (0..3) |_| {
        _ = try db.createPipelineTask("Ready", "d", "/repo", "", "");
    }
    for (0..2) |_| {
        const sid = try db.createPipelineTask("Sleeping", "d", "/repo", "", "");
        try db.sqlite_db.execute(
            "UPDATE pipeline_tasks SET retry_after = '9999-12-31 23:59:59' WHERE id = ?1",
            .{sid},
        );
    }

    // limit=2 should give 2 of the 3 ready tasks
    const tasks = try db.getActivePipelineTasks(alloc, 2);
    try std.testing.expectEqual(@as(usize, 2), tasks.len);
    for (tasks) |t| {
        try std.testing.expectEqualStrings("Ready", t.title);
    }
}

// =============================================================================
// AC4 — dead_letter status and last_error are persisted
// =============================================================================

test "AC4: updateTaskStatus to dead_letter persists the status" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Failing Task", "desc", "/repo", "", "");
    try db.updateTaskStatus(id, "dead_letter");

    const task = (try db.getPipelineTask(alloc, id)).?;
    try std.testing.expectEqualStrings("dead_letter", task.status);
}

test "AC4: dead_letter task retains last_error set before status change" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id  = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    const msg = "test suite: 3 failed, 0 passed\nexit code 1";
    try db.updateTaskError(id, msg);
    try db.updateTaskStatus(id, "dead_letter");

    const task = (try db.getPipelineTask(alloc, id)).?;
    try std.testing.expectEqualStrings("dead_letter", task.status);
    try std.testing.expectEqualStrings(msg, task.last_error);
}

test "AC4: dead_letter status does not reset attempt counter" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    try db.incrementTaskAttempt(id);
    try db.incrementTaskAttempt(id);
    try db.updateTaskStatus(id, "dead_letter");

    const task = (try db.getPipelineTask(alloc, id)).?;
    try std.testing.expectEqual(@as(i64, 2), task.attempt);
}

// =============================================================================
// AC5 — dead_letter tasks are invisible to the scheduler
// =============================================================================

test "AC5: dead_letter task does not appear in getActivePipelineTasks" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Dead Task", "desc", "/repo", "", "");
    try db.updateTaskStatus(id, "dead_letter");

    const tasks = try db.getActivePipelineTasks(alloc, 10);
    for (tasks) |t| {
        try std.testing.expect(t.id != id);
    }
}

test "AC5: getNextPipelineTask returns null when only task is dead_letter" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Dead Task", "desc", "/repo", "", "");
    try db.updateTaskStatus(id, "dead_letter");

    const next = try db.getNextPipelineTask(alloc);
    if (next) |t| {
        try std.testing.expect(t.id != id);
    }
}

test "AC5: getActivePipelineTaskCount does not count dead_letter" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("Dead",   "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("Active", "d", "/repo", "", "");
    try db.updateTaskStatus(id1, "dead_letter");
    _ = id2; // stays backlog

    const count = try db.getActivePipelineTaskCount();
    // Only id2 (backlog) is active
    try std.testing.expectEqual(@as(i64, 1), count);
}

test "AC5: dead_letter alongside active tasks — only active ones returned" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id_dead   = try db.createPipelineTask("Dead",    "d", "/repo", "", "");
    const id_active = try db.createPipelineTask("Active",  "d", "/repo", "", "");
    const id_impl   = try db.createPipelineTask("Impl",    "d", "/repo", "", "");

    try db.updateTaskStatus(id_dead, "dead_letter");
    try db.updateTaskStatus(id_impl, "impl");
    _ = id_active; // backlog

    const tasks = try db.getActivePipelineTasks(alloc, 10);
    try std.testing.expectEqual(@as(usize, 2), tasks.len);
    for (tasks) |t| {
        try std.testing.expect(t.id != id_dead);
    }
}

// =============================================================================
// AC6 — requeueDeadLetterTask resets all fields
// =============================================================================

test "AC6: requeueDeadLetterTask sets status to backlog" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    try db.updateTaskStatus(id, "dead_letter");
    try db.requeueDeadLetterTask(id);

    const task = (try db.getPipelineTask(alloc, id)).?;
    try std.testing.expectEqualStrings("backlog", task.status);
}

test "AC6: requeueDeadLetterTask resets attempt to 0" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    try db.incrementTaskAttempt(id);
    try db.incrementTaskAttempt(id);
    try db.incrementTaskAttempt(id);
    try db.updateTaskStatus(id, "dead_letter");
    try db.requeueDeadLetterTask(id);

    const task = (try db.getPipelineTask(alloc, id)).?;
    try std.testing.expectEqual(@as(i64, 0), task.attempt);
}

test "AC6: requeueDeadLetterTask clears branch" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    try db.updateTaskBranch(id, "task-42-impl");
    try db.updateTaskStatus(id, "dead_letter");
    try db.requeueDeadLetterTask(id);

    const task = (try db.getPipelineTask(alloc, id)).?;
    try std.testing.expectEqualStrings("", task.branch);
}

test "AC6: requeueDeadLetterTask clears session_id" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    try db.setTaskSessionId(id, "sess-abc123");
    try db.updateTaskStatus(id, "dead_letter");
    try db.requeueDeadLetterTask(id);

    const task = (try db.getPipelineTask(alloc, id)).?;
    try std.testing.expectEqualStrings("", task.session_id);
}

test "AC6: requeueDeadLetterTask clears last_error" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    try db.updateTaskError(id, "subprocess exited with code 1");
    try db.updateTaskStatus(id, "dead_letter");
    try db.requeueDeadLetterTask(id);

    const task = (try db.getPipelineTask(alloc, id)).?;
    try std.testing.expectEqualStrings("", task.last_error);
}

test "AC6: requeueDeadLetterTask clears retry_after — task reappears in active list" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    // Set a future retry_after and dead_letter together
    try db.setTaskRetryAfter(id, 3600);
    try db.updateTaskStatus(id, "dead_letter");

    // Before requeue: task must not appear
    {
        const tasks = try db.getActivePipelineTasks(alloc, 10);
        for (tasks) |t| {
            try std.testing.expect(t.id != id);
        }
    }

    try db.requeueDeadLetterTask(id);

    // After requeue: task must reappear (status=backlog, retry_after='')
    {
        const tasks = try db.getActivePipelineTasks(alloc, 10);
        var found = false;
        for (tasks) |t| {
            if (t.id == id) { found = true; break; }
        }
        try std.testing.expect(found);
    }
}

test "AC6: requeueDeadLetterTask resets all five fields atomically" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Task", "desc", "/repo", "creator", "notify");
    try db.updateTaskBranch(id, "task-99");
    try db.setTaskSessionId(id, "sess-xyz");
    try db.updateTaskError(id, "build failed");
    try db.incrementTaskAttempt(id);
    try db.incrementTaskAttempt(id);
    try db.setTaskRetryAfter(id, 3600);
    try db.updateTaskStatus(id, "dead_letter");

    try db.requeueDeadLetterTask(id);

    const task = (try db.getPipelineTask(alloc, id)).?;
    try std.testing.expectEqualStrings("backlog",  task.status);
    try std.testing.expectEqual(@as(i64, 0),       task.attempt);
    try std.testing.expectEqualStrings("",         task.branch);
    try std.testing.expectEqualStrings("",         task.session_id);
    try std.testing.expectEqualStrings("",         task.last_error);
    // Other fields must be untouched
    try std.testing.expectEqualStrings("Task",     task.title);
    try std.testing.expectEqualStrings("creator",  task.created_by);
    try std.testing.expectEqualStrings("notify",   task.notify_chat);
}

// =============================================================================
// AC7 — requeueDeadLetterTask is a no-op for non-dead_letter tasks
// =============================================================================

test "AC7: requeueDeadLetterTask on backlog task leaves status unchanged" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    // status is 'backlog'
    try db.requeueDeadLetterTask(id);

    const task = (try db.getPipelineTask(alloc, id)).?;
    try std.testing.expectEqualStrings("backlog", task.status);
}

test "AC7: requeueDeadLetterTask on failed task leaves status unchanged" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    try db.updateTaskStatus(id, "failed");
    try db.requeueDeadLetterTask(id);

    const task = (try db.getPipelineTask(alloc, id)).?;
    try std.testing.expectEqualStrings("failed", task.status);
}

test "AC7: requeueDeadLetterTask on impl task leaves all fields unchanged" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    try db.updateTaskStatus(id, "impl");
    try db.incrementTaskAttempt(id);
    try db.updateTaskBranch(id, "task-77");

    try db.requeueDeadLetterTask(id);

    const task = (try db.getPipelineTask(alloc, id)).?;
    try std.testing.expectEqualStrings("impl",    task.status);
    try std.testing.expectEqual(@as(i64, 1),      task.attempt);
    try std.testing.expectEqualStrings("task-77", task.branch);
}

test "AC7: requeueDeadLetterTask on merged task is a no-op" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    try db.updateTaskStatus(id, "merged");
    try db.requeueDeadLetterTask(id);

    const task = (try db.getPipelineTask(alloc, id)).?;
    try std.testing.expectEqualStrings("merged", task.status);
}

// =============================================================================
// AC8 — getPipelineStats.failed includes dead_letter
// =============================================================================

test "AC8: single dead_letter task is counted in getPipelineStats.failed" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Dead Task", "desc", "/repo", "", "");
    try db.updateTaskStatus(id, "dead_letter");

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 1), stats.failed);
    try std.testing.expectEqual(@as(i64, 0), stats.active);
    try std.testing.expectEqual(@as(i64, 1), stats.total);
}

test "AC8: dead_letter and failed are both counted in getPipelineStats.failed" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", "");
    const id3 = try db.createPipelineTask("T3", "d", "/repo", "", "");

    try db.updateTaskStatus(id1, "dead_letter");
    try db.updateTaskStatus(id2, "failed");
    _ = id3; // stays backlog (active)

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 2), stats.failed); // dead_letter + failed
    try std.testing.expectEqual(@as(i64, 1), stats.active);
    try std.testing.expectEqual(@as(i64, 3), stats.total);
}

test "AC8: dead_letter is not counted as active in getPipelineStats" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", "");
    try db.updateTaskStatus(id1, "dead_letter");
    _ = id2; // backlog

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 1), stats.active); // only backlog
    try std.testing.expectEqual(@as(i64, 1), stats.failed); // dead_letter
    try std.testing.expectEqual(@as(i64, 2), stats.total);
}

test "AC8: getPipelineStats.failed is zero when no failed or dead_letter tasks" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.createPipelineTask("T1", "d", "/repo", "", ""); // backlog
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", "");
    try db.updateTaskStatus(id2, "merged");

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 0), stats.failed);
}

test "AC8: all four status categories work correctly together" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // 2 active, 1 merged, 1 failed, 2 dead_letter
    const id1 = try db.createPipelineTask("T1-active",      "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("T2-active",      "d", "/repo", "", "");
    const id3 = try db.createPipelineTask("T3-merged",      "d", "/repo", "", "");
    const id4 = try db.createPipelineTask("T4-failed",      "d", "/repo", "", "");
    const id5 = try db.createPipelineTask("T5-dead_letter", "d", "/repo", "", "");
    const id6 = try db.createPipelineTask("T6-dead_letter", "d", "/repo", "", "");

    try db.updateTaskStatus(id2, "impl");
    try db.updateTaskStatus(id3, "merged");
    try db.updateTaskStatus(id4, "failed");
    try db.updateTaskStatus(id5, "dead_letter");
    try db.updateTaskStatus(id6, "dead_letter");
    _ = id1; // stays backlog

    const stats = try db.getPipelineStats();
    try std.testing.expectEqual(@as(i64, 6), stats.total);
    try std.testing.expectEqual(@as(i64, 2), stats.active);
    try std.testing.expectEqual(@as(i64, 1), stats.merged);
    try std.testing.expectEqual(@as(i64, 3), stats.failed); // failed + 2×dead_letter
}

// =============================================================================
// getDeadLetterTasks — dedicated listing function
// =============================================================================

test "getDeadLetterTasks: empty when no dead_letter tasks exist" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.createPipelineTask("Active Task", "desc", "/repo", "", "");

    const tasks = try db.getDeadLetterTasks(alloc, 10);
    try std.testing.expectEqual(@as(usize, 0), tasks.len);
}

test "getDeadLetterTasks: returns only dead_letter tasks" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("Dead 1", "desc", "/repo", "", "");
    const id2 = try db.createPipelineTask("Dead 2", "desc", "/repo", "", "");
    const id3 = try db.createPipelineTask("Active", "desc", "/repo", "", "");

    try db.updateTaskStatus(id1, "dead_letter");
    try db.updateTaskStatus(id2, "dead_letter");
    _ = id3; // stays backlog

    const tasks = try db.getDeadLetterTasks(alloc, 10);
    try std.testing.expectEqual(@as(usize, 2), tasks.len);
    for (tasks) |t| {
        try std.testing.expectEqualStrings("dead_letter", t.status);
        try std.testing.expect(t.id != id3);
    }
}

test "getDeadLetterTasks: excludes failed tasks (different status)" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id_dead   = try db.createPipelineTask("Dead",   "desc", "/repo", "", "");
    const id_failed = try db.createPipelineTask("Failed", "desc", "/repo", "", "");

    try db.updateTaskStatus(id_dead,   "dead_letter");
    try db.updateTaskStatus(id_failed, "failed");

    const tasks = try db.getDeadLetterTasks(alloc, 10);
    try std.testing.expectEqual(@as(usize, 1), tasks.len);
    try std.testing.expectEqual(id_dead, tasks[0].id);
    try std.testing.expectEqualStrings("dead_letter", tasks[0].status);
}

test "getDeadLetterTasks: respects the limit parameter" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    for (0..5) |_| {
        const id = try db.createPipelineTask("Dead", "desc", "/repo", "", "");
        try db.updateTaskStatus(id, "dead_letter");
    }

    const tasks = try db.getDeadLetterTasks(alloc, 3);
    try std.testing.expectEqual(@as(usize, 3), tasks.len);
}

test "getDeadLetterTasks: returned tasks have dead_letter status and non-empty fields" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Exhausted Task", "some desc", "/repo/path", "creator", "");
    try db.updateTaskError(id, "failed: compilation error");
    try db.updateTaskStatus(id, "dead_letter");

    const tasks = try db.getDeadLetterTasks(alloc, 10);
    try std.testing.expectEqual(@as(usize, 1), tasks.len);
    try std.testing.expectEqualStrings("dead_letter",          tasks[0].status);
    try std.testing.expectEqualStrings("Exhausted Task",       tasks[0].title);
    try std.testing.expectEqualStrings("failed: compilation error", tasks[0].last_error);
    try std.testing.expectEqual(id, tasks[0].id);
}

// =============================================================================
// AC11 — clearAllDispatched does NOT clear retry_after
// =============================================================================

test "AC11: clearAllDispatched clears dispatched_at but leaves retry_after intact" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Sleeping Task", "desc", "/repo", "", "");

    // Simulate dispatch then set a future retry_after
    try db.markTaskDispatched(id);
    try db.setTaskRetryAfter(id, 3600);

    // Confirm the task is dispatched
    try std.testing.expect(db.isTaskDispatched(id));

    // Simulate restart: clearAllDispatched runs
    try db.clearAllDispatched();

    // dispatched_at cleared → no longer dispatched
    try std.testing.expect(!db.isTaskDispatched(id));

    // retry_after still blocks the task from the active list
    const tasks = try db.getActivePipelineTasks(alloc, 10);
    for (tasks) |t| {
        try std.testing.expect(t.id != id);
    }
}

test "AC11: clearAllDispatched leaves a past retry_after task eligible" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Past-Due Task", "desc", "/repo", "", "");
    try db.markTaskDispatched(id);

    // Set a past retry_after (already elapsed)
    try db.sqlite_db.execute(
        "UPDATE pipeline_tasks SET retry_after = '2000-01-01 00:00:00' WHERE id = ?1",
        .{id},
    );

    try db.clearAllDispatched();

    // Task has past retry_after → should be eligible
    const tasks = try db.getActivePipelineTasks(alloc, 10);
    var found = false;
    for (tasks) |t| {
        if (t.id == id) { found = true; break; }
    }
    try std.testing.expect(found);
}

// =============================================================================
// AC12 — retry_after column migration is idempotent
// =============================================================================

test "AC12: applying the retry_after ALTER TABLE twice does not error" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    // Db.init() already runs the migration which adds retry_after.
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Running the same ALTER TABLE again must be silently swallowed.
    // (Matches the execQuiet pattern used by other column migrations.)
    db.sqlite_db.execQuiet(
        "ALTER TABLE pipeline_tasks ADD COLUMN retry_after TEXT DEFAULT ''",
    ) catch {};
    db.sqlite_db.execQuiet(
        "ALTER TABLE pipeline_tasks ADD COLUMN retry_after TEXT DEFAULT ''",
    ) catch {};

    // DB must still be fully functional after duplicate migrations
    const id = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    try db.setTaskRetryAfter(id, 60);
    const tasks = try db.getActivePipelineTasks(alloc, 10);
    // Task has a 60-second future retry_after → must be excluded
    for (tasks) |t| {
        try std.testing.expect(t.id != id);
    }
}

// =============================================================================
// Edge cases
// =============================================================================

test "Edge: max_attempts=1 task can be set to dead_letter on first error" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("One-Shot Task", "desc", "/repo", "", "");
    // Force max_attempts=1
    try db.sqlite_db.execute(
        "UPDATE pipeline_tasks SET max_attempts = 1 WHERE id = ?1",
        .{id},
    );
    try db.updateTaskError(id, "first and only error");
    try db.updateTaskStatus(id, "dead_letter");

    const task = (try db.getPipelineTask(alloc, id)).?;
    try std.testing.expectEqualStrings("dead_letter", task.status);
    try std.testing.expectEqualStrings("first and only error", task.last_error);
    try std.testing.expectEqual(@as(i64, 1), task.max_attempts);
}

test "Edge: newly created task has empty retry_after and is immediately active" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("New Task", "desc", "/repo", "", "");

    const tasks = try db.getActivePipelineTasks(alloc, 10);
    var found = false;
    for (tasks) |t| {
        if (t.id == id) { found = true; break; }
    }
    try std.testing.expect(found);
}

test "Edge: requeueDeadLetterTask twice is safe — second call is a no-op" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    try db.updateTaskStatus(id, "dead_letter");

    // First call: task goes back to backlog
    try db.requeueDeadLetterTask(id);
    // Second call: status is now 'backlog', WHERE clause doesn't match → no-op
    try db.requeueDeadLetterTask(id);

    const task = (try db.getPipelineTask(alloc, id)).?;
    try std.testing.expectEqualStrings("backlog", task.status);
}

test "Edge: retry_after is ignored for non-active statuses (dead_letter already excluded)" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // A task that is dead_letter WITH a past retry_after is still not returned
    // (status IN clause excludes dead_letter regardless of retry_after)
    const id = try db.createPipelineTask("Task", "desc", "/repo", "", "");
    try db.sqlite_db.execute(
        "UPDATE pipeline_tasks SET retry_after = '2000-01-01 00:00:00' WHERE id = ?1",
        .{id},
    );
    try db.updateTaskStatus(id, "dead_letter");

    const tasks = try db.getActivePipelineTasks(alloc, 10);
    for (tasks) |t| {
        try std.testing.expect(t.id != id);
    }
}

test "Edge: multiple sleeping tasks all excluded from getActivePipelineTasks" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // 3 ready, 4 sleeping
    for (0..3) |_| {
        _ = try db.createPipelineTask("Ready", "d", "/repo", "", "");
    }
    for (0..4) |_| {
        const sid = try db.createPipelineTask("Sleeping", "d", "/repo", "", "");
        try db.sqlite_db.execute(
            "UPDATE pipeline_tasks SET retry_after = '9999-12-31 23:59:59' WHERE id = ?1",
            .{sid},
        );
    }

    // Only the 3 ready tasks should be returned by the scheduler query
    const tasks = try db.getActivePipelineTasks(alloc, 100);
    try std.testing.expectEqual(@as(usize, 3), tasks.len);
    for (tasks) |t| {
        try std.testing.expectEqualStrings("Ready", t.title);
    }
}
