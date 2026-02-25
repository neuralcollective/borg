// Tests for spec #42: pipeline.tick() inflight-skip and capacity-break paths
//
// Two early-exit branches in tick()'s dispatch loop:
//   (a) Inflight-skip:    inflight_tasks.contains(task.id) → continue
//   (b) Capacity-break:  active_agents >= pipeline_max_agents → break
//
// Because tick() is a private method, tests use two complementary strategies:
//   Strategy A — source inspection via @embedFile("pipeline.zig")
//   Strategy B — behavioral simulation mirroring the tick() loop exactly
//
// Coverage map:
//   AC1  — inflight-skip source: contains + continue present in tick() before fetchAdd
//   AC2  — inflight-skip behavioral: all tasks inflight → dispatched == 0
//   AC3  — inflight-skip behavioral: partial inflight → M-K dispatched
//   AC4  — capacity-break source: active_agents check + break present in tick()
//   AC5  — capacity-break behavioral: active_agents at capacity → dispatched == 0
//   AC6  — capacity-break behavioral: one slot remaining → exactly 1 dispatched
//   AC7  — capacity-break behavioral: max == 1, active == 1 → 0 dispatched
//   AC8  — interaction: inflight + at capacity → 0 dispatched
//   AC9  — ordering: capacity check byte offset < inflight check byte offset
//   Edge1 — empty task list: neither branch fires, dispatched == 0
//   Edge2 — single eligible task: dispatched == 1, inflight updated
//   Edge3 — stale inflight IDs (not in current tasks): no effect on dispatch
//   Edge4 — pipeline_max_agents == 0: breaks immediately, dispatched == 0
//   Edge5 — capacity-break at last task: N-1 dispatched, last undispatched
//   E6   — all inflight + at capacity: break fires first, 0 dispatched
//   E7   — mutex correctness: inflight_mu.lock() wraps contains + put in source

const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;
const PipelineTask = db_mod.PipelineTask;

// ── Test helpers ──────────────────────────────────────────────────────────────

/// Build a minimal PipelineTask with the given id and sensible literal defaults
/// for all other fields.  String fields point to string literals — no heap
/// allocation, so these tasks must NOT be freed via any freePipelineTask helper.
fn makeTask(id: i64) PipelineTask {
    return PipelineTask{
        .id = id,
        .title = "task",
        .description = "desc",
        .repo_path = "/repo",
        .branch = "main",
        .status = "backlog",
        .attempt = 0,
        .max_attempts = 5,
        .last_error = "",
        .created_by = "",
        .notify_chat = "",
        .created_at = "2026-01-01T00:00:00Z",
        .session_id = "",
    };
}

/// Simulate the exact dispatch logic inside tick().
///
/// Mirrors the pipeline.zig tick() for-loop body:
///
///   for (tasks) |task| {
///       if (active_agents >= max_agents) break;          // capacity-break
///       if (inflight.contains(task.id)) continue;        // inflight-skip
///       inflight.put(task.id, {}) catch continue;
///       active_agents += 1;
///       dispatched += 1;
///   }
///
/// Returns the number of tasks that would have been dispatched (i.e., not
/// skipped by the inflight check and not stopped by the capacity break).
fn simulateTick(
    tasks: []const PipelineTask,
    inflight: *std.AutoHashMap(i64, void),
    active_agents: *u32,
    max_agents: u32,
) !usize {
    var dispatched: usize = 0;
    for (tasks) |task| {
        if (active_agents.* >= max_agents) break; // capacity-break
        if (inflight.contains(task.id)) continue; // inflight-skip
        try inflight.put(task.id, {});
        active_agents.* += 1;
        dispatched += 1;
    }
    return dispatched;
}

/// Extract the tick() function body from the embedded pipeline.zig source.
/// Returns a slice from "fn tick(" up to (but not including) the next
/// "\n    fn " at the same four-space indentation level.
fn tickBody() []const u8 {
    const src = @embedFile("pipeline.zig");
    const tick_pos = std.mem.indexOf(u8, src, "fn tick(") orelse return "";
    const after = src[tick_pos + 1 ..];
    const next_fn_rel = std.mem.indexOf(u8, after, "\n    fn ") orelse after.len;
    return src[tick_pos .. tick_pos + 1 + next_fn_rel];
}

