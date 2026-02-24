const std = @import("std");
const git_mod = @import("git.zig");
pub const ExecResult = git_mod.ExecResult;

pub const Gt = struct {
    allocator: std.mem.Allocator,
    repo_path: []const u8,

    pub fn init(allocator: std.mem.Allocator, repo_path: []const u8) Gt {
        return .{ .allocator = allocator, .repo_path = repo_path };
    }

    pub fn exec(self: *Gt, args: []const []const u8) !ExecResult {
        var argv = std.ArrayList([]const u8).init(self.allocator);
        defer argv.deinit();
        try argv.appendSlice(&.{ "gt", "--no-interactive" });
        try argv.appendSlice(args);

        var child = std.process.Child.init(argv.items, self.allocator);
        child.cwd = self.repo_path;
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

    pub fn repoInit(self: *Gt, trunk: []const u8) !ExecResult {
        return self.exec(&.{ "repo", "init", "--trunk", trunk });
    }

    pub fn repoSync(self: *Gt) !ExecResult {
        return self.exec(&.{ "repo", "sync", "--force" });
    }

    pub fn create(self: *Gt, name: []const u8, message: []const u8) !ExecResult {
        return self.exec(&.{ "create", name, "-m", message, "--all" });
    }

    pub fn restack(self: *Gt) !ExecResult {
        return self.exec(&.{"restack"});
    }

    pub fn submit(self: *Gt) !ExecResult {
        return self.exec(&.{ "submit", "--no-edit", "--publish" });
    }

    pub fn submitStack(self: *Gt) !ExecResult {
        return self.exec(&.{ "submit", "--stack", "--no-edit", "--publish" });
    }

    pub fn checkout(self: *Gt, branch: []const u8) !ExecResult {
        return self.exec(&.{ "checkout", branch });
    }

    pub fn logShort(self: *Gt) !ExecResult {
        return self.exec(&.{ "log", "short" });
    }

    pub fn branchTrack(self: *Gt, branch: []const u8, parent: []const u8) !ExecResult {
        return self.exec(&.{ "branch", "track", branch, "--parent", parent });
    }

    pub fn branchDelete(self: *Gt, name: []const u8) !ExecResult {
        return self.exec(&.{ "branch", "delete", "--force", name });
    }
};

// ── Tests ──────────────────────────────────────────────────────────────

test "Gt struct initializes correctly" {
    const gt = Gt.init(std.testing.allocator, "/tmp/test-repo");
    try std.testing.expectEqualStrings("/tmp/test-repo", gt.repo_path);
}
