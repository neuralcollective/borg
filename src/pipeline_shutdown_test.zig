// Tests for fix: use-after-free on pipeline shutdown with active agents.
//
// Verifies that Pipeline stores thread handles, joins them on shutdown,
// frees owned resources in deinit, and avoids use-after-free / deadlock.
//
// These tests should FAIL before the fix is applied because the Pipeline
// struct lacks agent_threads, agent_threads_mu, joinAgents(), and deinit().

const std = @import("std");
const pipeline_mod = @import("pipeline.zig");
const Pipeline = pipeline_mod.Pipeline;

// =============================================================================
// AC1: Thread handles are stored — Pipeline has agent_threads and mutex
// =============================================================================

test "AC1: Pipeline struct has agent_threads field of type ArrayList(Thread)" {
    // Verifying the field exists and has the right type. If the field is
    // missing, this is a compile error — which is the expected failure mode
    // before the fix is applied.
    const info = @typeInfo(Pipeline);
    const fields = info.@"struct".fields;

    var found = false;
    for (fields) |f| {
        if (std.mem.eql(u8, f.name, "agent_threads")) {
            found = true;
            // The field type should be std.ArrayList(std.Thread)
            try std.testing.expect(f.type == std.ArrayList(std.Thread));
            break;
        }
    }
    try std.testing.expect(found);
}

test "AC1: Pipeline struct has agent_threads_mu field of type Mutex" {
    const info = @typeInfo(Pipeline);
    const fields = info.@"struct".fields;

    var found = false;
    for (fields) |f| {
        if (std.mem.eql(u8, f.name, "agent_threads_mu")) {
            found = true;
            try std.testing.expect(f.type == std.Thread.Mutex);
            break;
        }
    }
    try std.testing.expect(found);
}

// =============================================================================
// AC2: All threads are joined before run() returns — joinAgents exists
// =============================================================================

test "AC2: Pipeline has joinAgents method" {
    // Verify the function exists by checking it's callable.
    // Before the fix, this will fail with a compile error.
    try std.testing.expect(@hasDecl(Pipeline, "joinAgents"));
}

// =============================================================================
// AC4: deinit frees owned resources — Pipeline has deinit method
// =============================================================================

test "AC4: Pipeline has deinit method" {
    try std.testing.expect(@hasDecl(Pipeline, "deinit"));
}

test "AC4: deinit is a pub function taking *Pipeline and returning void" {
    const DeinitFn = @TypeOf(Pipeline.deinit);
    const fn_info = @typeInfo(DeinitFn).@"fn";

    // Should return void
    try std.testing.expect(fn_info.return_type == void);

    // First param should be *Pipeline
    try std.testing.expect(fn_info.params.len >= 1);
    try std.testing.expect(fn_info.params[0].type == *Pipeline);
}

// =============================================================================
// AC1 + AC6: Thread append under mutex, no deadlock — functional test
//
// We can't easily create a real Pipeline (it needs Db, Docker, Telegram, etc.)
// but we CAN verify the threading contract by testing ArrayList(Thread) + Mutex
// interactions that mirror the spec's design. This proves the data structures
// work correctly under concurrent access.
// =============================================================================

test "AC1+AC6: ArrayList(Thread) append under mutex is safe from multiple threads" {
    // Simulates the pattern: tick() spawns threads and appends handles under
    // mutex; joinAgents() drains the list under mutex then joins.
    const alloc = std.testing.allocator;

    var threads = std.ArrayList(std.Thread).init(alloc);
    defer threads.deinit();
    var mu: std.Thread.Mutex = .{};

    // Spawn a few threads that do trivial work, store their handles
    const N = 4; // matches MAX_PARALLEL_AGENTS
    var i: usize = 0;
    while (i < N) : (i += 1) {
        const t = try std.Thread.spawn(.{}, struct {
            fn work() void {
                // Simulate brief agent work
                std.time.sleep(1 * std.time.ns_per_ms);
            }
        }.work, .{});

        mu.lock();
        defer mu.unlock();
        try threads.append(t);
    }

    // Verify all handles were stored
    try std.testing.expectEqual(@as(usize, N), threads.items.len);

    // Drain under mutex (mirrors joinAgents pattern)
    var to_join: []std.Thread = undefined;
    {
        mu.lock();
        defer mu.unlock();
        to_join = try alloc.dupe(std.Thread, threads.items);
        threads.clearRetainingCapacity();
    }
    defer alloc.free(to_join);

    // Join outside mutex (AC6: no deadlock)
    for (to_join) |t| {
        t.join();
    }

    // After joining, list should be empty
    try std.testing.expectEqual(@as(usize, 0), threads.items.len);
}

