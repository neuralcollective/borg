// Tests for spec #30: Fix failing build on main — resilient git hash in build.zig
//
// Verifies that build.zig resolves the git commit hash using
// std.process.Child.run() with a "unknown" fallback instead of b.run(), so
// that the build succeeds inside git worktrees, in directories without .git,
// and on systems without git installed.
//
// To include in the build, add to src/main.zig:
//   test { _ = @import("build_git_hash_test.zig"); }
//
// All tests below FAIL before the fix because `zig build test` itself cannot
// complete — build.zig panics at line 25 when `git rev-parse --short HEAD`
// exits non-zero inside a git worktree.

const std = @import("std");
const build_options = @import("build_options");

// =============================================================================
// AC1 / AC3 / AC4 — Source check: b.run() no longer used for git hash
//
// Before the fix, build.zig line 25 reads:
//   const git_hash = b.run(&.{ "git", "rev-parse", "--short", "HEAD" });
// After the fix, that line must be gone (replaced by Child.run).
// =============================================================================

test "AC1/AC3/AC4: build.zig does not use b.run() for git hash" {
    const src = @embedFile("../build.zig");
    // The old panicking call must have been removed.
    // "b.run(" appearing in the context of the git rev-parse command indicates
    // the fix was not applied.
    //
    // We search for the exact old pattern. A simple occurrence of "b.run(" could
    // theoretically exist for other reasons, so we look for the git argument too.
    const old_pattern_git = std.mem.indexOf(u8, src, "b.run(&.{ \"git\"");
    try std.testing.expect(old_pattern_git == null);
}

test "AC1/AC3/AC4: build.zig does not call b.run() with rev-parse" {
    const src = @embedFile("../build.zig");
    const old_pattern = std.mem.indexOf(u8, src, "rev-parse");
    // If rev-parse still appears it must NOT be inside a b.run() call.
    // Confirm b.run and rev-parse don't co-exist.
    if (old_pattern != null) {
        // rev-parse is present somewhere — make sure it's not inside b.run(
        const brun_pos = std.mem.indexOf(u8, src, "b.run(") orelse {
            return; // b.run( absent entirely — fix is applied
        };
        // Both present: check they aren't adjacent (within 80 chars of each other).
        const diff = if (old_pattern.? > brun_pos)
            old_pattern.? - brun_pos
        else
            brun_pos - old_pattern.?;
        try std.testing.expect(diff > 80);
    }
}

// =============================================================================
// AC1 / AC3 / AC4 — Source check: std.process.Child.run() is now used
// =============================================================================

test "AC1/AC3/AC4: build.zig uses std.process.Child.run() for git hash" {
    const src = @embedFile("../build.zig");
    // After the fix, the git hash block must use Child.run.
    const has_child_run = std.mem.indexOf(u8, src, "std.process.Child.run(") != null or
        std.mem.indexOf(u8, src, "Child.run(.{") != null;
    try std.testing.expect(has_child_run);
}

test "AC1/AC3/AC4: build.zig fallback literal 'unknown' is present" {
    const src = @embedFile("../build.zig");
    // The fallback value must be the string literal "unknown".
    try std.testing.expect(std.mem.indexOf(u8, src, "\"unknown\"") != null);
}

test "AC1/AC3/AC4: build.zig has catch-based fallback for Child.run error" {
    const src = @embedFile("../build.zig");
    // The fix must catch errors from Child.run and fall back to "unknown".
    // Either "catch break :blk" or a simpler "catch" must appear near the git call.
    const has_catch_fallback = std.mem.indexOf(u8, src, "catch break :blk") != null or
        std.mem.indexOf(u8, src, "catch |") != null;
    try std.testing.expect(has_catch_fallback);
}

test "AC1/AC3/AC4: build.zig checks Exited exit code before using git output" {
    const src = @embedFile("../build.zig");
    // The fix must verify the process exited cleanly (not killed by signal, etc.)
    // by checking result.term == .Exited and result.term.Exited == 0.
    const has_exited_check = std.mem.indexOf(u8, src, ".Exited") != null;
    try std.testing.expect(has_exited_check);
}

// =============================================================================
// AC6 — Compile-time type check: build_options.git_hash is []const u8
//
// Any source file doing @import("build_options").git_hash expects a []const u8.
// This test verifies the type is unchanged after the fix.
// =============================================================================

test "AC6: build_options.git_hash is a slice of const u8" {
    const T = @TypeOf(build_options.git_hash);
    const info = @typeInfo(T);
    // Must be a slice (pointer to array), not a comptime integer or other type.
    try std.testing.expect(info == .pointer);
    try std.testing.expect(info.pointer.child == u8);
    try std.testing.expect(info.pointer.is_const);
}

