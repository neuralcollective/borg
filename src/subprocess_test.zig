const std = @import("std");
const subprocess = @import("subprocess.zig");
const PipeOutput = subprocess.PipeOutput;

// Helper: spawn a child that writes `size` bytes to the given fd (1=stdout, 2=stderr)
// and `other_size` bytes to the other fd. Uses /bin/sh + dd for portability.
fn spawnWriter(allocator: std.mem.Allocator, stdout_size: usize, stderr_size: usize) !std.process.Child {
    // Build a shell command that writes exact byte counts to stdout and stderr.
    // "head -c N /dev/zero" writes N null bytes and is available on Linux.
    const cmd = try std.fmt.allocPrint(allocator, "head -c {d} /dev/zero 1>&1; head -c {d} /dev/zero 1>&2", .{ stdout_size, stderr_size });
    defer allocator.free(cmd);

    var child = std.process.Child.init(&.{ "/bin/sh", "-c", cmd }, allocator);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();
    return child;
}

// ── AC2: Deadlock resolved — large stderr, minimal stdout ──────────────

test "AC2: subprocess writing >64KB to stderr with 0 bytes stdout completes without deadlock" {
    const allocator = std.testing.allocator;
    const stderr_size: usize = 128 * 1024; // 128KB — well over the ~64KB pipe buffer

    var child = try spawnWriter(allocator, 0, stderr_size);
    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    const term = try child.wait();

    try std.testing.expectEqual(@as(usize, 0), output.stdout.len);
    try std.testing.expectEqual(stderr_size, output.stderr.len);
    // Verify exit success
    switch (term) {
        .Exited => |code| try std.testing.expectEqual(@as(u8, 0), code),
        else => return error.UnexpectedTermination,
    }
}

test "AC2: subprocess writing exactly 64KB to stderr completes" {
    const allocator = std.testing.allocator;
    const stderr_size: usize = 64 * 1024;

    var child = try spawnWriter(allocator, 0, stderr_size);
    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqual(@as(usize, 0), output.stdout.len);
    try std.testing.expectEqual(stderr_size, output.stderr.len);
}

// ── AC3: Deadlock resolved (reverse) — large stdout, minimal stderr ────

test "AC3: subprocess writing >64KB to stdout with 0 bytes stderr completes without deadlock" {
    const allocator = std.testing.allocator;
    const stdout_size: usize = 128 * 1024;

    var child = try spawnWriter(allocator, stdout_size, 0);
    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    const term = try child.wait();

    try std.testing.expectEqual(stdout_size, output.stdout.len);
    try std.testing.expectEqual(@as(usize, 0), output.stderr.len);
    switch (term) {
        .Exited => |code| try std.testing.expectEqual(@as(u8, 0), code),
        else => return error.UnexpectedTermination,
    }
}

// ── AC2+AC3 combined: large output on BOTH streams simultaneously ──────

test "AC2+AC3: subprocess writing >64KB to both stdout and stderr completes" {
    const allocator = std.testing.allocator;
    const size: usize = 128 * 1024;

    var child = try spawnWriter(allocator, size, size);
    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqual(size, output.stdout.len);
    try std.testing.expectEqual(size, output.stderr.len);
}

// ── AC6: Behavioral equivalence — data integrity ───────────────────────

test "AC6: collectOutput captures exact byte content from stdout" {
    const allocator = std.testing.allocator;

    // Use a command that writes known content to stdout
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo 'hello stdout'" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqualStrings("hello stdout\n", output.stdout);
    try std.testing.expectEqual(@as(usize, 0), output.stderr.len);
}

test "AC6: collectOutput captures exact byte content from stderr" {
    const allocator = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo 'hello stderr' >&2" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqual(@as(usize, 0), output.stdout.len);
    try std.testing.expectEqualStrings("hello stderr\n", output.stderr);
}

test "AC6: collectOutput captures content on both streams simultaneously" {
    const allocator = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo 'out'; echo 'err' >&2" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqualStrings("out\n", output.stdout);
    try std.testing.expectEqualStrings("err\n", output.stderr);
}

// ── AC7: Thread cleanup — stderr reader thread always joined ───────────

test "AC7: no thread leak after successful collectOutput" {
    const allocator = std.testing.allocator;

    // Run multiple collectOutput calls in sequence. If threads leaked,
    // we'd eventually exhaust thread resources.
    for (0..20) |_| {
        var child = try spawnWriter(allocator, 1024, 1024);
        var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
        output.deinit();
        _ = try child.wait();
    }
}

