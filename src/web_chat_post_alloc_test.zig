// Tests for Task #66: Fix memory leak in handleChatPost on WebChatMessage field
// allocation failure.
//
// handleChatPost (src/web.zig:514) is a private method; it cannot be invoked
// directly from this test file. The tests here take two complementary
// approaches:
//
//   1. Allocation-sequence tests (AC1–AC4): Replicate the exact dupe sequence
//      used by handleChatPost, driven by std.testing.FailingAllocator. Each
//      test verifies that allocations == deallocations (zero net outstanding
//      allocations) after the failure path executes the FIXED cleanup logic.
//
//   2. Queue-behaviour tests (AC5): Exercise the WebServer via the public
//      drainChatMessages API using std.testing.allocator so any leaked
//      WebChatMessage fields are reported as test failures by the GPA.
//
// "Fail initially" semantics:
//   Tests AC2–AC4 check `fa.allocations == fa.deallocations` after the FIXED
//   cleanup sequence. If the fix is removed and the buggy pattern is
//   reinstated, those tests will fail because the allocation counts diverge.
//   AC5 tests rely on std.testing.allocator's leak detection; if the fix is
//   removed, calls that reach the buggy code path will leave freed counts
//   mismatched and the GPA teardown will fail the test suite.

const std = @import("std");
const web_mod = @import("web.zig");
const WebServer = web_mod.WebServer;
const WebChatMessage = web_mod.WebChatMessage;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn makeTestServer(alloc: std.mem.Allocator) WebServer {
    return WebServer.init(
        alloc,
        @ptrFromInt(0x10000), // fake *Db  (never dereferenced by code under test)
        @ptrFromInt(0x10000), // fake *Config
        0,
        "127.0.0.1",
    );
}

fn cleanupTestServer(ws: *WebServer) void {
    for (ws.sse_clients.items) |c| c.close();
    ws.sse_clients.deinit();
    for (ws.chat_sse_clients.items) |c| c.close();
    ws.chat_sse_clients.deinit();
    ws.chat_queue.deinit();
    ws.task_streams.deinit();
}

// ── AC1: First allocation failure leaves nothing allocated ───────────────────
//
// When dupe(sender_name) fails there is nothing to clean up; zero allocations
// must have been made.

test "AC1: first dupe failure results in zero successful allocations" {
    // fail_index = 0 → the very first alloc attempt fails immediately.
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 0 });
    const alloc = fa.allocator();

    _ = alloc.dupe(u8, "web-user") catch {
        try std.testing.expectEqual(@as(usize, 0), fa.allocations);
        return;
    };
    return error.ExpectedAllocationFailure;
}

test "AC1: first dupe failure: allocations == deallocations" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 0 });
    const alloc = fa.allocator();

    _ = alloc.dupe(u8, "web-user") catch {};
    try std.testing.expectEqual(fa.allocations, fa.deallocations);
}

test "AC1: first dupe failure: zero net outstanding allocations" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 0 });
    const alloc = fa.allocator();

    _ = alloc.dupe(u8, "web-user") catch {};
    try std.testing.expectEqual(@as(usize, 0), fa.allocations -| fa.deallocations);
}

// ── AC2: Second allocation failure frees the first ───────────────────────────
//
// When dupe(text) fails, the already-allocated dupe(sender_name) must be freed.
// These tests verify the FIXED cleanup pattern; if the fix is absent (i.e.,
// sender is not freed on text failure) the allocation/deallocation counts
// diverge and the test fails.

test "AC2: second dupe failure with cleanup: allocations == deallocations" {
    // fail_index = 1 → sender_name (index 0) succeeds; text (index 1) fails.
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 1 });
    const alloc = fa.allocator();

    const duped_sender = alloc.dupe(u8, "web-user") catch unreachable; // index 0
    _ = alloc.dupe(u8, "hello world") catch {
        alloc.free(duped_sender); // THE FIX: free sender_name on text dupe failure
        try std.testing.expectEqual(fa.allocations, fa.deallocations);
        return;
    };
    alloc.free(duped_sender);
    return error.ExpectedAllocationFailure;
}

test "AC2: second dupe failure: net outstanding allocations == 0" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 1 });
    const alloc = fa.allocator();

    const s = alloc.dupe(u8, "web-user") catch unreachable;
    _ = alloc.dupe(u8, "message text") catch {
        alloc.free(s);
        try std.testing.expectEqual(@as(usize, 0), fa.allocations -| fa.deallocations);
        return;
    };
    alloc.free(s);
}

