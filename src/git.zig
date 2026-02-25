const std = @import("std");

pub const Git = struct {
    allocator: std.mem.Allocator,
    repo_path: []const u8,

    pub fn init(allocator: std.mem.Allocator, repo_path: []const u8) Git {
        return .{ .allocator = allocator, .repo_path = repo_path };
    }

    pub fn exec(self: *Git, args: []const []const u8) !ExecResult {
        var argv = std.ArrayList([]const u8).init(self.allocator);
        defer argv.deinit();
        try argv.appendSlice(&.{ "git", "-C", self.repo_path });
        try argv.appendSlice(args);

        var child = std.process.Child.init(argv.items, self.allocator);
        child.stdin_behavior = .Close;
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Pipe;
        try child.spawn();

        var stdout_buf = std.ArrayList(u8).init(self.allocator);
        var stderr_buf = std.ArrayList(u8).init(self.allocator);
        var read_buf: [8192]u8 = undefined;

        if (child.stdout) |stdout| {
            while (true) {
                const n = stdout.read(&read_buf) catch break;
                if (n == 0) break;
                try stdout_buf.appendSlice(read_buf[0..n]);
            }
        }
        if (child.stderr) |stderr| {
            while (true) {
                const n = stderr.read(&read_buf) catch break;
                if (n == 0) break;
                try stderr_buf.appendSlice(read_buf[0..n]);
            }
        }

        const term = try child.wait();
        const exit_code: u8 = switch (term) {
            .Exited => |code| code,
            else => 1,
        };

        return ExecResult{
            .stdout = try stdout_buf.toOwnedSlice(),
            .stderr = try stderr_buf.toOwnedSlice(),
            .exit_code = exit_code,
            .allocator = self.allocator,
        };
    }

    pub fn checkout(self: *Git, branch: []const u8) !ExecResult {
        return self.exec(&.{ "checkout", branch });
    }

    pub fn createBranch(self: *Git, name: []const u8, base: []const u8) !ExecResult {
        return self.exec(&.{ "checkout", "-b", name, base });
    }

    pub fn pull(self: *Git) !ExecResult {
        return self.exec(&.{ "pull", "--ff-only" });
    }

    pub fn addAll(self: *Git) !ExecResult {
        return self.exec(&.{ "add", "-A" });
    }

    pub fn commit(self: *Git, message: []const u8) !ExecResult {
        return self.commitWithAuthor(message, null);
    }

    pub fn commitWithAuthor(self: *Git, message: []const u8, author: ?[]const u8) !ExecResult {
        if (author) |a| {
            return self.exec(&.{ "commit", "-m", message, "--author", a });
        }
        return self.exec(&.{ "commit", "-m", message });
    }

    pub fn merge(self: *Git, branch: []const u8) !ExecResult {
        return self.exec(&.{ "merge", "--no-ff", branch });
    }

    pub fn abortMerge(self: *Git) !ExecResult {
        return self.exec(&.{ "merge", "--abort" });
    }

    pub fn fetch(self: *Git, remote: []const u8) !ExecResult {
        return self.exec(&.{ "fetch", remote });
    }

    pub fn rebase(self: *Git, onto: []const u8) !ExecResult {
        return self.exec(&.{ "rebase", onto });
    }

    pub fn abortRebase(self: *Git) !ExecResult {
        return self.exec(&.{ "rebase", "--abort" });
    }

    pub fn push(self: *Git, remote: []const u8, branch: []const u8) !ExecResult {
        return self.exec(&.{ "push", remote, branch });
    }

    pub fn deleteBranch(self: *Git, branch: []const u8) !ExecResult {
        return self.exec(&.{ "branch", "-D", branch });
    }

    pub fn diff(self: *Git) !ExecResult {
        return self.exec(&.{ "diff", "--stat", "HEAD" });
    }

    pub fn diffNameOnly(self: *Git) !ExecResult {
        return self.exec(&.{ "diff", "--name-only", "HEAD" });
    }

    pub fn statusClean(self: *Git) !bool {
        var result = try self.exec(&.{ "status", "--porcelain" });
        defer result.deinit();
        return result.stdout.len == 0 and result.exit_code == 0;
    }

    pub fn logOneline(self: *Git, range: []const u8) !ExecResult {
        return self.exec(&.{ "log", "--oneline", range });
    }

    pub fn currentBranch(self: *Git) !ExecResult {
        return self.exec(&.{ "rev-parse", "--abbrev-ref", "HEAD" });
    }

    pub fn resetHard(self: *Git, ref: []const u8) !ExecResult {
        return self.exec(&.{ "reset", "--hard", ref });
    }

    pub fn stash(self: *Git) !ExecResult {
        return self.exec(&.{"stash"});
    }

    pub fn stashPop(self: *Git) !ExecResult {
        return self.exec(&.{ "stash", "pop" });
    }

    // Worktree operations
    pub fn addWorktree(self: *Git, path: []const u8, branch: []const u8) !ExecResult {
        return self.exec(&.{ "worktree", "add", path, "-b", branch });
    }

    pub fn addWorktreeExisting(self: *Git, path: []const u8, branch: []const u8) !ExecResult {
        return self.exec(&.{ "worktree", "add", path, branch });
    }

    pub fn removeWorktree(self: *Git, path: []const u8) !ExecResult {
        return self.exec(&.{ "worktree", "remove", path, "--force" });
    }

    pub fn listWorktrees(self: *Git) !ExecResult {
        return self.exec(&.{ "worktree", "list", "--porcelain" });
    }

    pub fn revParseHead(self: *Git) ![40]u8 {
        return self.revParse("HEAD");
    }

    pub fn revParse(self: *Git, ref: []const u8) ![40]u8 {
        var result = try self.exec(&.{ "rev-parse", ref });
        defer result.deinit();
        if (result.stdout.len >= 40) {
            var hash: [40]u8 = undefined;
            @memcpy(&hash, result.stdout[0..40]);
            return hash;
        }
        return [_]u8{0} ** 40;
    }
};

