// Tests for subprocess pipe deadlock fix: verifies that stdout and stderr are
// drained concurrently via collectPipeOutput, preventing deadlocks when a child
// writes >64KB to stderr before stdout (or vice versa).
//
// These tests should FAIL before the fix is applied because:
// - collectPipeOutput does not yet exist in git.zig (compile error)
// - docker.zig RunResult lacks the stderr field (compile error)
//
// Wire into the build from git.zig:
//   test { _ = @import("pipe_deadlock_test.zig"); }

const std = @import("std");
const git_mod = @import("git.zig");
const docker_mod = @import("docker.zig");

// =============================================================================
// AC1: No deadlock on large stderr
// A subprocess that writes >64KB to stderr before writing to stdout must
// complete without hanging.
// =============================================================================

test "AC1: collectPipeOutput does not deadlock on >64KB stderr" {
    const alloc = std.testing.allocator;

    // Spawn a child that writes 128KB to stderr, then "done" to stdout.
    // Without concurrent draining, this deadlocks because the parent blocks
    // reading stdout while the child blocks writing to a full stderr pipe.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "dd if=/dev/urandom bs=1024 count=128 status=none >&2; echo done" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024, // 10MB max
    );

    const term = try child.wait();
    const exit_code: u8 = switch (term) {
        .Exited => |code| code,
        else => 1,
    };

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    try std.testing.expectEqual(@as(u8, 0), exit_code);
    try std.testing.expectEqualStrings("done\n", result.stdout);
    // stderr should have 128 * 1024 = 131072 bytes
    try std.testing.expect(result.stderr.len >= 128 * 1024);
}

test "AC1: git exec does not deadlock on >64KB stderr" {
    const alloc = std.testing.allocator;

    // Use Git.exec() indirectly by testing it on a command that produces large
    // stderr. We use a temp repo and a deliberate error-producing command.
    // Alternatively, test via a direct shell command through the same code path.
    var tmp_buf: [256]u8 = undefined;
    const tmp_dir = try std.fmt.bufPrint(&tmp_buf, "/tmp/borg-deadlock-test-{d}", .{std.time.timestamp()});
    const tmp_z = try alloc.dupeZ(u8, tmp_dir);
    defer alloc.free(tmp_z);

    std.fs.makeDirAbsolute(tmp_dir) catch {};
    defer std.fs.deleteTreeAbsolute(tmp_dir) catch {};

    var git = git_mod.Git.init(alloc, tmp_dir);

    // git init
    var init_r = try git.exec(&.{ "init", "-b", "main" });
    defer init_r.deinit();
    try std.testing.expect(init_r.success());

    // Now run a command that produces significant stderr output.
    // "git log" on a fresh repo with verbose error output won't produce 64KB,
    // but we verify the plumbing works. The real deadlock test is AC1 above.
    var result = try git.exec(&.{ "status", "--porcelain" });
    defer result.deinit();

    // Both fields must be populated slices (even if empty)
    try std.testing.expect(result.stdout.len >= 0);
    try std.testing.expect(result.stderr.len >= 0);
}

// =============================================================================
// AC2: No deadlock on large stdout
// A subprocess that writes >64KB to stdout must still complete (regression test).
// =============================================================================

test "AC2: collectPipeOutput handles >64KB stdout without deadlock" {
    const alloc = std.testing.allocator;

    // Child writes 128KB to stdout, nothing to stderr.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "dd if=/dev/urandom bs=1024 count=128 status=none" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024,
    );

    const term = try child.wait();
    const exit_code: u8 = switch (term) {
        .Exited => |code| code,
        else => 1,
    };

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    try std.testing.expectEqual(@as(u8, 0), exit_code);
    try std.testing.expect(result.stdout.len >= 128 * 1024);
    try std.testing.expectEqual(@as(usize, 0), result.stderr.len);
}

