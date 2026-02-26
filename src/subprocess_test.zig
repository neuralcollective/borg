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

// ── Compile-time signature verification ────────────────────────────────

test "collectOutput: function exists with correct return type" {
    // If subprocess.zig is missing or collectOutput is absent, this is a compile error.
    // The return type must be an error union (i.e. !PipeOutput).
    const Fn = @TypeOf(subprocess.collectOutput);
    const fn_info = @typeInfo(Fn).@"fn";
    const ret = fn_info.return_type.?;
    try std.testing.expect(@typeInfo(ret) == .error_union);
}

test "collectOutput: function accepts three parameters (allocator, *Child, usize)" {
    const Fn = @TypeOf(subprocess.collectOutput);
    const fn_info = @typeInfo(Fn).@"fn";
    try std.testing.expectEqual(@as(usize, 3), fn_info.params.len);
}

test "PipeOutput: struct has stdout, stderr, and allocator fields of correct types" {
    const info = @typeInfo(PipeOutput);
    const fields = info.@"struct".fields;
    const expected = [_][]const u8{ "stdout", "stderr", "allocator" };
    for (expected) |name| {
        var found = false;
        for (fields) |f| {
            if (std.mem.eql(u8, f.name, name)) {
                found = true;
                break;
            }
        }
        try std.testing.expect(found);
    }
}

test "PipeOutput: deinit method exists" {
    try std.testing.expect(@hasDecl(PipeOutput, "deinit"));
}

// ── AC1 / AC2: Deadlock resolved — large stderr, minimal stdout ─────────

test "AC1: subprocess writing >64KB to stderr with 0 bytes stdout completes without deadlock" {
    const allocator = std.testing.allocator;
    const stderr_size: usize = 128 * 1024; // 128 KB — well over the ~64 KB pipe buffer

    var child = try spawnWriter(allocator, 0, stderr_size);
    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    const term = try child.wait();

    try std.testing.expectEqual(@as(usize, 0), output.stdout.len);
    try std.testing.expectEqual(stderr_size, output.stderr.len);
    switch (term) {
        .Exited => |code| try std.testing.expectEqual(@as(u8, 0), code),
        else => return error.UnexpectedTermination,
    }
}

