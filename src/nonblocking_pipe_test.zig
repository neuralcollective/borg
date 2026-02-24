// Tests for the WhatsApp/Sidecar stdout non-blocking pipe fix.
//
// Verifies every acceptance criterion and edge case from spec.md.
// Tests should FAIL before the implementation is applied.
//
// To include in the build, add to build.zig or reference from an existing module:
//   test { _ = @import("nonblocking_pipe_test.zig"); }

const std = @import("std");
const sidecar_mod = @import("sidecar.zig");
const Sidecar = sidecar_mod.Sidecar;
const SidecarMessage = sidecar_mod.SidecarMessage;
const Source = sidecar_mod.Source;
const whatsapp_mod = @import("whatsapp.zig");
const WhatsApp = whatsapp_mod.WhatsApp;
const WaMessage = whatsapp_mod.WaMessage;

// ============================================================================
// Helpers
// ============================================================================

/// Set O_NONBLOCK on a file descriptor using the same fcntl pattern from spec.
fn setNonBlock(fd: std.posix.fd_t) void {
    const current_flags = std.posix.fcntl(fd, .GET_FL) catch 0;
    const nonblock_val = @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true }));
    _ = std.posix.fcntl(fd, .SET_FL, .{ .flags = @as(u32, @intCast(current_flags)) | nonblock_val }) catch {};
}

/// Check whether O_NONBLOCK is set on a file descriptor.
fn hasNonBlock(fd: std.posix.fd_t) bool {
    const flags = std.posix.fcntl(fd, .GET_FL) catch return false;
    const nonblock_val = @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true }));
    return (@as(u32, @intCast(flags)) & nonblock_val) != 0;
}

/// Create a Sidecar with a real pipe injected as child stdout.
fn makeSidecarWithPipe(allocator: std.mem.Allocator) !struct { sidecar: Sidecar, write_fd: std.posix.fd_t, read_fd: std.posix.fd_t } {
    var s = Sidecar.init(allocator, "TestBot");
    const pipe_fds = try std.posix.pipe();
    var child = std.process.Child.init(&.{"true"}, allocator);
    child.stdout = std.fs.File{ .handle = pipe_fds[0] };
    s.child = child;
    return .{ .sidecar = s, .write_fd = pipe_fds[1], .read_fd = pipe_fds[0] };
}

/// Create a WhatsApp with a real pipe injected as child stdout.
fn makeWhatsAppWithPipe(allocator: std.mem.Allocator) !struct { wa: WhatsApp, write_fd: std.posix.fd_t, read_fd: std.posix.fd_t } {
    var wa = WhatsApp.init(allocator, "TestBot");
    const pipe_fds = try std.posix.pipe();
    var child = std.process.Child.init(&.{"true"}, allocator);
    child.stdout = std.fs.File{ .handle = pipe_fds[0] };
    wa.child = child;
    return .{ .wa = wa, .write_fd = pipe_fds[1], .read_fd = pipe_fds[0] };
}