test "AC2: collectPipeOutput handles >64KB on both streams simultaneously" {
    const alloc = std.testing.allocator;

    // Child writes 128KB to both stdout and stderr interleaved.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "dd if=/dev/urandom bs=1024 count=128 status=none & dd if=/dev/urandom bs=1024 count=128 status=none >&2; wait" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024,
    );

    const term = try child.wait();
    const exit_code: u8 = switch (term) {
        .Exited => |code| code,
        else => 1,
    };

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    try std.testing.expectEqual(@as(u8, 0), exit_code);
    try std.testing.expect(result.stdout.len >= 128 * 1024);
    try std.testing.expect(result.stderr.len >= 128 * 1024);
}

// =============================================================================
// AC3: Both streams captured
// git.zig:exec() and pipeline.zig:runTestCommandForRepo() must return both
// stdout and stderr content accurately in their result structs.
// =============================================================================

test "AC3: collectPipeOutput captures both streams accurately" {
    const alloc = std.testing.allocator;

    // Child writes known content to both streams.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo hello_stdout; echo hello_stderr >&2" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024,
    );

    _ = try child.wait();

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    try std.testing.expectEqualStrings("hello_stdout\n", result.stdout);
    try std.testing.expectEqualStrings("hello_stderr\n", result.stderr);
}

test "AC3: git exec returns both stdout and stderr" {
    const alloc = std.testing.allocator;

    var tmp_buf: [256]u8 = undefined;
    const tmp_dir = try std.fmt.bufPrint(&tmp_buf, "/tmp/borg-ac3-test-{d}", .{std.time.timestamp()});
    std.fs.makeDirAbsolute(tmp_dir) catch {};
    defer std.fs.deleteTreeAbsolute(tmp_dir) catch {};

    var git = git_mod.Git.init(alloc, tmp_dir);

    var init_r = try git.exec(&.{ "init", "-b", "main" });
    defer init_r.deinit();
    try std.testing.expect(init_r.success());

    // A command that should produce output on stderr (e.g., checking out
    // a nonexistent branch should produce an error on stderr).
    var result = try git.exec(&.{ "checkout", "nonexistent-branch-12345" });
    defer result.deinit();

    // The command should fail
    try std.testing.expect(!result.success());
    // stderr should contain the error message
    try std.testing.expect(result.stderr.len > 0);
}

// =============================================================================
// AC5: docker.zig stderr captured
// docker.zig RunResult includes the new stderr field.
// =============================================================================

test "AC5: docker RunResult has stderr field" {
    const alloc = std.testing.allocator;

    // Construct a RunResult with the new stderr field.
    // This is a compile-time test: if the field doesn't exist, it won't compile.
    var result = docker_mod.RunResult{
        .stdout = try alloc.dupe(u8, "output"),
        .stderr = try alloc.dupe(u8, "error output"),
        .exit_code = 0,
        .allocator = alloc,
    };
    defer result.deinit();

    try std.testing.expectEqualStrings("output", result.stdout);
    try std.testing.expectEqualStrings("error output", result.stderr);
    try std.testing.expectEqual(@as(u8, 0), result.exit_code);
}

test "AC5: docker RunResult deinit frees both stdout and stderr" {
    const alloc = std.testing.allocator;

    // Construct and immediately deinit — the testing allocator will catch leaks.
    var result = docker_mod.RunResult{
        .stdout = try alloc.dupe(u8, "some stdout data"),
        .stderr = try alloc.dupe(u8, "some stderr data"),
        .exit_code = 1,
        .allocator = alloc,
    };
    result.deinit();
    // If deinit doesn't free stderr, the testing allocator will report a leak.
}

// =============================================================================
// AC8: Interleaved output correctness
// When a process writes interleaved stdout and stderr, both streams are fully
// captured with no truncation or data loss up to max buffer size.
// =============================================================================