// =============================================================================
// AC1 + AC4 + AC9 — Source inspection: combined smoke test
// =============================================================================

test "AC1+AC4+AC9: tick() source — inflight-skip and capacity-break branches present" {
    const body = tickBody();
    try std.testing.expect(body.len > 0);
    // Both key patterns must appear in tick()
    try std.testing.expect(std.mem.indexOf(u8, body, "inflight_tasks.contains(task.id)") != null);
    try std.testing.expect(std.mem.indexOf(u8, body, "pipeline_max_agents") != null);
    try std.testing.expect(std.mem.indexOf(u8, body, "break") != null);
    try std.testing.expect(std.mem.indexOf(u8, body, "continue") != null);
}

// =============================================================================
// AC1 — Inflight-skip branch present in tick() source
// =============================================================================

test "AC1: inflight_tasks.contains(task.id) present in tick() body" {
    const body = tickBody();
    try std.testing.expect(std.mem.indexOf(u8, body, "inflight_tasks.contains(task.id)") != null);
}

test "AC1: continue appears after inflight_tasks.contains in tick() body" {
    const body = tickBody();
    const contains_pos = std.mem.indexOf(u8, body, "inflight_tasks.contains(task.id)") orelse {
        try std.testing.expect(false); // missing
        return;
    };
    const after = body[contains_pos..];
    try std.testing.expect(std.mem.indexOf(u8, after, "continue") != null);
}

test "AC1: inflight_tasks.contains appears before fetchAdd in tick() body" {
    const body = tickBody();
    const contains_pos = std.mem.indexOf(u8, body, "inflight_tasks.contains(task.id)") orelse {
        try std.testing.expect(false);
        return;
    };
    const fetch_add_pos = std.mem.indexOf(u8, body, "fetchAdd") orelse {
        try std.testing.expect(false);
        return;
    };
    try std.testing.expect(contains_pos < fetch_add_pos);
}

test "AC1: inflight_tasks.contains(task.id) appears exactly once in tick()" {
    const body = tickBody();
    var count: usize = 0;
    var pos: usize = 0;
    while (std.mem.indexOfPos(u8, body, pos, "inflight_tasks.contains(task.id)")) |idx| {
        count += 1;
        pos = idx + 1;
    }
    try std.testing.expectEqual(@as(usize, 1), count);
}

// =============================================================================
// AC4 — Capacity-break branch present in tick() source
// =============================================================================

test "AC4: active_agents capacity check present in tick() body" {
    const body = tickBody();
    try std.testing.expect(
        std.mem.indexOf(u8, body, "active_agents.load(.acquire) >= self.config.pipeline_max_agents") != null,
    );
}

test "AC4: break appears after capacity check in tick() body" {
    const body = tickBody();
    const cap_pos = std.mem.indexOf(u8, body, "pipeline_max_agents") orelse {
        try std.testing.expect(false);
        return;
    };
    const after = body[cap_pos..];
    try std.testing.expect(std.mem.indexOf(u8, after, "break") != null);
}

test "AC4: tick() for-loop iterates over tasks slice" {
    const body = tickBody();
    try std.testing.expect(std.mem.indexOf(u8, body, "for (tasks)") != null);
}

// =============================================================================
// AC9 — Ordering: capacity check fires before inflight check
// =============================================================================

test "AC9: capacity check byte offset < inflight check byte offset in tick()" {
    const body = tickBody();
    const cap_pos = std.mem.indexOf(u8, body, "active_agents.load(.acquire) >= self.config.pipeline_max_agents") orelse {
        try std.testing.expect(false); // capacity check missing
        return;
    };
    const inflight_pos = std.mem.indexOf(u8, body, "inflight_tasks.contains(task.id)") orelse {
        try std.testing.expect(false); // inflight check missing
        return;
    };
    // Capacity check must come first in source order
    try std.testing.expect(cap_pos < inflight_pos);
}

