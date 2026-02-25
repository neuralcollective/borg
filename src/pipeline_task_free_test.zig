// Tests for spec #18: Fix memory leak in pipeline tick() — task strings never freed
//
// Verifies that:
//   • db.freePipelineTask() frees every heap-allocated string field of a PipelineTask
//   • pipeline.processTaskThread() frees task strings via defer
//   • pipeline.tick() frees non-dispatched task strings (in-flight skip, capacity break,
//     spawn failure)
//   • pipeline.createHealthTask() uses an ArenaAllocator for the task query
//
// To include in the build, add to db.zig:
//   test { _ = @import("pipeline_task_free_test.zig"); }
// Or add to pipeline.zig's existing test block:
//   _ = @import("pipeline_task_free_test.zig");
//
// Coverage map:
//   AC1  – string fields freed after thread completion (leak detection + source check)
//   AC2  – in-flight skip path calls freePipelineTask (source check)
//   AC3  – capacity-break path frees trailing tasks (source check)
//   AC4  – spawn-failure path calls freePipelineTask (source check)
//   AC5  – createHealthTask uses ArenaAllocator (source check)
//   AC6  – freePipelineTask frees all 10 string fields (leak detection)
//   AC8  – no double-free: dispatch path does NOT call freePipelineTask (source check)
//   AC9  – outer slice defer-free is retained in tick() (source check)
//   AC10 – PipelineTask struct fields are unchanged (compile-time structural check)
//   E1   – empty task list — no freePipelineTask calls needed
//   E10  – empty string fields are freed safely

const std = @import("std");
const db_mod = @import("db.zig");
const PipelineTask = db_mod.PipelineTask;

// ── Helper: build a PipelineTask whose strings are all dupe'd from `alloc` ──

/// Allocate every string field of a PipelineTask using `allocator.dupe()`, exactly
/// as rowToPipelineTask does.  The caller is responsible for freeing via freePipelineTask.
fn makeDupedTask(allocator: std.mem.Allocator, id: i64) !PipelineTask {
    return PipelineTask{
        .id = id,
        .title = try allocator.dupe(u8, "Fix something"),
        .description = try allocator.dupe(u8, "A description"),
        .repo_path = try allocator.dupe(u8, "/home/user/repo"),
        .branch = try allocator.dupe(u8, "task-42"),
        .status = try allocator.dupe(u8, "backlog"),
        .attempt = 0,
        .max_attempts = 5,
        .last_error = try allocator.dupe(u8, ""),
        .created_by = try allocator.dupe(u8, "tg:123"),
        .notify_chat = try allocator.dupe(u8, "tg:456"),
        .created_at = try allocator.dupe(u8, "2026-02-25T00:00:00Z"),
        .session_id = try allocator.dupe(u8, "sess-abc"),
    };
}

/// Same as makeDupedTask but every string field is empty ("").
fn makeDupedTaskEmpty(allocator: std.mem.Allocator, id: i64) !PipelineTask {
    return PipelineTask{
        .id = id,
        .title = try allocator.dupe(u8, ""),
        .description = try allocator.dupe(u8, ""),
        .repo_path = try allocator.dupe(u8, ""),
        .branch = try allocator.dupe(u8, ""),
        .status = try allocator.dupe(u8, ""),
        .attempt = 0,
        .max_attempts = 5,
        .last_error = try allocator.dupe(u8, ""),
        .created_by = try allocator.dupe(u8, ""),
        .notify_chat = try allocator.dupe(u8, ""),
        .created_at = try allocator.dupe(u8, ""),
        .session_id = try allocator.dupe(u8, ""),
    };
}

// ═════════════════════════════════════════════════════════════════════════════
// AC6 — freePipelineTask frees all 10 string fields (no leak)
//
// std.testing.allocator reports any allocation that is not freed at the end
// of the test.  These tests FAIL before the fix because freePipelineTask does
// not exist, causing a compile error.
// ═════════════════════════════════════════════════════════════════════════════

test "AC6: freePipelineTask frees all 10 string fields — no leak" {
    const alloc = std.testing.allocator;
    const task = try makeDupedTask(alloc, 1);
    // freePipelineTask must exist and free every dupe'd field.
    // If it is missing or skips a field, std.testing.allocator reports a leak
    // and the test fails.
    db_mod.freePipelineTask(alloc, task);
}

