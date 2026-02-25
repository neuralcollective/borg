// Tests for git.zig merge() and deleteBranch() operations.
//
// Verifies that:
//   - merge() with --no-ff succeeds on a fast-forwardable branch and creates
//     a real merge commit (two parents).
//   - merge() succeeds on truly diverged branches (three-way merge).
//   - deleteBranch() removes a merged branch and exits 0.
//   - deleteBranch() on a non-existent branch returns a non-zero ExecResult
//     (never a Zig error) with diagnostic stderr output.
//   - Edge cases: no-op merge on identical branch, deleting the currently
//     checked-out branch, and non-empty output on a successful merge.
//
// Each test creates an isolated temporary git repository under /tmp and
// cleans up via defer.  The private makeTempRepo helper initialises a repo
// with one commit on `main`.
//
// To include in the build, git.zig must contain:
//   test { _ = @import("git_merge_delete_test.zig"); }

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
/// `main` (a `seed.txt` file), configures user.email and user.name, and
/// returns the absolute path (heap-allocated via `allocator`).
///
/// Caller must clean up:
///   defer allocator.free(path);
///   defer std.fs.deleteTreeAbsolute(path) catch {};
fn makeTempRepo(allocator: std.mem.Allocator) ![]const u8 {
    const path = try std.fmt.allocPrint(
        allocator,
        "/tmp/borg-md-repo-{x}",
        .{uniqueId()},
    );
    errdefer allocator.free(path);
    try std.fs.makeDirAbsolute(path);

    var git = Git.init(allocator, path);

    var r1 = try git.exec(&.{ "init", "-b", "main" });
    defer r1.deinit();
    if (!r1.success()) return error.GitInitFailed;

    var r2 = try git.exec(&.{ "config", "user.email", "test@borg.test" });
    defer r2.deinit();

    var r3 = try git.exec(&.{ "config", "user.name", "Borg Test" });
    defer r3.deinit();

    const seed = try std.fmt.allocPrint(allocator, "{s}/seed.txt", .{path});
    defer allocator.free(seed);
    try std.fs.cwd().writeFile(.{ .sub_path = seed, .data = "seed\n" });

    var r4 = try git.addAll();
    defer r4.deinit();

    var r5 = try git.commit("initial commit");
    defer r5.deinit();
    if (!r5.success()) return error.GitCommitFailed;

    return path;
}

// ── AC1: --no-ff merge on fast-forwardable branch creates a merge commit ──────

test "AC1: merge fast-forwardable branch with --no-ff succeeds and creates merge commit" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    // feature: one commit ahead of main; main has no new commits since the
    // fork, so a plain fast-forward would be possible.
    var branch_r = try git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), branch_r.exit_code);

    const feat_file = try std.fmt.allocPrint(alloc, "{s}/feature.txt", .{repo_path});
    defer alloc.free(feat_file);
    try std.fs.cwd().writeFile(.{ .sub_path = feat_file, .data = "feature content\n" });

    var add_r = try git.addAll();
    defer add_r.deinit();
    try std.testing.expect(add_r.success());

    var commit_r = try git.commit("add feature.txt");
    defer commit_r.deinit();
    try std.testing.expect(commit_r.success());

    // Back to main with no additional commits (fast-forward eligible without --no-ff).
    var co_main = try git.checkout("main");
    defer co_main.deinit();
    try std.testing.expectEqual(@as(u8, 0), co_main.exit_code);

    // AC1: merge must succeed.
    var merge_r = try git.merge("feature");
    defer merge_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), merge_r.exit_code);
    try std.testing.expect(merge_r.success());

    // --no-ff must have been honoured: HEAD must have a second parent.
    var parent2 = try git.exec(&.{ "rev-parse", "--verify", "HEAD^2" });
    defer parent2.deinit();
    try std.testing.expectEqual(@as(u8, 0), parent2.exit_code);
    try std.testing.expect(parent2.success());
}

// ── AC2: three-way merge of diverged branches succeeds ────────────────────────

test "AC2: merge with unrelated changes on both branches succeeds (true three-way)" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    // feature: add a file that main doesn't have.
    var branch_r = try git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), branch_r.exit_code);

    const feat_file = try std.fmt.allocPrint(alloc, "{s}/feature.txt", .{repo_path});
    defer alloc.free(feat_file);
    try std.fs.cwd().writeFile(.{ .sub_path = feat_file, .data = "feature content\n" });

    var add_feat = try git.addAll();
    defer add_feat.deinit();
    try std.testing.expect(add_feat.success());

    var commit_feat = try git.commit("add feature.txt");
    defer commit_feat.deinit();
    try std.testing.expect(commit_feat.success());

    // main: advance past the fork point with an unrelated commit.
    var co_main = try git.checkout("main");
    defer co_main.deinit();
    try std.testing.expectEqual(@as(u8, 0), co_main.exit_code);

    const main2_file = try std.fmt.allocPrint(alloc, "{s}/main2.txt", .{repo_path});
    defer alloc.free(main2_file);
    try std.fs.cwd().writeFile(.{ .sub_path = main2_file, .data = "main2 content\n" });

    var add_main2 = try git.addAll();
    defer add_main2.deinit();
    try std.testing.expect(add_main2.success());

    var commit_main2 = try git.commit("advance main");
    defer commit_main2.deinit();
    try std.testing.expect(commit_main2.success());

    // AC2: three-way merge must succeed.
    var merge_r = try git.merge("feature");
    defer merge_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), merge_r.exit_code);
    try std.testing.expect(merge_r.success());

    // A genuine merge commit must have been created (HEAD has two parents).
    var parent2 = try git.exec(&.{ "rev-parse", "--verify", "HEAD^2" });
    defer parent2.deinit();
    try std.testing.expectEqual(@as(u8, 0), parent2.exit_code);
    try std.testing.expect(parent2.success());

    // Working tree must be clean after a successful merge.
    const clean = try git.statusClean();
    try std.testing.expect(clean);
}