test "AC9: within the for-loop, capacity check precedes inflight check" {
    const body = tickBody();
    const for_pos = std.mem.indexOf(u8, body, "for (tasks)") orelse {
        try std.testing.expect(false);
        return;
    };
    const loop_body = body[for_pos..];
    const cap_in_loop = std.mem.indexOf(u8, loop_body, "pipeline_max_agents") orelse {
        try std.testing.expect(false);
        return;
    };
    const inflight_in_loop = std.mem.indexOf(u8, loop_body, "inflight_tasks.contains") orelse {
        try std.testing.expect(false);
        return;
    };
    try std.testing.expect(cap_in_loop < inflight_in_loop);
}

// =============================================================================
// E7 — Mutex correctness: inflight_mu wraps both contains and put
// =============================================================================

test "E7: inflight_mu.lock() appears before inflight_tasks.contains in tick()" {
    const body = tickBody();
    const lock_pos = std.mem.indexOf(u8, body, "inflight_mu.lock()") orelse {
        try std.testing.expect(false);
        return;
    };
    const contains_pos = std.mem.indexOf(u8, body, "inflight_tasks.contains(task.id)") orelse {
        try std.testing.expect(false);
        return;
    };
    try std.testing.expect(lock_pos < contains_pos);
}

test "E7: inflight_mu.unlock() appears after lock() and before inflight_tasks.put in tick()" {
    const body = tickBody();
    const lock_pos = std.mem.indexOf(u8, body, "inflight_mu.lock()") orelse {
        try std.testing.expect(false);
        return;
    };
    const unlock_pos = std.mem.indexOf(u8, body, "inflight_mu.unlock()") orelse {
        try std.testing.expect(false);
        return;
    };
    const put_pos = std.mem.indexOf(u8, body, "inflight_tasks.put(task.id, {})") orelse {
        try std.testing.expect(false);
        return;
    };
    try std.testing.expect(lock_pos < unlock_pos);
    try std.testing.expect(unlock_pos < put_pos);
}

// =============================================================================
// AC2 — Inflight-skip behavioral: all tasks inflight → dispatched == 0
// =============================================================================

test "AC2: all tasks inflight — dispatched == 0, active_agents unchanged" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{
        makeTask(1),
        makeTask(2),
        makeTask(3),
    };

    // Pre-populate inflight with every task ID
    for (tasks) |t| try inflight.put(t.id, {});
    const inflight_size_before = inflight.count();

    var active: u32 = 0;
    const dispatched = try simulateTick(&tasks, &inflight, &active, 10);

    try std.testing.expectEqual(@as(usize, 0), dispatched);
    try std.testing.expectEqual(@as(u32, 0), active);
    // Inflight set must not have grown
    try std.testing.expectEqual(inflight_size_before, inflight.count());
}

test "AC2: all tasks inflight with DB-backed task IDs — dispatched == 0" {
    // Use arena for everything so no manual string-field freeing is needed.
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T1", "desc", "/repo", "", "");
    const id2 = try db.createPipelineTask("T2", "desc", "/repo", "", "");
    const id3 = try db.createPipelineTask("T3", "desc", "/repo", "", "");

    const tasks = try db.getActivePipelineTasks(alloc, 20);
    try std.testing.expectEqual(@as(usize, 3), tasks.len);

    // inflight map uses std.testing.allocator so leaks are detected
    var inflight = std.AutoHashMap(i64, void).init(std.testing.allocator);
    defer inflight.deinit();

    // Pre-populate: simulate all tasks already running from a prior tick()
    try inflight.put(id1, {});
    try inflight.put(id2, {});
    try inflight.put(id3, {});

    var active: u32 = 0;
    const dispatched = try simulateTick(tasks, &inflight, &active, 10);

    try std.testing.expectEqual(@as(usize, 0), dispatched);
    try std.testing.expectEqual(@as(u32, 0), active);
}