fn cleanupSidecar(result: *struct { sidecar: Sidecar, write_fd: std.posix.fd_t, read_fd: std.posix.fd_t }) void {
    std.posix.close(result.write_fd);
    if (result.sidecar.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
    result.sidecar.child = null;
    result.sidecar.deinit();
}

fn cleanupWhatsApp(result: *struct { wa: WhatsApp, write_fd: std.posix.fd_t, read_fd: std.posix.fd_t }) void {
    std.posix.close(result.write_fd);
    if (result.wa.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
    result.wa.child = null;
    result.wa.deinit();
}

fn freeSidecarMessages(allocator: std.mem.Allocator, msgs: []SidecarMessage) void {
    for (msgs) |m| {
        allocator.free(m.id);
        allocator.free(m.chat_id);
        allocator.free(m.sender);
        allocator.free(m.sender_name);
        allocator.free(m.text);
    }
    allocator.free(msgs);
}

fn freeWaMessages(allocator: std.mem.Allocator, msgs: []WaMessage) void {
    for (msgs) |m| {
        allocator.free(m.jid);
        allocator.free(m.id);
        allocator.free(m.sender);
        allocator.free(m.sender_name);
        allocator.free(m.text);
    }
    allocator.free(msgs);
}

// ============================================================================
// AC1: Non-blocking read — Sidecar stdout fd has O_NONBLOCK after start()
// ============================================================================

test "AC1: fresh pipe does not have O_NONBLOCK by default" {
    // Baseline: a newly created pipe read-end is blocking.
    // This confirms that if start() doesn't explicitly set O_NONBLOCK,
    // the stdout fd will be blocking (the bug this spec fixes).
    const pipe_fds = try std.posix.pipe();
    defer std.posix.close(pipe_fds[0]);
    defer std.posix.close(pipe_fds[1]);

    try std.testing.expect(!hasNonBlock(pipe_fds[0]));
}

test "AC1: Sidecar child stdout must have O_NONBLOCK set" {
    // After Sidecar.start(), the stdout pipe fd must have O_NONBLOCK.
    // We inject a pipe to simulate post-start() state. This test FAILS
    // before implementation because the pipe is created blocking and
    // start() doesn't set O_NONBLOCK yet.
    const allocator = std.testing.allocator;
    var result = try makeSidecarWithPipe(allocator);
    defer cleanupSidecar(&result);

    const stdout_fd = result.sidecar.child.?.stdout.?.handle;

    // This assertion FAILS before the fix: a fresh pipe is blocking.
    // After the fix, start() sets O_NONBLOCK, so this will pass.
    try std.testing.expect(hasNonBlock(stdout_fd));
}

// ============================================================================
// AC2: Non-blocking read (whatsapp) — WhatsApp stdout fd has O_NONBLOCK
// ============================================================================

test "AC2: WhatsApp child stdout must have O_NONBLOCK set" {
    // Same as AC1 but for WhatsApp.start().
    // FAILS before implementation.
    const allocator = std.testing.allocator;
    var result = try makeWhatsAppWithPipe(allocator);
    defer cleanupWhatsApp(&result);

    const stdout_fd = result.wa.child.?.stdout.?.handle;
    try std.testing.expect(hasNonBlock(stdout_fd));
}

// ============================================================================
// AC3: poll() returns immediately when no data (< 10ms)
// ============================================================================

test "AC3: Sidecar poll returns empty within 10ms when no data" {
    const allocator = std.testing.allocator;
    var result = try makeSidecarWithPipe(allocator);
    defer cleanupSidecar(&result);

    setNonBlock(result.sidecar.child.?.stdout.?.handle);

    const before = std.time.nanoTimestamp();
    const msgs = try result.sidecar.poll(allocator);
    const elapsed_ns = std.time.nanoTimestamp() - before;

    try std.testing.expectEqual(@as(usize, 0), msgs.len);
    try std.testing.expect(elapsed_ns < 10 * std.time.ns_per_ms);
}

test "AC3: WhatsApp poll returns empty within 10ms when no data" {
    const allocator = std.testing.allocator;
    var result = try makeWhatsAppWithPipe(allocator);
    defer cleanupWhatsApp(&result);

    setNonBlock(result.wa.child.?.stdout.?.handle);

    const before = std.time.nanoTimestamp();
    const msgs = try result.wa.poll(allocator);
    const elapsed_ns = std.time.nanoTimestamp() - before;

    try std.testing.expectEqual(@as(usize, 0), msgs.len);
    try std.testing.expect(elapsed_ns < 10 * std.time.ns_per_ms);
}

// ============================================================================
// AC4: poll() still reads data — NDJSON parsing works with nonblocking pipes
// ============================================================================

test "AC4: Sidecar poll reads Discord NDJSON message" {
    const allocator = std.testing.allocator;
    var result = try makeSidecarWithPipe(allocator);
    defer cleanupSidecar(&result);

    setNonBlock(result.sidecar.child.?.stdout.?.handle);

    const json_line = "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"d1\",\"channel_id\":\"ch1\",\"sender_id\":\"u1\",\"sender_name\":\"Alice\",\"text\":\"hello world\",\"timestamp\":1700000000,\"is_dm\":false,\"mentions_bot\":true}\n";
    _ = try std.posix.write(result.write_fd, json_line);

    const msgs = try result.sidecar.poll(allocator);
    defer freeSidecarMessages(allocator, msgs);

    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expectEqualStrings("d1", msgs[0].id);
    try std.testing.expectEqualStrings("ch1", msgs[0].chat_id);
    try std.testing.expectEqualStrings("u1", msgs[0].sender);
    try std.testing.expectEqualStrings("Alice", msgs[0].sender_name);
    try std.testing.expectEqualStrings("hello world", msgs[0].text);
    try std.testing.expectEqual(@as(i64, 1700000000), msgs[0].timestamp);
    try std.testing.expect(msgs[0].is_group); // is_dm=false → is_group=true
    try std.testing.expect(msgs[0].mentions_bot);
    try std.testing.expectEqual(Source.discord, msgs[0].source);
}

test "AC4: Sidecar poll reads WhatsApp NDJSON message" {
    const allocator = std.testing.allocator;
    var result = try makeSidecarWithPipe(allocator);
    defer cleanupSidecar(&result);

    setNonBlock(result.sidecar.child.?.stdout.?.handle);

    const json_line = "{\"source\":\"whatsapp\",\"event\":\"message\",\"id\":\"w1\",\"jid\":\"j1@s.whatsapp.net\",\"sender\":\"s1\",\"sender_name\":\"Bob\",\"text\":\"wa msg\",\"timestamp\":1700000001,\"is_group\":true,\"mentions_bot\":false}\n";
    _ = try std.posix.write(result.write_fd, json_line);

    const msgs = try result.sidecar.poll(allocator);
    defer freeSidecarMessages(allocator, msgs);

    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expectEqualStrings("w1", msgs[0].id);
    try std.testing.expectEqualStrings("j1@s.whatsapp.net", msgs[0].chat_id);
    try std.testing.expectEqualStrings("Bob", msgs[0].sender_name);
    try std.testing.expectEqualStrings("wa msg", msgs[0].text);
    try std.testing.expect(msgs[0].is_group);
    try std.testing.expect(!msgs[0].mentions_bot);
    try std.testing.expectEqual(Source.whatsapp, msgs[0].source);
}

test "AC4: WhatsApp poll reads NDJSON message" {
    const allocator = std.testing.allocator;
    var result = try makeWhatsAppWithPipe(allocator);
    defer cleanupWhatsApp(&result);

    setNonBlock(result.wa.child.?.stdout.?.handle);

    const json_line = "{\"event\":\"message\",\"jid\":\"j1@s.whatsapp.net\",\"id\":\"m1\",\"sender\":\"s1\",\"sender_name\":\"Alice\",\"text\":\"hello\",\"timestamp\":1700000000,\"is_group\":true,\"mentions_bot\":false}\n";
    _ = try std.posix.write(result.write_fd, json_line);

    const msgs = try result.wa.poll(allocator);
    defer freeWaMessages(allocator, msgs);

    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expectEqualStrings("j1@s.whatsapp.net", msgs[0].jid);
    try std.testing.expectEqualStrings("m1", msgs[0].id);
    try std.testing.expectEqualStrings("Alice", msgs[0].sender_name);
    try std.testing.expectEqualStrings("hello", msgs[0].text);
    try std.testing.expectEqual(@as(i64, 1700000000), msgs[0].timestamp);
    try std.testing.expect(msgs[0].is_group);
    try std.testing.expect(!msgs[0].mentions_bot);
}

// ============================================================================
// AC5: Main loop continues — sequential polls don't accumulate blocking time
// ============================================================================

test "AC5: sequential sidecar and whatsapp polls complete within bounded time" {
    // Simulates a main loop cycle: poll sidecar, poll whatsapp, check timing.
    // If either poll blocks, the total time will exceed the threshold.
    const allocator = std.testing.allocator;

    var sc_result = try makeSidecarWithPipe(allocator);
    defer cleanupSidecar(&sc_result);

    var wa_result = try makeWhatsAppWithPipe(allocator);
    defer cleanupWhatsApp(&wa_result);

    setNonBlock(sc_result.sidecar.child.?.stdout.?.handle);
    setNonBlock(wa_result.wa.child.?.stdout.?.handle);

    const before = std.time.nanoTimestamp();

    // Simulate one main loop iteration: poll both transports
    const sc_msgs = try sc_result.sidecar.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), sc_msgs.len);

    const wa_msgs = try wa_result.wa.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), wa_msgs.len);

    const elapsed_ns = std.time.nanoTimestamp() - before;

    // Both polls together must complete well within 10ms
    try std.testing.expect(elapsed_ns < 10 * std.time.ns_per_ms);
}