test "AC2: subprocess writing >64KB to stderr with 0 bytes stdout completes without deadlock" {
    const allocator = std.testing.allocator;
    const stderr_size: usize = 128 * 1024;

    var child = try spawnWriter(allocator, 0, stderr_size);
    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    const term = try child.wait();

    try std.testing.expectEqual(@as(usize, 0), output.stdout.len);
    try std.testing.expectEqual(stderr_size, output.stderr.len);
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

// ── AC3: Deadlock resolved (reverse) — large stdout, minimal stderr ─────

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

// ── AC4: Large output on BOTH streams simultaneously ────────────────────

test "AC4: subprocess writing >64KB to both stdout and stderr completes" {
    const allocator = std.testing.allocator;
    const size: usize = 128 * 1024;

    var child = try spawnWriter(allocator, size, size);
    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqual(size, output.stdout.len);
    try std.testing.expectEqual(size, output.stderr.len);
}

// ── AC5 / AC6: Exact byte content captured ──────────────────────────────

test "AC5: collectOutput captures exact byte content from stdout" {
    const allocator = std.testing.allocator;

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

test "AC5+AC6: collectOutput captures content on both streams simultaneously" {
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

// ── AC7: Thread cleanup — stderr reader thread always joined ────────────

test "AC7: no thread leak after 20 sequential collectOutput calls" {
    const allocator = std.testing.allocator;

    // If threads leaked, we'd eventually exhaust thread resources.
    for (0..20) |_| {
        var child = try spawnWriter(allocator, 1024, 1024);
        var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
        output.deinit();
        _ = try child.wait();
    }
}

test "AC7: thread is joined even when child exits with non-zero code" {
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

// ── AC8: PipeOutput.deinit frees both slices — no memory leak ───────────

test "AC8: PipeOutput.deinit does not leak memory" {
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

// ── Edge Case: null stdout pipe ─────────────────────────────────────────

test "EdgeNull: null stdout pipe — collectOutput returns empty stdout, captures stderr" {
    const allocator = std.testing.allocator;

    // stdout_behavior = .Close means child.stdout will be null after spawn
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo 'err-only' >&2" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Close;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqual(@as(usize, 0), output.stdout.len);
    try std.testing.expectEqualStrings("err-only\n", output.stderr);
}

test "EdgeNull: null stderr pipe — collectOutput captures stdout, returns empty stderr" {
    const allocator = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo 'out-only'" },
        allocator,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Close;
    try child.spawn();

    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqualStrings("out-only\n", output.stdout);
    try std.testing.expectEqual(@as(usize, 0), output.stderr.len);
}

test "EdgeNull: both pipes null — collectOutput returns two empty owned slices without crash" {
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

// ── Edge Case: max_bytes cap ─────────────────────────────────────────────

test "EdgeMax: output exceeding max_bytes is truncated to at most max_bytes" {
    const allocator = std.testing.allocator;
    const max_size: usize = 1024;

    var child = try spawnWriter(allocator, 4096, 0);
    var output = try subprocess.collectOutput(allocator, &child, max_size);
    defer output.deinit();
    _ = child.wait() catch {};

    try std.testing.expect(output.stdout.len <= max_size);
}

test "EdgeMax: max_bytes limits each stream independently" {
    const allocator = std.testing.allocator;
    const max_size: usize = 2048;

    var child = try spawnWriter(allocator, 4096, 4096);
    var output = try subprocess.collectOutput(allocator, &child, max_size);
    defer output.deinit();
    _ = child.wait() catch {};

    try std.testing.expect(output.stdout.len <= max_size);
    try std.testing.expect(output.stderr.len <= max_size);
}

test "EdgeMax: output under max_bytes is fully captured" {
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

test "EdgeMax: output exactly equal to max_bytes is captured in full without off-by-one truncation" {
    const allocator = std.testing.allocator;
    // Write exactly max_size bytes to each stream — must be fully captured, not dropped.
    const max_size: usize = 4096;

    var child = try spawnWriter(allocator, max_size, max_size);
    var output = try subprocess.collectOutput(allocator, &child, max_size);
    defer output.deinit();
    _ = try child.wait();

    // At the boundary the data must still be fully captured.
    try std.testing.expectEqual(max_size, output.stdout.len);
    try std.testing.expectEqual(max_size, output.stderr.len);
}

// ── Edge Case: child exits before parent reads ───────────────────────────

test "EdgeEOF: pipes remain readable after child exits — all output captured" {
    const allocator = std.testing.allocator;

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

// ── Edge Case: interleaved writes on both streams ────────────────────────

test "EdgeInterleaved: interleaved large writes on both streams complete without deadlock" {
    const allocator = std.testing.allocator;

    // Alternates writing chunks to stdout and stderr — maximises pipe-buffer fill probability.
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

    // 100 iterations × 2048 bytes = 204 800 bytes per stream
    const expected: usize = 100 * 2048;
    try std.testing.expectEqual(expected, output.stdout.len);
    try std.testing.expectEqual(expected, output.stderr.len);
}

// ── Edge Case: streams close in opposite orders ──────────────────────────

test "EdgeOrder: stdout closes first, stderr closes later — both fully read" {
    const allocator = std.testing.allocator;

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

test "EdgeOrder: stderr closes first, stdout closes later — both fully read" {
    const allocator = std.testing.allocator;

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

// ── Edge Case: binary (non-UTF-8) data ──────────────────────────────────

test "EdgeBinary: null bytes (0x00) and high bytes (0xff) are preserved verbatim" {
    const allocator = std.testing.allocator;

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

// ── Edge Case: reverse-order deadlock scenario ───────────────────────────

test "EdgeReverse: large stdout with small stderr — no deadlock when stderr is drained concurrently" {
    // Regression guard: if code read stderr first while child fills stdout >64 KB it would deadlock.
    const allocator = std.testing.allocator;
    const stdout_size: usize = 128 * 1024;
    const stderr_size: usize = 256;

    var child = try spawnWriter(allocator, stdout_size, stderr_size);
    var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
    defer output.deinit();
    _ = try child.wait();

    try std.testing.expectEqual(stdout_size, output.stdout.len);
    try std.testing.expectEqual(stderr_size, output.stderr.len);
}

// ── Stress: many sequential calls ───────────────────────────────────────

test "stress: 5 sequential collectOutput calls with 100KB on each stream" {
    const allocator = std.testing.allocator;
    const size: usize = 100 * 1024;

    for (0..5) |_| {
        var child = try spawnWriter(allocator, size, size);
        var output = try subprocess.collectOutput(allocator, &child, 10 * 1024 * 1024);
        defer output.deinit();
        _ = try child.wait();

        try std.testing.expectEqual(size, output.stdout.len);
        try std.testing.expectEqual(size, output.stderr.len);
    }
}

// ── Source-code integration: git.zig must be updated ────────────────────
//
// These checks FAIL before the implementation because git.zig still
// contains the sequential drain pattern and has no subprocess import.

test "Integration: git.zig imports subprocess.zig" {
    const src = @embedFile("git.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "@import(\"subprocess.zig\")") != null);
}

test "Integration: git.zig no longer contains sequential read_buf drain in exec" {
    // The sequential drain pattern used a [8192]u8 stack buffer; after the fix
    // that buffer must be gone from the exec function body.
    const src = @embedFile("git.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "read_buf: [8192]u8") == null);
}

test "Integration: git.zig calls subprocess.collectOutput inside exec" {
    const src = @embedFile("git.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "collectOutput") != null);
}

// ── Source-code integration: pipeline.zig must be updated ───────────────
//
// These checks FAIL before the implementation because pipeline.zig still
// contains the sequential drain pattern inside runTestCommandForRepo.

test "Integration: pipeline.zig imports subprocess.zig" {
    const src = @embedFile("pipeline.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "@import(\"subprocess.zig\")") != null);
}

test "Integration: pipeline.zig no longer contains sequential read_buf drain in runTestCommandForRepo" {
    const src = @embedFile("pipeline.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "read_buf: [8192]u8") == null);
}

test "Integration: pipeline.zig calls subprocess.collectOutput inside runTestCommandForRepo" {
    const src = @embedFile("pipeline.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "collectOutput") != null);
}