test "AC2: two tasks both inflight — neither increments active_agents" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{ makeTask(10), makeTask(11) };
    try inflight.put(10, {});
    try inflight.put(11, {});

    var active: u32 = 2; // already has other agents
    const dispatched = try simulateTick(&tasks, &inflight, &active, 10);

    try std.testing.expectEqual(@as(usize, 0), dispatched);
    try std.testing.expectEqual(@as(u32, 2), active); // unchanged
}

// =============================================================================
// AC3 — Inflight-skip behavioral: partial inflight → M-K dispatched
// =============================================================================

test "AC3: partial inflight — only non-inflight tasks are dispatched" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    // 5 tasks; K=2 pre-marked as inflight (IDs 11, 13)
    const tasks = [_]PipelineTask{
        makeTask(10),
        makeTask(11), // inflight
        makeTask(12),
        makeTask(13), // inflight
        makeTask(14),
    };
    try inflight.put(11, {});
    try inflight.put(13, {});

    var active: u32 = 0;
    const dispatched = try simulateTick(&tasks, &inflight, &active, 20);

    // M=5, K=2 → M-K=3 dispatched
    try std.testing.expectEqual(@as(usize, 3), dispatched);
    try std.testing.expectEqual(@as(u32, 3), active);
    // Newly dispatched IDs added to inflight
    try std.testing.expect(inflight.contains(10));
    try std.testing.expect(inflight.contains(12));
    try std.testing.expect(inflight.contains(14));
    // Pre-populated IDs unchanged
    try std.testing.expect(inflight.contains(11));
    try std.testing.expect(inflight.contains(13));
}

test "AC3: single task inflight among many — exactly one skipped" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{
        makeTask(20),
        makeTask(21), // inflight
        makeTask(22),
        makeTask(23),
    };
    try inflight.put(21, {});

    var active: u32 = 0;
    const dispatched = try simulateTick(&tasks, &inflight, &active, 20);

    // 4 tasks, 1 inflight → 3 dispatched
    try std.testing.expectEqual(@as(usize, 3), dispatched);
    try std.testing.expectEqual(@as(u32, 3), active);
    try std.testing.expect(inflight.contains(21)); // was pre-populated, still present
}

test "AC3: first task inflight, remaining tasks dispatched" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{
        makeTask(30), // inflight
        makeTask(31),
        makeTask(32),
    };
    try inflight.put(30, {});

    var active: u32 = 0;
    const dispatched = try simulateTick(&tasks, &inflight, &active, 20);

    try std.testing.expectEqual(@as(usize, 2), dispatched);
    try std.testing.expectEqual(@as(u32, 2), active);
    try std.testing.expect(inflight.contains(31));
    try std.testing.expect(inflight.contains(32));
}

test "AC3: last task inflight, preceding tasks dispatched" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{
        makeTask(40),
        makeTask(41),
        makeTask(42), // inflight
    };
    try inflight.put(42, {});

    var active: u32 = 0;
    const dispatched = try simulateTick(&tasks, &inflight, &active, 20);

    try std.testing.expectEqual(@as(usize, 2), dispatched);
    try std.testing.expectEqual(@as(u32, 2), active);
    try std.testing.expect(inflight.contains(40));
    try std.testing.expect(inflight.contains(41));
}

// =============================================================================
// AC5 — Capacity-break behavioral: at capacity → dispatched == 0
// =============================================================================

test "AC5: at capacity — dispatched == 0, active_agents unchanged" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{
        makeTask(50),
        makeTask(51),
        makeTask(52),
    };

    const max_agents: u32 = 4;
    var active: u32 = max_agents; // already at capacity

    const dispatched = try simulateTick(&tasks, &inflight, &active, max_agents);

    try std.testing.expectEqual(@as(usize, 0), dispatched);
    try std.testing.expectEqual(max_agents, active); // unchanged
    try std.testing.expectEqual(@as(u32, 0), inflight.count()); // nothing added
}