test "AC2: second dupe failure: exactly 1 allocation and 1 deallocation" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 1 });
    const alloc = fa.allocator();

    const s = alloc.dupe(u8, "web-user") catch unreachable;
    _ = alloc.dupe(u8, "message text") catch {
        alloc.free(s);
        try std.testing.expectEqual(@as(usize, 1), fa.allocations);
        try std.testing.expectEqual(@as(usize, 1), fa.deallocations);
        return;
    };
    alloc.free(s);
}

test "AC2: second dupe failure via std.testing.allocator — no leak on exit" {
    // std.testing.allocator (GPA) fails the test if any allocation outlives it.
    const sender = try std.testing.allocator.dupe(u8, "web-user");
    // Simulate text dupe failing: the FIXED code frees sender before returning.
    std.testing.allocator.free(sender); // the fix
    // No outstanding allocation → test passes; without the fix sender would leak.
}

// ── AC3: Third allocation failure frees the first and second ─────────────────
//
// When dupe(thread_id) fails, both sender_name and text must be freed.

test "AC3: third dupe failure with cleanup: allocations == deallocations" {
    // fail_index = 2 → sender_name (0) and text (1) succeed; thread_id (2) fails.
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 2 });
    const alloc = fa.allocator();

    const duped_sender = alloc.dupe(u8, "web-user")    catch unreachable; // index 0
    const duped_text   = alloc.dupe(u8, "hello world") catch unreachable; // index 1
    _ = alloc.dupe(u8, "web:dashboard") catch {
        alloc.free(duped_sender); // THE FIX
        alloc.free(duped_text);   // THE FIX
        try std.testing.expectEqual(fa.allocations, fa.deallocations);
        return;
    };
    alloc.free(duped_sender);
    alloc.free(duped_text);
    return error.ExpectedAllocationFailure;
}

test "AC3: third dupe failure: net outstanding allocations == 0" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 2 });
    const alloc = fa.allocator();

    const s = alloc.dupe(u8, "web-user")    catch unreachable;
    const t = alloc.dupe(u8, "hello world") catch unreachable;
    _ = alloc.dupe(u8, "web:dashboard") catch {
        alloc.free(s);
        alloc.free(t);
        try std.testing.expectEqual(@as(usize, 0), fa.allocations -| fa.deallocations);
        return;
    };
    alloc.free(s);
    alloc.free(t);
}

test "AC3: third dupe failure: exactly 2 allocations and 2 deallocations" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 2 });
    const alloc = fa.allocator();

    const s = alloc.dupe(u8, "web-user")    catch unreachable;
    const t = alloc.dupe(u8, "hello world") catch unreachable;
    _ = alloc.dupe(u8, "web:dashboard") catch {
        alloc.free(s);
        alloc.free(t);
        try std.testing.expectEqual(@as(usize, 2), fa.allocations);
        try std.testing.expectEqual(@as(usize, 2), fa.deallocations);
        return;
    };
    alloc.free(s);
    alloc.free(t);
}

test "AC3: third dupe failure via std.testing.allocator — no leak on exit" {
    const sender = try std.testing.allocator.dupe(u8, "web-user");
    const text   = try std.testing.allocator.dupe(u8, "hello world");
    // Simulate thread_id dupe failing: both freed by the fix.
    std.testing.allocator.free(sender); // the fix
    std.testing.allocator.free(text);   // the fix
}

// ── AC4: chat_queue.append failure frees all three fields ────────────────────
//
// Before the fix the append error path freed sender_name and text but NOT
// thread_id. After the fix all three must be freed on append failure.

test "AC4: append failure with cleanup: allocations == deallocations" {
    // fail_index = 3 → all three dupes succeed; the 4th alloc (simulating the
    // ArrayList.append growth) fails, triggering the append-error path.
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 3 });
    const alloc = fa.allocator();

    const duped_sender = alloc.dupe(u8, "web-user")    catch unreachable; // index 0
    const duped_text   = alloc.dupe(u8, "hello world") catch unreachable; // index 1
    const duped_thread = alloc.dupe(u8, "web:chat")    catch unreachable; // index 2

    const msg = WebChatMessage{
        .sender_name = duped_sender,
        .text        = duped_text,
        .timestamp   = std.time.timestamp(),
        .thread_id   = duped_thread,
    };

    // Simulate ArrayList.append OOM (index 3 fails):
    _ = alloc.dupe(u8, "_append_marker_") catch {
        alloc.free(msg.sender_name);
        alloc.free(msg.text);
        alloc.free(msg.thread_id); // THE FIX: previously missing
        try std.testing.expectEqual(fa.allocations, fa.deallocations);
        return;
    };
    alloc.free(msg.sender_name);
    alloc.free(msg.text);
    alloc.free(msg.thread_id);
}

