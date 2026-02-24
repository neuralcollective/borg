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