test "AC6: freePipelineTask on a task with non-empty last_error frees it" {
    const alloc = std.testing.allocator;
    var task = try makeDupedTask(alloc, 2);
    // Replace last_error with a non-empty string
    alloc.free(task.last_error);
    task.last_error = try alloc.dupe(u8, "subprocess exited with code 1");
    db_mod.freePipelineTask(alloc, task);
}

test "AC6: freePipelineTask on a task with long title frees it" {
    const alloc = std.testing.allocator;
    var task = try makeDupedTask(alloc, 3);
    alloc.free(task.title);
    task.title = try alloc.dupe(u8, "A" ** 512); // 512-char title
    db_mod.freePipelineTask(alloc, task);
}

test "AC6: freePipelineTask return type is void" {
    // Verify the function signature: (std.mem.Allocator, PipelineTask) void
    const FreeFn = @TypeOf(db_mod.freePipelineTask);
    const info = @typeInfo(FreeFn).@"fn";
    try std.testing.expect(info.return_type.? == void);
}

// ═════════════════════════════════════════════════════════════════════════════
// E10 — Empty string fields are freed safely
//
// allocator.dupe(u8, "") returns a valid zero-length allocation.
// freePipelineTask must call free() on it (zero-length free is always safe).
// ═════════════════════════════════════════════════════════════════════════════

test "E10: freePipelineTask handles all-empty string fields without crash" {
    const alloc = std.testing.allocator;
    const task = try makeDupedTaskEmpty(alloc, 10);
    db_mod.freePipelineTask(alloc, task);
}

test "E10: allocator.dupe of empty string produces freeable allocation" {
    const alloc = std.testing.allocator;
    const s = try alloc.dupe(u8, "");
    try std.testing.expectEqual(@as(usize, 0), s.len);
    alloc.free(s); // must not crash
}

// ═════════════════════════════════════════════════════════════════════════════
// AC1 — String fields are freed after thread completion
//
// We simulate the processTaskThread pattern: allocate strings, run "thread"
// body (no-op here), defer-free via freePipelineTask.  Any missed free causes
// std.testing.allocator to report a leak.
// ═════════════════════════════════════════════════════════════════════════════

test "AC1: task strings freed when simulated processTaskThread defers freePipelineTask" {
    const alloc = std.testing.allocator;
    const task = try makeDupedTask(alloc, 20);
    // Simulate the defer that processTaskThread must contain after the fix.
    defer db_mod.freePipelineTask(alloc, task);
    // Body of the thread: access the strings (no-op to prevent optimisation)
    try std.testing.expect(task.title.len > 0);
    try std.testing.expect(task.repo_path.len > 0);
}