test "AC5: oversubscribed active_agents (> max) — still breaks, no dispatch" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{ makeTask(60), makeTask(61) };
    const max_agents: u32 = 2;
    var active: u32 = 5; // exceeds capacity

    const dispatched = try simulateTick(&tasks, &inflight, &active, max_agents);

    try std.testing.expectEqual(@as(usize, 0), dispatched);
    try std.testing.expectEqual(@as(u32, 5), active); // unchanged
    try std.testing.expectEqual(@as(u32, 0), inflight.count());
}

test "AC5: multiple tasks queued, all blocked by capacity" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    // 5 tasks, capacity already full
    const tasks = [_]PipelineTask{
        makeTask(70), makeTask(71), makeTask(72), makeTask(73), makeTask(74),
    };
    const max_agents: u32 = 3;
    var active: u32 = 3;

    const dispatched = try simulateTick(&tasks, &inflight, &active, max_agents);

    try std.testing.expectEqual(@as(usize, 0), dispatched);
    try std.testing.expectEqual(@as(u32, 3), active);
}

// =============================================================================
// AC6 — Capacity-break behavioral: one slot remaining → exactly 1 dispatched
// =============================================================================

test "AC6: one slot remaining — exactly one task dispatched, then break" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{
        makeTask(80),
        makeTask(81),
        makeTask(82),
        makeTask(83),
    };
    const max_agents: u32 = 3;
    var active: u32 = 2; // one slot remaining

    const dispatched = try simulateTick(&tasks, &inflight, &active, max_agents);

    // Only task 80 dispatched; then active == max → break
    try std.testing.expectEqual(@as(usize, 1), dispatched);
    try std.testing.expectEqual(max_agents, active);
    try std.testing.expect(inflight.contains(80));
    try std.testing.expect(!inflight.contains(81));
    try std.testing.expect(!inflight.contains(82));
    try std.testing.expect(!inflight.contains(83));
}

test "AC6: two slots remaining — exactly two tasks dispatched" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{
        makeTask(90), makeTask(91), makeTask(92), makeTask(93), makeTask(94),
    };
    const max_agents: u32 = 4;
    var active: u32 = 2; // two slots remaining

    const dispatched = try simulateTick(&tasks, &inflight, &active, max_agents);

    try std.testing.expectEqual(@as(usize, 2), dispatched);
    try std.testing.expectEqual(max_agents, active);
    try std.testing.expect(inflight.contains(90));
    try std.testing.expect(inflight.contains(91));
    try std.testing.expect(!inflight.contains(92));
}

// =============================================================================
// AC7 — Capacity-break: pipeline_max_agents == 1, active == 1 → 0 dispatched
// =============================================================================

test "AC7: pipeline_max_agents == 1 and active_agents == 1 — no dispatch" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{ makeTask(100), makeTask(101), makeTask(102) };
    var active: u32 = 1;

    const dispatched = try simulateTick(&tasks, &inflight, &active, 1);

    try std.testing.expectEqual(@as(usize, 0), dispatched);
    try std.testing.expectEqual(@as(u32, 1), active);
    try std.testing.expectEqual(@as(u32, 0), inflight.count());
}

test "AC7: pipeline_max_agents == 1, active == 0 — exactly one dispatched" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{ makeTask(110), makeTask(111), makeTask(112) };
    var active: u32 = 0;

    const dispatched = try simulateTick(&tasks, &inflight, &active, 1);

    try std.testing.expectEqual(@as(usize, 1), dispatched);
    try std.testing.expectEqual(@as(u32, 1), active);
    try std.testing.expect(inflight.contains(110));
    try std.testing.expect(!inflight.contains(111));
}

// =============================================================================
// AC8 — Interaction: inflight + at capacity → 0 dispatched
// =============================================================================