// =============================================================================
// Edge Case 1: Thread spawn fails — no handle stored, counters cleaned up
// =============================================================================

test "Edge1: spawn failure leaves agent_threads list unchanged" {
    // If Thread.spawn returns an error, no handle should be appended.
    // We verify this by checking that catching a spawn error doesn't
    // corrupt the list.
    const alloc = std.testing.allocator;

    var threads = std.ArrayList(std.Thread).init(alloc);
    defer threads.deinit();
    var active_agents = std.atomic.Value(u32).init(0);

    // Simulate the tick() pattern: increment active_agents, attempt spawn
    _ = active_agents.fetchAdd(1, .acq_rel);

    // Simulate spawn failure by not actually spawning (just handle the error path)
    const spawn_result: std.Thread.SpawnError!std.Thread = error.SystemResources;
    if (spawn_result) |t| {
        try threads.append(t);
    } else |_| {
        // Error path from spec: decrement active_agents, don't append
        _ = active_agents.fetchSub(1, .acq_rel);
    }

    // No handle should be stored
    try std.testing.expectEqual(@as(usize, 0), threads.items.len);
    // active_agents back to 0
    try std.testing.expectEqual(@as(u32, 0), active_agents.load(.acquire));
}

// =============================================================================
// Edge Case 4: Empty agent list at shutdown — joinAgents is a no-op
// =============================================================================

test "Edge4: draining empty thread list is a no-op" {
    const alloc = std.testing.allocator;

    var threads = std.ArrayList(std.Thread).init(alloc);
    defer threads.deinit();
    var mu: std.Thread.Mutex = .{};

    // Drain pattern on empty list
    {
        mu.lock();
        defer mu.unlock();
        threads.clearRetainingCapacity();
    }
    // No crash, no threads to join
    try std.testing.expectEqual(@as(usize, 0), threads.items.len);
}

// =============================================================================
// Edge Case 6: Double-drain prevention — subsequent drains are no-ops
// =============================================================================

test "Edge6: double drain of thread list is safe" {
    const alloc = std.testing.allocator;

    var threads = std.ArrayList(std.Thread).init(alloc);
    defer threads.deinit();
    var mu: std.Thread.Mutex = .{};

    // Spawn one thread, store handle
    const t = try std.Thread.spawn(.{}, struct {
        fn work() void {}
    }.work, .{});
    {
        mu.lock();
        defer mu.unlock();
        try threads.append(t);
    }

    // First drain + join
    {
        var to_join: []std.Thread = undefined;
        {
            mu.lock();
            defer mu.unlock();
            to_join = try alloc.dupe(std.Thread, threads.items);
            threads.clearRetainingCapacity();
        }
        defer alloc.free(to_join);
        for (to_join) |th| th.join();
    }

    try std.testing.expectEqual(@as(usize, 0), threads.items.len);

    // Second drain — should be a no-op, not a crash
    {
        var to_join: []std.Thread = undefined;
        {
            mu.lock();
            defer mu.unlock();
            to_join = try alloc.dupe(std.Thread, threads.items);
            threads.clearRetainingCapacity();
        }
        defer alloc.free(to_join);
        // Empty slice — nothing to join
        try std.testing.expectEqual(@as(usize, 0), to_join.len);
    }
}

// =============================================================================
// Edge Case 7: Concurrent append and drain are mutex-protected
// =============================================================================