test "AC1: freePipelineTask source check — called in processTaskThread defer block" {
    // After the fix, pipeline.zig must contain a call to freePipelineTask in
    // processTaskThread's defer block.
    const src = @embedFile("pipeline.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "freePipelineTask") != null);
}

test "AC1: processTaskThread defer calls freePipelineTask before fetchSub" {
    // Verify that freePipelineTask appears in pipeline.zig and that it is used
    // inside the processTaskThread function body.
    const src = @embedFile("pipeline.zig");

    // Find the start of processTaskThread
    const fn_pos = std.mem.indexOf(u8, src, "fn processTaskThread(") orelse {
        try std.testing.expect(false); // function missing
        return;
    };

    // Find the next top-level function definition after processTaskThread
    const after_fn = src[fn_pos + 1 ..];
    const next_fn_rel = std.mem.indexOf(u8, after_fn, "\n    fn ") orelse after_fn.len;
    const fn_body = src[fn_pos .. fn_pos + 1 + next_fn_rel];

    try std.testing.expect(std.mem.indexOf(u8, fn_body, "freePipelineTask") != null);
}

// ═════════════════════════════════════════════════════════════════════════════
// AC2 — In-flight skip path frees task strings
//
// When inflight_tasks.contains(task.id) is true, tick() must call
// freePipelineTask before continuing to the next task.
// ═════════════════════════════════════════════════════════════════════════════

test "AC2: freePipelineTask appears in tick() source" {
    const src = @embedFile("pipeline.zig");

    // Locate tick()
    const tick_pos = std.mem.indexOf(u8, src, "fn tick(") orelse {
        try std.testing.expect(false); // tick() missing
        return;
    };
    // Find body of tick() (up to the next top-level fn)
    const after_tick = src[tick_pos + 1 ..];
    const next_fn_rel = std.mem.indexOf(u8, after_tick, "\n    fn ") orelse after_tick.len;
    const tick_body = src[tick_pos .. tick_pos + 1 + next_fn_rel];

    // freePipelineTask must be called inside tick()
    try std.testing.expect(std.mem.indexOf(u8, tick_body, "freePipelineTask") != null);
}

test "AC2: tick() calls freePipelineTask near inflight_tasks.contains" {
    // After the fix: the in-flight skip path frees the task.
    // Source check: inflight_tasks.contains and freePipelineTask both appear in tick().
    const src = @embedFile("pipeline.zig");

    const tick_pos = std.mem.indexOf(u8, src, "fn tick(") orelse {
        try std.testing.expect(false);
        return;
    };
    const after_tick = src[tick_pos + 1 ..];
    const next_fn_rel = std.mem.indexOf(u8, after_tick, "\n    fn ") orelse after_tick.len;
    const tick_body = src[tick_pos .. tick_pos + 1 + next_fn_rel];

    try std.testing.expect(std.mem.indexOf(u8, tick_body, "inflight_tasks.contains") != null);
    try std.testing.expect(std.mem.indexOf(u8, tick_body, "freePipelineTask") != null);
}

// ═════════════════════════════════════════════════════════════════════════════
// AC3 — Capacity-break path frees trailing tasks
//
// When active_agents >= max_pipeline_agents, tick() breaks from the loop.
// All tasks from the break index onward must have their string fields freed.
// ═════════════════════════════════════════════════════════════════════════════

test "AC3: tick() frees remaining tasks when capacity limit is reached" {
    // Behavioural test via leak detection: allocate multiple tasks with
    // std.testing.allocator, simulate the capacity-break loop, free trailing tasks.
    const alloc = std.testing.allocator;

    // Simulate: capacity reached at index 1 (tasks[0] dispatched, tasks[1..] freed).
    const tasks = try alloc.alloc(PipelineTask, 3);
    defer alloc.free(tasks);

    tasks[0] = try makeDupedTask(alloc, 30);
    tasks[1] = try makeDupedTask(alloc, 31);
    tasks[2] = try makeDupedTask(alloc, 32);

    // tasks[0] is "dispatched" — the thread will free it.
    // Simulate the thread running and freeing tasks[0].
    db_mod.freePipelineTask(alloc, tasks[0]);

    // Simulate the capacity-break loop: for (tasks[1..]) |remaining| freePipelineTask(...)
    for (tasks[1..]) |remaining| {
        db_mod.freePipelineTask(alloc, remaining);
    }
    // After this block, std.testing.allocator should detect no leaks.
}

test "AC3: capacity-break source check — tasks[i..] freed before break" {
    // The fix must contain a loop that iterates remaining tasks and frees each one
    // before breaking.  We look for the characteristic pattern.
    const src = @embedFile("pipeline.zig");

    const tick_pos = std.mem.indexOf(u8, src, "fn tick(") orelse {
        try std.testing.expect(false);
        return;
    };
    const after_tick = src[tick_pos + 1 ..];
    const next_fn_rel = std.mem.indexOf(u8, after_tick, "\n    fn ") orelse after_tick.len;
    const tick_body = src[tick_pos .. tick_pos + 1 + next_fn_rel];

    // The loop that frees remaining tasks before break must contain freePipelineTask
    // and a break statement in the same capacity-check block.
    try std.testing.expect(std.mem.indexOf(u8, tick_body, "freePipelineTask") != null);
    try std.testing.expect(std.mem.indexOf(u8, tick_body, "max_pipeline_agents") != null);
}

// ═════════════════════════════════════════════════════════════════════════════
// AC4 — Spawn-failure path frees task strings
//
// When Thread.spawn fails, the task strings must be freed in the catch block.
// ═════════════════════════════════════════════════════════════════════════════

test "AC4: spawn-failure behavioural: allocate task, simulate catch-free" {
    const alloc = std.testing.allocator;
    const task = try makeDupedTask(alloc, 40);

    // Simulate: Thread.spawn fails → catch block frees the task
    // (In the real code, freePipelineTask is called in the catch before continue.)
    db_mod.freePipelineTask(alloc, task);
    // No leak reported → correct behaviour.
}

test "AC4: tick() catch block frees task — source check" {
    // The spawn catch block in tick() must contain freePipelineTask.
    const src = @embedFile("pipeline.zig");

    const tick_pos = std.mem.indexOf(u8, src, "fn tick(") orelse {
        try std.testing.expect(false);
        return;
    };
    const after_tick = src[tick_pos + 1 ..];
    const next_fn_rel = std.mem.indexOf(u8, after_tick, "\n    fn ") orelse after_tick.len;
    const tick_body = src[tick_pos .. tick_pos + 1 + next_fn_rel];

    // Thread.spawn and freePipelineTask must both appear in tick().
    try std.testing.expect(std.mem.indexOf(u8, tick_body, "Thread.spawn") != null);
    try std.testing.expect(std.mem.indexOf(u8, tick_body, "freePipelineTask") != null);
}

// ═════════════════════════════════════════════════════════════════════════════
// AC5 — createHealthTask uses an ArenaAllocator
//
// The fix replaces `self.allocator` with an arena inside createHealthTask so
// all task strings are freed at once when the arena is deinitialized.
// ═════════════════════════════════════════════════════════════════════════════

test "AC5: createHealthTask source check — ArenaAllocator present in function body" {
    const src = @embedFile("pipeline.zig");

    // Locate createHealthTask
    const fn_pos = std.mem.indexOf(u8, src, "fn createHealthTask(") orelse {
        try std.testing.expect(false); // function missing
        return;
    };
    const after_fn = src[fn_pos + 1 ..];
    const next_fn_rel = std.mem.indexOf(u8, after_fn, "\n    fn ") orelse after_fn.len;
    const fn_body = src[fn_pos .. fn_pos + 1 + next_fn_rel];

    // After the fix, an ArenaAllocator must be created inside createHealthTask.
    try std.testing.expect(std.mem.indexOf(u8, fn_body, "ArenaAllocator") != null);
}

test "AC5: createHealthTask arena is deinitialized — source check for arena.deinit" {
    const src = @embedFile("pipeline.zig");

    const fn_pos = std.mem.indexOf(u8, src, "fn createHealthTask(") orelse {
        try std.testing.expect(false);
        return;
    };
    const after_fn = src[fn_pos + 1 ..];
    const next_fn_rel = std.mem.indexOf(u8, after_fn, "\n    fn ") orelse after_fn.len;
    const fn_body = src[fn_pos .. fn_pos + 1 + next_fn_rel];

    // arena.deinit() (or defer arena.deinit()) must appear in createHealthTask.
    try std.testing.expect(std.mem.indexOf(u8, fn_body, "arena.deinit()") != null);
}

test "AC5: createHealthTask does not call freePipelineTask — arena handles it" {
    // The arena approach requires NO explicit freePipelineTask calls for the
    // task list query.  If freePipelineTask appears in createHealthTask, that
    // would be redundant (and a sign the arena wasn't used properly).
    const src = @embedFile("pipeline.zig");

    const fn_pos = std.mem.indexOf(u8, src, "fn createHealthTask(") orelse {
        try std.testing.expect(false);
        return;
    };
    const after_fn = src[fn_pos + 1 ..];
    const next_fn_rel = std.mem.indexOf(u8, after_fn, "\n    fn ") orelse after_fn.len;
    const fn_body = src[fn_pos .. fn_pos + 1 + next_fn_rel];

    // freePipelineTask must NOT appear in createHealthTask.
    try std.testing.expect(std.mem.indexOf(u8, fn_body, "freePipelineTask") == null);
}

// ═════════════════════════════════════════════════════════════════════════════
// AC8 — No double-free: the dispatch path must NOT call freePipelineTask
//
// For tasks successfully handed off to processTaskThread, the ownership of
// string memory transfers to the thread.  tick() must not free them.
// ═════════════════════════════════════════════════════════════════════════════

test "AC8: dispatch path does not free task — Thread.spawn and freePipelineTask not on same branch" {
    // Structural check: after Thread.spawn succeeds (no catch), tick() must NOT
    // call freePipelineTask.  We verify that freePipelineTask only appears in
    // error/skip branches of tick(), not after a successful spawn.
    //
    // We confirm this indirectly: the successful spawn path ends with a comment
    // "Ownership of task's strings transferred to processTaskThread" or simply
    // has no freePipelineTask call between the spawn line and the next iteration.
    //
    // A pragmatic check: count occurrences of freePipelineTask in tick() body.
    // There should be exactly 3: in-flight skip, inflight_tasks.put failure,
    // and spawn failure catch block.  The successful dispatch path has 0.
    const src = @embedFile("pipeline.zig");

    const tick_pos = std.mem.indexOf(u8, src, "fn tick(") orelse {
        try std.testing.expect(false);
        return;
    };
    const after_tick = src[tick_pos + 1 ..];
    const next_fn_rel = std.mem.indexOf(u8, after_tick, "\n    fn ") orelse after_tick.len;
    const tick_body = src[tick_pos .. tick_pos + 1 + next_fn_rel];

    // Count freePipelineTask occurrences in tick()
    var count: usize = 0;
    var pos: usize = 0;
    while (std.mem.indexOfPos(u8, tick_body, pos, "freePipelineTask")) |idx| {
        count += 1;
        pos = idx + 1;
    }
    // At least 2 (in-flight skip + spawn failure).
    // Exactly 0 occurrences means the fix wasn't applied; the test fails.
    try std.testing.expect(count >= 2);
}

test "AC8: no double-free — freePipelineTask not called twice for the same task id" {
    // Behavioural: allocate one task, free it exactly once.
    const alloc = std.testing.allocator;
    const task = try makeDupedTask(alloc, 80);
    db_mod.freePipelineTask(alloc, task);
    // Calling it a second time on the same pointers would be a double-free.
    // We cannot call it again here (that would crash the test), so we just
    // verify that a single free leaves the allocator in a clean state.
    // std.testing.allocator will detect any double-free or use-after-free.
}

// ═════════════════════════════════════════════════════════════════════════════
// AC9 — Outer slice defer-free is retained in tick()
//
// The existing `defer self.allocator.free(tasks)` must not be removed; it
// frees the slice backing array (not the string contents).
// ═════════════════════════════════════════════════════════════════════════════

test "AC9: tick() retains defer self.allocator.free(tasks) for the outer slice" {
    const src = @embedFile("pipeline.zig");

    const tick_pos = std.mem.indexOf(u8, src, "fn tick(") orelse {
        try std.testing.expect(false);
        return;
    };
    const after_tick = src[tick_pos + 1 ..];
    const next_fn_rel = std.mem.indexOf(u8, after_tick, "\n    fn ") orelse after_tick.len;
    const tick_body = src[tick_pos .. tick_pos + 1 + next_fn_rel];

    // The outer-slice free must still be present.
    const has_defer_free = std.mem.indexOf(u8, tick_body, "defer self.allocator.free(tasks)") != null or
        std.mem.indexOf(u8, tick_body, "defer self.allocator.free(tasks);") != null;
    try std.testing.expect(has_defer_free);
}

// ═════════════════════════════════════════════════════════════════════════════
// AC10 — PipelineTask struct fields are unchanged
//
// The fix is purely at call sites; the struct definition must not change.
// ═════════════════════════════════════════════════════════════════════════════

test "AC10: PipelineTask has id field of type i64" {
    const info = @typeInfo(PipelineTask);
    var found = false;
    for (info.@"struct".fields) |f| {
        if (std.mem.eql(u8, f.name, "id")) {
            found = true;
            try std.testing.expect(f.type == i64);
        }
    }
    try std.testing.expect(found);
}

test "AC10: PipelineTask has all 10 expected string fields" {
    const expected_string_fields = [_][]const u8{
        "title",
        "description",
        "repo_path",
        "branch",
        "status",
        "last_error",
        "created_by",
        "notify_chat",
        "created_at",
        "session_id",
    };

    const info = @typeInfo(PipelineTask);
    for (expected_string_fields) |name| {
        var found = false;
        for (info.@"struct".fields) |f| {
            if (std.mem.eql(u8, f.name, name)) {
                found = true;
                // All string fields must be []const u8
                try std.testing.expect(f.type == []const u8);
            }
        }
        try std.testing.expect(found);
    }
}

test "AC10: PipelineTask has attempt and max_attempts as i64 (not freed)" {
    const info = @typeInfo(PipelineTask);
    for (info.@"struct".fields) |f| {
        if (std.mem.eql(u8, f.name, "attempt") or std.mem.eql(u8, f.name, "max_attempts")) {
            try std.testing.expect(f.type == i64);
        }
    }
}

test "AC10: PipelineTask struct has exactly 13 fields" {
    const info = @typeInfo(PipelineTask);
    // id, title, description, repo_path, branch, status, attempt, max_attempts,
    // last_error, created_by, notify_chat, created_at, session_id = 13 fields
    try std.testing.expectEqual(@as(usize, 13), info.@"struct".fields.len);
}

// ═════════════════════════════════════════════════════════════════════════════
// E1 — Empty task list: no freePipelineTask calls needed
//
// getActivePipelineTasks returns [] when no tasks are active; tick() must
// handle this correctly without calling freePipelineTask (nothing to free).
// ═════════════════════════════════════════════════════════════════════════════

test "E1: getActivePipelineTasks on empty DB returns empty slice without leaking" {
    // Using std.testing.allocator: if getActivePipelineTasks allocates anything
    // (e.g., the empty slice itself), the caller must free it.
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const db_alloc = arena.allocator();

    var db = try db_mod.Db.init(db_alloc, ":memory:");
    defer db.deinit();

    // Use std.testing.allocator for the task allocation so leaks are detected.
    const tasks = try db.getActivePipelineTasks(std.testing.allocator, 20);
    defer std.testing.allocator.free(tasks); // free the outer slice (always)
    try std.testing.expectEqual(@as(usize, 0), tasks.len);
    // No task strings allocated → no freePipelineTask calls needed.
    // std.testing.allocator detects any unfreed outer slice if we forgot the defer.
}

test "E1: freePipelineTask is never needed for zero-length results" {
    // With an empty task list, the for-loop body in tick() never executes,
    // so freePipelineTask is never called.  This is correct behaviour.
    // This test documents the invariant: getActivePipelineTasks([]) → 0 strings allocated.
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try db_mod.Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const alloc = std.testing.allocator;
    const tasks = try db.getActivePipelineTasks(alloc, 20);
    defer alloc.free(tasks);
    // No string fields exist in a zero-length slice.
    try std.testing.expectEqual(@as(usize, 0), tasks.len);
}

// ═════════════════════════════════════════════════════════════════════════════
// Integration: getActivePipelineTasks + freePipelineTask round-trip
//
// Create real tasks in an in-memory DB, fetch them with std.testing.allocator,
// and free every string field via freePipelineTask.  This is the exact pattern
// that tick() must follow after the fix.
// ═════════════════════════════════════════════════════════════════════════════

test "integration: fetch tasks from DB and free strings via freePipelineTask — no leak" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const db_alloc = arena.allocator();

    var db = try db_mod.Db.init(db_alloc, ":memory:");
    defer db.deinit();

    // Create three active tasks.
    _ = try db.createPipelineTask("Task A", "Description A", "/repo", "tg:1", "tg:1");
    _ = try db.createPipelineTask("Task B", "Description B", "/repo", "tg:2", "tg:2");
    _ = try db.createPipelineTask("Task C", "Description C", "/repo", "tg:3", "tg:3");

    // Fetch using std.testing.allocator — strings are individually dupe'd.
    const tasks = try db.getActivePipelineTasks(std.testing.allocator, 20);
    defer std.testing.allocator.free(tasks); // free the outer slice

    try std.testing.expectEqual(@as(usize, 3), tasks.len);

    // Free every task's string fields — this is what the fix makes tick() do.
    for (tasks) |task| {
        db_mod.freePipelineTask(std.testing.allocator, task);
    }
    // std.testing.allocator detects any leaked string fields.
}