test "AC6: build_options.git_hash is assignable to []const u8" {
    const hash: []const u8 = build_options.git_hash;
    // If the above compiles and the length is accessible, the type is correct.
    // len >= 0 is always true for a slice — this proves the type is a slice.
    try std.testing.expect(hash.len == hash.len);
}

// =============================================================================
// AC2 — Runtime value: hash is "unknown" or a valid short SHA
//
// In a normal git checkout, git_hash is a 7–10 character hex string.
// In a worktree or non-git environment, it is "unknown".
// Either value is acceptable.
// =============================================================================

test "AC2: build_options.git_hash is 'unknown' or a non-empty hex string" {
    const hash = build_options.git_hash;
    if (std.mem.eql(u8, hash, "unknown")) {
        // Fallback path — git was unavailable or failed (e.g., inside a worktree).
        return;
    }
    // Non-fallback path: must be a non-empty hex string (short SHA).
    try std.testing.expect(hash.len > 0);
    for (hash) |c| {
        const is_hex = (c >= '0' and c <= '9') or
            (c >= 'a' and c <= 'f') or
            (c >= 'A' and c <= 'F');
        try std.testing.expect(is_hex);
    }
}

test "AC2: build_options.git_hash is not an empty string" {
    // An empty hash indicates a bug: git returned empty stdout with exit 0,
    // and the build did not apply a minimum-length guard.
    // Per spec edge case E2, "" is an acceptable sentinel, but the normal
    // expectation is "unknown" for error paths.
    // This test documents the current value for diagnostic purposes.
    const hash = build_options.git_hash;
    // Both "unknown" and a hex SHA are non-empty; only a broken build produces "".
    try std.testing.expect(hash.len > 0);
}

// =============================================================================
// AC1 — Meta-test: the build succeeded in the current environment
//
// If this test runs at all, zig build test succeeded, which means the build
// did not panic on git rev-parse. This is the primary acceptance criterion.
// =============================================================================

test "AC1: build succeeded in current environment (test is running)" {
    // By definition: if we reach this line, zig build test completed the
    // compilation phase without panicking on git rev-parse.
    // In a git worktree, git_hash must be "unknown".
    // In a normal checkout, git_hash is a hex SHA.
    const hash = build_options.git_hash;
    const valid = std.mem.eql(u8, hash, "unknown") or hash.len >= 7;
    try std.testing.expect(valid);
}

// =============================================================================
// AC3 — Logic test: fallback returns "unknown" when git binary is not found
//
// Mirrors the proposed build.zig fix: run git with an empty PATH so the
// binary cannot be located. Child.run() returns FileNotFound (or similar),
// which must be caught and mapped to "unknown".
// =============================================================================

test "AC3: fallback logic returns 'unknown' when git binary is not on PATH" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    // Build an env map identical to the current environment but with PATH
    // replaced by a directory that definitely does not contain git.
    var env = try std.process.getEnvMap(alloc);
    try env.put("PATH", "/tmp/no-git-here-borg-test");

    // Run the exact fallback logic from the proposed build.zig fix.
    const hash: []const u8 = blk: {
        const result = std.process.Child.run(.{
            .allocator = alloc,
            .argv = &.{ "git", "rev-parse", "--short", "HEAD" },
            .env_map = &env,
        }) catch break :blk "unknown"; // git not found → error → "unknown"
        if (result.term == .Exited and result.term.Exited == 0) {
            break :blk std.mem.trim(u8, result.stdout, &std.ascii.whitespace);
        }
        break :blk "unknown";
    };

    try std.testing.expectEqualStrings("unknown", hash);
}

// =============================================================================
// AC4 — Logic test: fallback returns "unknown" in a directory without .git
//
// Run git rev-parse in a freshly created temp directory that has no .git
// subdirectory. git exits non-zero; the fallback must produce "unknown".
// =============================================================================