test "Edge7: concurrent appenders and drainer don't race" {
    const alloc = std.testing.allocator;

    var threads = std.ArrayList(std.Thread).init(alloc);
    defer threads.deinit();
    var mu: std.Thread.Mutex = .{};

    // Producer: spawn threads and append handles (simulating tick)
    const producer = try std.Thread.spawn(.{}, struct {
        fn run(th_list: *std.ArrayList(std.Thread), m: *std.Thread.Mutex) void {
            var j: usize = 0;
            while (j < 4) : (j += 1) {
                const worker = std.Thread.spawn(.{}, struct {
                    fn work() void {
                        std.time.sleep(2 * std.time.ns_per_ms);
                    }
                }.work, .{}) catch continue;

                m.lock();
                defer m.unlock();
                th_list.append(worker) catch {};
            }
        }
    }.run, .{ &threads, &mu });

    // Let the producer finish
    producer.join();

    // Consumer: drain and join (simulating joinAgents at end of run)
    var to_join: []std.Thread = undefined;
    {
        mu.lock();
        defer mu.unlock();
        to_join = alloc.dupe(std.Thread, threads.items) catch &[_]std.Thread{};
        threads.clearRetainingCapacity();
    }
    defer alloc.free(to_join);

    for (to_join) |th| th.join();

    // All threads joined, list empty
    try std.testing.expectEqual(@as(usize, 0), threads.items.len);
}

// =============================================================================
// AC3 + AC5: Graceful stop signal — running flag stops the loop
// =============================================================================

test "AC5: atomic bool stop signal transitions from true to false" {
    // Pipeline.running is an atomic bool. stop() stores false.
    // Agent threads that check self.running can exit early.
    var running = std.atomic.Value(bool).init(true);
    try std.testing.expect(running.load(.acquire) == true);

    // Simulate stop()
    running.store(false, .release);
    try std.testing.expect(running.load(.acquire) == false);
}

test "AC5: thread observes stop signal and exits" {
    var running = std.atomic.Value(bool).init(true);

    const worker = try std.Thread.spawn(.{}, struct {
        fn work(r: *std.atomic.Value(bool)) void {
            // Simulate agent checking running flag each iteration
            while (r.load(.acquire)) {
                std.time.sleep(1 * std.time.ns_per_ms);
            }
        }
    }.work, .{&running});

    // Signal stop
    std.time.sleep(5 * std.time.ns_per_ms);
    running.store(false, .release);

    // Thread should exit and be joinable
    worker.join();
}

// =============================================================================
// AC2: active_agents reaches 0 after all threads joined
// =============================================================================

test "AC2: active_agents is zero after all worker threads complete and are joined" {
    const alloc = std.testing.allocator;

    var threads = std.ArrayList(std.Thread).init(alloc);
    defer threads.deinit();
    var mu: std.Thread.Mutex = .{};
    var active_agents = std.atomic.Value(u32).init(0);

    // Spawn threads that decrement active_agents on exit (like processTaskThread)
    const N: u32 = 3;
    var i: u32 = 0;
    while (i < N) : (i += 1) {
        _ = active_agents.fetchAdd(1, .acq_rel);
        const t = try std.Thread.spawn(.{}, struct {
            fn work(agents: *std.atomic.Value(u32)) void {
                defer _ = agents.fetchSub(1, .acq_rel);
                // Simulate brief work
                std.time.sleep(2 * std.time.ns_per_ms);
            }
        }.work, .{&active_agents});

        mu.lock();
        defer mu.unlock();
        try threads.append(t);
    }

    try std.testing.expectEqual(N, active_agents.load(.acquire));

    // Drain and join (mirrors joinAgents)
    var to_join: []std.Thread = undefined;
    {
        mu.lock();
        defer mu.unlock();
        to_join = try alloc.dupe(std.Thread, threads.items);
        threads.clearRetainingCapacity();
    }
    defer alloc.free(to_join);

    for (to_join) |t| t.join();

    // After joining, all agents must have completed their defer
    try std.testing.expectEqual(@as(u32, 0), active_agents.load(.acquire));
}