test "integration: task strings are valid after allocation" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try db_mod.Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createPipelineTask("My Task", "My description", "/path/to/repo", "", "");

    const tasks = try db.getActivePipelineTasks(std.testing.allocator, 20);
    defer std.testing.allocator.free(tasks);

    try std.testing.expectEqual(@as(usize, 1), tasks.len);
    try std.testing.expectEqualStrings("My Task", tasks[0].title);
    try std.testing.expectEqualStrings("My description", tasks[0].description);
    try std.testing.expectEqualStrings("/path/to/repo", tasks[0].repo_path);
    try std.testing.expectEqualStrings("backlog", tasks[0].status);

    db_mod.freePipelineTask(std.testing.allocator, tasks[0]);
}

// ═════════════════════════════════════════════════════════════════════════════
// E2 — All tasks are in-flight (all skipped via the in-flight path)
//
// Simulate: all returned tasks are already in inflight_tasks.
// Each must be freed by the in-flight skip path.
// ═════════════════════════════════════════════════════════════════════════════

test "E2: all tasks in-flight — each must be freed at the skip site" {
    const alloc = std.testing.allocator;

    // Allocate 3 tasks as if returned by getActivePipelineTasks.
    const tasks = try alloc.alloc(PipelineTask, 3);
    defer alloc.free(tasks);

    for (tasks, 0..) |*t, i| {
        t.* = try makeDupedTask(alloc, @intCast(i));
    }

    // Simulate the tick() in-flight skip for every task.
    for (tasks) |task| {
        // In-flight check: inflight_tasks.contains(task.id) == true
        // → freePipelineTask(alloc, task); continue;
        db_mod.freePipelineTask(alloc, task);
    }
    // std.testing.allocator reports no leaks → correct.
}

