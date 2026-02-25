// Tests for git.zig worktree operations: addWorktree, addWorktreeExisting,
// removeWorktree, and listWorktrees.
//
// Each test creates an isolated temporary git repository under /tmp and
// cleans up via defer.  A private makeTempRepo helper initialises a repo
// with one commit on `main`; makeEmptyRepo creates a repo with no commits.
//
// To include in the build, git.zig must contain:
//   test { _ = @import("git_worktree_test.zig"); }

const std = @import("std");
const git_mod = @import("git.zig");
const Git = git_mod.Git;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Returns a random u64 suitable for unique path suffixes.
fn uniqueId() u64 {
    var buf: [8]u8 = undefined;
    std.crypto.random.bytes(&buf);
    return std.mem.readInt(u64, &buf, .little);
}

/// Creates a temporary directory, initialises a git repo with one commit on
/// `main`, and returns the absolute path (heap-allocated via `allocator`).
///
/// Caller must clean up:
///   defer allocator.free(path);
///   defer std.fs.deleteTreeAbsolute(path) catch {};
fn makeTempRepo(allocator: std.mem.Allocator) ![]const u8 {
    const path = try std.fmt.allocPrint(
        allocator,
        "/tmp/borg-wt-repo-{x}",
        .{uniqueId()},
    );
    try std.fs.makeDirAbsolute(path);

    var git = Git.init(allocator, path);

    var r1 = try git.exec(&.{ "init", "-b", "main" });
    defer r1.deinit();

    var r2 = try git.exec(&.{ "config", "user.email", "test@borg.test" });
    defer r2.deinit();

    var r3 = try git.exec(&.{ "config", "user.name", "Borg Test" });
    defer r3.deinit();

    // Write a file and commit so HEAD exists.
    const file_path = try std.fmt.allocPrint(allocator, "{s}/README.md", .{path});
    defer allocator.free(file_path);
    try std.fs.cwd().writeFile(.{ .sub_path = file_path, .data = "# test repo\n" });

    var r4 = try git.addAll();
    defer r4.deinit();

    var r5 = try git.commit("initial commit");
    defer r5.deinit();

    return path;
}

/// Creates a temporary directory, initialises a git repo with NO commits, and
/// returns the absolute path (heap-allocated via `allocator`).
///
/// Caller must clean up:
///   defer allocator.free(path);
///   defer std.fs.deleteTreeAbsolute(path) catch {};
fn makeEmptyRepo(allocator: std.mem.Allocator) ![]const u8 {
    const path = try std.fmt.allocPrint(
        allocator,
        "/tmp/borg-wt-empty-{x}",
        .{uniqueId()},
    );
    try std.fs.makeDirAbsolute(path);

    var git = Git.init(allocator, path);

    var r1 = try git.exec(&.{ "init", "-b", "main" });
    defer r1.deinit();

    var r2 = try git.exec(&.{ "config", "user.email", "test@borg.test" });
    defer r2.deinit();

    var r3 = try git.exec(&.{ "config", "user.name", "Borg Test" });
    defer r3.deinit();

    return path;
}

// ── AC1: addWorktree creates the directory on disk ────────────────────────────

test "AC1: addWorktree creates directory on disk" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    const wt_path = try std.fmt.allocPrint(alloc, "/tmp/borg-wt-work-{x}", .{uniqueId()});
    defer alloc.free(wt_path);
    defer std.fs.deleteTreeAbsolute(wt_path) catch {};

    var git = Git.init(alloc, repo_path);
    var result = try git.addWorktree(wt_path, "feature-wt");
    defer result.deinit();

    try std.testing.expectEqual(@as(u8, 0), result.exit_code);

    // The worktree directory must exist on disk.
    std.fs.accessAbsolute(wt_path, .{}) catch |err| {
        std.debug.print("Expected worktree directory at {s}, got: {}\n", .{ wt_path, err });
        return err;
    };
}

// ── AC2: listWorktrees output includes the new worktree path ─────────────────