test "AC4: fallback logic returns 'unknown' in a directory without .git" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var tmp = std.testing.tmpDir(.{});
    defer tmp.cleanup();

    // Get the absolute path to the temp dir so we can pass it as cwd.
    var tmp_path_buf: [std.fs.max_path_bytes]u8 = undefined;
    const tmp_path = try tmp.dir.realpath(".", &tmp_path_buf);

    const hash: []const u8 = blk: {
        const result = std.process.Child.run(.{
            .allocator = alloc,
            .argv = &.{ "git", "rev-parse", "--short", "HEAD" },
            .cwd = tmp_path,
        }) catch break :blk "unknown"; // git not found at all → "unknown"
        if (result.term == .Exited and result.term.Exited == 0) {
            // Unexpected success (tmp dir somehow inside a git repo) — still valid.
            break :blk std.mem.trim(u8, result.stdout, &std.ascii.whitespace);
        }
        // git failed (non-zero exit in non-git dir) → "unknown"
        break :blk "unknown";
    };

    // In a plain temp dir git must fail; "unknown" is the expected result.
    // If git somehow succeeded (e.g. /tmp is inside a git repo), we accept it.
    const is_unknown = std.mem.eql(u8, hash, "unknown");
    const is_hex = hash.len >= 7 and for (hash) |c| {
        if (!std.ascii.isHex(c)) break false;
    } else true;
    try std.testing.expect(is_unknown or is_hex);
}

test "AC4: git exits non-zero in a directory without .git" {
    const alloc = std.testing.allocator;

    var tmp = std.testing.tmpDir(.{});
    defer tmp.cleanup();

    var tmp_path_buf: [std.fs.max_path_bytes]u8 = undefined;
    const tmp_path = try tmp.dir.realpath(".", &tmp_path_buf);

    const result = std.process.Child.run(.{
        .allocator = alloc,
        .argv = &.{ "git", "rev-parse", "--short", "HEAD" },
        .cwd = tmp_path,
    }) catch return; // git not installed — AC3 covers this case, skip here
    defer alloc.free(result.stdout);
    defer alloc.free(result.stderr);

    // git must not succeed in a plain temp directory.
    const exited_zero = switch (result.term) {
        .Exited => |code| code == 0,
        else => false,
    };
    // If the temp dir happens to be inside a git repo, git will succeed.
    // We can't guarantee the host environment, so we only assert if stdout
    // is empty (which would indicate a genuine non-git directory).
    if (result.stdout.len == 0) {
        try std.testing.expect(!exited_zero);
    }
}

// =============================================================================
// E2 — Edge case: git exits 0 but returns empty stdout
//
// std.mem.trim on "" yields "". The build proceeds with git_hash = "".
// The fallback logic must not crash on empty output.
// =============================================================================

test "E2: std.mem.trim on empty git output yields empty string" {
    const empty: []const u8 = "";
    const trimmed = std.mem.trim(u8, empty, &std.ascii.whitespace);
    // Result is "" — the build proceeds without crashing.
    try std.testing.expectEqual(@as(usize, 0), trimmed.len);
}

test "E2: fallback logic handles empty stdout gracefully" {
    // Simulate: git exits 0 but stdout is "".
    // The fix's inner branch: std.mem.trim("", whitespace) = "".
    // The build must not panic.
    const alloc = std.testing.allocator;
    const fake_stdout = try alloc.dupe(u8, "");
    defer alloc.free(fake_stdout);

    const hash = std.mem.trim(u8, fake_stdout, &std.ascii.whitespace);
    // hash == "" is an acceptable (though unusual) git_hash value.
    try std.testing.expectEqual(@as(usize, 0), hash.len);
}

// =============================================================================
// E3 — Edge case: git exits 0 but stdout is only whitespace/newline
//
// git rev-parse normally appends a newline; std.mem.trim strips it.
// If stdout is only whitespace, the result is "".
// =============================================================================

test "E3: std.mem.trim on whitespace-only git output yields empty string" {
    const ws: []const u8 = "\n";
    const trimmed = std.mem.trim(u8, ws, &std.ascii.whitespace);
    try std.testing.expectEqual(@as(usize, 0), trimmed.len);
}

test "E3: std.mem.trim strips leading and trailing newlines from git output" {
    const raw: []const u8 = "abc1234\n";
    const trimmed = std.mem.trim(u8, raw, &std.ascii.whitespace);
    try std.testing.expectEqualStrings("abc1234", trimmed);
}

test "E3: std.mem.trim strips CRLF line endings" {
    const raw: []const u8 = "abc1234\r\n";
    const trimmed = std.mem.trim(u8, raw, &std.ascii.whitespace);
    try std.testing.expectEqualStrings("abc1234", trimmed);
}

// =============================================================================
// E4 — Edge case: git process killed by a signal
//
// result.term is .Signal rather than .Exited; the condition
// `result.term == .Exited and result.term.Exited == 0` is false,
// so the fallback must produce "unknown".
// =============================================================================