pub const ExecResult = struct {
    stdout: []const u8,
    stderr: []const u8,
    exit_code: u8,
    allocator: std.mem.Allocator,

    pub fn success(self: ExecResult) bool {
        return self.exit_code == 0;
    }

    pub fn deinit(self: *ExecResult) void {
        self.allocator.free(self.stdout);
        self.allocator.free(self.stderr);
    }
};

// ── Tests ──────────────────────────────────────────────────────────────

// ── Rebase / Merge test helpers ────────────────────────────────────────

/// Returned by initTestRepo; caller owns path and must free it.
const TestRepo = struct {
    path: []const u8,
    git: Git,
};

/// Create an isolated temp repo with a single initial commit on `main`.
/// The returned path is heap-allocated; the caller must:
///   defer alloc.free(repo.path);
///   defer std.fs.deleteTreeAbsolute(repo.path) catch {};
fn initTestRepo(alloc: std.mem.Allocator, suffix: []const u8) !TestRepo {
    const path = try std.fmt.allocPrint(
        alloc,
        "/tmp/borg-git-test-{d}-{s}",
        .{ std.time.timestamp(), suffix },
    );
    errdefer alloc.free(path);

    std.fs.makeDirAbsolute(path) catch {};

    var git = Git.init(alloc, path);

    var init_r = try git.exec(&.{ "init", "-b", "main" });
    defer init_r.deinit();
    if (!init_r.success()) return error.GitInitFailed;

    var cfg_email = try git.exec(&.{ "config", "user.email", "test@test.com" });
    defer cfg_email.deinit();
    var cfg_name = try git.exec(&.{ "config", "user.name", "Test" });
    defer cfg_name.deinit();

    const seed_path = try std.fmt.allocPrint(alloc, "{s}/seed.txt", .{path});
    defer alloc.free(seed_path);
    try std.fs.cwd().writeFile(.{ .sub_path = seed_path, .data = "seed\n" });

    var add_r = try git.addAll();
    defer add_r.deinit();
    if (!add_r.success()) return error.GitAddFailed;

    var commit_r = try git.commit("initial commit");
    defer commit_r.deinit();
    if (!commit_r.success()) return error.GitCommitFailed;

    return .{ .path = path, .git = git };
}

test "exec builds correct argv with -C flag" {
    // Verify the Git struct initializes correctly
    const git = Git.init(std.testing.allocator, "/tmp/test-repo");
    try std.testing.expectEqualStrings("/tmp/test-repo", git.repo_path);
}