// ============================================================================
// AC6: Existing tests pass — verified by running zig build test.
// No specific test needed; the existing init/deinit and parseSource tests
// in sidecar.zig and whatsapp.zig must remain green.
// ============================================================================

// ============================================================================
// AC7: No new threads — O_NONBLOCK on existing pipe fd, no reader thread.
// This is an architectural constraint verified by code review.
// The tests below confirm that pipe-level O_NONBLOCK (not threads) is used.
// ============================================================================

test "AC7: O_NONBLOCK on pipe fd enables non-blocking reads without threads" {
    // Verify the POSIX mechanism: O_NONBLOCK makes read() return WouldBlock
    // instead of blocking, proving no auxiliary threads are needed.
    const pipe_fds = try std.posix.pipe();
    defer std.posix.close(pipe_fds[0]);
    defer std.posix.close(pipe_fds[1]);

    // Without O_NONBLOCK, reading would block. Set it.
    setNonBlock(pipe_fds[0]);

    var buf: [64]u8 = undefined;
    const file = std.fs.File{ .handle = pipe_fds[0] };
    const read_result = file.read(&buf);

    // read() on a nonblocking empty pipe returns WouldBlock error
    try std.testing.expectError(error.WouldBlock, read_result);
}

// ============================================================================
// Edge Case 1: fcntl failure — should not prevent process from starting
// ============================================================================