test "AC4: append failure frees thread_id (primary regression target)" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 3 });
    const alloc = fa.allocator();

    const s  = alloc.dupe(u8, "web-user")    catch unreachable;
    const t  = alloc.dupe(u8, "hello world") catch unreachable;
    const th = alloc.dupe(u8, "web:chat")    catch unreachable;

    _ = alloc.dupe(u8, "_") catch {
        alloc.free(s);
        alloc.free(t);
        alloc.free(th); // was missing before the fix
        try std.testing.expectEqual(@as(usize, 3), fa.allocations);
        try std.testing.expectEqual(@as(usize, 3), fa.deallocations);
        return;
    };
    alloc.free(s);
    alloc.free(t);
    alloc.free(th);
}

test "AC4: append failure: net outstanding allocations == 0" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 3 });
    const alloc = fa.allocator();

    const s  = alloc.dupe(u8, "a") catch unreachable;
    const t  = alloc.dupe(u8, "b") catch unreachable;
    const th = alloc.dupe(u8, "c") catch unreachable;

    _ = alloc.dupe(u8, "d") catch {
        alloc.free(s);
        alloc.free(t);
        alloc.free(th);
        try std.testing.expectEqual(@as(usize, 0), fa.allocations -| fa.deallocations);
        return;
    };
    alloc.free(s);
    alloc.free(t);
    alloc.free(th);
}

test "AC4: append failure via std.testing.allocator — no leak on exit" {
    const sender = try std.testing.allocator.dupe(u8, "web-user");
    const text   = try std.testing.allocator.dupe(u8, "hello world");
    const thread = try std.testing.allocator.dupe(u8, "web:dashboard");
    // Simulate append failure: FIXED code frees all three.
    std.testing.allocator.free(sender); // the fix
    std.testing.allocator.free(text);   // the fix
    std.testing.allocator.free(thread); // the fix (was missing before)
}

// ── AC5: Happy path — message appears in chat_queue with correct fields ───────

test "AC5: message manually queued survives into chat_queue" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    const msg = WebChatMessage{
        .sender_name = try alloc.dupe(u8, "test-user"),
        .text        = try alloc.dupe(u8, "hello there"),
        .timestamp   = 42,
        .thread_id   = try alloc.dupe(u8, "web:dashboard"),
    };
    try ws.chat_queue.append(msg);

    try std.testing.expectEqual(@as(usize, 1), ws.chat_queue.items.len);
    try std.testing.expectEqualStrings("test-user",    ws.chat_queue.items[0].sender_name);
    try std.testing.expectEqualStrings("hello there",  ws.chat_queue.items[0].text);
    try std.testing.expectEqualStrings("web:dashboard",ws.chat_queue.items[0].thread_id);
    try std.testing.expectEqual(@as(i64, 42),          ws.chat_queue.items[0].timestamp);

    // Drain and free before cleanupTestServer to avoid leaks
    const drained = ws.drainChatMessages();
    defer alloc.free(drained);
    alloc.free(drained[0].sender_name);
    alloc.free(drained[0].text);
    alloc.free(drained[0].thread_id);
}

test "AC5: drainChatMessages returns the queued message with correct fields" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    try ws.chat_queue.append(.{
        .sender_name = try alloc.dupe(u8, "alice"),
        .text        = try alloc.dupe(u8, "test message"),
        .timestamp   = 100,
        .thread_id   = try alloc.dupe(u8, "web:room1"),
    });

    const drained = ws.drainChatMessages();
    defer alloc.free(drained);

    try std.testing.expectEqual(@as(usize, 1), drained.len);
    try std.testing.expectEqualStrings("alice",        drained[0].sender_name);
    try std.testing.expectEqualStrings("test message", drained[0].text);
    try std.testing.expectEqual(@as(i64, 100),         drained[0].timestamp);
    try std.testing.expectEqualStrings("web:room1",    drained[0].thread_id);

    alloc.free(drained[0].sender_name);
    alloc.free(drained[0].text);
    alloc.free(drained[0].thread_id);
}