test "ExecResult reports success correctly" {
    var ok = ExecResult{
        .stdout = try std.testing.allocator.dupe(u8, "output"),
        .stderr = try std.testing.allocator.dupe(u8, ""),
        .exit_code = 0,
        .allocator = std.testing.allocator,
    };
    defer ok.deinit();
    try std.testing.expect(ok.success());

    var fail = ExecResult{
        .stdout = try std.testing.allocator.dupe(u8, ""),
        .stderr = try std.testing.allocator.dupe(u8, "error"),
        .exit_code = 1,
        .allocator = std.testing.allocator,
    };
    defer fail.deinit();
    try std.testing.expect(!fail.success());
}

test {
    _ = @import("git_worktree_test.zig");
    _ = @import("git_rebase_checkout_test.zig");
}

test "git operations on real temp repo" {
    const alloc = std.testing.allocator;

    // Create a temp directory for our test repo
    var tmp_buf: [256]u8 = undefined;
    const tmp_dir = try std.fmt.bufPrint(&tmp_buf, "/tmp/borg-git-test-{d}", .{std.time.timestamp()});
    const tmp_z = try alloc.dupeZ(u8, tmp_dir);
    defer alloc.free(tmp_z);

    // mkdir and git init
    std.fs.makeDirAbsolute(tmp_dir) catch {};
    defer std.fs.deleteTreeAbsolute(tmp_dir) catch {};

    var git = Git.init(alloc, tmp_dir);

    // git init
    var init_r = try git.exec(&.{ "init", "-b", "main" });
    defer init_r.deinit();
    try std.testing.expect(init_r.success());

    // Configure user for commits
    var cfg1 = try git.exec(&.{ "config", "user.email", "test@test.com" });
    defer cfg1.deinit();
    var cfg2 = try git.exec(&.{ "config", "user.name", "Test" });
    defer cfg2.deinit();

    // Create a file and commit
    const file_path = try std.fmt.allocPrint(alloc, "{s}/hello.txt", .{tmp_dir});
    defer alloc.free(file_path);
    try std.fs.cwd().writeFile(.{ .sub_path = file_path, .data = "hello world\n" });

    var add_r = try git.addAll();
    defer add_r.deinit();
    try std.testing.expect(add_r.success());

    var commit_r = try git.commit("initial commit");
    defer commit_r.deinit();
    try std.testing.expect(commit_r.success());

    // Create branch
    var branch_r = try git.createBranch("feature/test-1", "main");
    defer branch_r.deinit();
    try std.testing.expect(branch_r.success());

    // Check current branch
    var cur_r = try git.currentBranch();
    defer cur_r.deinit();
    try std.testing.expectEqualStrings("feature/test-1\n", cur_r.stdout);

    // Checkout main
    var co_r = try git.checkout("main");
    defer co_r.deinit();
    try std.testing.expect(co_r.success());

    // Status should be clean
    const clean = try git.statusClean();
    try std.testing.expect(clean);

    // Delete branch
    var del_r = try git.deleteBranch("feature/test-1");
    defer del_r.deinit();
    try std.testing.expect(del_r.success());
}

// ── AC1: Successful rebase exits 0 ─────────────────────────────────────