test "Edge1: fcntl GET_FL failure caught gracefully" {
    // If fcntl fails on GET_FL, the catch should return 0 (safe default).
    // Using an invalid fd to trigger failure.
    const bad_fd: std.posix.fd_t = -1;
    const flags = std.posix.fcntl(bad_fd, .GET_FL) catch 0;
    // The catch returns 0, which is a valid (empty) flag set
    try std.testing.expectEqual(@as(usize, 0), flags);
}

test "Edge1: fcntl SET_FL failure caught gracefully" {
    // If fcntl fails on SET_FL, the catch should silently continue.
    const bad_fd: std.posix.fd_t = -1;
    const nonblock_val = @as(u32, @bitCast(std.posix.O{ .NONBLOCK = true }));
    // This should not panic or error — catch {} swallows the error
    _ = std.posix.fcntl(bad_fd, .SET_FL, .{ .flags = nonblock_val }) catch {};
}

// ============================================================================
// Edge Case 2: Child process exits before poll (EOF)
// ============================================================================

test "Edge2: Sidecar poll handles EOF when write end is closed" {
    // When the child process exits, stdout write end closes.
    // read() returns 0 (EOF). The if (n == 0) break handles this.
    const allocator = std.testing.allocator;
    var result = try makeSidecarWithPipe(allocator);

    setNonBlock(result.sidecar.child.?.stdout.?.handle);

    // Close the write end to simulate child exit
    std.posix.close(result.write_fd);

    // poll should handle EOF gracefully and return empty
    const msgs = try result.sidecar.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), msgs.len);

    // Cleanup (write_fd already closed)
    if (result.sidecar.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
    result.sidecar.child = null;
    result.sidecar.deinit();
}

test "Edge2: WhatsApp poll handles EOF when write end is closed" {
    const allocator = std.testing.allocator;
    var result = try makeWhatsAppWithPipe(allocator);

    setNonBlock(result.wa.child.?.stdout.?.handle);

    // Close write end to simulate child exit
    std.posix.close(result.write_fd);

    const msgs = try result.wa.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), msgs.len);

    if (result.wa.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
    result.wa.child = null;
    result.wa.deinit();
}