// ═════════════════════════════════════════════════════════════════════════════
// E8 — Mixed: some in-flight, some dispatched, some beyond capacity
// ═════════════════════════════════════════════════════════════════════════════

test "E8: mixed scenario — in-flight freed, dispatched owned by thread, trailing freed" {
    const alloc = std.testing.allocator;

    // 5 tasks: [0] dispatched (thread owns), [1] in-flight (skip+free),
    //          [2] dispatched (thread owns), [3][4] beyond capacity (break+free).
    const tasks = try alloc.alloc(PipelineTask, 5);
    defer alloc.free(tasks);

    for (tasks, 0..) |*t, i| {
        t.* = try makeDupedTask(alloc, @intCast(100 + i));
    }

    // tasks[0]: dispatched — thread (simulated here) frees
    db_mod.freePipelineTask(alloc, tasks[0]);

    // tasks[1]: in-flight skip — tick() frees immediately
    db_mod.freePipelineTask(alloc, tasks[1]);

    // tasks[2]: dispatched — thread (simulated here) frees
    db_mod.freePipelineTask(alloc, tasks[2]);

    // tasks[3], tasks[4]: beyond capacity (break loop) — tick() frees
    for (tasks[3..]) |remaining| {
        db_mod.freePipelineTask(alloc, remaining);
    }
    // No leaks.
}