test "AC2: listWorktrees output includes new worktree path" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    const wt_path = try std.fmt.allocPrint(alloc, "/tmp/borg-wt-work-{x}", .{uniqueId()});
    defer alloc.free(wt_path);
    defer std.fs.deleteTreeAbsolute(wt_path) catch {};

    var git = Git.init(alloc, repo_path);

    var add_r = try git.addWorktree(wt_path, "feature-wt");
    defer add_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), add_r.exit_code);

    var list_r = try git.listWorktrees();
    defer list_r.deinit();

    try std.testing.expectEqual(@as(u8, 0), list_r.exit_code);
    try std.testing.expect(std.mem.indexOf(u8, list_r.stdout, wt_path) != null);
}

// ── AC3: listWorktrees porcelain output includes the branch name ──────────────

test "AC3: listWorktrees porcelain output includes new branch name" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    const wt_path = try std.fmt.allocPrint(alloc, "/tmp/borg-wt-work-{x}", .{uniqueId()});
    defer alloc.free(wt_path);
    defer std.fs.deleteTreeAbsolute(wt_path) catch {};

    var git = Git.init(alloc, repo_path);

    var add_r = try git.addWorktree(wt_path, "feature-wt");
    defer add_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), add_r.exit_code);

    var list_r = try git.listWorktrees();
    defer list_r.deinit();

    try std.testing.expectEqual(@as(u8, 0), list_r.exit_code);
    // Porcelain format contains "branch refs/heads/feature-wt".
    try std.testing.expect(std.mem.indexOf(u8, list_r.stdout, "feature-wt") != null);
}

// ── AC4: removeWorktree succeeds and the directory is deleted ────────────────

test "AC4: removeWorktree succeeds and directory is deleted from disk" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    const wt_path = try std.fmt.allocPrint(alloc, "/tmp/borg-wt-work-{x}", .{uniqueId()});
    defer alloc.free(wt_path);
    // No defer deleteTree for wt_path — removeWorktree must remove it.

    var git = Git.init(alloc, repo_path);

    var add_r = try git.addWorktree(wt_path, "feature-wt");
    defer add_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), add_r.exit_code);

    // Confirm the directory exists before removal.
    try std.fs.accessAbsolute(wt_path, .{});

    var rm_r = try git.removeWorktree(wt_path);
    defer rm_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), rm_r.exit_code);

    // The directory must no longer exist.
    try std.testing.expectError(error.FileNotFound, std.fs.accessAbsolute(wt_path, .{}));
}

// ── AC5: listWorktrees no longer lists the removed worktree ──────────────────

test "AC5: listWorktrees does not contain removed worktree path" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    const wt_path = try std.fmt.allocPrint(alloc, "/tmp/borg-wt-work-{x}", .{uniqueId()});
    defer alloc.free(wt_path);

    var git = Git.init(alloc, repo_path);

    var add_r = try git.addWorktree(wt_path, "feature-wt");
    defer add_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), add_r.exit_code);

    var rm_r = try git.removeWorktree(wt_path);
    defer rm_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), rm_r.exit_code);

    var list_r = try git.listWorktrees();
    defer list_r.deinit();

    try std.testing.expectEqual(@as(u8, 0), list_r.exit_code);
    // The removed worktree path must not appear anywhere in the output.
    try std.testing.expect(std.mem.indexOf(u8, list_r.stdout, wt_path) == null);
}

// ── AC6: addWorktreeExisting succeeds with a pre-existing branch ──────────────

test "AC6: addWorktreeExisting succeeds with pre-existing branch" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    const wt_path = try std.fmt.allocPrint(alloc, "/tmp/borg-wt-work-{x}", .{uniqueId()});
    defer alloc.free(wt_path);
    defer std.fs.deleteTreeAbsolute(wt_path) catch {};

    var git = Git.init(alloc, repo_path);

    // Create a branch then check back out to main so the branch is free.
    var branch_r = try git.createBranch("existing-branch", "main");
    defer branch_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), branch_r.exit_code);

    var co_r = try git.checkout("main");
    defer co_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), co_r.exit_code);

    // Check the existing branch out into a new worktree.
    var add_r = try git.addWorktreeExisting(wt_path, "existing-branch");
    defer add_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), add_r.exit_code);

    // The worktree directory must exist.
    std.fs.accessAbsolute(wt_path, .{}) catch |err| {
        std.debug.print("Expected worktree directory at {s}, got: {}\n", .{ wt_path, err });
        return err;
    };
}