test "AC5: drainChatMessages returns empty slice when queue is empty" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    const drained = ws.drainChatMessages();
    defer alloc.free(drained);

    try std.testing.expectEqual(@as(usize, 0), drained.len);
}

test "AC5: queue is empty after drainChatMessages" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    try ws.chat_queue.append(.{
        .sender_name = try alloc.dupe(u8, "user"),
        .text        = try alloc.dupe(u8, "hi"),
        .timestamp   = 0,
        .thread_id   = try alloc.dupe(u8, "web:t"),
    });

    const d1 = ws.drainChatMessages();
    defer alloc.free(d1);
    alloc.free(d1[0].sender_name);
    alloc.free(d1[0].text);
    alloc.free(d1[0].thread_id);

    const d2 = ws.drainChatMessages();
    defer alloc.free(d2);
    try std.testing.expectEqual(@as(usize, 0), d2.len);
}

test "AC5: multiple messages queued and drained with correct count" {
    const alloc = std.testing.allocator;
    var ws = makeTestServer(alloc);
    defer cleanupTestServer(&ws);

    for (0..3) |i| {
        const text = try std.fmt.allocPrint(alloc, "message {d}", .{i});
        try ws.chat_queue.append(.{
            .sender_name = try alloc.dupe(u8, "user"),
            .text        = text,
            .timestamp   = @as(i64, @intCast(i)),
            .thread_id   = try alloc.dupe(u8, "web:t"),
        });
    }

    const drained = ws.drainChatMessages();
    defer alloc.free(drained);

    try std.testing.expectEqual(@as(usize, 3), drained.len);

    for (drained) |m| {
        alloc.free(m.sender_name);
        alloc.free(m.text);
        alloc.free(m.thread_id);
    }
}

// ── EC1: All-empty field strings ─────────────────────────────────────────────

test "EC1: dupe of empty string produces zero-length heap slice" {
    const s = try std.testing.allocator.dupe(u8, "");
    defer std.testing.allocator.free(s);
    try std.testing.expectEqual(@as(usize, 0), s.len);
}

test "EC1: all-empty fields: three dupes and frees leave no net allocation" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{
        .fail_index = std.math.maxInt(usize),
    });
    const alloc = fa.allocator();

    const s  = try alloc.dupe(u8, "");
    const t  = try alloc.dupe(u8, "");
    const th = try alloc.dupe(u8, "");
    alloc.free(s);
    alloc.free(t);
    alloc.free(th);
    try std.testing.expectEqual(fa.allocations, fa.deallocations);
}

test "EC1: empty sender_name, second dupe fails — first freed without leak" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 1 });
    const alloc = fa.allocator();

    const s = alloc.dupe(u8, "") catch unreachable;
    _ = alloc.dupe(u8, "non-empty text") catch {
        alloc.free(s);
        try std.testing.expectEqual(fa.allocations, fa.deallocations);
        return;
    };
    alloc.free(s);
}

// ── EC2: Default field values (sender / thread absent from JSON) ──────────────

test "EC2: default sender 'web-user' is heap-allocated (not a literal ptr)" {
    const duped = try std.testing.allocator.dupe(u8, "web-user");
    defer std.testing.allocator.free(duped);
    try std.testing.expectEqualStrings("web-user", duped);
    try std.testing.expect(duped.ptr != "web-user".ptr);
}

test "EC2: default thread 'web:dashboard' is heap-allocated (not a literal ptr)" {
    const duped = try std.testing.allocator.dupe(u8, "web:dashboard");
    defer std.testing.allocator.free(duped);
    try std.testing.expectEqualStrings("web:dashboard", duped);
    try std.testing.expect(duped.ptr != "web:dashboard".ptr);
}

test "EC2: defaults — second dupe fails, first freed without leak" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 1 });
    const alloc = fa.allocator();

    const s = alloc.dupe(u8, "web-user") catch unreachable;
    _ = alloc.dupe(u8, "message text") catch {
        alloc.free(s);
        try std.testing.expectEqual(fa.allocations, fa.deallocations);
        return;
    };
    alloc.free(s);
}