// ── AC3: deleteBranch removes a merged branch ────────────────────────────────

test "AC3: deleteBranch removes a merged branch and exits 0" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    // feature: one commit ahead of main.
    var branch_r = try git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), branch_r.exit_code);

    const feat_file = try std.fmt.allocPrint(alloc, "{s}/feature.txt", .{repo_path});
    defer alloc.free(feat_file);
    try std.fs.cwd().writeFile(.{ .sub_path = feat_file, .data = "feature content\n" });

    var add_r = try git.addAll();
    defer add_r.deinit();
    try std.testing.expect(add_r.success());

    var commit_r = try git.commit("add feature.txt");
    defer commit_r.deinit();
    try std.testing.expect(commit_r.success());

    // Merge from main.
    var co_main = try git.checkout("main");
    defer co_main.deinit();
    try std.testing.expectEqual(@as(u8, 0), co_main.exit_code);

    var merge_r = try git.merge("feature");
    defer merge_r.deinit();
    try std.testing.expect(merge_r.success());

    // AC3: delete the merged feature branch.
    var del_r = try git.deleteBranch("feature");
    defer del_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), del_r.exit_code);
    try std.testing.expect(del_r.success());

    // The branch must no longer appear in `git branch --list feature`.
    var list_r = try git.exec(&.{ "branch", "--list", "feature" });
    defer list_r.deinit();
    try std.testing.expect(list_r.success());
    try std.testing.expectEqualStrings("", list_r.stdout);
}

// ── AC4: deleteBranch on a non-existent branch returns non-zero ───────────────

test "AC4: deleteBranch on a non-existent branch returns non-zero ExecResult" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    // Must not propagate a Zig error — only an ExecResult with non-zero exit.
    var result = try git.deleteBranch("branch-does-not-exist");
    defer result.deinit();

    try std.testing.expect(result.exit_code != 0);
    try std.testing.expect(!result.success());
    // git emits a diagnostic message to stderr for unknown branches.
    try std.testing.expect(result.stderr.len > 0);
}

// ── E1: no-op merge of a branch identical to HEAD ────────────────────────────

test "E1: merge of a branch identical to HEAD (no new commits) succeeds" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    // feature branches from main with no additional commits — identical to main.
    var branch_r = try git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), branch_r.exit_code);

    // Back to main; feature tip == main tip.
    var co_main = try git.checkout("main");
    defer co_main.deinit();
    try std.testing.expectEqual(@as(u8, 0), co_main.exit_code);

    // git merge --no-ff on an identical branch still creates a merge commit.
    var merge_r = try git.merge("feature");
    defer merge_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), merge_r.exit_code);
    try std.testing.expect(merge_r.success());

    // Working tree must be clean after the merge.
    const clean = try git.statusClean();
    try std.testing.expect(clean);
}

// ── E2: deleteBranch on the currently checked-out branch fails ───────────────

test "E2: deleteBranch on the currently checked-out branch fails" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    // Create and stay on feature.
    var branch_r = try git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), branch_r.exit_code);

    // Confirm we are on feature.
    var cur = try git.currentBranch();
    defer cur.deinit();
    try std.testing.expect(std.mem.startsWith(u8, cur.stdout, "feature"));

    // git refuses to delete the currently active branch even with -D.
    // Must not propagate a Zig error — only a non-zero ExecResult.
    var del_r = try git.deleteBranch("feature");
    defer del_r.deinit();
    try std.testing.expect(del_r.exit_code != 0);
    try std.testing.expect(!del_r.success());
}

// ── E3: merge output is non-empty on success ─────────────────────────────────

test "E3: merge stdout or stderr is non-empty on a successful three-way merge" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    // Same diverged setup as AC2.
    var branch_r = try git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), branch_r.exit_code);

    const feat_file = try std.fmt.allocPrint(alloc, "{s}/feature.txt", .{repo_path});
    defer alloc.free(feat_file);
    try std.fs.cwd().writeFile(.{ .sub_path = feat_file, .data = "feature content\n" });

    var add_feat = try git.addAll();
    defer add_feat.deinit();
    try std.testing.expect(add_feat.success());

    var commit_feat = try git.commit("add feature.txt");
    defer commit_feat.deinit();
    try std.testing.expect(commit_feat.success());

    var co_main = try git.checkout("main");
    defer co_main.deinit();
    try std.testing.expectEqual(@as(u8, 0), co_main.exit_code);

    const main2_file = try std.fmt.allocPrint(alloc, "{s}/main2.txt", .{repo_path});
    defer alloc.free(main2_file);
    try std.fs.cwd().writeFile(.{ .sub_path = main2_file, .data = "main2 content\n" });

    var add_main2 = try git.addAll();
    defer add_main2.deinit();
    try std.testing.expect(add_main2.success());

    var commit_main2 = try git.commit("advance main");
    defer commit_main2.deinit();
    try std.testing.expect(commit_main2.success());

    var merge_r = try git.merge("feature");
    defer merge_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), merge_r.exit_code);
    try std.testing.expect(merge_r.success());

    // E3: callers can safely inspect output; it must not be empty.
    try std.testing.expect(merge_r.stdout.len > 0 or merge_r.stderr.len > 0);
}
