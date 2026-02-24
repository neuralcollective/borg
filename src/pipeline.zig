const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;
const docker_mod = @import("docker.zig");
const Docker = docker_mod.Docker;
const tg_mod = @import("telegram.zig");
const Telegram = tg_mod.Telegram;
const git_mod = @import("git.zig");
const Git = git_mod.Git;
const json_mod = @import("json.zig");
const agent_mod = @import("agent.zig");
const Config = @import("config.zig").Config;

const TICK_INTERVAL_S = 30;
const AGENT_TIMEOUT_S = 600;
const MAX_BACKLOG_SIZE = 5;
const SEED_COOLDOWN_S = 3600; // Min 1h between seed attempts

pub const AgentPersona = enum {
    manager,
    qa,
    worker,
};

pub const Pipeline = struct {
    allocator: std.mem.Allocator,
    db: *Db,
    docker: *Docker,
    tg: *Telegram,
    config: *Config,
    running: std.atomic.Value(bool),
    update_ready: std.atomic.Value(bool),
    last_release_ts: i64,
    last_seed_ts: i64,
    startup_head: [40]u8,

    pub fn init(
        allocator: std.mem.Allocator,
        db: *Db,
        docker: *Docker,
        tg: *Telegram,
        config: *Config,
    ) Pipeline {
        var git = Git.init(allocator, config.pipeline_repo);
        const head = git.revParseHead() catch [_]u8{0} ** 40;

        return .{
            .allocator = allocator,
            .db = db,
            .docker = docker,
            .tg = tg,
            .config = config,
            .running = std.atomic.Value(bool).init(true),
            .update_ready = std.atomic.Value(bool).init(false),
            .last_release_ts = std.time.timestamp(),
            .last_seed_ts = 0,
            .startup_head = head,
        };
    }

    pub fn run(self: *Pipeline) void {
        std.log.info("Pipeline thread started for repo: {s}", .{self.config.pipeline_repo});

        while (self.running.load(.acquire)) {
            self.tick() catch |err| {
                std.log.err("Pipeline tick error: {}", .{err});
            };

            self.checkReleaseTrain() catch |err| {
                std.log.err("Release train error: {}", .{err});
            };

            std.time.sleep(TICK_INTERVAL_S * std.time.ns_per_s);
        }

        std.log.info("Pipeline thread stopped", .{});
    }

    pub fn stop(self: *Pipeline) void {
        self.running.store(false, .release);
    }

    fn tick(self: *Pipeline) !void {
        const task = (try self.db.getNextPipelineTask(self.allocator)) orelse {
            // Nothing to do - look for more work
            try self.seedIfIdle();
            return;
        };

        std.log.info("Pipeline processing task #{d} [{s}]: {s}", .{ task.id, task.status, task.title });

        if (std.mem.eql(u8, task.status, "backlog")) {
            try self.setupBranch(task);
        } else if (std.mem.eql(u8, task.status, "spec")) {
            try self.runSpecPhase(task);
        } else if (std.mem.eql(u8, task.status, "qa")) {
            try self.runQaPhase(task);
        } else if (std.mem.eql(u8, task.status, "impl") or std.mem.eql(u8, task.status, "retry")) {
            try self.runImplPhase(task);
        } else if (std.mem.eql(u8, task.status, "rebase")) {
            try self.runRebasePhase(task);
        }
    }

    fn seedIfIdle(self: *Pipeline) !void {
        const now = std.time.timestamp();
        const cooldown: i64 = if (self.config.continuous_mode) 60 else SEED_COOLDOWN_S;
        if (now - self.last_seed_ts < cooldown) return;

        // Don't seed if there are already active tasks
        const active = try self.db.getActivePipelineTaskCount();
        if (active >= MAX_BACKLOG_SIZE) return;

        std.log.info("Pipeline idle, scanning repo for improvements...", .{});
        self.last_seed_ts = now;

        self.config.refreshOAuthToken();

        // Run a seeder agent against the repo to discover refactoring tasks
        var prompt_buf = std.ArrayList(u8).init(self.allocator);
        defer prompt_buf.deinit();
        const w = prompt_buf.writer();

        try w.writeAll(
            \\Analyze this codebase and identify 1-3 concrete, small improvements.
            \\Focus on refactoring and quality - NOT new features.
            \\
            \\Good tasks: extract duplicated code, improve error handling for a specific
            \\function, simplify a complex conditional, add missing test coverage for an
            \\edge case, fix a subtle bug, improve a variable name for clarity.
            \\
            \\Bad tasks: add new features, rewrite entire modules, add documentation,
            \\change the architecture, add dependencies.
            \\
            \\For each improvement, output EXACTLY this format (one per task):
            \\
            \\TASK_START
            \\TITLE: <short imperative title, max 80 chars>
            \\DESCRIPTION: <2-4 sentences explaining what to change and why>
            \\TASK_END
            \\
            \\Output ONLY the task blocks above. No other text.
        );

        const result = self.spawnAgent(.manager, prompt_buf.items, self.config.pipeline_repo) catch |err| {
            std.log.err("Seed agent failed: {}", .{err});
            return;
        };
        defer self.allocator.free(result.output);
        defer if (result.new_session_id) |sid| self.allocator.free(sid);

        self.db.storeTaskOutput(0, "seed", result.output, 0) catch {};

        // Parse TASK_START/TASK_END blocks from output
        var created: u32 = 0;
        var remaining = result.output;
        while (std.mem.indexOf(u8, remaining, "TASK_START")) |start_pos| {
            remaining = remaining[start_pos + "TASK_START".len ..];
            const end_pos = std.mem.indexOf(u8, remaining, "TASK_END") orelse break;
            const block = std.mem.trim(u8, remaining[0..end_pos], &[_]u8{ ' ', '\t', '\n', '\r' });
            remaining = remaining[end_pos + "TASK_END".len ..];

            // Extract TITLE: and DESCRIPTION: lines
            var title: []const u8 = "";
            var desc_start: usize = 0;
            const desc_end: usize = block.len;
            var lines = std.mem.splitScalar(u8, block, '\n');
            while (lines.next()) |line| {
                const trimmed = std.mem.trim(u8, line, &[_]u8{ ' ', '\t', '\r' });
                if (std.mem.startsWith(u8, trimmed, "TITLE:")) {
                    title = std.mem.trim(u8, trimmed["TITLE:".len..], &[_]u8{ ' ', '\t' });
                } else if (std.mem.startsWith(u8, trimmed, "DESCRIPTION:")) {
                    // Everything from DESCRIPTION: to end of block
                    desc_start = @intFromPtr(trimmed.ptr) - @intFromPtr(block.ptr) + "DESCRIPTION:".len;
                    break;
                }
            }

            if (title.len == 0) continue;
            const description = if (desc_start < desc_end)
                std.mem.trim(u8, block[desc_start..desc_end], &[_]u8{ ' ', '\t', '\n', '\r' })
            else
                title;

            _ = self.db.createPipelineTask(
                title,
                description,
                self.config.pipeline_repo,
                "seeder",
                self.config.pipeline_admin_chat,
            ) catch |err| {
                std.log.err("Failed to create seeded task: {}", .{err});
                continue;
            };

            created += 1;
            if (active + created >= MAX_BACKLOG_SIZE) break;
        }

        if (created > 0) {
            std.log.info("Seeded {d} new task(s) from codebase analysis", .{created});
            self.notify(self.config.pipeline_admin_chat, std.fmt.allocPrint(self.allocator, "Pipeline seeded {d} new task(s) from codebase analysis", .{created}) catch return);
        } else {
            std.log.info("Seed scan found no actionable improvements", .{});
        }
    }

    fn worktreePath(self: *Pipeline, task_id: i64) ![]const u8 {
        return std.fmt.allocPrint(self.allocator, "{s}/.worktrees/task-{d}", .{ self.config.pipeline_repo, task_id });
    }

    fn setupBranch(self: *Pipeline, task: db_mod.PipelineTask) !void {
        var git = Git.init(self.allocator, self.config.pipeline_repo);

        // Pull latest main
        var pull = try git.exec(&.{ "fetch", "origin", "main" });
        defer pull.deinit();

        // Create worktree with new branch from main
        var branch_buf: [128]u8 = undefined;
        const branch = try std.fmt.bufPrint(&branch_buf, "feature/task-{d}", .{task.id});

        // Ensure .worktrees directory exists
        const wt_dir = try std.fmt.allocPrint(self.allocator, "{s}/.worktrees", .{self.config.pipeline_repo});
        defer self.allocator.free(wt_dir);
        std.fs.makeDirAbsolute(wt_dir) catch {};

        const wt_path = try self.worktreePath(task.id);
        defer self.allocator.free(wt_path);

        var wt = try git.exec(&.{ "worktree", "add", wt_path, "-b", branch, "origin/main" });
        defer wt.deinit();
        if (!wt.success()) {
            std.log.err("git worktree add failed: {s}", .{wt.stderr});
            try self.db.updateTaskStatus(task.id, "failed");
            try self.db.updateTaskError(task.id, wt.stderr);
            return;
        }

        try self.db.updateTaskBranch(task.id, branch);
        try self.db.updateTaskStatus(task.id, "spec");
        std.log.info("Created worktree {s} (branch {s}) for task #{d}", .{ wt_path, branch, task.id });
    }

    fn cleanupWorktree(self: *Pipeline, task: db_mod.PipelineTask) void {
        const wt_path = self.worktreePath(task.id) catch return;
        defer self.allocator.free(wt_path);
        var git = Git.init(self.allocator, self.config.pipeline_repo);
        var rm = git.removeWorktree(wt_path) catch return;
        defer rm.deinit();
        if (rm.success()) {
            std.log.info("Cleaned up worktree for task #{d}", .{task.id});
        }
    }

    fn runSpecPhase(self: *Pipeline, task: db_mod.PipelineTask) !void {
        const wt_path = try self.worktreePath(task.id);
        defer self.allocator.free(wt_path);
        var wt_git = Git.init(self.allocator, wt_path);

        // Get file listing for context
        var ls = try wt_git.exec(&.{ "ls-files", "--full-name" });
        defer ls.deinit();

        var prompt_buf = std.ArrayList(u8).init(self.allocator);
        defer prompt_buf.deinit();
        const w = prompt_buf.writer();

        try w.print("Task #{d}: {s}\n\n", .{ task.id, task.title });
        try w.print("Description:\n{s}\n\n", .{task.description});
        try w.writeAll("Repository files:\n");
        try w.writeAll(ls.stdout[0..@min(ls.stdout.len, 4000)]);
        try w.writeAll(
            \\
            \\Write a file called `spec.md` at the repository root containing:
            \\1. Task summary (2-3 sentences)
            \\2. Files to modify (exact paths)
            \\3. Files to create (exact paths)
            \\4. Function/type signatures for new or changed code
            \\5. Acceptance criteria (testable assertions)
            \\6. Edge cases to handle
            \\
            \\Do NOT modify any source files. Only write spec.md.
        );

        const result = self.spawnAgent(.manager, prompt_buf.items, wt_path) catch |err| {
            try self.failTask(task, "manager agent spawn failed", @errorName(err));
            return;
        };
        defer self.allocator.free(result.output);
        defer if (result.new_session_id) |sid| self.allocator.free(sid);

        self.db.storeTaskOutput(task.id, "spec", result.output, 0) catch {};

        // Commit spec.md in worktree
        var add = try wt_git.addAll();
        defer add.deinit();
        var commit = try wt_git.commit("spec: generate spec.md for task");
        defer commit.deinit();

        if (!commit.success()) {
            // No changes? Manager didn't write spec.md
            try self.failTask(task, "manager produced no output", commit.stderr);
            return;
        }

        try self.db.updateTaskStatus(task.id, "qa");
        self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d}: spec ready, starting QA", .{task.id}));
    }

    fn runQaPhase(self: *Pipeline, task: db_mod.PipelineTask) !void {
        const wt_path = try self.worktreePath(task.id);
        defer self.allocator.free(wt_path);
        var wt_git = Git.init(self.allocator, wt_path);

        var prompt_buf = std.ArrayList(u8).init(self.allocator);
        defer prompt_buf.deinit();
        const w = prompt_buf.writer();

        try w.writeAll(
            \\Read the spec.md file in the repository root.
            \\Write test files that verify every acceptance criterion listed in spec.md.
            \\
            \\Rules:
            \\- Only create or modify test files (files matching *_test.* or in a tests/ directory)
            \\- Tests must be deterministic and self-contained
            \\- Tests should FAIL initially since the features are not yet implemented
            \\- Include both happy-path and edge-case tests
            \\- Do NOT write implementation code
        );

        const result = self.spawnAgent(.qa, prompt_buf.items, wt_path) catch |err| {
            try self.failTask(task, "QA agent spawn failed", @errorName(err));
            return;
        };
        defer self.allocator.free(result.output);
        defer if (result.new_session_id) |sid| self.allocator.free(sid);

        self.db.storeTaskOutput(task.id, "qa", result.output, 0) catch {};

        var add = try wt_git.addAll();
        defer add.deinit();
        var commit = try wt_git.commit("test: add tests from QA agent");
        defer commit.deinit();

        if (!commit.success()) {
            try self.failTask(task, "QA produced no test files", commit.stderr);
            return;
        }

        try self.db.updateTaskStatus(task.id, "impl");
        self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d}: tests written, starting implementation", .{task.id}));
    }

    fn runImplPhase(self: *Pipeline, task: db_mod.PipelineTask) !void {
        const wt_path = try self.worktreePath(task.id);
        defer self.allocator.free(wt_path);
        var wt_git = Git.init(self.allocator, wt_path);

        // Idempotency: if a previous run left passing code, skip the agent
        if (self.runTestCommand(wt_path)) |pre_test| {
            defer self.allocator.free(pre_test.stdout);
            defer self.allocator.free(pre_test.stderr);
            if (pre_test.exit_code == 0) {
                try self.db.updateTaskStatus(task.id, "done");
                try self.db.enqueueForIntegration(task.id, task.branch);
                self.cleanupWorktree(task);
                std.log.info("Task #{d} tests already pass, skipping agent", .{task.id});
                self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} passed all tests! Queued for release train.", .{task.id}));
                return;
            }
        } else |_| {}

        // Build prompt with error context for retries
        var prompt_buf = std.ArrayList(u8).init(self.allocator);
        defer prompt_buf.deinit();
        const w = prompt_buf.writer();

        try w.writeAll(
            \\Read spec.md for the specification and the existing test files.
            \\Write implementation code that makes all tests pass.
            \\
            \\Rules:
            \\- Only modify files listed in spec.md under "Files to modify" or "Files to create"
            \\- Do NOT modify test files
            \\- Follow existing code conventions
            \\- Keep changes minimal and focused
        );

        if (std.mem.eql(u8, task.status, "retry") and task.last_error.len > 0) {
            try w.writeAll("\n\nPrevious attempt failed. Test output:\n```\n");
            const err_tail = if (task.last_error.len > 3000) task.last_error[task.last_error.len - 3000 ..] else task.last_error;
            try w.writeAll(err_tail);
            try w.writeAll("\n```\nFix the failures.");
        }

        const result = self.spawnAgent(.worker, prompt_buf.items, wt_path) catch |err| {
            try self.failTask(task, "worker agent spawn failed", @errorName(err));
            return;
        };
        defer self.allocator.free(result.output);
        defer if (result.new_session_id) |sid| self.allocator.free(sid);

        self.db.storeTaskOutput(task.id, "impl", result.output, 0) catch {};

        // Commit implementation in worktree
        var add = try wt_git.addAll();
        defer add.deinit();
        var commit = try wt_git.commit("impl: implementation from worker agent");
        defer commit.deinit();

        // Run tests in worktree
        const test_result = self.runTestCommand(wt_path) catch |err| {
            try self.failTask(task, "test command execution failed", @errorName(err));
            return;
        };
        defer self.allocator.free(test_result.stdout);
        defer self.allocator.free(test_result.stderr);

        {
            const test_combined = std.fmt.allocPrint(self.allocator, "EXIT {d}\n--- stdout ---\n{s}\n--- stderr ---\n{s}", .{
                test_result.exit_code,
                test_result.stdout[0..@min(test_result.stdout.len, 8000)],
                test_result.stderr[0..@min(test_result.stderr.len, 8000)],
            }) catch null;
            if (test_combined) |tc| {
                defer self.allocator.free(tc);
                self.db.storeTaskOutput(task.id, "test", tc, @intCast(test_result.exit_code)) catch {};
            }
        }

        if (test_result.exit_code == 0) {
            try self.db.updateTaskStatus(task.id, "done");
            try self.db.enqueueForIntegration(task.id, task.branch);
            self.cleanupWorktree(task);
            std.log.info("Task #{d} passed tests, queued for integration", .{task.id});
            self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} passed all tests! Queued for release train.", .{task.id}));
        } else {
            if (task.attempt + 1 >= task.max_attempts) {
                const combined = try std.fmt.allocPrint(self.allocator, "stdout:\n{s}\nstderr:\n{s}", .{
                    test_result.stdout[0..@min(test_result.stdout.len, 2000)],
                    test_result.stderr[0..@min(test_result.stderr.len, 2000)],
                });
                defer self.allocator.free(combined);
                try self.db.updateTaskError(task.id, combined);
                try self.db.updateTaskStatus(task.id, "failed");
                std.log.warn("Task #{d} failed after {d} attempts", .{ task.id, task.attempt + 1 });
                self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} FAILED after {d} attempts.", .{ task.id, task.attempt + 1 }));
            } else {
                const combined = try std.fmt.allocPrint(self.allocator, "{s}\n{s}", .{ test_result.stdout, test_result.stderr });
                defer self.allocator.free(combined);
                try self.db.updateTaskError(task.id, combined[0..@min(combined.len, 4000)]);
                try self.db.incrementTaskAttempt(task.id);
                try self.db.updateTaskStatus(task.id, "retry");
                std.log.info("Task #{d} test failed, retry {d}/{d}", .{ task.id, task.attempt + 1, task.max_attempts });
            }
        }
    }

    fn runRebasePhase(self: *Pipeline, task: db_mod.PipelineTask) !void {
        if (task.branch.len == 0) {
            try self.failTask(task, "rebase: no branch set", "");
            return;
        }

        // Ensure worktree exists (may have been cleaned up after 'done')
        const wt_path = try self.worktreePath(task.id);
        defer self.allocator.free(wt_path);

        const wt_exists = blk: {
            std.fs.accessAbsolute(wt_path, .{}) catch break :blk false;
            break :blk true;
        };
        if (!wt_exists) {
            var repo_git = Git.init(self.allocator, self.config.pipeline_repo);
            const wt_dir = try std.fmt.allocPrint(self.allocator, "{s}/.worktrees", .{self.config.pipeline_repo});
            defer self.allocator.free(wt_dir);
            std.fs.makeDirAbsolute(wt_dir) catch {};

            var wt = try repo_git.addWorktreeExisting(wt_path, task.branch);
            defer wt.deinit();
            if (!wt.success()) {
                try self.failTask(task, "rebase: worktree checkout failed", wt.stderr);
                return;
            }
        }

        var wt_git = Git.init(self.allocator, wt_path);

        // Fetch latest main and attempt rebase
        var fetch_r = try wt_git.fetch("origin");
        defer fetch_r.deinit();

        var rebase_r = try wt_git.rebase("origin/main");
        defer rebase_r.deinit();

        if (!rebase_r.success()) {
            // Rebase has conflicts — abort and let worker agent fix them
            var abort = try wt_git.abortRebase();
            defer abort.deinit();

            std.log.info("Task #{d} rebase conflicts, spawning worker to resolve", .{task.id});

            var prompt_buf = std.ArrayList(u8).init(self.allocator);
            defer prompt_buf.deinit();
            const w = prompt_buf.writer();

            try w.writeAll(
                \\This branch has merge conflicts with main. Your job:
                \\1. Run `git fetch origin && git rebase origin/main` to start the rebase
                \\2. Resolve ALL conflicts in the affected files
                \\3. `git add` the resolved files and `git rebase --continue`
                \\4. Repeat until the rebase is complete
                \\5. Make sure the code compiles and tests pass after resolving
                \\
                \\Read spec.md for context on what this branch does.
            );

            if (task.last_error.len > 0) {
                try w.writeAll("\n\nPrevious error context:\n```\n");
                const err_tail = if (task.last_error.len > 2000) task.last_error[task.last_error.len - 2000 ..] else task.last_error;
                try w.writeAll(err_tail);
                try w.writeAll("\n```");
            }

            const result = self.spawnAgent(.worker, prompt_buf.items, wt_path) catch |err| {
                try self.failTask(task, "rebase: worker agent failed", @errorName(err));
                return;
            };
            defer self.allocator.free(result.output);
            defer if (result.new_session_id) |sid| self.allocator.free(sid);

            self.db.storeTaskOutput(task.id, "rebase", result.output, 0) catch {};
        }

        // Run tests on the rebased branch
        const test_result = self.runTestCommand(wt_path) catch |err| {
            try self.failTask(task, "rebase: test execution failed", @errorName(err));
            return;
        };
        defer self.allocator.free(test_result.stdout);
        defer self.allocator.free(test_result.stderr);

        if (test_result.exit_code == 0) {
            // Push the rebased branch and re-queue
            var push_r = try wt_git.exec(&.{ "push", "--force-with-lease", "origin", task.branch });
            defer push_r.deinit();

            try self.db.updateTaskStatus(task.id, "done");
            try self.db.enqueueForIntegration(task.id, task.branch);
            self.cleanupWorktree(task);
            std.log.info("Task #{d} rebased and re-queued for integration", .{task.id});
            self.notify(task.notify_chat, std.fmt.allocPrint(self.allocator, "Task #{d} \"{s}\" rebased successfully, re-queued for release.", .{ task.id, task.title }) catch return);
        } else {
            // Tests still fail after rebase — retry if attempts remain
            if (task.attempt + 1 >= task.max_attempts) {
                const combined = try std.fmt.allocPrint(self.allocator, "stdout:\n{s}\nstderr:\n{s}", .{
                    test_result.stdout[0..@min(test_result.stdout.len, 2000)],
                    test_result.stderr[0..@min(test_result.stderr.len, 2000)],
                });
                defer self.allocator.free(combined);
                try self.db.updateTaskError(task.id, combined);
                try self.db.updateTaskStatus(task.id, "failed");
                std.log.warn("Task #{d} failed rebase after {d} attempts", .{ task.id, task.attempt + 1 });
                self.notify(task.notify_chat, std.fmt.allocPrint(self.allocator, "Task #{d} FAILED rebase after {d} attempts.", .{ task.id, task.attempt + 1 }) catch return);
            } else {
                const combined = try std.fmt.allocPrint(self.allocator, "{s}\n{s}", .{ test_result.stdout, test_result.stderr });
                defer self.allocator.free(combined);
                try self.db.updateTaskError(task.id, combined[0..@min(combined.len, 4000)]);
                try self.db.incrementTaskAttempt(task.id);
                // Stay in rebase status — will retry next tick
                std.log.info("Task #{d} rebase tests failed, retry {d}/{d}", .{ task.id, task.attempt + 1, task.max_attempts });
            }
        }
    }

    fn runTestCommand(self: *Pipeline, cwd: []const u8) !TestResult {
        var child = std.process.Child.init(
            &.{ "/bin/sh", "-c", self.config.pipeline_test_cmd },
            self.allocator,
        );
        child.stdin_behavior = .Close;
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Pipe;

        child.cwd = cwd;

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

        return TestResult{
            .stdout = try stdout_buf.toOwnedSlice(),
            .stderr = try stderr_buf.toOwnedSlice(),
            .exit_code = exit_code,
        };
    }

    // --- Macro Loop: Release Train ---

    fn checkReleaseTrain(self: *Pipeline) !void {
        const now = std.time.timestamp();
        if (!self.config.continuous_mode) {
            const interval: i64 = @intCast(@as(u64, self.config.release_interval_mins) * 60);
            if (now - self.last_release_ts < interval) return;
        }

        // Check if there's anything queued
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const queued = try self.db.getQueuedBranches(arena.allocator());
        if (queued.len == 0) return;

        std.log.info("Release train starting with {d} branches", .{queued.len});
        try self.runReleaseTrain(queued);
        self.last_release_ts = std.time.timestamp();
    }

    fn runReleaseTrain(self: *Pipeline, queued: []db_mod.QueueEntry) !void {
        var git = Git.init(self.allocator, self.config.pipeline_repo);

        self.notify(self.config.pipeline_admin_chat, try self.allocator.dupe(u8, "Release train starting..."));

        // 1. Go to main
        var co = try git.checkout("main");
        defer co.deinit();
        var pull = try git.pull();
        defer pull.deinit();

        // 2. Create release-candidate branch
        const rc_name = "release-candidate";
        var rc = try git.createBranch(rc_name, "main");
        defer rc.deinit();
        if (!rc.success()) {
            // RC branch might already exist, delete and retry
            var del = try git.deleteBranch(rc_name);
            defer del.deinit();
            var rc2 = try git.createBranch(rc_name, "main");
            defer rc2.deinit();
        }

        var co_rc = try git.checkout(rc_name);
        defer co_rc.deinit();

        // 3. Merge branches one by one, test after each
        var merged = std.ArrayList([]const u8).init(self.allocator);
        defer merged.deinit();
        var excluded = std.ArrayList([]const u8).init(self.allocator);
        defer excluded.deinit();

        for (queued) |entry| {
            try self.db.updateQueueStatus(entry.id, "merging", null);

            var merge = try git.merge(entry.branch);
            defer merge.deinit();

            if (!merge.success()) {
                // Merge conflict — abort, send back for rebase
                var abort = try git.abortMerge();
                defer abort.deinit();
                try self.db.updateQueueStatus(entry.id, "excluded", "merge conflict");
                try self.db.updateTaskError(entry.task_id, "Excluded from release: merge conflict — rebasing");
                try self.db.incrementTaskAttempt(entry.task_id);
                try excluded.append(entry.branch);
                if (self.db.getPipelineTask(self.allocator, entry.task_id) catch null) |task| {
                    if (task.attempt + 1 >= task.max_attempts) {
                        try self.db.updateTaskStatus(entry.task_id, "failed");
                        self.notify(task.notify_chat, std.fmt.allocPrint(self.allocator, "Task #{d} \"{s}\" failed after {d} attempts (merge conflicts).", .{ task.id, task.title, task.max_attempts }) catch continue);
                    } else {
                        try self.db.updateTaskStatus(entry.task_id, "rebase");
                        self.notify(task.notify_chat, std.fmt.allocPrint(self.allocator, "Task #{d} \"{s}\" has merge conflicts — rebasing (attempt {d}/{d}).", .{ task.id, task.title, task.attempt + 1, task.max_attempts }) catch continue);
                    }
                } else {
                    try self.db.updateTaskStatus(entry.task_id, "rebase");
                }
                continue;
            }

            // Run global tests on the release-candidate
            const test_result = self.runTestCommand(self.config.pipeline_repo) catch {
                try excluded.append(entry.branch);
                continue;
            };
            defer self.allocator.free(test_result.stdout);
            defer self.allocator.free(test_result.stderr);

            if (test_result.exit_code != 0) {
                // Tests failed after merge — revert, send back for rebase
                var reset = try git.resetHard("HEAD~1");
                defer reset.deinit();
                try self.db.updateQueueStatus(entry.id, "excluded", "tests failed after merge");
                try self.db.updateTaskError(entry.task_id, test_result.stderr[0..@min(test_result.stderr.len, 4000)]);
                try self.db.incrementTaskAttempt(entry.task_id);
                try excluded.append(entry.branch);
                if (self.db.getPipelineTask(self.allocator, entry.task_id) catch null) |task| {
                    if (task.attempt + 1 >= task.max_attempts) {
                        try self.db.updateTaskStatus(entry.task_id, "failed");
                        self.notify(task.notify_chat, std.fmt.allocPrint(self.allocator, "Task #{d} \"{s}\" failed after {d} attempts (integration tests).", .{ task.id, task.title, task.max_attempts }) catch continue);
                    } else {
                        try self.db.updateTaskStatus(entry.task_id, "rebase");
                        self.notify(task.notify_chat, std.fmt.allocPrint(self.allocator, "Task #{d} \"{s}\" failed integration tests — rebasing (attempt {d}/{d}).", .{ task.id, task.title, task.attempt + 1, task.max_attempts }) catch continue);
                    }
                } else {
                    try self.db.updateTaskStatus(entry.task_id, "rebase");
                }
                continue;
            }

            // Success!
            try self.db.updateQueueStatus(entry.id, "merged", null);
            try self.db.updateTaskStatus(entry.task_id, "merged");
            try merged.append(entry.branch);

            // Notify the task's originating chat
            if (self.db.getPipelineTask(self.allocator, entry.task_id) catch null) |task| {
                self.notify(task.notify_chat, std.fmt.allocPrint(self.allocator, "Task #{d} \"{s}\" merged to main.", .{ task.id, task.title }) catch continue);
            }
        }

        if (merged.items.len == 0) {
            // Nothing merged, clean up
            var co_main = try git.checkout("main");
            defer co_main.deinit();
            var del = try git.deleteBranch(rc_name);
            defer del.deinit();
            self.notify(self.config.pipeline_admin_chat, try self.allocator.dupe(u8, "Release train: no branches merged."));
            return;
        }

        // 4. Fast-forward main
        var co_main = try git.checkout("main");
        defer co_main.deinit();
        var ff = try git.merge(rc_name);
        defer ff.deinit();
        var push = try git.push("origin", "main");
        defer push.deinit();

        // 5. Cleanup
        var del_rc = try git.deleteBranch(rc_name);
        defer del_rc.deinit();
        for (merged.items) |branch| {
            var del = try git.deleteBranch(branch);
            defer del.deinit();
        }

        // 5b. Self-update: check if main has advanced past our startup commit
        self.checkSelfUpdate();

        // 6. Generate and send digest
        const digest = try self.generateDigest(merged.items, excluded.items);
        self.notify(self.config.pipeline_admin_chat, digest);
        std.log.info("Release train complete: {d} merged, {d} excluded", .{ merged.items.len, excluded.items.len });
    }

    fn generateDigest(self: *Pipeline, merged: [][]const u8, excluded_branches: [][]const u8) ![]const u8 {
        var buf = std.ArrayList(u8).init(self.allocator);
        const w = buf.writer();

        try w.writeAll("*Release Train Complete*\n\n");
        try w.print("Merged: {d} branch(es)\n", .{merged.len});

        for (merged) |branch| {
            try w.print("  + {s}\n", .{branch});
        }

        if (excluded_branches.len > 0) {
            try w.print("\nExcluded: {d} branch(es)\n", .{excluded_branches.len});
            for (excluded_branches) |branch| {
                try w.print("  - {s}\n", .{branch});
            }
        }

        return buf.toOwnedSlice();
    }

    // --- Self-Update ---

    fn checkSelfUpdate(self: *Pipeline) void {
        var git = Git.init(self.allocator, self.config.pipeline_repo);
        const current_head = git.revParseHead() catch return;

        if (std.mem.eql(u8, &current_head, &self.startup_head)) return;
        if (std.mem.eql(u8, &self.startup_head, &([_]u8{0} ** 40))) return;

        std.log.info("Self-update: main HEAD changed, rebuilding...", .{});
        self.notify(self.config.pipeline_admin_chat, self.allocator.dupe(u8, "Self-update: new commits detected, rebuilding...") catch return);

        // Run zig build in the repo
        var child = std.process.Child.init(
            &.{ "zig", "build" },
            self.allocator,
        );
        child.cwd = self.config.pipeline_repo;
        child.stdin_behavior = .Close;
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Pipe;
        child.spawn() catch |err| {
            std.log.err("Self-update: spawn build failed: {}", .{err});
            return;
        };

        var stderr_buf = std.ArrayList(u8).init(self.allocator);
        defer stderr_buf.deinit();
        var read_buf: [8192]u8 = undefined;
        if (child.stderr) |stderr| {
            while (true) {
                const n = stderr.read(&read_buf) catch break;
                if (n == 0) break;
                stderr_buf.appendSlice(read_buf[0..n]) catch break;
            }
        }
        // Drain stdout too
        if (child.stdout) |stdout| {
            while (true) {
                const n = stdout.read(&read_buf) catch break;
                if (n == 0) break;
            }
        }

        const term = child.wait() catch |err| {
            std.log.err("Self-update: wait failed: {}", .{err});
            return;
        };
        const exit_code: u8 = switch (term) {
            .Exited => |code| code,
            else => 1,
        };

        if (exit_code != 0) {
            std.log.err("Self-update: build failed (exit {d}): {s}", .{ exit_code, stderr_buf.items[0..@min(stderr_buf.items.len, 500)] });
            self.notify(self.config.pipeline_admin_chat, self.allocator.dupe(u8, "Self-update: build FAILED, continuing with old binary.") catch return);
            return;
        }

        std.log.info("Self-update: build succeeded, scheduling restart", .{});
        self.notify(self.config.pipeline_admin_chat, self.allocator.dupe(u8, "Self-update: build succeeded, restarting...") catch return);
        self.update_ready.store(true, .release);
        self.running.store(false, .release);
    }

    // --- Agent Spawning ---

    fn spawnAgent(self: *Pipeline, persona: AgentPersona, prompt: []const u8, workdir: []const u8) !agent_mod.AgentResult {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const tmp = arena.allocator();

        self.config.refreshOAuthToken();

        const system_prompt = getSystemPrompt(persona);
        const allowed_tools = getAllowedTools(persona);

        // Build JSON input
        var input = std.ArrayList(u8).init(tmp);
        const esc_prompt = try json_mod.escapeString(tmp, prompt);
        const esc_sys = try json_mod.escapeString(tmp, system_prompt);
        try input.writer().print("{{\"prompt\":\"{s}\",\"systemPrompt\":\"{s}\",\"model\":\"{s}\",\"allowedTools\":\"{s}\",\"workdir\":\"/workspace/repo\"}}", .{
            esc_prompt, esc_sys, self.config.model, allowed_tools,
        });

        // Container name
        var name_buf: [128]u8 = undefined;
        const container_name = try std.fmt.bufPrint(&name_buf, "borg-pipeline-{s}-{d}", .{
            @tagName(persona), std.time.timestamp(),
        });

        // Env vars
        var oauth_buf: [4096]u8 = undefined;
        const oauth_env = try std.fmt.bufPrint(&oauth_buf, "CLAUDE_CODE_OAUTH_TOKEN={s}", .{self.config.oauth_token});
        var model_buf: [256]u8 = undefined;
        const model_env = try std.fmt.bufPrint(&model_buf, "CLAUDE_MODEL={s}", .{self.config.model});

        const env = [_][]const u8{
            oauth_env,
            model_env,
            "HOME=/home/node",
            "NODE_OPTIONS=--max-old-space-size=384",
        };

        // Bind mount worktree directory into container
        var bind_buf: [1024]u8 = undefined;
        const repo_bind = try std.fmt.bufPrint(&bind_buf, "{s}:/workspace/repo", .{workdir});

        const binds = [_][]const u8{repo_bind};

        std.log.info("Spawning {s} agent: {s}", .{ @tagName(persona), container_name });

        // Start timeout watchdog
        var agent_done = std.atomic.Value(bool).init(false);
        const name_for_watchdog = try self.allocator.dupe(u8, container_name);
        const watchdog = std.Thread.spawn(.{}, agentTimeoutWatchdog, .{
            &agent_done, self.docker, name_for_watchdog, AGENT_TIMEOUT_S,
        }) catch null;

        var run_result = try self.docker.runWithStdio(docker_mod.ContainerConfig{
            .image = self.config.container_image,
            .name = container_name,
            .env = &env,
            .binds = &binds,
            .memory_limit = 1024 * 1024 * 1024, // 1GB for pipeline agents
        }, input.items);
        defer run_result.deinit();

        // Cancel watchdog
        agent_done.store(true, .release);
        if (watchdog) |w| w.join();
        self.allocator.free(name_for_watchdog);

        std.log.info("{s} agent done (exit={d}, {d} bytes)", .{ @tagName(persona), run_result.exit_code, run_result.stdout.len });

        return try agent_mod.parseNdjson(self.allocator, run_result.stdout);
    }

    fn agentTimeoutWatchdog(done: *std.atomic.Value(bool), docker: *Docker, name: []const u8, timeout_s: i64) void {
        const deadline = std.time.timestamp() + timeout_s;
        while (std.time.timestamp() < deadline) {
            if (done.load(.acquire)) return;
            std.time.sleep(5 * std.time.ns_per_s);
        }
        if (!done.load(.acquire)) {
            std.log.warn("Agent timeout ({d}s): killing container {s}", .{ timeout_s, name });
            docker.killContainer(name) catch {};
        }
    }

    // --- Helpers ---

    fn failTask(self: *Pipeline, task: db_mod.PipelineTask, reason: []const u8, detail: []const u8) !void {
        std.log.err("Task #{d} failed: {s}: {s}", .{ task.id, reason, detail[0..@min(detail.len, 200)] });
        try self.db.updateTaskStatus(task.id, "failed");
        try self.db.updateTaskError(task.id, detail[0..@min(detail.len, 4000)]);
        // Keep worktree for debugging - don't clean up on failure
        self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} failed: {s}", .{ task.id, reason }));
    }

    fn notify(self: *Pipeline, chat_id: []const u8, message: []const u8) void {
        defer self.allocator.free(message);
        if (chat_id.len == 0) return;
        // Strip "tg:" prefix for Telegram API
        const raw_id = if (std.mem.startsWith(u8, chat_id, "tg:")) chat_id[3..] else chat_id;
        self.tg.sendMessage(raw_id, message, null) catch |err| {
            std.log.err("Pipeline notify failed: {}", .{err});
        };
    }
};