test "AC7: thread joined even when child exits with error" {
    const allocator = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo 'fail-out'; echo 'fail-err' >&2; exit 42" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    const term = try child.wait();

    // Thread is joined, output captured, even on non-zero exit
    try std.testing.expectEqualStrings("fail-out\n", output.stdout);
    try std.testing.expectEqualStrings("fail-err\n", output.stderr);
    switch (term) {
        .Exited => |code| try std.testing.expectEqual(@as(u8, 42), code),
        else => return error.UnexpectedTermination,
    }
}

// ── AC8: Unit test in subprocess.zig (verified here too) ───────────────

test "AC8: spawn child producing >64KB on stderr — both streams fully captured" {
    const allocator = std.testing.allocator;
    const large_size: usize = 128 * 1024;

    // Write 128KB of 'A' to stderr, and a known string to stdout
    const cmd = try std.fmt.allocPrint(allocator,
        "echo 'marker'; head -c {d} /dev/zero | tr '\\0' 'A' >&2", .{large_size});
    defer allocator.free(cmd);

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", cmd },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqualStrings("marker\n", output.stdout);
    try std.testing.expectEqual(large_size, output.stderr.len);
    // Verify stderr content is all 'A'
    for (output.stderr) |byte| {
        try std.testing.expectEqual(@as(u8, 'A'), byte);
    }
}

// ── Edge Case 1: Child closes stdout before stderr ─────────────────────

test "Edge1: child closes stdout before stderr — both streams fully read" {
    const allocator = std.testing.allocator;

    // stdout closes immediately (no output), stderr writes after a brief delay
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo 'late-stderr' >&2" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqual(@as(usize, 0), output.stdout.len);
    try std.testing.expectEqualStrings("late-stderr\n", output.stderr);
}

test "Edge1: child closes stderr before stdout — both streams fully read" {
    const allocator = std.testing.allocator;

    // stderr closes immediately, stdout writes after
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo 'late-stdout'" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqualStrings("late-stdout\n", output.stdout);
    try std.testing.expectEqual(@as(usize, 0), output.stderr.len);
}

// ── Edge Case 2: Zero output on one or both streams ────────────────────

test "Edge2: child produces zero output on both streams — returns empty slices" {
    const allocator = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "true" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    // Must return empty slices, not null
    try std.testing.expectEqual(@as(usize, 0), output.stdout.len);
    try std.testing.expectEqual(@as(usize, 0), output.stderr.len);
}

test "Edge2: child produces output only on stdout — stderr is empty slice" {
    const allocator = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo 'only-stdout'" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqualStrings("only-stdout\n", output.stdout);
    try std.testing.expectEqual(@as(usize, 0), output.stderr.len);
}

test "Edge2: child produces output only on stderr — stdout is empty slice" {
    const allocator = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo 'only-stderr' >&2" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqual(@as(usize, 0), output.stdout.len);
    try std.testing.expectEqualStrings("only-stderr\n", output.stderr);
}

// ── Edge Case 3: Child exits before all output is read ─────────────────

test "Edge3: pipes remain readable after child exit — all output captured" {
    const allocator = std.testing.allocator;

    // Child writes data and exits immediately; parent must still drain pipes
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "printf 'aaa'; printf 'bbb' >&2" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqualStrings("aaa", output.stdout);
    try std.testing.expectEqualStrings("bbb", output.stderr);
}

// ── Edge Case 5: Very large output respects max_size ───────────────────

test "Edge5: output exceeding max_size is truncated" {
    const allocator = std.testing.allocator;
    const max_size: usize = 1024; // 1KB limit

    // Write 4KB to stdout
    var child = try spawnWriter(allocator, 4096, 0);
    var output = try subprocess.collectOutput(allocator, &child, max_size);
    defer output.deinit();
    _ = child.wait() catch {};

    // stdout should be capped at max_size
    try std.testing.expect(output.stdout.len <= max_size);
}

test "Edge5: max_size limits each stream independently" {
    const allocator = std.testing.allocator;
    const max_size: usize = 2048;

    // Write 4KB to both streams
    var child = try spawnWriter(allocator, 4096, 4096);
    var output = try subprocess.collectOutput(allocator, &child, max_size);
    defer output.deinit();
    _ = child.wait() catch {};

    // Each stream independently limited
    try std.testing.expect(output.stdout.len <= max_size);
    try std.testing.expect(output.stderr.len <= max_size);
}

test "Edge5: output under max_size is fully captured" {
    const allocator = std.testing.allocator;
    const max_size: usize = 10 * 1024 * 1024;
    const write_size: usize = 1024;

    var child = try spawnWriter(allocator, write_size, write_size);
    var output = try subprocess.collectOutput(allocator, &child, max_size);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqual(write_size, output.stdout.len);
    try std.testing.expectEqual(write_size, output.stderr.len);
}