// ── AC7: listWorktrees always lists the main worktree ────────────────────────

test "AC7: listWorktrees always lists the main worktree path" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    const wt_path = try std.fmt.allocPrint(alloc, "/tmp/borg-wt-work-{x}", .{uniqueId()});
    defer alloc.free(wt_path);
    defer std.fs.deleteTreeAbsolute(wt_path) catch {};

    var git = Git.init(alloc, repo_path);

    // Before any extra worktree: main repo appears and output starts with "worktree ".
    {
        var list_r = try git.listWorktrees();
        defer list_r.deinit();
        try std.testing.expectEqual(@as(u8, 0), list_r.exit_code);
        try std.testing.expect(std.mem.indexOf(u8, list_r.stdout, repo_path) != null);
        try std.testing.expect(std.mem.startsWith(u8, list_r.stdout, "worktree "));
    }

    // After adding a secondary worktree: main repo still appears first.
    var add_r = try git.addWorktree(wt_path, "feature-wt");
    defer add_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), add_r.exit_code);

    {
        var list_r = try git.listWorktrees();
        defer list_r.deinit();
        try std.testing.expectEqual(@as(u8, 0), list_r.exit_code);
        try std.testing.expect(std.mem.indexOf(u8, list_r.stdout, repo_path) != null);
        try std.testing.expect(std.mem.startsWith(u8, list_r.stdout, "worktree "));
    }

    // After removing the secondary worktree: main repo still appears.
    var rm_r = try git.removeWorktree(wt_path);
    defer rm_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), rm_r.exit_code);

    {
        var list_r = try git.listWorktrees();
        defer list_r.deinit();
        try std.testing.expectEqual(@as(u8, 0), list_r.exit_code);
        try std.testing.expect(std.mem.indexOf(u8, list_r.stdout, repo_path) != null);
        try std.testing.expect(std.mem.startsWith(u8, list_r.stdout, "worktree "));
    }
}

// ── E1: addWorktree with a duplicate branch name fails ───────────────────────

test "E1: addWorktree with already-existing branch name fails" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    const id = uniqueId();
    const wt_path1 = try std.fmt.allocPrint(alloc, "/tmp/borg-wt-work-{x}-a", .{id});
    defer alloc.free(wt_path1);
    defer std.fs.deleteTreeAbsolute(wt_path1) catch {};

    const wt_path2 = try std.fmt.allocPrint(alloc, "/tmp/borg-wt-work-{x}-b", .{id});
    defer alloc.free(wt_path2);
    // wt_path2 must not be created.

    var git = Git.init(alloc, repo_path);

    // First addWorktree succeeds and creates "dup-branch".
    var add1 = try git.addWorktree(wt_path1, "dup-branch");
    defer add1.deinit();
    try std.testing.expectEqual(@as(u8, 0), add1.exit_code);

    // Second addWorktree tries to create "dup-branch" again — must fail.
    var add2 = try git.addWorktree(wt_path2, "dup-branch");
    defer add2.deinit();
    try std.testing.expect(add2.exit_code != 0);
    try std.testing.expect(add2.stderr.len > 0);

    // The second worktree directory must not have been created.
    try std.testing.expectError(error.FileNotFound, std.fs.accessAbsolute(wt_path2, .{}));
}

// ── E2: addWorktree fails when destination path already exists and is non-empty

test "E2: addWorktree fails when destination path already exists non-empty" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    const wt_path = try std.fmt.allocPrint(alloc, "/tmp/borg-wt-work-{x}", .{uniqueId()});
    defer alloc.free(wt_path);
    defer std.fs.deleteTreeAbsolute(wt_path) catch {};

    // Pre-create the directory with content so git cannot use it.
    try std.fs.makeDirAbsolute(wt_path);
    const blocker = try std.fmt.allocPrint(alloc, "{s}/occupied.txt", .{wt_path});
    defer alloc.free(blocker);
    try std.fs.cwd().writeFile(.{ .sub_path = blocker, .data = "occupied\n" });

    var git = Git.init(alloc, repo_path);
    var result = try git.addWorktree(wt_path, "new-branch-e2");
    defer result.deinit();

    // git must refuse because the target directory is not empty.
    try std.testing.expect(result.exit_code != 0);
}