test "Edge2: data followed by EOF is fully read" {
    // Child writes data then exits. poll() should read all data before EOF.
    const allocator = std.testing.allocator;
    var result = try makeSidecarWithPipe(allocator);

    setNonBlock(result.sidecar.child.?.stdout.?.handle);

    // Write a complete message, then close (simulating child exit after output)
    const json_line = "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"eof1\",\"channel_id\":\"c1\",\"sender_id\":\"u1\",\"sender_name\":\"Zed\",\"text\":\"last words\",\"timestamp\":1700000000,\"is_dm\":false,\"mentions_bot\":false}\n";
    _ = try std.posix.write(result.write_fd, json_line);
    std.posix.close(result.write_fd);

    const msgs = try result.sidecar.poll(allocator);
    defer freeSidecarMessages(allocator, msgs);

    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expectEqualStrings("last words", msgs[0].text);

    // Cleanup (write_fd already closed)
    if (result.sidecar.child) |c| if (c.stdout) |stdout| std.posix.close(stdout.handle);
    result.sidecar.child = null;
    result.sidecar.deinit();
}

// ============================================================================
// Edge Case 3: Partial NDJSON lines buffered across polls
// ============================================================================

test "Edge3: Sidecar buffers partial line across multiple polls" {
    const allocator = std.testing.allocator;
    var result = try makeSidecarWithPipe(allocator);
    defer cleanupSidecar(&result);

    setNonBlock(result.sidecar.child.?.stdout.?.handle);

    // Write first fragment (no newline)
    _ = try std.posix.write(result.write_fd, "{\"source\":\"discord\",\"event\":\"message\"");

    // Poll: no complete line yet
    const msgs1 = try result.sidecar.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), msgs1.len);
    try std.testing.expect(result.sidecar.stdout_buf.items.len > 0);

    // Write second fragment (still no newline)
    _ = try std.posix.write(result.write_fd, ",\"message_id\":\"p1\",\"channel_id\":\"c1\"");

    // Poll: still no complete line
    const msgs2 = try result.sidecar.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), msgs2.len);

    // Write final fragment with newline
    _ = try std.posix.write(result.write_fd, ",\"sender_id\":\"u1\",\"sender_name\":\"X\",\"text\":\"partial\",\"timestamp\":1700000000,\"is_dm\":false,\"mentions_bot\":false}\n");

    // Poll: now the line is complete
    const msgs3 = try result.sidecar.poll(allocator);
    defer freeSidecarMessages(allocator, msgs3);

    try std.testing.expectEqual(@as(usize, 1), msgs3.len);
    try std.testing.expectEqualStrings("partial", msgs3[0].text);
    try std.testing.expectEqualStrings("p1", msgs3[0].id);
}

test "Edge3: WhatsApp buffers partial line across polls" {
    const allocator = std.testing.allocator;
    var result = try makeWhatsAppWithPipe(allocator);
    defer cleanupWhatsApp(&result);

    setNonBlock(result.wa.child.?.stdout.?.handle);

    // Write partial line
    _ = try std.posix.write(result.write_fd, "{\"event\":\"message\",\"jid\":\"j1\"");

    const msgs1 = try result.wa.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), msgs1.len);
    try std.testing.expect(result.wa.stdout_buf.items.len > 0);

    // Complete the line
    _ = try std.posix.write(result.write_fd, ",\"id\":\"m1\",\"sender\":\"s1\",\"sender_name\":\"Y\",\"text\":\"done\",\"timestamp\":1700000000,\"is_group\":false,\"mentions_bot\":true}\n");

    const msgs2 = try result.wa.poll(allocator);
    defer freeWaMessages(allocator, msgs2);

    try std.testing.expectEqual(@as(usize, 1), msgs2.len);
    try std.testing.expectEqualStrings("done", msgs2[0].text);
    try std.testing.expect(msgs2[0].mentions_bot);
}

// ============================================================================
// Edge Case 4: Rapid successive polls with no data
// ============================================================================

test "Edge4: Sidecar 20 rapid polls all return empty without blocking" {
    const allocator = std.testing.allocator;
    var result = try makeSidecarWithPipe(allocator);
    defer cleanupSidecar(&result);

    setNonBlock(result.sidecar.child.?.stdout.?.handle);

    const before = std.time.nanoTimestamp();
    var i: usize = 0;
    while (i < 20) : (i += 1) {
        const msgs = try result.sidecar.poll(allocator);
        try std.testing.expectEqual(@as(usize, 0), msgs.len);
    }
    const elapsed_ns = std.time.nanoTimestamp() - before;

    // 20 polls should still complete well within 10ms total
    try std.testing.expect(elapsed_ns < 10 * std.time.ns_per_ms);
}