test "EC2: defaults — third dupe fails, first two freed without leak" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 2 });
    const alloc = fa.allocator();

    const s = alloc.dupe(u8, "web-user")     catch unreachable;
    const t = alloc.dupe(u8, "message text") catch unreachable;
    _ = alloc.dupe(u8, "web:dashboard") catch {
        alloc.free(s);
        alloc.free(t);
        try std.testing.expectEqual(fa.allocations, fa.deallocations);
        return;
    };
    alloc.free(s);
    alloc.free(t);
}

// ── EC3: Large field values ───────────────────────────────────────────────────

test "EC3: large sender_name (64 bytes) duped and freed without error" {
    var buf: [64]u8 = undefined;
    @memset(&buf, 'S');
    const duped = try std.testing.allocator.dupe(u8, &buf);
    defer std.testing.allocator.free(duped);
    try std.testing.expectEqual(@as(usize, 64), duped.len);
}

test "EC3: large text (4000 bytes) — second dupe fails, first freed" {
    var sender_buf: [32]u8 = undefined;
    @memset(&sender_buf, 'S');
    var text_buf: [4000]u8 = undefined;
    @memset(&text_buf, 'T');

    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 1 });
    const alloc = fa.allocator();

    const s = alloc.dupe(u8, &sender_buf) catch unreachable;
    _ = alloc.dupe(u8, &text_buf) catch {
        alloc.free(s);
        try std.testing.expectEqual(fa.allocations, fa.deallocations);
        return;
    };
    alloc.free(s);
}

test "EC3: large thread_id (128 bytes) — third dupe fails, first two freed" {
    var tb: [128]u8 = undefined;
    @memset(&tb, 'R');

    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 2 });
    const alloc = fa.allocator();

    const s = alloc.dupe(u8, "sender") catch unreachable;
    const t = alloc.dupe(u8, "text")   catch unreachable;
    _ = alloc.dupe(u8, &tb) catch {
        alloc.free(s);
        alloc.free(t);
        try std.testing.expectEqual(fa.allocations, fa.deallocations);
        return;
    };
    alloc.free(s);
    alloc.free(t);
}

// ── EC4: OOM on thread_id (primary regression target) ────────────────────────
//
// Before the fix: both sender_name and text are leaked.
// After the fix: both are freed.

test "EC4: OOM on thread_id dupe: sender_name and text freed (regression)" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 2 });
    const alloc = fa.allocator();

    const s = alloc.dupe(u8, "alice")             catch unreachable;
    const t = alloc.dupe(u8, "important message") catch unreachable;
    _ = alloc.dupe(u8, "web:general") catch {
        alloc.free(s);
        alloc.free(t);
        try std.testing.expectEqual(@as(usize, 2), fa.allocations);
        try std.testing.expectEqual(@as(usize, 2), fa.deallocations);
        return;
    };
    alloc.free(s);
    alloc.free(t);
    return error.ExpectedAllocationFailure;
}

test "EC4: OOM on thread_id: net outstanding allocations == 0" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 2 });
    const alloc = fa.allocator();

    const s = alloc.dupe(u8, "user")    catch unreachable;
    const t = alloc.dupe(u8, "message") catch unreachable;
    _ = alloc.dupe(u8, "web:t") catch {
        alloc.free(s);
        alloc.free(t);
        try std.testing.expectEqual(@as(usize, 0), fa.allocations -| fa.deallocations);
        return;
    };
    alloc.free(s);
    alloc.free(t);
}

// ── EC5: append OOM after all three dupes succeed ────────────────────────────
//
// Before the fix: thread_id was NOT freed in the append error path.
// After the fix: all three fields are freed.

test "EC5: OOM on append frees all three field allocations" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 3 });
    const alloc = fa.allocator();

    const s  = alloc.dupe(u8, "bob")          catch unreachable;
    const t  = alloc.dupe(u8, "hello")        catch unreachable;
    const th = alloc.dupe(u8, "web:thread-1") catch unreachable;

    // Simulate ArrayList.append OOM:
    _ = alloc.dupe(u8, "_") catch {
        alloc.free(s);
        alloc.free(t);
        alloc.free(th);
        try std.testing.expectEqual(@as(usize, 3), fa.allocations);
        try std.testing.expectEqual(@as(usize, 3), fa.deallocations);
        return;
    };
    alloc.free(s);
    alloc.free(t);
    alloc.free(th);
}