pub fn getSystemPrompt(persona: AgentPersona) []const u8 {
    return switch (persona) {
        .manager =>
        \\You are the Manager agent in an autonomous engineering pipeline.
        \\Your job is to read a task description and the codebase, then produce
        \\a spec.md file at the repository root.
        \\
        \\spec.md must contain:
        \\1. Task summary (2-3 sentences)
        \\2. Files to modify (exact paths)
        \\3. Files to create (exact paths)
        \\4. Function/type signatures for new or changed code
        \\5. Acceptance criteria (specific, testable assertions)
        \\6. Edge cases to handle
        \\
        \\Rules:
        \\- You have READ-ONLY access to source code
        \\- You may ONLY write the file spec.md
        \\- Be specific about file paths and function names
        \\- Do NOT write any implementation code
        ,
        .qa =>
        \\You are the QA agent in an autonomous engineering pipeline.
        \\Your job is to read spec.md and write comprehensive test files.
        \\
        \\Rules:
        \\- Read spec.md for requirements and acceptance criteria
        \\- Write test files ONLY (files matching *_test.* or in tests/ directories)
        \\- Tests must be deterministic and runnable with the project's test command
        \\- Each acceptance criterion must have at least one test
        \\- Include happy-path AND edge-case tests
        \\- Tests should FAIL initially (features not yet implemented)
        \\- Do NOT write implementation code
        \\- Do NOT modify non-test files
        ,
        .worker =>
        \\You are the Worker agent in an autonomous engineering pipeline.
        \\Your job is to write implementation code that passes all existing tests.
        \\
        \\Rules:
        \\- Read spec.md for the specification
        \\- Read test files to understand expected behavior
        \\- Only modify files listed in spec.md under "Files to modify/create"
        \\- Do NOT modify test files
        \\- Do NOT add dependencies without spec approval
        \\- Follow existing code conventions
        \\- Write minimal code to pass all tests
        ,
    };
}