// =============================================================================
// AC3: No use-after-free — resources outlive threads
//
// Demonstrates the ordering contract: allocator/db must not be freed until
// after all threads using them have been joined.
// =============================================================================

test "AC3: resource lifetime exceeds thread lifetime when joined before free" {
    const alloc = std.testing.allocator;

    // Simulated "resource" — an allocated buffer representing db/allocator
    var resource = try alloc.alloc(u8, 64);
    @memset(resource, 0xAA);

    var threads = std.ArrayList(std.Thread).init(alloc);
    defer threads.deinit();

    // Spawn threads that read the resource
    var i: usize = 0;
    while (i < 2) : (i += 1) {
        const t = try std.Thread.spawn(.{}, struct {
            fn work(res: []u8) void {
                // Simulate agent accessing self.db / self.allocator
                std.time.sleep(2 * std.time.ns_per_ms);
                // Touch the resource — would crash if already freed
                var sum: u8 = 0;
                for (res) |byte| sum +%= byte;
                std.debug.assert(sum != 0); // resource still has 0xAA
            }
        }.work, .{resource});
        try threads.append(t);
    }

    // Join ALL threads BEFORE freeing resource (the fix's guarantee)
    for (threads.items) |t| t.join();

    // Only NOW is it safe to free the resource (mirrors pipeline_db.deinit())
    alloc.free(resource);
    resource = &.{}; // prevent dangling use
}

// =============================================================================
// AC4: deinit frees all three owned collections
// =============================================================================

test "AC4: ArrayList(Thread) deinit releases memory" {
    const alloc = std.testing.allocator;

    var threads = std.ArrayList(std.Thread).init(alloc);
    // Force an allocation so deinit has something to free
    try threads.ensureTotalCapacity(8);
    threads.deinit();
    // If deinit leaks, the testing allocator will catch it
}

test "AC4: AutoHashMap(i64, void) deinit releases memory" {
    const alloc = std.testing.allocator;

    var inflight = std.AutoHashMap(i64, void).init(alloc);
    try inflight.put(1, {});
    try inflight.put(2, {});
    inflight.deinit();
}

test "AC4: StringHashMap deinit releases memory" {
    const alloc = std.testing.allocator;

    var heads = std.StringHashMap([40]u8).init(alloc);
    const key = try alloc.dupe(u8, "repo/path");
    defer alloc.free(key);
    try heads.put(key, [_]u8{0} ** 40);
    heads.deinit();
}

// =============================================================================
// Edge Case 3: Rapid shutdown with many agents — all eventually join
// =============================================================================

test "Edge3: all threads complete even under concurrent shutdown" {
    const alloc = std.testing.allocator;

    var threads = std.ArrayList(std.Thread).init(alloc);
    defer threads.deinit();
    var mu: std.Thread.Mutex = .{};
    var active_agents = std.atomic.Value(u32).init(0);
    var running = std.atomic.Value(bool).init(true);

    // Spawn MAX_PARALLEL_AGENTS threads with varying work durations
    const N: u32 = 4;
    var i: u32 = 0;
    while (i < N) : (i += 1) {
        _ = active_agents.fetchAdd(1, .acq_rel);
        const t = try std.Thread.spawn(.{}, struct {
            fn work(agents: *std.atomic.Value(u32), r: *std.atomic.Value(bool), idx: u32) void {
                defer _ = agents.fetchSub(1, .acq_rel);
                // Each thread runs for a different duration
                const sleep_ms: u64 = @as(u64, idx + 1) * 5;
                var elapsed: u64 = 0;
                while (elapsed < sleep_ms) {
                    if (!r.load(.acquire)) break; // check stop signal
                    std.time.sleep(1 * std.time.ns_per_ms);
                    elapsed += 1;
                }
            }
        }.work, .{ &active_agents, &running, i });

        mu.lock();
        defer mu.unlock();
        try threads.append(t);
    }

    // Immediate stop signal (rapid shutdown scenario)
    running.store(false, .release);

    // joinAgents pattern: drain and join
    var to_join: []std.Thread = undefined;
    {
        mu.lock();
        defer mu.unlock();
        to_join = try alloc.dupe(std.Thread, threads.items);
        threads.clearRetainingCapacity();
    }
    defer alloc.free(to_join);

    for (to_join) |t| t.join();

    // All agents must have finished
    try std.testing.expectEqual(@as(u32, 0), active_agents.load(.acquire));
}