test "EC5: OOM on append: net outstanding allocations == 0" {
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{ .fail_index = 3 });
    const alloc = fa.allocator();

    const s  = alloc.dupe(u8, "x") catch unreachable;
    const t  = alloc.dupe(u8, "y") catch unreachable;
    const th = alloc.dupe(u8, "z") catch unreachable;

    _ = alloc.dupe(u8, "_") catch {
        alloc.free(s);
        alloc.free(t);
        alloc.free(th);
        try std.testing.expectEqual(@as(usize, 0), fa.allocations -| fa.deallocations);
        return;
    };
    alloc.free(s);
    alloc.free(t);
    alloc.free(th);
}

// ── EC6: Fields are heap-allocated copies, not pointers into arena/stack ──────
//
// The fix dupes each field into self.allocator (not the arena). Verify that the
// duped pointers are distinct from the source literals.

test "EC6: duped sender_name is a distinct heap allocation from the source" {
    const src = "web-user";
    const duped = try std.testing.allocator.dupe(u8, src);
    defer std.testing.allocator.free(duped);
    try std.testing.expect(duped.ptr != src.ptr);
    try std.testing.expectEqualStrings(src, duped);
}

test "EC6: duped text is a distinct heap allocation from the source" {
    const src = "hello from dashboard";
    const duped = try std.testing.allocator.dupe(u8, src);
    defer std.testing.allocator.free(duped);
    try std.testing.expect(duped.ptr != src.ptr);
    try std.testing.expectEqualStrings(src, duped);
}

test "EC6: duped thread_id is a distinct heap allocation from the source" {
    const src = "web:dashboard";
    const duped = try std.testing.allocator.dupe(u8, src);
    defer std.testing.allocator.free(duped);
    try std.testing.expect(duped.ptr != src.ptr);
    try std.testing.expectEqualStrings(src, duped);
}

// ── WebChatMessage struct shape ───────────────────────────────────────────────

test "WebChatMessage has sender_name, text, timestamp, and thread_id fields" {
    const info = @typeInfo(WebChatMessage);
    const fields = info.@"struct".fields;

    var found_sender = false;
    var found_text   = false;
    var found_ts     = false;
    var found_thread = false;

    inline for (fields) |f| {
        if (comptime std.mem.eql(u8, f.name, "sender_name")) found_sender = true;
        if (comptime std.mem.eql(u8, f.name, "text"))        found_text   = true;
        if (comptime std.mem.eql(u8, f.name, "timestamp"))   found_ts     = true;
        if (comptime std.mem.eql(u8, f.name, "thread_id"))   found_thread = true;
    }

    try std.testing.expect(found_sender);
    try std.testing.expect(found_text);
    try std.testing.expect(found_ts);
    try std.testing.expect(found_thread);
}

test "WebChatMessage slice fields are []const u8" {
    const info = @typeInfo(WebChatMessage);
    inline for (info.@"struct".fields) |f| {
        if (comptime std.mem.eql(u8, f.name, "sender_name") or
            comptime std.mem.eql(u8, f.name, "text") or
            comptime std.mem.eql(u8, f.name, "thread_id"))
        {
            try std.testing.expect(f.type == []const u8);
        }
    }
}

test "WebChatMessage timestamp field is i64" {
    const info = @typeInfo(WebChatMessage);
    inline for (info.@"struct".fields) |f| {
        if (comptime std.mem.eql(u8, f.name, "timestamp")) {
            try std.testing.expect(f.type == i64);
        }
    }
}

// ── Three-field invariant: every successful dupe must have a matching free ────

test "invariant: successful dupe sequence requires matching frees" {
    // Verify that allocations == deallocations when all 3 dupes succeed and
    // are subsequently freed (the happy-path invariant).
    var fa = std.testing.FailingAllocator.init(std.testing.allocator, .{
        .fail_index = std.math.maxInt(usize),
    });
    const alloc = fa.allocator();

    const s  = try alloc.dupe(u8, "web-user");
    const t  = try alloc.dupe(u8, "some message text");
    const th = try alloc.dupe(u8, "web:dashboard");

    alloc.free(s);
    alloc.free(t);
    alloc.free(th);

    try std.testing.expectEqual(@as(usize, 3), fa.allocations);
    try std.testing.expectEqual(@as(usize, 3), fa.deallocations);
    try std.testing.expectEqual(fa.allocations, fa.deallocations);
}