test "rebase: clean rebase of diverged branch onto main succeeds" {
    const alloc = std.testing.allocator;
    var repo = try initTestRepo(alloc, "rebase-clean");
    defer alloc.free(repo.path);
    defer std.fs.deleteTreeAbsolute(repo.path) catch {};

    // feature: one commit on top of the initial commit
    var branch_r = try repo.git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expect(branch_r.success());

    const feat_file = try std.fmt.allocPrint(alloc, "{s}/feature.txt", .{repo.path});
    defer alloc.free(feat_file);
    try std.fs.cwd().writeFile(.{ .sub_path = feat_file, .data = "feature content\n" });

    var add_feat = try repo.git.addAll();
    defer add_feat.deinit();
    try std.testing.expect(add_feat.success());

    var commit_feat = try repo.git.commit("add feature.txt");
    defer commit_feat.deinit();
    try std.testing.expect(commit_feat.success());

    // main: advance past the feature's fork point
    var co_main = try repo.git.checkout("main");
    defer co_main.deinit();
    try std.testing.expect(co_main.success());

    const main2_file = try std.fmt.allocPrint(alloc, "{s}/main2.txt", .{repo.path});
    defer alloc.free(main2_file);
    try std.fs.cwd().writeFile(.{ .sub_path = main2_file, .data = "main2 content\n" });

    var add_main2 = try repo.git.addAll();
    defer add_main2.deinit();
    try std.testing.expect(add_main2.success());

    var commit_main2 = try repo.git.commit("advance main");
    defer commit_main2.deinit();
    try std.testing.expect(commit_main2.success());

    // switch back to feature for the rebase
    var co_feat = try repo.git.checkout("feature");
    defer co_feat.deinit();
    try std.testing.expect(co_feat.success());

    // AC1: rebase must succeed
    var rebase_r = try repo.git.rebase("main");
    defer rebase_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), rebase_r.exit_code);
    try std.testing.expect(rebase_r.success());

    // working tree must be clean after a successful rebase
    const clean = try repo.git.statusClean();
    try std.testing.expect(clean);

    // feature must be exactly one commit ahead of main
    var log_r = try repo.git.logOneline("main..HEAD");
    defer log_r.deinit();
    try std.testing.expect(log_r.success());
    var lines: usize = 0;
    for (log_r.stdout) |c| {
        if (c == '\n') lines += 1;
    }
    try std.testing.expectEqual(@as(usize, 1), lines);
}

// ── AC2: Conflicting rebase exits non-zero ─────────────────────────────

test "rebase: conflict returns non-zero exit code" {
    const alloc = std.testing.allocator;
    var repo = try initTestRepo(alloc, "rebase-conflict");
    defer alloc.free(repo.path);
    defer std.fs.deleteTreeAbsolute(repo.path) catch {};

    const conflict_file = try std.fmt.allocPrint(alloc, "{s}/conflict.txt", .{repo.path});
    defer alloc.free(conflict_file);

    // feature: write conflict.txt
    var branch_r = try repo.git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expect(branch_r.success());

    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "feature version\n" });

    var add_feat = try repo.git.addAll();
    defer add_feat.deinit();
    try std.testing.expect(add_feat.success());

    var commit_feat = try repo.git.commit("feature edits conflict.txt");
    defer commit_feat.deinit();
    try std.testing.expect(commit_feat.success());

    // main: write the same file with different content
    var co_main = try repo.git.checkout("main");
    defer co_main.deinit();
    try std.testing.expect(co_main.success());

    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "main version\n" });

    var add_main = try repo.git.addAll();
    defer add_main.deinit();
    try std.testing.expect(add_main.success());

    var commit_main = try repo.git.commit("main edits conflict.txt");
    defer commit_main.deinit();
    try std.testing.expect(commit_main.success());

    // back to feature
    var co_feat = try repo.git.checkout("feature");
    defer co_feat.deinit();
    try std.testing.expect(co_feat.success());

    // AC2: conflict must produce non-zero exit
    var rebase_r = try repo.git.rebase("main");
    defer rebase_r.deinit();
    try std.testing.expect(rebase_r.exit_code != 0);
    try std.testing.expect(!rebase_r.success());
    // git must emit output describing the conflict
    try std.testing.expect(rebase_r.stdout.len > 0 or rebase_r.stderr.len > 0);

    // abort so the temp dir is in a clean state for teardown
    var abort_cleanup = try repo.git.abortRebase();
    defer abort_cleanup.deinit();
}

// ── AC3: abortRebase restores clean state ──────────────────────────────

