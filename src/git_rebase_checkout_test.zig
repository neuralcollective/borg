// Tests for git.zig checkout() and rebase() error propagation.
//
// Verifies that:
//   - checkout() switches HEAD on success and returns a non-zero ExecResult
//     (never a Zig error) for a non-existent branch or a dirty working tree
//     that would be overwritten by the switch.
//   - rebase() exits 0 on a clean diverged branch, exits non-zero on a
//     conflict, and abortRebase() fully restores the repository state.
//
// Each test creates an isolated temporary git repository under /tmp and
// cleans up via defer.  The private makeTempRepo helper initialises a repo
// with one commit on `main`.
//
// To include in the build, git.zig must contain:
//   test { _ = @import("git_rebase_checkout_test.zig"); }

const std = @import("std");
const git_mod = @import("git.zig");
const Git = git_mod.Git;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn uniqueId() u64 {
    var buf: [8]u8 = undefined;
    std.crypto.random.bytes(&buf);
    return std.mem.readInt(u64, &buf, .little);
}

/// Creates a temporary directory, initialises a git repo with one commit on
/// `main` (a `seed.txt` file), and returns the absolute path.
///
/// Caller must clean up:
///   defer allocator.free(path);
///   defer std.fs.deleteTreeAbsolute(path) catch {};
fn makeTempRepo(allocator: std.mem.Allocator) ![]const u8 {
    const path = try std.fmt.allocPrint(
        allocator,
        "/tmp/borg-rc-repo-{x}",
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

// ── AC1: checkout switches HEAD to the target branch ─────────────────────────

test "AC1: checkout to existing branch switches HEAD" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    // Create a feature branch from main, then go back to main so we can
    // perform a clean checkout-to-feature.
    var branch_r = try git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), branch_r.exit_code);

    var co_main = try git.checkout("main");
    defer co_main.deinit();
    try std.testing.expectEqual(@as(u8, 0), co_main.exit_code);

    // Checkout feature — must succeed and update HEAD.
    var co_feat = try git.checkout("feature");
    defer co_feat.deinit();
    try std.testing.expectEqual(@as(u8, 0), co_feat.exit_code);
    try std.testing.expect(co_feat.success());

    var cur = try git.currentBranch();
    defer cur.deinit();
    try std.testing.expect(cur.success());
    try std.testing.expect(std.mem.startsWith(u8, cur.stdout, "feature"));
}

// ── AC2: checkout of a non-existent branch returns non-zero ──────────────────

test "AC2: checkout of non-existent branch returns non-zero ExecResult" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    // checkout() must not propagate a Zig error — it must return an ExecResult.
    var result = try git.checkout("branch-does-not-exist");
    defer result.deinit();

    try std.testing.expect(result.exit_code != 0);
    try std.testing.expect(!result.success());
    // git emits a diagnostic message to stderr for unknown refs.
    try std.testing.expect(result.stderr.len > 0);
}

// ── AC3: checkout with a dirty working tree that conflicts ────────────────────

test "AC3: checkout refused when local changes would be overwritten" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    // Commit conflict.txt on main.
    const conflict_file = try std.fmt.allocPrint(alloc, "{s}/conflict.txt", .{repo_path});
    defer alloc.free(conflict_file);
    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "main version\n" });
    var add_m = try git.addAll();
    defer add_m.deinit();
    var commit_m = try git.commit("main: add conflict.txt");
    defer commit_m.deinit();
    try std.testing.expect(commit_m.success());

    // Create feature and commit a different version of the same file.
    var branch_r = try git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expect(branch_r.success());

    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "feature version\n" });
    var add_f = try git.addAll();
    defer add_f.deinit();
    var commit_f = try git.commit("feature: modify conflict.txt");
    defer commit_f.deinit();
    try std.testing.expect(commit_f.success());

    // Switch back to main.
    var co_main = try git.checkout("main");
    defer co_main.deinit();
    try std.testing.expect(co_main.success());

    // Leave conflict.txt dirty (unstaged modification) — feature has a
    // different committed version, so git must refuse to overwrite it.
    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "locally modified\n" });

    var co_feat = try git.checkout("feature");
    defer co_feat.deinit();

    try std.testing.expect(co_feat.exit_code != 0);
    try std.testing.expect(!co_feat.success());
}

// ── AC4: clean rebase succeeds and places feature exactly one commit ahead ────