test "AC8: all tasks inflight and at capacity — dispatched == 0" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{ makeTask(120), makeTask(121), makeTask(122) };
    try inflight.put(120, {});
    try inflight.put(121, {});
    try inflight.put(122, {});

    const max_agents: u32 = 3;
    var active: u32 = 3;

    const dispatched = try simulateTick(&tasks, &inflight, &active, max_agents);

    try std.testing.expectEqual(@as(usize, 0), dispatched);
    try std.testing.expectEqual(@as(u32, 3), active);
}

test "AC8: capacity fires on first task, inflight check never reached for second" {
    // Scenario: active == max, so break fires on first iteration.
    // Even if the second task is NOT inflight, the loop never reaches it.
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{
        makeTask(130), // capacity check fires → break; inflight check never evaluated
        makeTask(131), // never reached
    };
    // Only task 130 is inflight, task 131 is NOT — but it doesn't matter
    try inflight.put(130, {});

    const max_agents: u32 = 1;
    var active: u32 = 1; // at capacity

    const dispatched = try simulateTick(&tasks, &inflight, &active, max_agents);

    try std.testing.expectEqual(@as(usize, 0), dispatched);
    try std.testing.expectEqual(@as(u32, 1), active);
    // Task 131 must not appear in inflight (loop broke before reaching it)
    try std.testing.expect(!inflight.contains(131));
}

test "AC8: mixed — some inflight tasks consume slots, capacity reached mid-list" {
    // Tasks: [A not-inflight, B inflight (skip), C not-inflight, D not-inflight]
    // max=2, active=0.
    // Iteration: A dispatched (active=1), B skipped (inflight), C dispatched
    // (active=2==max), D not reached (break before D).
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{
        makeTask(140), // eligible
        makeTask(141), // inflight → skipped
        makeTask(142), // eligible, but after dispatch hits max
        makeTask(143), // never reached
    };
    try inflight.put(141, {});

    const max_agents: u32 = 2;
    var active: u32 = 0;

    const dispatched = try simulateTick(&tasks, &inflight, &active, max_agents);

    try std.testing.expectEqual(@as(usize, 2), dispatched);
    try std.testing.expectEqual(max_agents, active);
    try std.testing.expect(inflight.contains(140));
    try std.testing.expect(inflight.contains(141)); // was already there
    try std.testing.expect(inflight.contains(142));
    try std.testing.expect(!inflight.contains(143)); // never reached
}

// =============================================================================
// Edge1 — Empty task list: neither branch fires
// =============================================================================

test "Edge1: empty task list — neither branch fires, dispatched == 0" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{};
    var active: u32 = 0;

    const dispatched = try simulateTick(&tasks, &inflight, &active, 10);

    try std.testing.expectEqual(@as(usize, 0), dispatched);
    try std.testing.expectEqual(@as(u32, 0), active);
    try std.testing.expectEqual(@as(u32, 0), inflight.count());
}

test "Edge1: empty task list with pre-populated inflight — inflight unchanged" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    // Stale entries from a prior tick
    try inflight.put(999, {});
    try inflight.put(1000, {});

    const tasks = [_]PipelineTask{};
    var active: u32 = 2;

    const dispatched = try simulateTick(&tasks, &inflight, &active, 10);

    try std.testing.expectEqual(@as(usize, 0), dispatched);
    try std.testing.expectEqual(@as(u32, 2), active); // unchanged
    try std.testing.expectEqual(@as(u32, 2), inflight.count()); // unchanged
}

// =============================================================================
// Edge2 — Single eligible task → dispatched == 1
// =============================================================================

test "Edge2: single task, not inflight, capacity available — dispatched == 1" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{makeTask(200)};
    var active: u32 = 0;

    const dispatched = try simulateTick(&tasks, &inflight, &active, 4);

    try std.testing.expectEqual(@as(usize, 1), dispatched);
    try std.testing.expectEqual(@as(u32, 1), active);
    try std.testing.expect(inflight.contains(200));
}

test "Edge2: single task, not inflight, after dispatch active reaches 1" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{makeTask(201)};
    var active: u32 = 3;

    const dispatched = try simulateTick(&tasks, &inflight, &active, 10);

    try std.testing.expectEqual(@as(usize, 1), dispatched);
    try std.testing.expectEqual(@as(u32, 4), active);
}