test "E4: Signal termination does not satisfy the Exited(0) condition" {
    const term: std.process.Child.Term = .{ .Signal = 9 };
    const is_success = switch (term) {
        .Exited => |code| code == 0,
        else => false,
    };
    try std.testing.expect(!is_success);
}

test "E4: Stopped termination does not satisfy the Exited(0) condition" {
    const term: std.process.Child.Term = .{ .Stopped = 19 };
    const is_success = switch (term) {
        .Exited => |code| code == 0,
        else => false,
    };
    try std.testing.expect(!is_success);
}

test "E4: Unknown termination does not satisfy the Exited(0) condition" {
    const term: std.process.Child.Term = .{ .Unknown = 0xFF };
    const is_success = switch (term) {
        .Exited => |code| code == 0,
        else => false,
    };
    try std.testing.expect(!is_success);
}

test "E4: non-zero Exited does not satisfy the Exited(0) condition" {
    const term: std.process.Child.Term = .{ .Exited = 128 };
    const is_success = switch (term) {
        .Exited => |code| code == 0,
        else => false,
    };
    try std.testing.expect(!is_success);
}

// =============================================================================
// E5 — Edge case: OOM / process spawn error
//
// When Child.run() returns an error (e.g., OutOfMemory, FileNotFound),
// the `catch break :blk "unknown"` arm must handle it.
// =============================================================================

test "E5: catch on Child.run error produces 'unknown' — logic unit test" {
    // We cannot easily force OOM in a unit test, but we can verify the logic:
    // any error from Child.run is caught and mapped to "unknown".
    //
    // Simulate by calling Child.run with a non-existent binary path.
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    const hash: []const u8 = blk: {
        const result = std.process.Child.run(.{
            .allocator = alloc,
            .argv = &.{ "/nonexistent-binary-borg-test-30", "rev-parse", "--short", "HEAD" },
        }) catch break :blk "unknown"; // FileNotFound → "unknown"
        if (result.term == .Exited and result.term.Exited == 0) {
            break :blk std.mem.trim(u8, result.stdout, &std.ascii.whitespace);
        }
        break :blk "unknown";
    };

    try std.testing.expectEqualStrings("unknown", hash);
}

// =============================================================================
// E1 — Edge case: detached HEAD / shallow clone
//
// git rev-parse --short HEAD succeeds even on shallow clones or detached HEAD.
// The build must embed the real hash in these cases (not fall back to "unknown").
// =============================================================================

test "E1: Exited(0) with non-empty stdout does satisfy the success condition" {
    // Verify the success branch is correctly identified.
    const term: std.process.Child.Term = .{ .Exited = 0 };
    const is_success = switch (term) {
        .Exited => |code| code == 0,
        else => false,
    };
    try std.testing.expect(is_success);
}

test "E1: non-empty git output is preserved after trimming trailing newline" {
    // Simulate output from `git rev-parse --short HEAD` in a normal checkout.
    const raw: []const u8 = "a1b2c3d\n";
    const trimmed = std.mem.trim(u8, raw, &std.ascii.whitespace);
    try std.testing.expectEqualStrings("a1b2c3d", trimmed);
    try std.testing.expect(trimmed.len == 7);
}

// =============================================================================
// E8 — Edge case: multiple simultaneous builds
//
// Each zig build invocation runs its own Child.run(); there is no shared state.
// This is a structural property of the fix (no global variables, no locks).
// =============================================================================

test "E8: build.zig git hash block uses no global mutable state" {
    const src = @embedFile("../build.zig");
    // The fix must not introduce a 'var' at file scope for the git hash.
    // We verify there is no 'var git_hash' at file scope (only 'const').
    const bad_pattern = "var git_hash";
    try std.testing.expect(std.mem.indexOf(u8, src, bad_pattern) == null);
}

// =============================================================================
// AC5 — Regression: all existing unit tests still pass
//
// This is implicitly verified by the test runner completing successfully.
// We add a structural check: build.zig still wires build_options into exe_mod.
// =============================================================================

test "AC5: build.zig still calls addOptions to wire build_options into exe_mod" {
    const src = @embedFile("../build.zig");
    // The addOptions call must still be present after the refactoring.
    try std.testing.expect(std.mem.indexOf(u8, src, "addOptions(\"build_options\"") != null or
        std.mem.indexOf(u8, src, "addOptions(") != null);
}

test "AC5: build.zig still adds git_hash option to build_options" {
    const src = @embedFile("../build.zig");
    // The addOption call for git_hash must still be present.
    try std.testing.expect(std.mem.indexOf(u8, src, "git_hash") != null);
}