test "AC4: rebase of clean diverged branch onto main succeeds" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    // feature: add a file that main doesn't have.
    var branch_r = try git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expect(branch_r.success());

    const feat_file = try std.fmt.allocPrint(alloc, "{s}/feature.txt", .{repo_path});
    defer alloc.free(feat_file);
    try std.fs.cwd().writeFile(.{ .sub_path = feat_file, .data = "feature content\n" });
    var add_f = try git.addAll();
    defer add_f.deinit();
    var commit_f = try git.commit("add feature.txt");
    defer commit_f.deinit();
    try std.testing.expect(commit_f.success());

    // main: advance past the fork point with an unrelated commit.
    var co_main = try git.checkout("main");
    defer co_main.deinit();
    try std.testing.expect(co_main.success());

    const main2_file = try std.fmt.allocPrint(alloc, "{s}/main2.txt", .{repo_path});
    defer alloc.free(main2_file);
    try std.fs.cwd().writeFile(.{ .sub_path = main2_file, .data = "main2 content\n" });
    var add_m2 = try git.addAll();
    defer add_m2.deinit();
    var commit_m2 = try git.commit("advance main");
    defer commit_m2.deinit();
    try std.testing.expect(commit_m2.success());

    // Rebase feature onto the now-advanced main.
    var co_feat = try git.checkout("feature");
    defer co_feat.deinit();
    try std.testing.expect(co_feat.success());

    var rebase_r = try git.rebase("main");
    defer rebase_r.deinit();

    try std.testing.expectEqual(@as(u8, 0), rebase_r.exit_code);
    try std.testing.expect(rebase_r.success());

    // Working tree must be clean after a successful rebase.
    const clean = try git.statusClean();
    try std.testing.expect(clean);

    // feature must be exactly one commit ahead of main.
    var log_r = try git.logOneline("main..HEAD");
    defer log_r.deinit();
    try std.testing.expect(log_r.success());
    var line_count: usize = 0;
    for (log_r.stdout) |c| {
        if (c == '\n') line_count += 1;
    }
    try std.testing.expectEqual(@as(usize, 1), line_count);
}

// ── AC5: conflicting rebase returns non-zero exit code ───────────────────────

test "AC5: rebase with conflict returns non-zero exit code and non-empty output" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    const conflict_file = try std.fmt.allocPrint(alloc, "{s}/conflict.txt", .{repo_path});
    defer alloc.free(conflict_file);

    // feature: write conflict.txt.
    var branch_r = try git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expect(branch_r.success());

    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "feature version\n" });
    var add_f = try git.addAll();
    defer add_f.deinit();
    var commit_f = try git.commit("feature: write conflict.txt");
    defer commit_f.deinit();
    try std.testing.expect(commit_f.success());

    // main: write an incompatible version of the same file.
    var co_main = try git.checkout("main");
    defer co_main.deinit();
    try std.testing.expect(co_main.success());

    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "main version\n" });
    var add_m = try git.addAll();
    defer add_m.deinit();
    var commit_m = try git.commit("main: write conflict.txt");
    defer commit_m.deinit();
    try std.testing.expect(commit_m.success());

    var co_feat = try git.checkout("feature");
    defer co_feat.deinit();
    try std.testing.expect(co_feat.success());

    // Rebase must fail.
    var rebase_r = try git.rebase("main");
    defer rebase_r.deinit();

    try std.testing.expect(rebase_r.exit_code != 0);
    try std.testing.expect(!rebase_r.success());
    // git must emit output describing the conflict.
    try std.testing.expect(rebase_r.stdout.len > 0 or rebase_r.stderr.len > 0);

    // Abort so the tmp dir can be removed cleanly.
    var abort_r = try git.abortRebase();
    defer abort_r.deinit();
}

// ── AC6: abortRebase restores HEAD and clean working tree ────────────────────