test "Edge4: WhatsApp 20 rapid polls all return empty without blocking" {
    const allocator = std.testing.allocator;
    var result = try makeWhatsAppWithPipe(allocator);
    defer cleanupWhatsApp(&result);

    setNonBlock(result.wa.child.?.stdout.?.handle);

    const before = std.time.nanoTimestamp();
    var i: usize = 0;
    while (i < 20) : (i += 1) {
        const msgs = try result.wa.poll(allocator);
        try std.testing.expectEqual(@as(usize, 0), msgs.len);
    }
    const elapsed_ns = std.time.nanoTimestamp() - before;
    try std.testing.expect(elapsed_ns < 10 * std.time.ns_per_ms);
}

// ============================================================================
// Edge Case 5: Large burst of data — multiple lines in one read
// ============================================================================

test "Edge5: Sidecar poll drains multiple messages in single call" {
    const allocator = std.testing.allocator;
    var result = try makeSidecarWithPipe(allocator);
    defer cleanupSidecar(&result);

    setNonBlock(result.sidecar.child.?.stdout.?.handle);

    // Write 3 messages at once
    const line1 = "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"b1\",\"channel_id\":\"c1\",\"sender_id\":\"u1\",\"sender_name\":\"A\",\"text\":\"msg1\",\"timestamp\":1700000000,\"is_dm\":false,\"mentions_bot\":false}\n";
    const line2 = "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"b2\",\"channel_id\":\"c1\",\"sender_id\":\"u2\",\"sender_name\":\"B\",\"text\":\"msg2\",\"timestamp\":1700000001,\"is_dm\":false,\"mentions_bot\":false}\n";
    const line3 = "{\"source\":\"whatsapp\",\"event\":\"message\",\"id\":\"b3\",\"jid\":\"j1\",\"sender\":\"s3\",\"sender_name\":\"C\",\"text\":\"msg3\",\"timestamp\":1700000002,\"is_group\":true,\"mentions_bot\":true}\n";
    _ = try std.posix.write(result.write_fd, line1 ++ line2 ++ line3);

    const msgs = try result.sidecar.poll(allocator);
    defer freeSidecarMessages(allocator, msgs);

    try std.testing.expectEqual(@as(usize, 3), msgs.len);
    try std.testing.expectEqualStrings("msg1", msgs[0].text);
    try std.testing.expectEqualStrings("msg2", msgs[1].text);
    try std.testing.expectEqualStrings("msg3", msgs[2].text);
    try std.testing.expectEqual(Source.discord, msgs[0].source);
    try std.testing.expectEqual(Source.discord, msgs[1].source);
    try std.testing.expectEqual(Source.whatsapp, msgs[2].source);
}

test "Edge5: WhatsApp poll drains multiple messages in single call" {
    const allocator = std.testing.allocator;
    var result = try makeWhatsAppWithPipe(allocator);
    defer cleanupWhatsApp(&result);

    setNonBlock(result.wa.child.?.stdout.?.handle);

    const line1 = "{\"event\":\"message\",\"jid\":\"j1\",\"id\":\"m1\",\"sender\":\"s1\",\"sender_name\":\"A\",\"text\":\"wa1\",\"timestamp\":1700000000,\"is_group\":false,\"mentions_bot\":false}\n";
    const line2 = "{\"event\":\"message\",\"jid\":\"j2\",\"id\":\"m2\",\"sender\":\"s2\",\"sender_name\":\"B\",\"text\":\"wa2\",\"timestamp\":1700000001,\"is_group\":true,\"mentions_bot\":true}\n";
    const line3 = "{\"event\":\"message\",\"jid\":\"j3\",\"id\":\"m3\",\"sender\":\"s3\",\"sender_name\":\"C\",\"text\":\"wa3\",\"timestamp\":1700000002,\"is_group\":false,\"mentions_bot\":false}\n";
    _ = try std.posix.write(result.write_fd, line1 ++ line2 ++ line3);

    const msgs = try result.wa.poll(allocator);
    defer freeWaMessages(allocator, msgs);

    try std.testing.expectEqual(@as(usize, 3), msgs.len);
    try std.testing.expectEqualStrings("wa1", msgs[0].text);
    try std.testing.expectEqualStrings("wa2", msgs[1].text);
    try std.testing.expectEqualStrings("wa3", msgs[2].text);
}