// ── Edge Case 8: checkSelfUpdate reverse order deadlock ────────────────

test "Edge8: reverse deadlock scenario — large stdout with stderr drained first" {
    // This simulates the checkSelfUpdate bug: if code tried to read stderr
    // first while child fills stdout >64KB, it would deadlock.
    // With concurrent reading, this must complete.
    const allocator = std.testing.allocator;
    const stdout_size: usize = 128 * 1024;
    const stderr_size: usize = 256; // small stderr

    var child = try spawnWriter(allocator, stdout_size, stderr_size);
    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqual(stdout_size, output.stdout.len);
    try std.testing.expectEqual(stderr_size, output.stderr.len);
}

// ── PipeOutput.deinit frees memory correctly ───────────────────────────

test "PipeOutput.deinit does not leak memory" {
    const allocator = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo 'test'; echo 'err' >&2" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    _ = try child.wait();

    // If deinit doesn't free properly, testing.allocator will catch the leak
    output.deinit();
}

// ── PipeOutput struct has expected fields ───────────────────────────────

test "PipeOutput struct has stdout, stderr, and allocator fields" {
    const allocator = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "true" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 1024);
    defer output.deinit();
    _ = try child.wait();

    // Verify struct fields exist and have correct types
    const _stdout: []u8 = output.stdout;
    const _stderr: []u8 = output.stderr;
    const _alloc: std.mem.Allocator = output.allocator;
    _ = _stdout;
    _ = _stderr;
    _ = _alloc;
}

// ── Stress test: many concurrent collectOutput calls ───────────────────

test "stress: multiple sequential collectOutput calls with large output" {
    const allocator = std.testing.allocator;
    const size: usize = 100 * 1024; // 100KB each

    for (0..5) |_| {
        var child = try spawnWriter(allocator, size, size);
        var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
        defer output.deinit();
        _ = try child.wait();

        try std.testing.expectEqual(size, output.stdout.len);
        try std.testing.expectEqual(size, output.stderr.len);
    }
}

// ── Binary data preserved correctly ────────────────────────────────────

test "AC6: binary data with null bytes is preserved" {
    const allocator = std.testing.allocator;

    // Write binary data containing null bytes to stdout
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "printf '\\x00\\x01\\x02\\xff'" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqual(@as(usize, 4), output.stdout.len);
    try std.testing.expectEqual(@as(u8, 0x00), output.stdout[0]);
    try std.testing.expectEqual(@as(u8, 0x01), output.stdout[1]);
    try std.testing.expectEqual(@as(u8, 0x02), output.stdout[2]);
    try std.testing.expectEqual(@as(u8, 0xff), output.stdout[3]);
}

// ── Edge Case 8: null stdout or stderr pipe ────────────────────────────

test "Edge8: null stdout pipe — collectOutput returns empty stdout, captures stderr" {
    const allocator = std.testing.allocator;

    // stdout_behavior = .Close means child.stdout will be null after spawn
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo 'err-only' >&2" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Close; // stdout pipe is null
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    // No null-pointer dereference; stdout is treated as zero bytes
    try std.testing.expectEqual(@as(usize, 0), output.stdout.len);
    try std.testing.expectEqualStrings("err-only\n", output.stderr);
}

test "Edge8: null stderr pipe — collectOutput captures stdout, returns empty stderr" {
    const allocator = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo 'out-only'" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Close; // stderr pipe is null
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqualStrings("out-only\n", output.stdout);
    // No null-pointer dereference; stderr is treated as zero bytes
    try std.testing.expectEqual(@as(usize, 0), output.stderr.len);
}

test "Edge8: both pipes null — collectOutput returns two empty slices without crash" {
    const allocator = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "true" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Close;
    child.stderr_behavior = .Close;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqual(@as(usize, 0), output.stdout.len);
    try std.testing.expectEqual(@as(usize, 0), output.stderr.len);
}

// ── Interleaved writes on both streams ─────────────────────────────────

test "AC2+AC3: interleaved large writes on both streams complete without deadlock" {
    const allocator = std.testing.allocator;

    // This command alternates writing chunks to stdout and stderr,
    // maximizing the chance of filling both pipe buffers.
    const cmd =
        "i=0; while [ $i -lt 100 ]; do " ++
        "head -c 2048 /dev/zero; " ++
        "head -c 2048 /dev/zero >&2; " ++
        "i=$((i+1)); done";

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", cmd },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    // 100 iterations * 2048 bytes = 204800 bytes per stream
    const expected: usize = 100 * 2048;
    try std.testing.expectEqual(expected, output.stdout.len);
    try std.testing.expectEqual(expected, output.stderr.len);
}