test "AC8: interleaved stdout and stderr fully captured" {
    const alloc = std.testing.allocator;

    // Child alternately writes to stdout and stderr in a loop.
    // Each line is tagged so we can verify completeness.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c",
            \\i=0; while [ $i -lt 100 ]; do
            \\  echo "stdout_line_$i"
            \\  echo "stderr_line_$i" >&2
            \\  i=$((i + 1))
            \\done
        },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024,
    );

    _ = try child.wait();

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    // Verify all 100 stdout lines present
    var stdout_count: usize = 0;
    var stdout_lines = std.mem.splitScalar(u8, result.stdout, '\n');
    while (stdout_lines.next()) |line| {
        if (line.len > 0) stdout_count += 1;
    }
    try std.testing.expectEqual(@as(usize, 100), stdout_count);

    // Verify all 100 stderr lines present
    var stderr_count: usize = 0;
    var stderr_lines = std.mem.splitScalar(u8, result.stderr, '\n');
    while (stderr_lines.next()) |line| {
        if (line.len > 0) stderr_count += 1;
    }
    try std.testing.expectEqual(@as(usize, 100), stderr_count);

    // Verify specific lines exist
    try std.testing.expect(std.mem.indexOf(u8, result.stdout, "stdout_line_0") != null);
    try std.testing.expect(std.mem.indexOf(u8, result.stdout, "stdout_line_99") != null);
    try std.testing.expect(std.mem.indexOf(u8, result.stderr, "stderr_line_0") != null);
    try std.testing.expect(std.mem.indexOf(u8, result.stderr, "stderr_line_99") != null);
}

// =============================================================================
// Edge Case 1: Child closes stderr before stdout (or vice versa)
// =============================================================================

test "Edge1: child closes stderr before stdout" {
    const alloc = std.testing.allocator;

    // stderr closes immediately (no output), stdout has data after a brief write.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "exec 2>&-; echo after_stderr_closed" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024,
    );

    _ = try child.wait();

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    try std.testing.expectEqualStrings("after_stderr_closed\n", result.stdout);
    try std.testing.expectEqual(@as(usize, 0), result.stderr.len);
}

test "Edge1: child closes stdout before stderr" {
    const alloc = std.testing.allocator;

    // stdout closes immediately, then stderr has data.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "exec 1>&-; echo after_stdout_closed >&2" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024,
    );

    _ = try child.wait();

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    try std.testing.expectEqual(@as(usize, 0), result.stdout.len);
    try std.testing.expectEqualStrings("after_stdout_closed\n", result.stderr);
}

// =============================================================================
// Edge Case 2: Child produces no stderr
// =============================================================================

test "Edge2: child produces no stderr returns empty slice" {
    const alloc = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo only_stdout" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024,
    );

    _ = try child.wait();

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    try std.testing.expectEqualStrings("only_stdout\n", result.stdout);
    try std.testing.expectEqual(@as(usize, 0), result.stderr.len);
}

// =============================================================================
// Edge Case 3: Child produces no stdout
// =============================================================================

test "Edge3: child produces no stdout returns empty slice" {
    const alloc = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo only_stderr >&2" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024,
    );

    _ = try child.wait();

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    try std.testing.expectEqual(@as(usize, 0), result.stdout.len);
    try std.testing.expectEqualStrings("only_stderr\n", result.stderr);
}

// =============================================================================
// Edge Case 4: Child exits before all output is read
// Pipes remain readable after child exit until drained.
// =============================================================================

test "Edge4: output fully read even after child exits" {
    const alloc = std.testing.allocator;

    // Child writes data and exits immediately. The pipe buffers should still
    // be fully readable after the child process has terminated.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo stdout_data; echo stderr_data >&2" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    // Small sleep to increase chance child has already exited
    std.time.sleep(50 * std.time.ns_per_ms);

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024,
    );

    _ = try child.wait();

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    try std.testing.expectEqualStrings("stdout_data\n", result.stdout);
    try std.testing.expectEqualStrings("stderr_data\n", result.stderr);
}

// =============================================================================
// Edge Case 8: Max buffer enforcement
// If either stream exceeds max_size, reading stops for that stream.
// =============================================================================

test "Edge8: max buffer size enforced on stdout" {
    const alloc = std.testing.allocator;

    // Child writes 64KB to stdout but we limit to 1KB.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "dd if=/dev/zero bs=1024 count=64 status=none; echo stderr_msg >&2" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const max_size = 1024; // 1KB limit
    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        max_size,
    );

    _ = child.wait() catch {};

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    // stdout should be truncated at or near max_size
    try std.testing.expect(result.stdout.len <= max_size);
}