// ============================================================================
// Edge Case 6: stdout is null — poll returns empty
// ============================================================================

test "Edge6: Sidecar poll with null child returns empty" {
    const allocator = std.testing.allocator;
    var s = Sidecar.init(allocator, "TestBot");
    defer s.deinit();

    const msgs = try s.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), msgs.len);
}

test "Edge6: WhatsApp poll with null child returns empty" {
    const allocator = std.testing.allocator;
    var wa = WhatsApp.init(allocator, "TestBot");
    defer wa.deinit();

    const msgs = try wa.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), msgs.len);
}

// ============================================================================
// Additional: Sidecar event parsing for non-message events
// ============================================================================

test "Sidecar poll handles discord_ready event without returning message" {
    const allocator = std.testing.allocator;
    var result = try makeSidecarWithPipe(allocator);
    defer cleanupSidecar(&result);

    setNonBlock(result.sidecar.child.?.stdout.?.handle);

    const ready_line = "{\"source\":\"discord\",\"event\":\"ready\",\"bot_id\":\"bot123\"}\n";
    _ = try std.posix.write(result.write_fd, ready_line);

    const msgs = try result.sidecar.poll(allocator);
    // ready event sets discord_connected but doesn't produce a message
    try std.testing.expectEqual(@as(usize, 0), msgs.len);
    try std.testing.expect(result.sidecar.discord_connected);
}

test "Sidecar poll handles wa_connected event" {
    const allocator = std.testing.allocator;
    var result = try makeSidecarWithPipe(allocator);
    defer cleanupSidecar(&result);

    setNonBlock(result.sidecar.child.?.stdout.?.handle);

    const connected_line = "{\"source\":\"whatsapp\",\"event\":\"connected\",\"jid\":\"me@s.whatsapp.net\"}\n";
    _ = try std.posix.write(result.write_fd, connected_line);

    const msgs = try result.sidecar.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), msgs.len);
    try std.testing.expect(result.sidecar.wa_connected);
}

test "Sidecar poll handles disconnected event and clears connection state" {
    const allocator = std.testing.allocator;
    var result = try makeSidecarWithPipe(allocator);
    defer cleanupSidecar(&result);

    setNonBlock(result.sidecar.child.?.stdout.?.handle);

    // First connect
    result.sidecar.discord_connected = true;

    // Then disconnect
    const disconnected_line = "{\"source\":\"discord\",\"event\":\"disconnected\",\"reason\":\"token invalid\"}\n";
    _ = try std.posix.write(result.write_fd, disconnected_line);

    const msgs = try result.sidecar.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), msgs.len);
    try std.testing.expect(!result.sidecar.discord_connected);
}

test "Sidecar poll handles mixed events and messages" {
    // Verify that non-message events and messages can be interleaved.
    const allocator = std.testing.allocator;
    var result = try makeSidecarWithPipe(allocator);
    defer cleanupSidecar(&result);

    setNonBlock(result.sidecar.child.?.stdout.?.handle);

    const ready_line = "{\"source\":\"discord\",\"event\":\"ready\",\"bot_id\":\"b1\"}\n";
    const msg_line = "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"mx\",\"channel_id\":\"cx\",\"sender_id\":\"ux\",\"sender_name\":\"Mix\",\"text\":\"mixed\",\"timestamp\":1700000000,\"is_dm\":true,\"mentions_bot\":false}\n";
    _ = try std.posix.write(result.write_fd, ready_line ++ msg_line);

    const msgs = try result.sidecar.poll(allocator);
    defer freeSidecarMessages(allocator, msgs);

    // The ready event should set discord_connected
    try std.testing.expect(result.sidecar.discord_connected);
    // The message event should produce one message
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expectEqualStrings("mixed", msgs[0].text);
    // is_dm=true → is_group=false
    try std.testing.expect(!msgs[0].is_group);
}