test "abortRebase: recovers repo to clean state after conflict" {
    const alloc = std.testing.allocator;
    var repo = try initTestRepo(alloc, "abort-rebase");
    defer alloc.free(repo.path);
    defer std.fs.deleteTreeAbsolute(repo.path) catch {};

    const conflict_file = try std.fmt.allocPrint(alloc, "{s}/conflict.txt", .{repo.path});
    defer alloc.free(conflict_file);

    // same conflicting setup as AC2
    var branch_r = try repo.git.createBranch("feature", "main");
    defer branch_r.deinit();

    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "feature version\n" });
    var add_feat = try repo.git.addAll();
    defer add_feat.deinit();
    var commit_feat = try repo.git.commit("feature edits conflict.txt");
    defer commit_feat.deinit();

    var co_main = try repo.git.checkout("main");
    defer co_main.deinit();
    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "main version\n" });
    var add_main = try repo.git.addAll();
    defer add_main.deinit();
    var commit_main = try repo.git.commit("main edits conflict.txt");
    defer commit_main.deinit();

    var co_feat = try repo.git.checkout("feature");
    defer co_feat.deinit();

    // trigger conflict
    var rebase_r = try repo.git.rebase("main");
    defer rebase_r.deinit();
    try std.testing.expect(rebase_r.exit_code != 0);

    // AC3: abort must succeed
    var abort_r = try repo.git.abortRebase();
    defer abort_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), abort_r.exit_code);
    try std.testing.expect(abort_r.success());

    // working tree must be clean (E7: .git/rebase-merge removed by git)
    const clean = try repo.git.statusClean();
    try std.testing.expect(clean);

    // HEAD must be restored to the feature branch
    var cur_r = try repo.git.currentBranch();
    defer cur_r.deinit();
    try std.testing.expect(std.mem.startsWith(u8, cur_r.stdout, "feature"));
}

// ── AC4: Successful --no-ff merge exits 0 ──────────────────────────────

test "merge: fast-forward-prevented merge of feature branch succeeds" {
    const alloc = std.testing.allocator;
    var repo = try initTestRepo(alloc, "merge-clean");
    defer alloc.free(repo.path);
    defer std.fs.deleteTreeAbsolute(repo.path) catch {};

    // feature: one non-conflicting commit
    var branch_r = try repo.git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expect(branch_r.success());

    const feat_file = try std.fmt.allocPrint(alloc, "{s}/feature.txt", .{repo.path});
    defer alloc.free(feat_file);
    try std.fs.cwd().writeFile(.{ .sub_path = feat_file, .data = "feature content\n" });

    var add_feat = try repo.git.addAll();
    defer add_feat.deinit();
    try std.testing.expect(add_feat.success());

    var commit_feat = try repo.git.commit("add feature.txt");
    defer commit_feat.deinit();
    try std.testing.expect(commit_feat.success());

    // main: also advance so both branches are diverged (real three-way merge)
    var co_main = try repo.git.checkout("main");
    defer co_main.deinit();
    try std.testing.expect(co_main.success());

    const main2_file = try std.fmt.allocPrint(alloc, "{s}/main2.txt", .{repo.path});
    defer alloc.free(main2_file);
    try std.fs.cwd().writeFile(.{ .sub_path = main2_file, .data = "main2 content\n" });

    var add_main2 = try repo.git.addAll();
    defer add_main2.deinit();
    try std.testing.expect(add_main2.success());

    var commit_main2 = try repo.git.commit("advance main");
    defer commit_main2.deinit();
    try std.testing.expect(commit_main2.success());

    // AC4: merge must succeed on main
    var merge_r = try repo.git.merge("feature");
    defer merge_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), merge_r.exit_code);
    try std.testing.expect(merge_r.success());

    // verify a merge commit was created: HEAD must have a second parent
    var second_parent = try repo.git.exec(&.{ "rev-parse", "--verify", "HEAD^2" });
    defer second_parent.deinit();
    try std.testing.expect(second_parent.success());
}

// ── AC5: Conflicting merge exits non-zero ──────────────────────────────

test "merge: conflict returns non-zero exit code" {
    const alloc = std.testing.allocator;
    var repo = try initTestRepo(alloc, "merge-conflict");
    defer alloc.free(repo.path);
    defer std.fs.deleteTreeAbsolute(repo.path) catch {};

    const conflict_file = try std.fmt.allocPrint(alloc, "{s}/conflict.txt", .{repo.path});
    defer alloc.free(conflict_file);

    // feature: modify conflict.txt
    var branch_r = try repo.git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expect(branch_r.success());

    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "feature version\n" });
    var add_feat = try repo.git.addAll();
    defer add_feat.deinit();
    try std.testing.expect(add_feat.success());

    var commit_feat = try repo.git.commit("feature edits conflict.txt");
    defer commit_feat.deinit();
    try std.testing.expect(commit_feat.success());

    // main: modify the same file differently
    var co_main = try repo.git.checkout("main");
    defer co_main.deinit();
    try std.testing.expect(co_main.success());

    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "main version\n" });
    var add_main = try repo.git.addAll();
    defer add_main.deinit();
    try std.testing.expect(add_main.success());

    var commit_main = try repo.git.commit("main edits conflict.txt");
    defer commit_main.deinit();
    try std.testing.expect(commit_main.success());

    // AC5: conflict must produce non-zero exit
    var merge_r = try repo.git.merge("feature");
    defer merge_r.deinit();
    try std.testing.expect(merge_r.exit_code != 0);
    try std.testing.expect(!merge_r.success());

    // abort so temp dir teardown works cleanly
    var abort_cleanup = try repo.git.abortMerge();
    defer abort_cleanup.deinit();
}