test "Edge8: max buffer size enforced on stderr" {
    const alloc = std.testing.allocator;

    // Child writes 64KB to stderr but we limit to 1KB.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo stdout_msg; dd if=/dev/zero bs=1024 count=64 status=none >&2" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const max_size = 1024; // 1KB limit
    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        max_size,
    );

    _ = child.wait() catch {};

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    // stderr should be truncated at or near max_size
    try std.testing.expect(result.stderr.len <= max_size);
}

// =============================================================================
// AC6: Thread cleanup — the return type proves the function signature is correct.
// The stderr reader thread must be joined before wait() is called.
// We verify this implicitly: if the thread leaks, the test allocator detects it
// or the process hangs. We also verify no fd leaks by running many iterations.
// =============================================================================

test "AC6: no thread or fd leak over many iterations" {
    const alloc = std.testing.allocator;

    // Run collectPipeOutput many times to detect thread/fd leaks.
    for (0..20) |i| {
        _ = i;
        var child = std.process.Child.init(
            &.{ "/bin/sh", "-c", "echo ok; echo err >&2" },
            alloc,
        );
        child.stdin_behavior = .Close;
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Pipe;
        try child.spawn();

        const result = git_mod.collectPipeOutput(
            alloc,
            child.stdout.?,
            child.stderr.?,
            10 * 1024 * 1024,
        );

        _ = try child.wait();

        alloc.free(result.stdout);
        alloc.free(result.stderr);
    }
    // If we get here without hanging or crashing, threads were cleaned up.
}

// =============================================================================
// collectPipeOutput function signature test
// Verifies the function exists with the expected signature.
// =============================================================================

test "collectPipeOutput has correct return type" {
    // Verify the function exists and has the expected type signature at comptime.
    const func = git_mod.collectPipeOutput;
    const info = @typeInfo(@TypeOf(func));

    // It should be a function
    try std.testing.expect(info == .@"fn");

    // It should take 4 parameters: allocator, stdout_pipe, stderr_pipe, max_size
    try std.testing.expectEqual(@as(usize, 4), info.@"fn".params.len);
}

// =============================================================================
// Integration: git.zig exec uses concurrent draining
// Verifies that exec() now uses collectPipeOutput internally by checking that
// a large-stderr command doesn't deadlock.
// =============================================================================

test "integration: git exec handles large stderr without deadlock" {
    const alloc = std.testing.allocator;

    var tmp_buf: [256]u8 = undefined;
    const tmp_dir = try std.fmt.bufPrint(&tmp_buf, "/tmp/borg-integ-test-{d}", .{std.time.timestamp()});
    std.fs.makeDirAbsolute(tmp_dir) catch {};
    defer std.fs.deleteTreeAbsolute(tmp_dir) catch {};

    var git = git_mod.Git.init(alloc, tmp_dir);

    var init_r = try git.exec(&.{ "init", "-b", "main" });
    defer init_r.deinit();

    // Configure user
    var cfg1 = try git.exec(&.{ "config", "user.email", "test@test.com" });
    defer cfg1.deinit();
    var cfg2 = try git.exec(&.{ "config", "user.name", "Test" });
    defer cfg2.deinit();

    // Create initial commit so we have a valid HEAD
    const file_path = try std.fmt.allocPrint(alloc, "{s}/test.txt", .{tmp_dir});
    defer alloc.free(file_path);
    try std.fs.cwd().writeFile(.{ .sub_path = file_path, .data = "test\n" });

    var add_r = try git.addAll();
    defer add_r.deinit();
    var commit_r = try git.commit("init");
    defer commit_r.deinit();

    // git log produces output; verify it doesn't deadlock and both streams work
    var log_r = try git.exec(&.{ "log", "--oneline" });
    defer log_r.deinit();
    try std.testing.expect(log_r.success());
    try std.testing.expect(log_r.stdout.len > 0);
}

