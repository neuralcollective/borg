# Spec: Fix use-after-free on pipeline shutdown with active agents

## Task Summary

Spawned `processTaskThread` threads in `pipeline.zig` are detached (handles discarded at line 139) because the return value of `std.Thread.spawn` is ignored. During shutdown, `Pipeline.run()` polls `active_agents` for up to 30 seconds then returns regardless, after which `main.zig` destroys `pipeline_db` and the GPA allocator. Detached agent threads that are still running will access freed `self.db` and `self.allocator`, causing use-after-free. The fix is to store thread handles and join them all during shutdown, ensuring no agent thread outlives the resources it depends on.

## Files to Modify

1. **`src/pipeline.zig`** — Store thread handles, add `deinit`, join threads on shutdown.
2. **`src/main.zig`** — Call `Pipeline.deinit()` during cleanup (if needed beyond existing `stop()`/join pattern).

## Files to Create

None.

## Function/Type Signatures

### `src/pipeline.zig`

#### Struct field additions to `Pipeline`

```zig
// Add to Pipeline struct fields:
agent_threads: std.ArrayList(std.Thread),
agent_threads_mu: std.Thread.Mutex,
```

#### `Pipeline.init` — modify to initialize new fields

```zig
// Add to the return struct literal:
.agent_threads = std.ArrayList(std.Thread).init(allocator),
.agent_threads_mu = .{},
```

#### `Pipeline.deinit` — new function

```zig
pub fn deinit(self: *Pipeline) void
```

Frees `agent_threads`, `inflight_tasks`, and `startup_heads`. Called after all threads have been joined.

#### `Pipeline.joinAgents` — new function

```zig
fn joinAgents(self: *Pipeline) void
```

Locks `agent_threads_mu`, drains the `agent_threads` list, unlocks, then joins each thread. Called from the end of `Pipeline.run()` replacing the current 30-second poll loop.

#### `Pipeline.tick` — modify thread spawn (line 139)

Change from discarding the thread handle:
```zig
// Current (broken):
_ = std.Thread.spawn(.{}, processTaskThread, .{ self, task }) catch { ... };

// New:
const t = std.Thread.spawn(.{}, processTaskThread, .{ self, task }) catch { ... };
self.agent_threads_mu.lock();
defer self.agent_threads_mu.unlock();
self.agent_threads.append(t) catch {};
```

#### `Pipeline.run` — modify shutdown sequence (lines 94-101)

Replace the 30-second `active_agents` poll loop with:
```zig
self.joinAgents();
```

This blocks until all spawned agent threads have actually exited, instead of giving up after 30 seconds.

### `src/main.zig`

#### Shutdown sequence (lines 509-515)

Add `pipeline.deinit()` call after joining the pipeline thread:
```zig
defer {
    if (pipeline) |*p| {
        p.stop();
        if (pipeline_thread) |t| t.join();
        p.deinit();
    }
    if (pipeline_db) |*pdb| pdb.deinit();
}
```

Similarly update the re-exec path (lines 729-731):
```zig
if (pipeline) |*p| {
    p.stop();
    if (pipeline_thread) |t| t.join();
    p.deinit();
}
```

## Acceptance Criteria

1. **Thread handles are stored**: Every successfully spawned `processTaskThread` thread handle is appended to `agent_threads` under `agent_threads_mu`.
2. **All threads are joined before `run()` returns**: `Pipeline.run()` calls `joinAgents()` which joins every thread in `agent_threads`. After `joinAgents()` returns, `active_agents` is 0.
3. **No use-after-free**: `pipeline_db.deinit()` and allocator destruction in `main.zig` only happen after `pipeline_thread.join()` returns, which only happens after all agent threads are joined. No agent thread can access `self.db` or `self.allocator` after they are freed.
4. **`deinit` frees owned resources**: `Pipeline.deinit()` calls `.deinit()` on `agent_threads`, `inflight_tasks`, and `startup_heads`.
5. **Graceful stop signal**: Agent threads that check `self.running` can exit early when `stop()` is called. Threads blocked on Docker I/O will complete naturally and then be joined.
6. **No deadlock**: `agent_threads_mu` is only held briefly during `append` (in `tick`) and during the drain (in `joinAgents`). It is never held while joining a thread. `processTaskThread` does not acquire `agent_threads_mu`.
7. **Build succeeds**: `zig build` and `zig build test` pass without errors.
8. **Re-exec path is safe**: The self-update code path (line 727+) also joins all agent threads via `p.stop()` + `pipeline_thread.join()` before calling `execve`.

## Edge Cases

1. **Thread spawn fails**: If `std.Thread.spawn` returns an error, no handle is stored, `active_agents` is decremented, and `inflight_tasks` entry is removed (existing behavior preserved).
2. **Agent thread panics**: A panicking thread will still be joinable; `join()` will return and propagate. The defer block in `processTaskThread` ensures `active_agents` and `inflight_tasks` are cleaned up even on error.
3. **Rapid shutdown with many agents**: If 4 agents (MAX_PARALLEL_AGENTS) are running long Docker operations when shutdown is requested, `joinAgents()` blocks until all 4 complete. The existing `AGENT_TIMEOUT_S` (600s) watchdog kills Docker containers, so threads will eventually return.
4. **Empty agent list at shutdown**: If no agents were ever spawned, `joinAgents()` is a no-op (empty list).
5. **`agent_threads` grows unboundedly during long runs**: Completed threads remain in the list until `joinAgents()` is called. Consider periodically reaping finished threads, but since MAX_PARALLEL_AGENTS is 4 and threads are short-lived relative to tick intervals, the list stays small in practice. A future optimization could reap joined threads in `tick`, but this is not required for correctness.
6. **Double-join prevention**: Once `joinAgents()` drains the list, subsequent calls are no-ops. The `stop()` + `pipeline_thread.join()` in `main.zig` ensures `run()` has returned (and thus `joinAgents()` has completed) before `deinit()` is called.
7. **Concurrent append and drain**: `agent_threads_mu` protects both `append` (from `tick` on the pipeline thread) and the drain (from `joinAgents` at end of `run`, also on the pipeline thread). Since both happen on the same thread, contention is minimal, but the mutex is still needed because `processTaskThread` could theoretically interact with the list in future changes.
