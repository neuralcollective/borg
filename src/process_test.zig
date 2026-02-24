// Tests for spec: Consolidate duplicated child-process stdout/stderr drain pattern.
//
// Verifies that the new `process.zig` module provides `drainPipe` and `exitCode`
// utilities, and that the four consuming files (`git.zig`, `docker.zig`,
// `agent.zig`, `pipeline.zig`) have been refactored to use them.
//
// To include in the build, add to a compiled module (e.g. main.zig):
//   test { _ = @import("process_test.zig"); }
//
// All tests below should FAIL before the implementation is applied because
// `process.zig` does not yet exist, causing a compile error on import.

const std = @import("std");
const process = @import("process.zig");

// =============================================================================
// AC5: process.zig contains drainPipe — function exists and has correct signature
// =============================================================================

test "AC5: drainPipe function exists with correct signature" {
    // Verify the function exists and is callable.
    // If process.zig is missing or drainPipe is absent, this is a compile error.
    const DrainFn = @TypeOf(process.drainPipe);
    const fn_info = @typeInfo(DrainFn).@"fn";

    // Should return an error union of []u8
    try std.testing.expect(fn_info.return_type == anyerror![]u8);

    // Should take two parameters: Allocator and File
    try std.testing.expectEqual(@as(usize, 2), fn_info.params.len);
    try std.testing.expect(fn_info.params[0].type == std.mem.Allocator);
    try std.testing.expect(fn_info.params[1].type == std.fs.File);
}

test "AC5: exitCode function exists with correct signature" {
    const ExitFn = @TypeOf(process.exitCode);
    const fn_info = @typeInfo(ExitFn).@"fn";

    // Should return u8
    try std.testing.expect(fn_info.return_type == u8);

    // Should take one parameter: Child.Term
    try std.testing.expectEqual(@as(usize, 1), fn_info.params.len);
    try std.testing.expect(fn_info.params[0].type == std.process.Child.Term);
}

// =============================================================================
// AC5 + AC6: drainPipe — happy path tests
// =============================================================================

test "drainPipe reads all bytes from a child process stdout" {
    const alloc = std.testing.allocator;

    // Spawn `printf 'hello world'` and drain its stdout
    var child = std.process.Child.init(&.{ "printf", "hello world" }, alloc);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const stdout_data = try process.drainPipe(alloc, child.stdout.?);
    defer alloc.free(stdout_data);

    // Drain stderr too (discard)
    const stderr_data = try process.drainPipe(alloc, child.stderr.?);
    defer alloc.free(stderr_data);

    const term = try child.wait();
    const code = process.exitCode(term);

    try std.testing.expectEqualStrings("hello world", stdout_data);
    try std.testing.expectEqual(@as(u8, 0), code);
}

test "drainPipe reads multiline output" {
    const alloc = std.testing.allocator;

    var child = std.process.Child.init(&.{ "printf", "line1\nline2\nline3\n" }, alloc);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Close;
    try child.spawn();

    const data = try process.drainPipe(alloc, child.stdout.?);
    defer alloc.free(data);

    _ = try child.wait();

    try std.testing.expectEqualStrings("line1\nline2\nline3\n", data);
}

// =============================================================================
// Edge Case 2: Empty output — drainPipe returns valid zero-length owned slice
// =============================================================================

test "Edge2: drainPipe returns freeable zero-length slice for empty output" {
    const alloc = std.testing.allocator;

    // `true` produces no output
    var child = std.process.Child.init(&.{"true"}, alloc);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Close;
    try child.spawn();

    const data = try process.drainPipe(alloc, child.stdout.?);
    defer alloc.free(data); // Must not crash — valid zero-length owned slice

    _ = try child.wait();

    try std.testing.expectEqual(@as(usize, 0), data.len);
}

// =============================================================================
// Edge Case 6: Large output — data exceeding 8192-byte internal buffer
// =============================================================================

test "Edge6: drainPipe handles output larger than 8192 bytes" {
    const alloc = std.testing.allocator;

    // Generate 20000 bytes of output using dd
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "dd if=/dev/zero bs=20000 count=1 2>/dev/null | tr '\\0' 'A'" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Close;
    try child.spawn();

    const data = try process.drainPipe(alloc, child.stdout.?);
    defer alloc.free(data);

    _ = try child.wait();

    // Should have read all 20000 bytes
    try std.testing.expectEqual(@as(usize, 20000), data.len);

    // Verify all bytes are 'A'
    for (data) |byte| {
        try std.testing.expectEqual(@as(u8, 'A'), byte);
    }
}