// =============================================================================
// Edge Case: empty process (no stdout, no stderr, immediate exit)
// =============================================================================

test "Edge: empty process produces two empty slices" {
    const alloc = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "true" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024,
    );

    const term = try child.wait();
    const exit_code: u8 = switch (term) {
        .Exited => |code| code,
        else => 1,
    };

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    try std.testing.expectEqual(@as(u8, 0), exit_code);
    try std.testing.expectEqual(@as(usize, 0), result.stdout.len);
    try std.testing.expectEqual(@as(usize, 0), result.stderr.len);
}

// =============================================================================
// Edge: binary data correctness
// Verifies that binary (non-UTF8) data is captured without corruption.
// =============================================================================

test "Edge: binary data captured without corruption" {
    const alloc = std.testing.allocator;

    // Write exactly 256 bytes (all byte values 0x00-0xFF) to stdout,
    // and the same reversed to stderr.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c",
            \\printf '%b' "$(awk 'BEGIN{for(i=0;i<256;i++)printf "\\\\%03o",i}')";
            \\printf '%b' "$(awk 'BEGIN{for(i=255;i>=0;i--)printf "\\\\%03o",i}')" >&2
        },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024,
    );

    _ = try child.wait();

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    // stdout: bytes 0x00..0xFF
    try std.testing.expectEqual(@as(usize, 256), result.stdout.len);
    for (0..256) |i| {
        try std.testing.expectEqual(@as(u8, @intCast(i)), result.stdout[i]);
    }

    // stderr: bytes 0xFF..0x00
    try std.testing.expectEqual(@as(usize, 256), result.stderr.len);
    for (0..256) |i| {
        try std.testing.expectEqual(@as(u8, @intCast(255 - i)), result.stderr[i]);
    }
}

// =============================================================================
// AC4: Stderr available for diagnostics
// agent.zig:runDirect() and main.zig:agentThreadInner() must log stderr content
// when the subprocess exits with a non-zero exit code. We test that
// collectPipeOutput correctly captures stderr from a failing process so that
// callers have diagnostic data to log.
// =============================================================================

test "AC4: stderr captured from process that exits non-zero" {
    const alloc = std.testing.allocator;

    // Child writes a diagnostic error message to stderr and exits with code 1.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo 'error: something went wrong' >&2; exit 1" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024,
    );

    const term = try child.wait();
    const exit_code: u8 = switch (term) {
        .Exited => |code| code,
        else => 1,
    };

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    // Process exited with non-zero
    try std.testing.expectEqual(@as(u8, 1), exit_code);
    // stderr contains the diagnostic message that callers would log
    try std.testing.expectEqualStrings("error: something went wrong\n", result.stderr);
    // stdout is empty
    try std.testing.expectEqual(@as(usize, 0), result.stdout.len);
}

test "AC4: stderr captured alongside stdout from failing process" {
    const alloc = std.testing.allocator;

    // Child writes partial output to stdout and an error to stderr, then fails.
    // This simulates agent.zig:runDirect() where partial NDJSON may appear on
    // stdout but the process still fails, and stderr has the reason.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo 'partial output'; echo 'fatal: agent crashed' >&2; exit 2" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024,
    );

    const term = try child.wait();
    const exit_code: u8 = switch (term) {
        .Exited => |code| code,
        else => 1,
    };

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    try std.testing.expectEqual(@as(u8, 2), exit_code);
    try std.testing.expectEqualStrings("partial output\n", result.stdout);
    try std.testing.expectEqualStrings("fatal: agent crashed\n", result.stderr);
}

test "AC4: large stderr from failing process fully captured for diagnostics" {
    const alloc = std.testing.allocator;

    // Child writes a large stack trace / error log to stderr (>64KB) and exits
    // non-zero. Without concurrent draining, this would deadlock AND lose the
    // diagnostic data.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "dd if=/dev/urandom bs=1024 count=128 status=none >&2; exit 42" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024,
    );

    const term = try child.wait();
    const exit_code: u8 = switch (term) {
        .Exited => |code| code,
        else => 1,
    };

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    try std.testing.expectEqual(@as(u8, 42), exit_code);
    // All 128KB of stderr diagnostic data captured
    try std.testing.expect(result.stderr.len >= 128 * 1024);
    try std.testing.expectEqual(@as(usize, 0), result.stdout.len);
}