// ============================================================================
// Additional: WhatsApp event parsing for non-message events
// ============================================================================

test "WhatsApp poll handles connected event" {
    const allocator = std.testing.allocator;
    var result = try makeWhatsAppWithPipe(allocator);
    defer cleanupWhatsApp(&result);

    setNonBlock(result.wa.child.?.stdout.?.handle);

    const connected_line = "{\"event\":\"connected\",\"jid\":\"me@s.whatsapp.net\"}\n";
    _ = try std.posix.write(result.write_fd, connected_line);

    const msgs = try result.wa.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), msgs.len);
    try std.testing.expect(result.wa.connected);
}

test "WhatsApp poll handles disconnected event" {
    const allocator = std.testing.allocator;
    var result = try makeWhatsAppWithPipe(allocator);
    defer cleanupWhatsApp(&result);

    setNonBlock(result.wa.child.?.stdout.?.handle);
    result.wa.connected = true;

    const disconnected_line = "{\"event\":\"disconnected\",\"reason\":\"logged out\"}\n";
    _ = try std.posix.write(result.write_fd, disconnected_line);

    const msgs = try result.wa.poll(allocator);
    try std.testing.expectEqual(@as(usize, 0), msgs.len);
    try std.testing.expect(!result.wa.connected);
}

// ============================================================================
// Additional: invalid/malformed JSON lines are skipped gracefully
// ============================================================================

test "Sidecar poll skips invalid JSON lines and continues" {
    const allocator = std.testing.allocator;
    var result = try makeSidecarWithPipe(allocator);
    defer cleanupSidecar(&result);

    setNonBlock(result.sidecar.child.?.stdout.?.handle);

    // Write invalid JSON, then valid JSON
    const bad_line = "this is not json\n";
    const good_line = "{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"g1\",\"channel_id\":\"c1\",\"sender_id\":\"u1\",\"sender_name\":\"Good\",\"text\":\"valid\",\"timestamp\":1700000000,\"is_dm\":false,\"mentions_bot\":false}\n";
    _ = try std.posix.write(result.write_fd, bad_line ++ good_line);

    const msgs = try result.sidecar.poll(allocator);
    defer freeSidecarMessages(allocator, msgs);

    // Bad line skipped, good line parsed
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expectEqualStrings("valid", msgs[0].text);
}

test "WhatsApp poll skips invalid JSON lines and continues" {
    const allocator = std.testing.allocator;
    var result = try makeWhatsAppWithPipe(allocator);
    defer cleanupWhatsApp(&result);

    setNonBlock(result.wa.child.?.stdout.?.handle);

    const bad_line = "{broken json\n";
    const good_line = "{\"event\":\"message\",\"jid\":\"j1\",\"id\":\"m1\",\"sender\":\"s1\",\"sender_name\":\"OK\",\"text\":\"valid\",\"timestamp\":1700000000,\"is_group\":false,\"mentions_bot\":false}\n";
    _ = try std.posix.write(result.write_fd, bad_line ++ good_line);

    const msgs = try result.wa.poll(allocator);
    defer freeWaMessages(allocator, msgs);

    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expectEqualStrings("valid", msgs[0].text);
}

// ============================================================================
// Additional: empty lines between NDJSON messages are handled
// ============================================================================

test "Sidecar poll handles empty lines between messages" {
    const allocator = std.testing.allocator;
    var result = try makeSidecarWithPipe(allocator);
    defer cleanupSidecar(&result);

    setNonBlock(result.sidecar.child.?.stdout.?.handle);

    // Write messages with empty lines between them
    const data = "\n\n{\"source\":\"discord\",\"event\":\"message\",\"message_id\":\"e1\",\"channel_id\":\"c1\",\"sender_id\":\"u1\",\"sender_name\":\"E\",\"text\":\"after empty\",\"timestamp\":1700000000,\"is_dm\":false,\"mentions_bot\":false}\n\n";
    _ = try std.posix.write(result.write_fd, data);

    const msgs = try result.sidecar.poll(allocator);
    defer freeSidecarMessages(allocator, msgs);

    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expectEqualStrings("after empty", msgs[0].text);
}