pub fn getAllowedTools(persona: AgentPersona) []const u8 {
    return switch (persona) {
        .manager => "Read,Glob,Grep,Write",
        .qa => "Read,Glob,Grep,Write",
        .worker => "Read,Glob,Grep,Write,Edit,Bash",
    };
}

const TestResult = struct {
    stdout: []const u8,
    stderr: []const u8,
    exit_code: u8,
};

// ── Tests ──────────────────────────────────────────────────────────────

test "getSystemPrompt returns non-empty for all personas" {
    try std.testing.expect(getSystemPrompt(.manager).len > 0);
    try std.testing.expect(getSystemPrompt(.qa).len > 0);
    try std.testing.expect(getSystemPrompt(.worker).len > 0);
}

test "getAllowedTools restricts manager and qa" {
    const mgr = getAllowedTools(.manager);
    const qa = getAllowedTools(.qa);
    const wrk = getAllowedTools(.worker);

    // Manager and QA should not have Bash or Edit
    try std.testing.expect(std.mem.indexOf(u8, mgr, "Bash") == null);
    try std.testing.expect(std.mem.indexOf(u8, qa, "Bash") == null);
    try std.testing.expect(std.mem.indexOf(u8, qa, "Edit") == null);

    // Worker has Bash and Edit
    try std.testing.expect(std.mem.indexOf(u8, wrk, "Bash") != null);
    try std.testing.expect(std.mem.indexOf(u8, wrk, "Edit") != null);
}

test "digest generation formatting" {
    const alloc = std.testing.allocator;

    var buf = std.ArrayList(u8).init(alloc);
    defer buf.deinit();
    const w = buf.writer();

    const merged = [_][]const u8{ "feature/task-1", "feature/task-2" };
    const excluded = [_][]const u8{"feature/task-3"};

    try w.writeAll("*Release Train Complete*\n\n");
    try w.print("Merged: {d} branch(es)\n", .{merged.len});
    for (merged) |branch| {
        try w.print("  + {s}\n", .{branch});
    }
    try w.print("\nExcluded: {d} branch(es)\n", .{excluded.len});
    for (excluded) |branch| {
        try w.print("  - {s}\n", .{branch});
    }

    const result = buf.items;
    try std.testing.expect(std.mem.indexOf(u8, result, "Merged: 2") != null);
    try std.testing.expect(std.mem.indexOf(u8, result, "feature/task-1") != null);
    try std.testing.expect(std.mem.indexOf(u8, result, "Excluded: 1") != null);
    try std.testing.expect(std.mem.indexOf(u8, result, "feature/task-3") != null);
}