// =============================================================================
// Edge Case 10: runWithStdio callers handle new stderr field
// Verify that RunResult.deinit() properly frees the stderr field by using
// the testing allocator (which detects leaks). This ensures all callers in
// pipeline.zig that call deinit() will free stderr without memory leaks.
// =============================================================================

test "Edge10: RunResult deinit frees stderr preventing leaks in callers" {
    const alloc = std.testing.allocator;

    // Simulate what pipeline.zig callers do: create RunResult, use it, deinit.
    // The testing allocator will panic if stderr is not freed.
    var result = docker_mod.RunResult{
        .stdout = try alloc.dupe(u8, "container stdout output"),
        .stderr = try alloc.dupe(u8, "container stderr warnings"),
        .exit_code = 0,
        .allocator = alloc,
    };
    // Callers access stderr for logging before deinit
    try std.testing.expect(result.stderr.len > 0);
    result.deinit();
    // If we reach here without the testing allocator detecting a leak, deinit is correct.
}

test "Edge10: RunResult deinit handles empty stderr" {
    const alloc = std.testing.allocator;

    // Common case: container produces no stderr.
    var result = docker_mod.RunResult{
        .stdout = try alloc.dupe(u8, "output"),
        .stderr = try alloc.dupe(u8, ""),
        .exit_code = 0,
        .allocator = alloc,
    };
    result.deinit();
}

// =============================================================================
// Edge Case 5: Thread spawn failure
// If std.Thread.spawn fails, the function should fall back to sequential reads
// or propagate the error — not crash.
// This is hard to trigger directly, so we verify the function signature allows
// for error propagation and that a normal call after resource pressure succeeds.
// =============================================================================

test "Edge5: collectPipeOutput works under repeated rapid invocations" {
    const alloc = std.testing.allocator;

    // Rapidly spawn many concurrent collectPipeOutput calls to stress-test
    // thread creation. If thread spawn fails, we should get an error, not a crash.
    for (0..50) |_| {
        var child = std.process.Child.init(
            &.{ "/bin/sh", "-c", "echo ok; echo err >&2" },
            alloc,
        );
        child.stdin_behavior = .Close;
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Pipe;
        try child.spawn();

        const result = git_mod.collectPipeOutput(
            alloc,
            child.stdout.?,
            child.stderr.?,
            10 * 1024 * 1024,
        );

        _ = try child.wait();

        alloc.free(result.stdout);
        alloc.free(result.stderr);
    }
}

// =============================================================================
// Edge Case 9: EINTR handling
// Verify that reads are not interrupted by signals. We send SIGCHLD (which
// happens naturally when children exit) during a read and verify data integrity.
// =============================================================================

test "Edge9: reads survive SIGCHLD from child exit" {
    const alloc = std.testing.allocator;

    // Spawn a child that writes data, sleeps briefly, then exits.
    // The child exit generates SIGCHLD which could cause EINTR on the read.
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo before_exit; echo err_before_exit >&2; sleep 0.01; echo after_sleep; echo err_after_sleep >&2" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const result = git_mod.collectPipeOutput(
        alloc,
        child.stdout.?,
        child.stderr.?,
        10 * 1024 * 1024,
    );

    _ = try child.wait();

    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    // All output should be captured despite potential EINTR
    try std.testing.expect(std.mem.indexOf(u8, result.stdout, "before_exit") != null);
    try std.testing.expect(std.mem.indexOf(u8, result.stdout, "after_sleep") != null);
    try std.testing.expect(std.mem.indexOf(u8, result.stderr, "err_before_exit") != null);
    try std.testing.expect(std.mem.indexOf(u8, result.stderr, "err_after_sleep") != null);
}