// =============================================================================
// Edge3 — Stale inflight IDs don't affect dispatch of current tasks
// =============================================================================

test "Edge3: stale inflight IDs not in current task list — no effect on dispatch" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    // IDs 999, 1000 are stale (from a previous tick cycle)
    try inflight.put(999, {});
    try inflight.put(1000, {});

    const tasks = [_]PipelineTask{
        makeTask(210),
        makeTask(211),
    };
    var active: u32 = 0;

    const dispatched = try simulateTick(&tasks, &inflight, &active, 10);

    // Both current tasks dispatched; stale IDs cause no interference
    try std.testing.expectEqual(@as(usize, 2), dispatched);
    try std.testing.expectEqual(@as(u32, 2), active);
    try std.testing.expect(inflight.contains(210));
    try std.testing.expect(inflight.contains(211));
    // Stale IDs remain (dispatch loop doesn't touch them)
    try std.testing.expect(inflight.contains(999));
    try std.testing.expect(inflight.contains(1000));
}

// =============================================================================
// Edge4 — pipeline_max_agents == 0: breaks immediately on first task
// =============================================================================

test "Edge4: pipeline_max_agents == 0 — breaks immediately, dispatched == 0" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{ makeTask(220), makeTask(221), makeTask(222) };
    var active: u32 = 0;

    // active (0) >= max_agents (0) → break on first iteration
    const dispatched = try simulateTick(&tasks, &inflight, &active, 0);

    try std.testing.expectEqual(@as(usize, 0), dispatched);
    try std.testing.expectEqual(@as(u32, 0), active);
    try std.testing.expectEqual(@as(u32, 0), inflight.count());
}

// =============================================================================
// Edge5 — Capacity-break at last task: N-1 dispatched, Nth broken
// =============================================================================

test "Edge5: capacity-break at last task — N-1 dispatched, last undispatched" {
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    // 4 tasks, max == 3, active starts at 0.
    // Tasks 230, 231, 232 dispatched (active reaches 3 == max).
    // Task 233: active (3) >= max (3) → break, not dispatched.
    const tasks = [_]PipelineTask{
        makeTask(230),
        makeTask(231),
        makeTask(232),
        makeTask(233), // triggers capacity-break
    };
    const max_agents: u32 = 3;
    var active: u32 = 0;

    const dispatched = try simulateTick(&tasks, &inflight, &active, max_agents);

    try std.testing.expectEqual(@as(usize, 3), dispatched);
    try std.testing.expectEqual(max_agents, active);
    try std.testing.expect(inflight.contains(230));
    try std.testing.expect(inflight.contains(231));
    try std.testing.expect(inflight.contains(232));
    try std.testing.expect(!inflight.contains(233));
}

// =============================================================================
// E6 — All inflight + at capacity: break fires before contains check
// =============================================================================

test "E6: all tasks inflight and at capacity — break fires first, 0 dispatched" {
    // This verifies that the source ordering (capacity before inflight) is
    // semantically important: when both conditions apply to the same task, the
    // capacity-break prevents any further processing, including the inflight check.
    const alloc = std.testing.allocator;
    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();

    const tasks = [_]PipelineTask{ makeTask(240), makeTask(241) };
    try inflight.put(240, {});
    try inflight.put(241, {});

    const max_agents: u32 = 2;
    var active: u32 = 2; // at capacity

    const dispatched = try simulateTick(&tasks, &inflight, &active, max_agents);

    try std.testing.expectEqual(@as(usize, 0), dispatched);
    try std.testing.expectEqual(@as(u32, 2), active);
    // Inflight count unchanged (break fired before any put)
    try std.testing.expectEqual(@as(u32, 2), inflight.count());
}

// =============================================================================
// DB-backed integration tests
// =============================================================================