test "AC6: abortRebase after conflict restores HEAD to feature and cleans tree" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    const conflict_file = try std.fmt.allocPrint(alloc, "{s}/conflict.txt", .{repo_path});
    defer alloc.free(conflict_file);

    // Same conflicting setup as AC5.
    var branch_r = try git.createBranch("feature", "main");
    defer branch_r.deinit();

    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "feature version\n" });
    var add_f = try git.addAll();
    defer add_f.deinit();
    var commit_f = try git.commit("feature: write conflict.txt");
    defer commit_f.deinit();

    var co_main = try git.checkout("main");
    defer co_main.deinit();
    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "main version\n" });
    var add_m = try git.addAll();
    defer add_m.deinit();
    var commit_m = try git.commit("main: write conflict.txt");
    defer commit_m.deinit();

    var co_feat = try git.checkout("feature");
    defer co_feat.deinit();

    // Trigger the conflict.
    var rebase_r = try git.rebase("main");
    defer rebase_r.deinit();
    try std.testing.expect(rebase_r.exit_code != 0);

    // abortRebase must succeed.
    var abort_r = try git.abortRebase();
    defer abort_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), abort_r.exit_code);
    try std.testing.expect(abort_r.success());

    // Working tree must be clean (.git/rebase-merge removed by git).
    const clean = try git.statusClean();
    try std.testing.expect(clean);

    // HEAD must be back on feature.
    var cur = try git.currentBranch();
    defer cur.deinit();
    try std.testing.expect(std.mem.startsWith(u8, cur.stdout, "feature"));
}

// ── E1: checkout of a commit SHA produces detached HEAD ──────────────────────

test "E1: checkout of commit SHA produces detached HEAD and exits 0" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    // Copy the HEAD SHA before any checkout so the buffer outlives rev_r.
    const sha: [40]u8 = blk: {
        var rev_r = try git.exec(&.{ "rev-parse", "HEAD" });
        defer rev_r.deinit();
        try std.testing.expect(rev_r.success());
        try std.testing.expect(rev_r.stdout.len >= 40);
        var h: [40]u8 = undefined;
        @memcpy(&h, rev_r.stdout[0..40]);
        break :blk h;
    };

    // Checkout the explicit SHA — detached HEAD is valid.
    var co_r = try git.checkout(&sha);
    defer co_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), co_r.exit_code);
    try std.testing.expect(co_r.success());

    // rev-parse --abbrev-ref HEAD returns "HEAD" when detached.
    var cur = try git.currentBranch();
    defer cur.deinit();
    try std.testing.expect(std.mem.startsWith(u8, cur.stdout, "HEAD"));
}

// ── E2: checkout to the current branch is a no-op ────────────────────────────

test "E2: checkout to the current branch is a no-op and exits 0" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    // Already on main — checking it out again must succeed without error.
    var co_r = try git.checkout("main");
    defer co_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), co_r.exit_code);
    try std.testing.expect(co_r.success());

    // HEAD must still be main.
    var cur = try git.currentBranch();
    defer cur.deinit();
    try std.testing.expect(std.mem.startsWith(u8, cur.stdout, "main"));
}

// ── E4: failed checkout leaves no partial git state ──────────────────────────

test "E4: failed checkout leaves no .git/MERGE_HEAD or .git/rebase-merge" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    // Attempt to checkout a branch that does not exist.
    var co_r = try git.checkout("no-such-branch");
    defer co_r.deinit();
    try std.testing.expect(co_r.exit_code != 0);

    // .git/MERGE_HEAD must not have been created.
    const merge_head = try std.fmt.allocPrint(alloc, "{s}/.git/MERGE_HEAD", .{repo_path});
    defer alloc.free(merge_head);
    try std.testing.expectError(error.FileNotFound, std.fs.accessAbsolute(merge_head, .{}));

    // .git/rebase-merge must not have been created.
    const rebase_dir = try std.fmt.allocPrint(alloc, "{s}/.git/rebase-merge", .{repo_path});
    defer alloc.free(rebase_dir);
    try std.testing.expectError(error.FileNotFound, std.fs.accessAbsolute(rebase_dir, .{}));

    // Working tree must still be clean.
    const clean = try git.statusClean();
    try std.testing.expect(clean);
}

// ── E5: rebase onto a non-existent ref returns non-zero ──────────────────────

test "E5: rebase onto non-existent ref returns non-zero exit code" {
    const alloc = std.testing.allocator;

    const repo_path = try makeTempRepo(alloc);
    defer alloc.free(repo_path);
    defer std.fs.deleteTreeAbsolute(repo_path) catch {};

    var git = Git.init(alloc, repo_path);

    // Be on a valid branch so the failure is purely from the bad target ref.
    var branch_r = try git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expect(branch_r.success());

    var rebase_r = try git.rebase("origin/nonexistent");
    defer rebase_r.deinit();

    try std.testing.expect(rebase_r.exit_code != 0);
    try std.testing.expect(!rebase_r.success());
}