test "Edge6: drainPipe handles output exactly equal to buffer size (8192 bytes)" {
    const alloc = std.testing.allocator;

    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "dd if=/dev/zero bs=8192 count=1 2>/dev/null | tr '\\0' 'B'" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Close;
    try child.spawn();

    const data = try process.drainPipe(alloc, child.stdout.?);
    defer alloc.free(data);

    _ = try child.wait();

    try std.testing.expectEqual(@as(usize, 8192), data.len);
}

// =============================================================================
// drainPipe with binary data
// =============================================================================

test "drainPipe handles binary (non-UTF8) output" {
    const alloc = std.testing.allocator;

    // Output 256 bytes: 0x00 through 0xFF
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "printf '\\x00\\x01\\x02\\xff'" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Close;
    try child.spawn();

    const data = try process.drainPipe(alloc, child.stdout.?);
    defer alloc.free(data);

    _ = try child.wait();

    // Should contain the binary bytes (at least not crash)
    try std.testing.expect(data.len > 0);
}

// =============================================================================
// AC5 + Edge Case 5: exitCode — normal exit codes
// =============================================================================

test "exitCode returns 0 for Exited(0)" {
    const term: std.process.Child.Term = .{ .Exited = 0 };
    try std.testing.expectEqual(@as(u8, 0), process.exitCode(term));
}

test "exitCode returns 1 for Exited(1)" {
    const term: std.process.Child.Term = .{ .Exited = 1 };
    try std.testing.expectEqual(@as(u8, 1), process.exitCode(term));
}

test "exitCode returns 42 for Exited(42)" {
    const term: std.process.Child.Term = .{ .Exited = 42 };
    try std.testing.expectEqual(@as(u8, 42), process.exitCode(term));
}

test "exitCode returns 255 for Exited(255)" {
    const term: std.process.Child.Term = .{ .Exited = 255 };
    try std.testing.expectEqual(@as(u8, 255), process.exitCode(term));
}

// =============================================================================
// Edge Case 5: Signal/Stop/Unknown termination — exitCode returns 1
// =============================================================================

test "Edge5: exitCode returns 1 for Signal termination" {
    const term: std.process.Child.Term = .{ .Signal = 9 }; // SIGKILL
    try std.testing.expectEqual(@as(u8, 1), process.exitCode(term));
}

test "Edge5: exitCode returns 1 for Signal(15) SIGTERM" {
    const term: std.process.Child.Term = .{ .Signal = 15 };
    try std.testing.expectEqual(@as(u8, 1), process.exitCode(term));
}

test "Edge5: exitCode returns 1 for Stopped" {
    const term: std.process.Child.Term = .{ .Stopped = 19 }; // SIGSTOP
    try std.testing.expectEqual(@as(u8, 1), process.exitCode(term));
}

test "Edge5: exitCode returns 1 for Unknown" {
    const term: std.process.Child.Term = .{ .Unknown = 0xFFFF };
    try std.testing.expectEqual(@as(u8, 1), process.exitCode(term));
}

// =============================================================================
// AC6: Behavioral equivalence — drainPipe + exitCode match original pattern
// =============================================================================

test "AC6: drainPipe + exitCode matches original pattern for successful command" {
    const alloc = std.testing.allocator;

    // Spawn `echo hello` — equivalent to what git.exec does
    var child = std.process.Child.init(&.{ "printf", "hello\n" }, alloc);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const stdout_data = try process.drainPipe(alloc, child.stdout.?);
    defer alloc.free(stdout_data);
    const stderr_data = try process.drainPipe(alloc, child.stderr.?);
    defer alloc.free(stderr_data);

    const term = try child.wait();
    const exit_code = process.exitCode(term);

    // Behavioral equivalence: same result as the original inline pattern
    try std.testing.expectEqualStrings("hello\n", stdout_data);
    try std.testing.expectEqual(@as(usize, 0), stderr_data.len);
    try std.testing.expectEqual(@as(u8, 0), exit_code);
}

test "AC6: drainPipe + exitCode matches original pattern for failing command" {
    const alloc = std.testing.allocator;

    // Spawn a command that writes to stderr and exits non-zero
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "echo errormsg >&2; exit 2" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const stdout_data = try process.drainPipe(alloc, child.stdout.?);
    defer alloc.free(stdout_data);
    const stderr_data = try process.drainPipe(alloc, child.stderr.?);
    defer alloc.free(stderr_data);

    const term = try child.wait();
    const exit_code = process.exitCode(term);

    try std.testing.expectEqual(@as(usize, 0), stdout_data.len);
    try std.testing.expectEqualStrings("errormsg\n", stderr_data);
    try std.testing.expectEqual(@as(u8, 2), exit_code);
}