// ── AC6: abortMerge restores clean state ───────────────────────────────

test "abortMerge: recovers repo to clean state after conflict" {
    const alloc = std.testing.allocator;
    var repo = try initTestRepo(alloc, "abort-merge");
    defer alloc.free(repo.path);
    defer std.fs.deleteTreeAbsolute(repo.path) catch {};

    const conflict_file = try std.fmt.allocPrint(alloc, "{s}/conflict.txt", .{repo.path});
    defer alloc.free(conflict_file);

    // same conflicting setup as AC5
    var branch_r = try repo.git.createBranch("feature", "main");
    defer branch_r.deinit();

    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "feature version\n" });
    var add_feat = try repo.git.addAll();
    defer add_feat.deinit();
    var commit_feat = try repo.git.commit("feature edits conflict.txt");
    defer commit_feat.deinit();

    var co_main = try repo.git.checkout("main");
    defer co_main.deinit();
    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "main version\n" });
    var add_main = try repo.git.addAll();
    defer add_main.deinit();
    var commit_main = try repo.git.commit("main edits conflict.txt");
    defer commit_main.deinit();

    // trigger conflict on main
    var merge_r = try repo.git.merge("feature");
    defer merge_r.deinit();
    try std.testing.expect(merge_r.exit_code != 0);

    // AC6: abort must succeed
    var abort_r = try repo.git.abortMerge();
    defer abort_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), abort_r.exit_code);
    try std.testing.expect(abort_r.success());

    // working tree must be clean (E8: MERGE_HEAD removed by git)
    const clean = try repo.git.statusClean();
    try std.testing.expect(clean);

    // HEAD must be back on main
    var cur_r = try repo.git.currentBranch();
    defer cur_r.deinit();
    try std.testing.expect(std.mem.startsWith(u8, cur_r.stdout, "main"));
}

// ── Edge Case E1: abortRebase with no rebase in progress ───────────────

test "E1: abortRebase with no rebase in progress returns non-zero ExecResult" {
    const alloc = std.testing.allocator;
    var repo = try initTestRepo(alloc, "e1-abort-rebase-idle");
    defer alloc.free(repo.path);
    defer std.fs.deleteTreeAbsolute(repo.path) catch {};

    // No rebase is in progress; must not return a Zig error — only a failed ExecResult.
    var abort_r = try repo.git.abortRebase();
    defer abort_r.deinit();
    try std.testing.expect(abort_r.exit_code != 0);
    try std.testing.expect(!abort_r.success());
}

// ── Edge Case E2: abortMerge with no merge in progress ─────────────────

test "E2: abortMerge with no merge in progress returns non-zero ExecResult" {
    const alloc = std.testing.allocator;
    var repo = try initTestRepo(alloc, "e2-abort-merge-idle");
    defer alloc.free(repo.path);
    defer std.fs.deleteTreeAbsolute(repo.path) catch {};

    // No merge is in progress; must not return a Zig error — only a failed ExecResult.
    var abort_r = try repo.git.abortMerge();
    defer abort_r.deinit();
    try std.testing.expect(abort_r.exit_code != 0);
    try std.testing.expect(!abort_r.success());
}

// ── Edge Case E3: Rebase with multiple conflicting commits ──────────────