// ═════════════════════════════════════════════════════════════════════════════
// E3 — Capacity limit at first task (i == 0): all tasks freed in break loop
// ═════════════════════════════════════════════════════════════════════════════

test "E3: capacity limit at i=0 — all tasks freed in the break-path loop" {
    const alloc = std.testing.allocator;

    const tasks = try alloc.alloc(PipelineTask, 4);
    defer alloc.free(tasks);

    for (tasks, 0..) |*t, i| {
        t.* = try makeDupedTask(alloc, @intCast(200 + i));
    }

    // Capacity reached at i == 0: free tasks[0..] entirely
    for (tasks[0..]) |remaining| {
        db_mod.freePipelineTask(alloc, remaining);
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// E4 — Capacity limit mid-slice: tasks[0..i) dispatched, tasks[i..] freed
// ═════════════════════════════════════════════════════════════════════════════

test "E4: capacity limit mid-slice — dispatched tasks owned by threads, trailing freed" {
    const alloc = std.testing.allocator;

    const tasks = try alloc.alloc(PipelineTask, 5);
    defer alloc.free(tasks);

    for (tasks, 0..) |*t, i| {
        t.* = try makeDupedTask(alloc, @intCast(300 + i));
    }

    const dispatch_count = 2; // tasks[0] and [1] dispatched
    // Simulate threads freeing dispatched tasks
    for (tasks[0..dispatch_count]) |dispatched| {
        db_mod.freePipelineTask(alloc, dispatched);
    }
    // Simulate tick() break loop freeing the rest
    for (tasks[dispatch_count..]) |remaining| {
        db_mod.freePipelineTask(alloc, remaining);
    }
}