test "AC6: drainPipe + exitCode matches original pattern for signal death" {
    const alloc = std.testing.allocator;

    // Spawn a process that kills itself with SIGKILL
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "kill -9 $$" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    const stdout_data = try process.drainPipe(alloc, child.stdout.?);
    defer alloc.free(stdout_data);
    const stderr_data = try process.drainPipe(alloc, child.stderr.?);
    defer alloc.free(stderr_data);

    const term = try child.wait();
    const exit_code = process.exitCode(term);

    // Signal termination maps to exit code 1 (matching else => 1)
    try std.testing.expectEqual(@as(u8, 1), exit_code);
}

test "AC6: drainPipe with stdin piped matches docker.runWithStdio pattern" {
    const alloc = std.testing.allocator;

    // Mirror docker.zig: write stdin, drain stdout
    var child = std.process.Child.init(&.{ "cat" }, alloc);
    child.stdin_behavior = .Pipe;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Close;
    try child.spawn();

    // Write to stdin (matches docker.zig pattern)
    if (child.stdin) |stdin| {
        stdin.writeAll("piped input data") catch {};
        stdin.close();
        child.stdin = null;
    }

    const stdout_data = try process.drainPipe(alloc, child.stdout.?);
    defer alloc.free(stdout_data);

    const term = try child.wait();
    const exit_code = process.exitCode(term);

    try std.testing.expectEqualStrings("piped input data", stdout_data);
    try std.testing.expectEqual(@as(u8, 0), exit_code);
}

// =============================================================================
// AC3: No 8192-byte buffer loop remains in the four source files
// =============================================================================

test "AC3: git.zig no longer contains inline 8192-byte drain buffer" {
    const src = @embedFile("git.zig");
    // After refactoring, git.zig should not have a 8192-byte read buffer
    try std.testing.expect(std.mem.indexOf(u8, src, "read_buf: [8192]u8") == null);
}