// ── E3: removeWorktree on a non-worktree path fails ──────────────────────────

test "E3: removeWorktree on a path that is not a worktree fails" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    const plain_dir = try std.fmt.allocPrint(alloc, "/tmp/borg-wt-plain-{x}", .{uniqueId()});
    defer alloc.free(plain_dir);
    defer std.fs.deleteTreeAbsolute(plain_dir) catch {};

    // A plain directory that is not a git worktree.
    try std.fs.makeDirAbsolute(plain_dir);

    var git = Git.init(alloc, repo_path);
    var result = try git.removeWorktree(plain_dir);
    defer result.deinit();

    try std.testing.expect(result.exit_code != 0);
}

// ── E4: removeWorktree called twice — second call must fail gracefully ────────

test "E4: removeWorktree called twice fails on the second call" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    const wt_path = try std.fmt.allocPrint(alloc, "/tmp/borg-wt-work-{x}", .{uniqueId()});
    defer alloc.free(wt_path);

    var git = Git.init(alloc, repo_path);

    var add_r = try git.addWorktree(wt_path, "branch-e4");
    defer add_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), add_r.exit_code);

    var rm1 = try git.removeWorktree(wt_path);
    defer rm1.deinit();
    try std.testing.expectEqual(@as(u8, 0), rm1.exit_code);

    // Second removal — git error, not a Zig panic or process-level failure.
    var rm2 = try git.removeWorktree(wt_path);
    defer rm2.deinit();
    try std.testing.expect(rm2.exit_code != 0);
}

// ── E5: listWorktrees on a fresh repo lists exactly one worktree block ────────

test "E5: listWorktrees on a fresh repo lists exactly one worktree block" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);
    var result = try git.listWorktrees();
    defer result.deinit();

    try std.testing.expectEqual(@as(u8, 0), result.exit_code);

    // Count lines that start with "worktree " — should be exactly 1.
    var count: usize = 0;
    var iter = std.mem.splitScalar(u8, result.stdout, '\n');
    while (iter.next()) |line| {
        if (std.mem.startsWith(u8, line, "worktree ")) {
            count += 1;
        }
    }
    try std.testing.expectEqual(@as(usize, 1), count);
}

// ── E6: addWorktreeExisting for a nonexistent branch fails ───────────────────

test "E6: addWorktreeExisting for a branch that does not exist fails" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    const wt_path = try std.fmt.allocPrint(alloc, "/tmp/borg-wt-work-{x}", .{uniqueId()});
    defer alloc.free(wt_path);
    // wt_path must not be created.

    var git = Git.init(alloc, repo_path);
    var result = try git.addWorktreeExisting(wt_path, "branch-does-not-exist");
    defer result.deinit();

    try std.testing.expect(result.exit_code != 0);

    // No directory must have been created at the target path.
    try std.testing.expectError(error.FileNotFound, std.fs.accessAbsolute(wt_path, .{}));
}

// ── E7: addWorktree on an empty repo (no commits) ────────────────────────────
//
// git 2.24+ infers --orphan when there is no HEAD commit, so addWorktree with
// -b succeeds and creates an orphan-branch worktree.  The test verifies the
// command completes without a Zig-level error and that the result is cleaned up
// properly, documenting that callers must not assume failure on empty repos.

test "E7: addWorktree on empty repo runs without process error" {
    const alloc = std.testing.allocator;

    const repo_path = try makeEmptyRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    const wt_path = try std.fmt.allocPrint(alloc, "/tmp/borg-wt-work-{x}", .{uniqueId()});
    defer alloc.free(wt_path);
    defer std.fs.deleteTreeAbsolute(wt_path) catch {};

    var git = Git.init(alloc, repo_path);
    // Must not return a Zig error (process spawn failure); exit code from git
    // depends on the git version — modern git (2.24+) succeeds via --orphan.
    var result = try git.addWorktree(wt_path, "feature-e7");
    defer result.deinit();

    // The command must complete (either success or a clean git error).
    // We only assert the Zig call itself does not propagate an error.
    _ = result.exit_code;
}