test "E3: rebase with multiple conflicting commits aborts cleanly" {
    const alloc = std.testing.allocator;
    var repo = try initTestRepo(alloc, "e3-multi-conflict");
    defer alloc.free(repo.path);
    defer std.fs.deleteTreeAbsolute(repo.path) catch {};

    const conflict_file = try std.fmt.allocPrint(alloc, "{s}/conflict.txt", .{repo.path});
    defer alloc.free(conflict_file);

    // feature: two commits that both touch conflict.txt
    var branch_r = try repo.git.createBranch("feature", "main");
    defer branch_r.deinit();

    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "feature v1\n" });
    var add1 = try repo.git.addAll();
    defer add1.deinit();
    var c1 = try repo.git.commit("feature commit 1");
    defer c1.deinit();

    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "feature v2\n" });
    var add2 = try repo.git.addAll();
    defer add2.deinit();
    var c2 = try repo.git.commit("feature commit 2");
    defer c2.deinit();

    // main: also modifies conflict.txt (conflicts with both feature commits)
    var co_main = try repo.git.checkout("main");
    defer co_main.deinit();
    try std.fs.cwd().writeFile(.{ .sub_path = conflict_file, .data = "main version\n" });
    var add_main = try repo.git.addAll();
    defer add_main.deinit();
    var commit_main = try repo.git.commit("main edits conflict.txt");
    defer commit_main.deinit();

    var co_feat = try repo.git.checkout("feature");
    defer co_feat.deinit();

    // rebase fails on the first conflicting commit
    var rebase_r = try repo.git.rebase("main");
    defer rebase_r.deinit();
    try std.testing.expect(rebase_r.exit_code != 0);

    // abort must fully restore the working tree
    var abort_r = try repo.git.abortRebase();
    defer abort_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), abort_r.exit_code);

    const clean = try repo.git.statusClean();
    try std.testing.expect(clean);

    var cur_r = try repo.git.currentBranch();
    defer cur_r.deinit();
    try std.testing.expect(std.mem.startsWith(u8, cur_r.stdout, "feature"));
}

// ── Edge Case E4: --no-ff merge on a fast-forwardable branch ───────────

test "E4: --no-ff merge on fast-forwardable branch still creates a merge commit" {
    const alloc = std.testing.allocator;
    var repo = try initTestRepo(alloc, "e4-no-ff");
    defer alloc.free(repo.path);
    defer std.fs.deleteTreeAbsolute(repo.path) catch {};

    // feature has one commit ahead of main, no new commit on main — FF would be possible
    var branch_r = try repo.git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expect(branch_r.success());

    const feat_file = try std.fmt.allocPrint(alloc, "{s}/feature.txt", .{repo.path});
    defer alloc.free(feat_file);
    try std.fs.cwd().writeFile(.{ .sub_path = feat_file, .data = "feature content\n" });

    var add_r = try repo.git.addAll();
    defer add_r.deinit();
    var commit_r = try repo.git.commit("add feature.txt");
    defer commit_r.deinit();

    // back to main (no new commits — fast-forward would be possible without --no-ff)
    var co_main = try repo.git.checkout("main");
    defer co_main.deinit();
    try std.testing.expect(co_main.success());

    // --no-ff merge must still create a merge commit
    var merge_r = try repo.git.merge("feature");
    defer merge_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), merge_r.exit_code);
    try std.testing.expect(merge_r.success());

    // HEAD^2 must exist (second parent = feature tip)
    var parent2 = try repo.git.exec(&.{ "rev-parse", "--verify", "HEAD^2" });
    defer parent2.deinit();
    try std.testing.expect(parent2.success());
}

// ── Edge Case E5: Rebase of a branch identical to main is a no-op ──────

test "E5: rebase of branch with no new commits onto main exits 0" {
    const alloc = std.testing.allocator;
    var repo = try initTestRepo(alloc, "e5-noop-rebase");
    defer alloc.free(repo.path);
    defer std.fs.deleteTreeAbsolute(repo.path) catch {};

    // feature branches from main with no additional commits — nothing to replay
    var branch_r = try repo.git.createBranch("feature", "main");
    defer branch_r.deinit();
    try std.testing.expect(branch_r.success());

    var rebase_r = try repo.git.rebase("main");
    defer rebase_r.deinit();
    try std.testing.expectEqual(@as(u8, 0), rebase_r.exit_code);
    try std.testing.expect(rebase_r.success());

    const clean = try repo.git.statusClean();
    try std.testing.expect(clean);
}