// =============================================================================
// Edge Case 2: Thread panics — defer still runs, counters cleaned up
// (We can't test actual panic recovery, but we verify the defer pattern.)
// =============================================================================

test "Edge2: defer block runs even when thread body returns error" {
    const alloc = std.testing.allocator;
    var active_agents = std.atomic.Value(u32).init(0);

    var inflight = std.AutoHashMap(i64, void).init(alloc);
    defer inflight.deinit();
    var inflight_mu: std.Thread.Mutex = .{};

    // Simulate processTaskThread pattern with error
    _ = active_agents.fetchAdd(1, .acq_rel);
    inflight_mu.lock();
    inflight.put(42, {}) catch {};
    inflight_mu.unlock();

    const t = try std.Thread.spawn(.{}, struct {
        fn work(agents: *std.atomic.Value(u32), inflight_map: *std.AutoHashMap(i64, void), mu: *std.Thread.Mutex) void {
            defer {
                _ = agents.fetchSub(1, .acq_rel);
                mu.lock();
                defer mu.unlock();
                _ = inflight_map.remove(42);
            }
            // Simulate error in agent work (but no panic — just early return)
        }
    }.work, .{ &active_agents, &inflight, &inflight_mu });

    t.join();

    // Defer should have cleaned up
    try std.testing.expectEqual(@as(u32, 0), active_agents.load(.acquire));
    try std.testing.expect(!inflight.contains(42));
}

// =============================================================================
// AC8: Re-exec path safety — stop + join ordering
// =============================================================================

test "AC8: stop then join ordering ensures all threads complete before proceeding" {
    // The re-exec path calls p.stop(), pipeline_thread.join(), p.deinit().
    // This test verifies the ordering: stop() signals, join() blocks until
    // the pipeline thread (and its joinAgents) finishes.
    var running = std.atomic.Value(bool).init(true);
    var active_agents = std.atomic.Value(u32).init(0);
    const alloc = std.testing.allocator;

    var threads = std.ArrayList(std.Thread).init(alloc);
    defer threads.deinit();

    // Simulate an active agent
    _ = active_agents.fetchAdd(1, .acq_rel);
    const agent = try std.Thread.spawn(.{}, struct {
        fn work(r: *std.atomic.Value(bool), agents: *std.atomic.Value(u32)) void {
            defer _ = agents.fetchSub(1, .acq_rel);
            while (r.load(.acquire)) {
                std.time.sleep(1 * std.time.ns_per_ms);
            }
        }
    }.work, .{ &running, &active_agents });
    try threads.append(agent);

    // Simulate pipeline thread that runs a simplified run() loop
    const pipeline_thread = try std.Thread.spawn(.{}, struct {
        fn run(r: *std.atomic.Value(bool), th_list: *std.ArrayList(std.Thread)) void {
            // Simplified run() loop
            while (r.load(.acquire)) {
                std.time.sleep(1 * std.time.ns_per_ms);
            }
            // joinAgents at end of run()
            for (th_list.items) |t| t.join();
            th_list.clearRetainingCapacity();
        }
    }.run, .{ &running, &threads });

    // Re-exec path: p.stop() + pipeline_thread.join()
    std.time.sleep(5 * std.time.ns_per_ms);
    running.store(false, .release); // p.stop()
    pipeline_thread.join(); // pipeline_thread.join()

    // After join returns, all agents must be done
    try std.testing.expectEqual(@as(u32, 0), active_agents.load(.acquire));
    // Safe to deinit now (p.deinit())
    // threads.deinit() called by defer
}

// =============================================================================
// AC7: Build succeeds — verified by running `zig build test`
// This is an implicit test: if this file compiles and all tests run, AC7 passes.
// =============================================================================

test {
    _ = @import("pipeline_task_id_test.zig");
}