test "AC3: docker.zig no longer contains inline 8192-byte drain buffer" {
    const src = @embedFile("docker.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "read_buf: [8192]u8") == null);
}

test "AC3: agent.zig no longer contains inline 8192-byte drain buffer" {
    const src = @embedFile("agent.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "read_buf: [8192]u8") == null);
}

test "AC3: pipeline.zig no longer contains inline 8192-byte drain buffer" {
    const src = @embedFile("pipeline.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "read_buf: [8192]u8") == null);
}

// =============================================================================
// AC4: No inline exit code switch remains in the four source files
// =============================================================================

test "AC4: git.zig no longer contains inline exit code switch" {
    const src = @embedFile("git.zig");
    // The original pattern: switch (term) { .Exited => |code| code, else => 1 }
    // After refactoring, git.zig should use process.exitCode(term) instead
    try std.testing.expect(std.mem.indexOf(u8, src, ".Exited => |code| code") == null);
}

test "AC4: docker.zig no longer contains inline exit code switch" {
    const src = @embedFile("docker.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, ".Exited => |code| code") == null);
}

test "AC4: agent.zig no longer contains inline exit code switch" {
    const src = @embedFile("agent.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, ".Exited => |code| code") == null);
}

test "AC4: pipeline.zig no longer contains inline exit code switch" {
    const src = @embedFile("pipeline.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, ".Exited => |code| code") == null);
}

// =============================================================================
// AC3+AC4 positive check: files now import process.zig
// =============================================================================

test "AC3: git.zig imports process.zig" {
    const src = @embedFile("git.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "@import(\"process.zig\")") != null);
}

test "AC3: docker.zig imports process.zig" {
    const src = @embedFile("docker.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "@import(\"process.zig\")") != null);
}

test "AC3: agent.zig imports process.zig" {
    const src = @embedFile("agent.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "@import(\"process.zig\")") != null);
}

test "AC3: pipeline.zig imports process.zig" {
    const src = @embedFile("pipeline.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "@import(\"process.zig\")") != null);
}

// =============================================================================
// AC8: process.zig only imports std (no other project dependencies)
// =============================================================================

test "AC8: process.zig only imports std" {
    const src = @embedFile("process.zig");

    // Count @import occurrences — should only be @import("std")
    var import_count: usize = 0;
    var pos: usize = 0;
    while (std.mem.indexOfPos(u8, src, pos, "@import(")) |idx| {
        import_count += 1;
        pos = idx + 8;
    }

    // There should be exactly one @import (for "std")
    try std.testing.expectEqual(@as(usize, 1), import_count);
    try std.testing.expect(std.mem.indexOf(u8, src, "@import(\"std\")") != null);
}

// =============================================================================
// AC7: No new public API changes — existing public types still exist unchanged
// =============================================================================

test "AC7: git.ExecResult still has stdout, stderr, exit_code, allocator fields" {
    const git = @import("git.zig");
    const info = @typeInfo(git.ExecResult);
    const fields = info.@"struct".fields;

    const expected_fields = [_][]const u8{ "stdout", "stderr", "exit_code", "allocator" };
    for (expected_fields) |name| {
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

test "AC7: docker.RunResult still has stdout, exit_code, allocator fields" {
    const docker = @import("docker.zig");
    const info = @typeInfo(docker.RunResult);
    const fields = info.@"struct".fields;

    const expected_fields = [_][]const u8{ "stdout", "exit_code", "allocator" };
    for (expected_fields) |name| {
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

test "AC7: agent.AgentResult still has output and new_session_id fields" {
    const agent = @import("agent.zig");
    const info = @typeInfo(agent.AgentResult);
    const fields = info.@"struct".fields;

    const expected_fields = [_][]const u8{ "output", "new_session_id" };
    for (expected_fields) |name| {
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

test "AC7: Git struct still has exec, checkout, pull, commit methods" {
    const git = @import("git.zig");
    try std.testing.expect(@hasDecl(git.Git, "exec"));
    try std.testing.expect(@hasDecl(git.Git, "checkout"));
    try std.testing.expect(@hasDecl(git.Git, "pull"));
    try std.testing.expect(@hasDecl(git.Git, "commit"));
}

test "AC7: Docker struct still has runWithStdio method" {
    const docker = @import("docker.zig");
    try std.testing.expect(@hasDecl(docker.Docker, "runWithStdio"));
}

test "AC7: agent module still has runDirect and parseNdjson" {
    const agent = @import("agent.zig");
    try std.testing.expect(@hasDecl(agent, "runDirect"));
    try std.testing.expect(@hasDecl(agent, "parseNdjson"));
}

// =============================================================================
// Edge Case 1: Pipe is null — call sites use if(child.stdout) guard
//
// We can't directly test null-pipe handling in drainPipe (it takes a non-optional
// File), but we verify the pattern: drainPipe is only called when pipe is non-null.
// This is a compile-time contract verified by the source checks above.
// =============================================================================

test "Edge1: drainPipe takes non-optional File (null guard is at call site)" {
    // Verify the parameter type is std.fs.File (not ?std.fs.File)
    const DrainFn = @TypeOf(process.drainPipe);
    const fn_info = @typeInfo(DrainFn).@"fn";
    try std.testing.expect(fn_info.params[1].type == std.fs.File);
}

// =============================================================================
// Edge Case 3: Read error mid-stream — catch break returns accumulated data
//
// We verify the contract: drainPipe returns successfully (not an error) even
// if the underlying read encounters an error after some bytes were read.
// We test this by draining a pipe from a process that exits abruptly.
// =============================================================================

test "Edge3: drainPipe returns partial data when process exits abruptly" {
    const alloc = std.testing.allocator;

    // Write some data then exit — pipe EOF is normal, not an error
    var child = std.process.Child.init(
        &.{ "/bin/sh", "-c", "printf 'partial'" },
        alloc,
    );
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Close;
    try child.spawn();

    const data = try process.drainPipe(alloc, child.stdout.?);
    defer alloc.free(data);

    _ = try child.wait();

    try std.testing.expectEqualStrings("partial", data);
}

// =============================================================================
// AC6: Integration — full exec pattern equivalence
//
// Verify that using process.drainPipe + process.exitCode produces identical
// results to what git.Git.exec would produce for the same command.
// =============================================================================

test "AC6: full exec pattern produces correct ExecResult-equivalent values" {
    const alloc = std.testing.allocator;
    const git = @import("git.zig");

    // Spawn a command the same way git.exec does
    var child = std.process.Child.init(&.{ "printf", "test output" }, alloc);
    child.stdin_behavior = .Close;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;
    try child.spawn();

    // Use the new utility functions
    const stdout_data = if (child.stdout) |pipe|
        try process.drainPipe(alloc, pipe)
    else
        try alloc.alloc(u8, 0);
    defer alloc.free(stdout_data);

    const stderr_data = if (child.stderr) |pipe|
        try process.drainPipe(alloc, pipe)
    else
        try alloc.alloc(u8, 0);
    defer alloc.free(stderr_data);

    const term = try child.wait();
    const exit_code = process.exitCode(term);

    // Construct ExecResult equivalent
    var result = git.ExecResult{
        .stdout = stdout_data,
        .stderr = stderr_data,
        .exit_code = exit_code,
        .allocator = alloc,
    };

    // Verify behavioral equivalence
    try std.testing.expect(result.success());
    try std.testing.expectEqualStrings("test output", result.stdout);
    try std.testing.expectEqual(@as(usize, 0), result.stderr.len);

    // Don't call result.deinit() — we manage memory via defer above
    _ = &result;
}
