# Task #18: Fix memory leak in pipeline tick() — task strings never freed

## 1. Task Summary

`pipeline.zig:tick()` calls `db.getActivePipelineTasks()`, which allocates ~9 string fields per
`PipelineTask` via `allocator.dupe()`. The existing `defer self.allocator.free(tasks)` only
frees the outer slice, leaving all per-task string fields leaked — roughly 10 allocations per
task every 30-second tick. Tasks dispatched to `processTaskThread` are passed by value (the
string slices are raw pointer+length pairs); the thread never frees them, and non-dispatched
tasks (in-flight skips, capacity breaks, spawn failures) are also never freed. The companion
call in `createHealthTask()` has the same outer-slice-only free pattern.

## 2. Files to Modify

| File | What changes |
|------|--------------|
| `src/db.zig` | Add `pub fn freePipelineTask` helper that frees all string fields of a `PipelineTask` |
| `src/pipeline.zig` | `processTaskThread`: defer-free task fields; `tick()`: free non-dispatched task fields; `createHealthTask()`: switch to arena allocator |

No new files are required.

## 3. Function / Type Signatures

### 3.1 New helper — `src/db.zig`

```zig
/// Frees every heap-allocated string field in a PipelineTask that was
/// produced by rowToPipelineTask (i.e. each field was allocator.dupe()'d).
/// The integer fields (id, attempt, max_attempts) are not heap-allocated
/// and must not be freed.
pub fn freePipelineTask(allocator: std.mem.Allocator, task: PipelineTask) void {
    allocator.free(task.title);
    allocator.free(task.description);
    allocator.free(task.repo_path);
    allocator.free(task.branch);
    allocator.free(task.status);
    allocator.free(task.last_error);
    allocator.free(task.created_by);
    allocator.free(task.notify_chat);
    allocator.free(task.created_at);
    allocator.free(task.session_id);
}
```

The `PipelineTask` struct in `db.zig` is unchanged.

### 3.2 Changed — `pipeline.zig:processTaskThread`

No signature change. Add task field cleanup to the existing defer block:

```zig
fn processTaskThread(self: *Pipeline, task: db_mod.PipelineTask) void {
    defer {
        db_mod.freePipelineTask(self.allocator, task); // NEW
        _ = self.active_agents.fetchSub(1, .acq_rel);
        self.inflight_mu.lock();
        defer self.inflight_mu.unlock();
        _ = self.inflight_tasks.remove(task.id);
    }
    // ... body unchanged ...
}
```

### 3.3 Changed — `pipeline.zig:tick`

No signature change. Three non-dispatch paths each need an explicit free:

```zig
fn tick(self: *Pipeline) !void {
    const tasks = try self.db.getActivePipelineTasks(self.allocator, 20);
    defer self.allocator.free(tasks);

    if (tasks.len == 0) { ... return; }

    for (tasks, 0..) |task, i| {
        if (self.active_agents.load(.acquire) >= self.config.max_pipeline_agents) {
            // Free all tasks from this index onward that will not be dispatched.
            for (tasks[i..]) |remaining| db_mod.freePipelineTask(self.allocator, remaining);
            break;
        }

        {
            self.inflight_mu.lock();
            defer self.inflight_mu.unlock();
            if (self.inflight_tasks.contains(task.id)) {
                db_mod.freePipelineTask(self.allocator, task); // NEW — in-flight skip
                continue;
            }
            self.inflight_tasks.put(task.id, {}) catch {
                db_mod.freePipelineTask(self.allocator, task); // NEW — put failure
                continue;
            };
        }

        _ = self.active_agents.fetchAdd(1, .acq_rel);
        std.log.info("Pipeline dispatching task #{d} [{s}] in {s}: {s}",
            .{ task.id, task.status, task.repo_path, task.title });

        _ = std.Thread.spawn(.{}, processTaskThread, .{ self, task }) catch {
            _ = self.active_agents.fetchSub(1, .acq_rel);
            self.inflight_mu.lock();
            defer self.inflight_mu.unlock();
            _ = self.inflight_tasks.remove(task.id);
            db_mod.freePipelineTask(self.allocator, task); // NEW — spawn failure
            continue;
        };
        // Ownership of task's strings transferred to processTaskThread.
    }
}
```

### 3.4 Changed — `pipeline.zig:createHealthTask`

No signature change. Replace `self.allocator` with a function-scoped arena so the entire
query (outer slice + all string fields) is freed in one shot on return:

```zig
fn createHealthTask(self: *Pipeline, ...) void {
    var arena = std.heap.ArenaAllocator.init(self.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const tasks = self.db.getActivePipelineTasks(alloc, 50) catch return;
    // No explicit free needed — arena.deinit() reclaims everything.
    for (tasks) |t| {
        if (std.mem.startsWith(u8, t.title, "Fix failing ") and
            std.mem.eql(u8, t.repo_path, repo_path)) return;
    }
    // ... rest unchanged (desc/title still use self.allocator + defer free) ...
}
```

Note: `desc` and `title` inside `createHealthTask` are already correctly freed with
`defer self.allocator.free(...)` and do not need to change.

## 4. Acceptance Criteria

**AC1 — String fields freed after thread completion.**
After `processTaskThread` returns, none of the `PipelineTask` string fields (title,
description, repo_path, branch, status, last_error, created_by, notify_chat, created_at,
session_id) remain reachable via any live pointer. A test using `std.testing.allocator`
(which detects leaks) must report zero leaked bytes for a simulated processTaskThread call.

**AC2 — Non-dispatched in-flight tasks freed in tick().**
When a task is already present in `inflight_tasks` (skip via `continue`), its string fields
are freed before the loop continues to the next task.

**AC3 — Capacity-break tasks freed in tick().**
When `active_agents >= max_pipeline_agents` triggers the break, every task from the break
index to `tasks.len - 1` (inclusive) has its string fields freed before the function returns.

**AC4 — Spawn-failure tasks freed in tick().**
When `std.Thread.spawn` fails, the task whose strings were about to be handed to a thread
has all its string fields freed in the catch block.

**AC5 — createHealthTask uses arena; zero individual task-string frees needed.**
`createHealthTask` creates a local `ArenaAllocator`, passes `arena.allocator()` to
`getActivePipelineTasks`, and calls `arena.deinit()` (via defer) before returning. No call
to `freePipelineTask` is required inside `createHealthTask` for the tasks list.

**AC6 — `freePipelineTask` covers all duped fields.**
`freePipelineTask` calls `allocator.free()` on exactly the ten `[]const u8` fields that
`rowToPipelineTask` allocates: title, description, repo_path, branch, status, last_error,
created_by, notify_chat, created_at, session_id. It does not touch the integer fields id,
attempt, or max_attempts.

**AC7 — Existing unit tests pass.**
`just t` completes with no new failures. In particular `src/pipeline_stats_test.zig` and
all other tests in the suite pass without modification.

**AC8 — No double-free.**
For tasks successfully dispatched to a thread, `tick()` does not free the string fields;
only `processTaskThread` does (via its defer). A run under a leak-detecting allocator
produces no `double_free` or `use_after_free` reports.

**AC9 — Outer slice still freed by existing defer.**
The `defer self.allocator.free(tasks)` in `tick()` is retained; it frees the slice backing
array. `freePipelineTask` is called for the per-element strings separately, avoiding a
double-free on the slice memory itself.

**AC10 — No change to PipelineTask struct.**
`db.PipelineTask` fields and `rowToPipelineTask` remain unchanged. The fix is purely at the
call sites.

## 5. Edge Cases

| # | Scenario | Expected behaviour |
|---|----------|--------------------|
| E1 | `getActivePipelineTasks` returns 0 tasks | No `freePipelineTask` calls; early return path is unaffected |
| E2 | All tasks are already in-flight | Every task in the loop hits the in-flight-skip path; all have their string fields freed via `freePipelineTask` before `continue` |
| E3 | `active_agents` reaches limit at the very first task (i == 0) | `tasks[0..]` (all tasks) are freed in the break-path loop before breaking |
| E4 | `active_agents` reaches limit mid-slice (i > 0) | Only `tasks[i..]` are freed in the break-path loop; tasks `[0..i)` were already dispatched (ownership transferred to threads) |
| E5 | `inflight_tasks.put()` fails (OOM in the hashmap) | Task fields freed immediately before `continue`; `active_agents` counter is not incremented, maintaining correctness |
| E6 | `Thread.spawn` fails (OS resource exhaustion) | Task is removed from `inflight_tasks`, `active_agents` is decremented, and string fields are freed in the catch block |
| E7 | `processTaskThread` is entered but the task targets a non-primary repo (early return) | The defer block at the top of the thread still runs, freeing all string fields before the thread exits |
| E8 | Multiple tasks dispatched in one tick, some in-flight and some new | Each in-flight task is freed at skip time; each dispatched task is freed by its thread; tasks beyond the capacity limit are freed in the break loop |
| E9 | `createHealthTask` finds a duplicate and returns early (inside the for loop) | `arena.deinit()` (deferred) cleans up the tasks slice and all string fields even on the early return path |
| E10 | A `PipelineTask` string field is an empty string (`""`) | `allocator.dupe(u8, "")` returns a valid zero-length allocation; `allocator.free()` on a zero-length slice is safe and must not be skipped |