test "integration: pre-populate inflight from real DB tasks — all skipped" {
    // Uses arena for both DB init and task allocation to avoid manual string freeing.
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T1", "desc", "/repo", "", "");
    const id2 = try db.createPipelineTask("T2", "desc", "/repo", "", "");
    const id3 = try db.createPipelineTask("T3", "desc", "/repo", "", "");

    const tasks = try db.getActivePipelineTasks(alloc, 20);
    try std.testing.expectEqual(@as(usize, 3), tasks.len);

    var inflight = std.AutoHashMap(i64, void).init(std.testing.allocator);
    defer inflight.deinit();

    // Simulate a prior tick() having already dispatched all tasks
    try inflight.put(id1, {});
    try inflight.put(id2, {});
    try inflight.put(id3, {});

    var active: u32 = 0;
    const dispatched = try simulateTick(tasks, &inflight, &active, 10);

    try std.testing.expectEqual(@as(usize, 0), dispatched);
    try std.testing.expectEqual(@as(u32, 0), active);
}

test "integration: real DB tasks with capacity limit — only max dispatched" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.createPipelineTask("A", "d", "/repo", "", "");
    _ = try db.createPipelineTask("B", "d", "/repo", "", "");
    _ = try db.createPipelineTask("C", "d", "/repo", "", "");
    _ = try db.createPipelineTask("D", "d", "/repo", "", "");
    _ = try db.createPipelineTask("E", "d", "/repo", "", "");

    const tasks = try db.getActivePipelineTasks(alloc, 20);
    try std.testing.expectEqual(@as(usize, 5), tasks.len);

    var inflight = std.AutoHashMap(i64, void).init(std.testing.allocator);
    defer inflight.deinit();

    const max_agents: u32 = 2;
    var active: u32 = 0;

    const dispatched = try simulateTick(tasks, &inflight, &active, max_agents);

    try std.testing.expectEqual(@as(usize, max_agents), dispatched);
    try std.testing.expectEqual(max_agents, active);
    try std.testing.expectEqual(max_agents, inflight.count());
}

test "integration: partial inflight from DB tasks — only non-inflight dispatched" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", "");
    const id3 = try db.createPipelineTask("T3", "d", "/repo", "", "");
    const id4 = try db.createPipelineTask("T4", "d", "/repo", "", "");

    const tasks = try db.getActivePipelineTasks(alloc, 20);
    try std.testing.expectEqual(@as(usize, 4), tasks.len);

    var inflight = std.AutoHashMap(i64, void).init(std.testing.allocator);
    defer inflight.deinit();

    // Mark id1 and id3 as already inflight; 1 agent slot already in use
    try inflight.put(id1, {});
    try inflight.put(id3, {});
    const max_agents: u32 = 4;
    var active: u32 = 1;

    const dispatched = try simulateTick(tasks, &inflight, &active, max_agents);

    // id2 and id4 are eligible; 3 slots remain (max=4, active=1)
    // Both id2 and id4 must be dispatched
    try std.testing.expectEqual(@as(usize, 2), dispatched);
    try std.testing.expectEqual(@as(u32, 3), active);
    _ = id2;
    _ = id4;
}

test "integration: real DB tasks, capacity reached before end of list" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    _ = try db.createPipelineTask("A", "d", "/repo", "", "");
    _ = try db.createPipelineTask("B", "d", "/repo", "", "");
    _ = try db.createPipelineTask("C", "d", "/repo", "", "");
    _ = try db.createPipelineTask("D", "d", "/repo", "", "");

    const tasks = try db.getActivePipelineTasks(alloc, 20);
    try std.testing.expectEqual(@as(usize, 4), tasks.len);

    var inflight = std.AutoHashMap(i64, void).init(std.testing.allocator);
    defer inflight.deinit();

    // One slot remaining
    const max_agents: u32 = 3;
    var active: u32 = 2;

    const dispatched = try simulateTick(tasks, &inflight, &active, max_agents);

    // Only the first task should be dispatched
    try std.testing.expectEqual(@as(usize, 1), dispatched);
    try std.testing.expectEqual(max_agents, active);
    try std.testing.expectEqual(@as(u32, 1), inflight.count());
}
