const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;
const docker_mod = @import("docker.zig");
const Docker = docker_mod.Docker;
const tg_mod = @import("telegram.zig");
const Telegram = tg_mod.Telegram;
const git_mod = @import("git.zig");
const Git = git_mod.Git;
const gt_mod = @import("gt.zig");
const Gt = gt_mod.Gt;
const json_mod = @import("json.zig");
const agent_mod = @import("agent.zig");
const Config = @import("config.zig").Config;

const TICK_INTERVAL_S = 30;
const REMOTE_CHECK_INTERVAL_S = 300; // Check for remote updates every 5 minutes
const AGENT_TIMEOUT_S = 600;
const MAX_BACKLOG_SIZE = 5;
const SEED_COOLDOWN_S = 3600; // Min 1h between seed attempts
const MAX_PARALLEL_AGENTS = 4;

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
    last_remote_check_ts: i64,
    startup_heads: std.StringHashMap([40]u8),

    // Pipelining: concurrent phase processing
    inflight_tasks: std.AutoHashMap(i64, void),
    inflight_mu: std.Thread.Mutex,
    active_agents: std.atomic.Value(u32),
    graphite_available: bool,

    pub fn init(
        allocator: std.mem.Allocator,
        db: *Db,
        docker: *Docker,
        tg: *Telegram,
        config: *Config,
    ) Pipeline {
        var heads = std.StringHashMap([40]u8).init(allocator);
        for (config.watched_repos) |repo| {
            var git = Git.init(allocator, repo.path);
            const head = git.revParseHead() catch [_]u8{0} ** 40;
            heads.put(repo.path, head) catch {};
        }

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
            .last_remote_check_ts = 0,
            .startup_heads = heads,
            .inflight_tasks = std.AutoHashMap(i64, void).init(allocator),
            .inflight_mu = .{},
            .active_agents = std.atomic.Value(u32).init(0),
            .graphite_available = false,
        };
    }

    pub fn run(self: *Pipeline) void {
        std.log.info("Pipeline thread started for {d} repo(s)", .{self.config.watched_repos.len});

        self.initGraphite();

        while (self.running.load(.acquire)) {
            self.tick() catch |err| {
                std.log.err("Pipeline tick error: {}", .{err});
            };

            self.checkReleaseTrain() catch |err| {
                std.log.err("Release train error: {}", .{err});
            };

            self.checkRemoteUpdates();

            std.time.sleep(TICK_INTERVAL_S * std.time.ns_per_s);
        }

        // Wait for running agents to finish (up to 30s)
        const deadline = std.time.timestamp() + 30;
        while (self.active_agents.load(.acquire) > 0 and std.time.timestamp() < deadline) {
            std.time.sleep(1 * std.time.ns_per_s);
        }
        if (self.active_agents.load(.acquire) > 0) {
            std.log.warn("Pipeline stopping with {d} agents still running", .{self.active_agents.load(.acquire)});
        }

        std.log.info("Pipeline thread stopped", .{});
    }

    pub fn stop(self: *Pipeline) void {
        self.running.store(false, .release);
    }

    pub fn getActiveAgentCount(self: *Pipeline) u32 {
        return self.active_agents.load(.acquire);
    }

    fn initGraphite(self: *Pipeline) void {
        if (!self.config.graphite_enabled) return;

        // Check if gt CLI is available
        var child = std.process.Child.init(&.{ "gt", "--version" }, self.allocator);
        child.stdin_behavior = .Close;
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Pipe;
        child.spawn() catch {
            std.log.warn("Graphite CLI (gt) not found, falling back to legacy release train", .{});
            return;
        };
        _ = child.wait() catch return;

        // Init Graphite for each watched repo
        for (self.config.watched_repos) |repo| {
            var gt = Gt.init(self.allocator, repo.path);
            var r = gt.repoInit("main") catch continue;
            defer r.deinit();
            if (r.success()) {
                std.log.info("Graphite initialized for {s}", .{repo.path});
            }
        }

        self.graphite_available = true;
        std.log.info("Graphite stacking enabled", .{});
    }

    fn tick(self: *Pipeline) !void {
        const tasks = try self.db.getActivePipelineTasks(self.allocator, 20);
        defer self.allocator.free(tasks);

        if (tasks.len == 0) {
            if (self.active_agents.load(.acquire) == 0) {
                try self.seedIfIdle();
            }
            return;
        }

        for (tasks) |task| {
            if (self.active_agents.load(.acquire) >= MAX_PARALLEL_AGENTS) break;

            // Skip if already in-flight
            {
                self.inflight_mu.lock();
                defer self.inflight_mu.unlock();
                if (self.inflight_tasks.contains(task.id)) continue;
                self.inflight_tasks.put(task.id, {}) catch continue;
            }

            _ = self.active_agents.fetchAdd(1, .acq_rel);
            std.log.info("Pipeline dispatching task #{d} [{s}] in {s}: {s}", .{ task.id, task.status, task.repo_path, task.title });

            _ = std.Thread.spawn(.{}, processTaskThread, .{ self, task }) catch {
                _ = self.active_agents.fetchSub(1, .acq_rel);
                self.inflight_mu.lock();
                defer self.inflight_mu.unlock();
                _ = self.inflight_tasks.remove(task.id);
                continue;
            };
        }
    }

    fn processTaskThread(self: *Pipeline, task: db_mod.PipelineTask) void {
        defer {
            _ = self.active_agents.fetchSub(1, .acq_rel);
            self.inflight_mu.lock();
            defer self.inflight_mu.unlock();
            _ = self.inflight_tasks.remove(task.id);
        }

        if (std.mem.eql(u8, task.status, "backlog")) {
            self.setupBranch(task) catch |err| {
                std.log.err("Task #{d} backlog error: {}", .{ task.id, err });
            };
        } else if (std.mem.eql(u8, task.status, "spec")) {
            self.runSpecPhase(task) catch |err| {
                std.log.err("Task #{d} spec error: {}", .{ task.id, err });
            };
        } else if (std.mem.eql(u8, task.status, "qa")) {
            self.runQaPhase(task) catch |err| {
                std.log.err("Task #{d} qa error: {}", .{ task.id, err });
            };
        } else if (std.mem.eql(u8, task.status, "impl") or std.mem.eql(u8, task.status, "retry")) {
            self.runImplPhase(task) catch |err| {
                std.log.err("Task #{d} impl error: {}", .{ task.id, err });
            };
        } else if (std.mem.eql(u8, task.status, "rebase")) {
            self.runRebasePhase(task) catch |err| {
                std.log.err("Task #{d} rebase error: {}", .{ task.id, err });
            };
        }
    }

    fn seedIfIdle(self: *Pipeline) !void {
        const now = std.time.timestamp();
        const cooldown: i64 = if (self.config.continuous_mode) 60 else SEED_COOLDOWN_S;
        if (now - self.last_seed_ts < cooldown) return;

        // Don't seed if there are already active tasks
        const active = try self.db.getActivePipelineTaskCount();
        if (active >= MAX_BACKLOG_SIZE) return;

        // Rotate seed mode: 0=refactoring, 1=bug hunting, 2=test coverage
        const seed_mode = blk: {
            const mode_str = self.db.getState(self.allocator, "seed_mode") catch null;
            const prev: u32 = if (mode_str) |s| std.fmt.parseInt(u32, s, 10) catch 0 else 0;
            if (mode_str) |s| self.allocator.free(s);
            const next = (prev + 1) % 3;
            var next_buf: [4]u8 = undefined;
            const next_str = std.fmt.bufPrint(&next_buf, "{d}", .{next}) catch "0";
            self.db.setState("seed_mode", next_str) catch {};
            break :blk next;
        };

        const mode_label = switch (seed_mode) {
            0 => "refactoring",
            1 => "bug audit",
            2 => "test coverage",
            else => "refactoring",
        };
        self.last_seed_ts = now;
        self.config.refreshOAuthToken();

        // Seed each watched repo
        var total_created: u32 = 0;
        const active_u32: u32 = @intCast(@max(active, 0));
        for (self.config.watched_repos) |repo| {
            if (active_u32 + total_created >= MAX_BACKLOG_SIZE) break;
            const created = self.seedRepo(repo.path, seed_mode, mode_label, active_u32 + total_created);
            total_created += created;
        }

        if (total_created > 0) {
            std.log.info("Seeded {d} new task(s) from codebase analysis", .{total_created});
            self.notify(self.config.pipeline_admin_chat, std.fmt.allocPrint(self.allocator, "Pipeline seeded {d} new task(s) from codebase analysis", .{total_created}) catch return);
        } else {
            std.log.info("Seed scan found no actionable improvements", .{});
        }
    }

    fn seedRepo(self: *Pipeline, repo_path: []const u8, seed_mode: u32, mode_label: []const u8, current_count: u32) u32 {
        std.log.info("Scanning {s} ({s} mode)...", .{ repo_path, mode_label });

        var prompt_buf = std.ArrayList(u8).init(self.allocator);
        defer prompt_buf.deinit();
        const w = prompt_buf.writer();

        switch (seed_mode) {
            0 => w.writeAll(
                \\Analyze this codebase and identify 1-3 concrete, small improvements.
                \\Focus on refactoring and quality - NOT new features.
                \\
                \\Good tasks: extract duplicated code, improve error handling for a specific
                \\function, simplify a complex conditional, fix a subtle bug, improve naming.
                \\
                \\Bad tasks: add new features, rewrite entire modules, add documentation,
                \\change the architecture, add dependencies.
            ) catch return 0,
            1 => w.writeAll(
                \\Audit this codebase for bugs, security vulnerabilities, and reliability issues.
                \\Focus on finding real problems - NOT style preferences.
                \\
                \\Look for: race conditions, memory leaks, resource leaks (unclosed files/sockets),
                \\error handling gaps (ignored errors, missing error paths), integer overflows,
                \\buffer overruns, SQL injection, command injection, path traversal, unvalidated
                \\input at system boundaries, deadlock potential, undefined behavior.
                \\
                \\For each real issue found, create a task to fix it. Skip false positives.
            ) catch return 0,
            else => w.writeAll(
                \\Analyze this codebase and identify gaps in test coverage.
                \\Focus on finding untested code paths that matter for correctness.
                \\
                \\Look for: functions with no test coverage, error paths never exercised,
                \\edge cases not covered (empty input, max values, concurrent access),
                \\integration points between modules that lack tests,
                \\complex conditionals where not all branches are tested.
                \\
                \\Create tasks to add specific test cases. Each task should target one
                \\function or module, not broad "add tests everywhere" tasks.
            ) catch return 0,
        }

        w.writeAll(
            \\
            \\
            \\For each improvement, output EXACTLY this format (one per task):
            \\
            \\TASK_START
            \\TITLE: <short imperative title, max 80 chars>
            \\DESCRIPTION: <2-4 sentences explaining what to change and why>
            \\TASK_END
            \\
            \\Output ONLY the task blocks above. No other text.
        ) catch return 0;

        const result = self.spawnAgent(.manager, prompt_buf.items, repo_path, null) catch |err| {
            std.log.err("Seed agent failed for {s}: {}", .{ repo_path, err });
            return 0;
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

            var title: []const u8 = "";
            var desc_start: usize = 0;
            const desc_end: usize = block.len;
            var lines = std.mem.splitScalar(u8, block, '\n');
            while (lines.next()) |line| {
                const trimmed = std.mem.trim(u8, line, &[_]u8{ ' ', '\t', '\r' });
                if (std.mem.startsWith(u8, trimmed, "TITLE:")) {
                    title = std.mem.trim(u8, trimmed["TITLE:".len..], &[_]u8{ ' ', '\t' });
                } else if (std.mem.startsWith(u8, trimmed, "DESCRIPTION:")) {
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
                repo_path,
                "seeder",
                self.config.pipeline_admin_chat,
            ) catch |err| {
                std.log.err("Failed to create seeded task: {}", .{err});
                continue;
            };

            created += 1;
            if (current_count + created >= MAX_BACKLOG_SIZE) break;
        }

        return created;
    }

    fn worktreePath(self: *Pipeline, repo_path: []const u8, task_id: i64) ![]const u8 {
        return std.fmt.allocPrint(self.allocator, "{s}/.worktrees/task-{d}", .{ repo_path, task_id });
    }

    fn setupBranch(self: *Pipeline, task: db_mod.PipelineTask) !void {
        var git = Git.init(self.allocator, task.repo_path);

        // Pull latest main
        var pull = try git.exec(&.{ "fetch", "origin", "main" });
        defer pull.deinit();

        var branch_buf: [128]u8 = undefined;
        const branch = try std.fmt.bufPrint(&branch_buf, "task-{d}", .{task.id});

        // Ensure .worktrees directory exists
        const wt_dir = try std.fmt.allocPrint(self.allocator, "{s}/.worktrees", .{task.repo_path});
        defer self.allocator.free(wt_dir);
        std.fs.makeDirAbsolute(wt_dir) catch {};

        const wt_path = try self.worktreePath(task.repo_path, task.id);
        defer self.allocator.free(wt_path);

        // Clean up stale worktree/branch from a previous attempt
        var rm_wt = try git.exec(&.{ "worktree", "remove", "--force", wt_path });
        defer rm_wt.deinit();
        if (!rm_wt.success()) {
            std.fs.deleteTreeAbsolute(wt_path) catch {};
            var prune = try git.exec(&.{ "worktree", "prune" });
            defer prune.deinit();
        }
        var del_branch = try git.exec(&.{ "branch", "-D", branch });
        defer del_branch.deinit();

        if (self.graphite_available) {
            // Graphite: create stacked branch, then attach worktree
            var co = try git.checkout("main");
            defer co.deinit();

            const title = try std.fmt.allocPrint(self.allocator, "task-{d}: {s}", .{ task.id, task.title });
            defer self.allocator.free(title);

            var gt = Gt.init(self.allocator, task.repo_path);
            var gt_create = try gt.create(branch, title);
            defer gt_create.deinit();

            if (!gt_create.success()) {
                // Fallback: create branch with git
                var wt = try git.exec(&.{ "worktree", "add", wt_path, "-b", branch, "origin/main" });
                defer wt.deinit();
                if (!wt.success()) {
                    std.log.err("git worktree add failed for task #{d}: {s}", .{ task.id, wt.stderr });
                    try self.db.updateTaskError(task.id, wt.stderr);
                    return;
                }
            } else {
                // Attach worktree to the gt-created branch
                var wt = try git.addWorktreeExisting(wt_path, branch);
                defer wt.deinit();
                if (!wt.success()) {
                    std.log.err("git worktree add failed for task #{d}: {s}", .{ task.id, wt.stderr });
                    try self.db.updateTaskError(task.id, wt.stderr);
                    return;
                }
            }
        } else {
            var wt = try git.exec(&.{ "worktree", "add", wt_path, "-b", branch, "origin/main" });
            defer wt.deinit();
            if (!wt.success()) {
                std.log.err("git worktree add failed for task #{d}: {s}", .{ task.id, wt.stderr });
                try self.db.updateTaskError(task.id, wt.stderr);
                return;
            }
        }

        try self.db.updateTaskBranch(task.id, branch);
        try self.db.updateTaskStatus(task.id, "spec");
        std.log.info("Created worktree {s} (branch {s}) for task #{d}", .{ wt_path, branch, task.id });
    }

    fn cleanupWorktree(self: *Pipeline, task: db_mod.PipelineTask) void {
        const wt_path = self.worktreePath(task.repo_path, task.id) catch return;
        defer self.allocator.free(wt_path);
        var git = Git.init(self.allocator, task.repo_path);
        var rm = git.removeWorktree(wt_path) catch return;
        defer rm.deinit();
        if (rm.success()) {
            std.log.info("Cleaned up worktree for task #{d}", .{task.id});
        }
    }

    fn runSpecPhase(self: *Pipeline, task: db_mod.PipelineTask) !void {
        const wt_path = try self.worktreePath(task.repo_path, task.id);
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

        const result = self.spawnAgent(.manager, prompt_buf.items, wt_path, null) catch |err| {
            try self.failTask(task, "manager agent spawn failed", @errorName(err));
            return;
        };
        defer self.allocator.free(result.output);

        // Store session for next phase
        if (result.new_session_id) |sid| {
            self.db.setTaskSessionId(task.id, sid) catch {};
            self.allocator.free(sid);
        }

        self.db.storeTaskOutput(task.id, "spec", result.output, 0) catch {};

        var add = try wt_git.addAll();
        defer add.deinit();
        var commit = try wt_git.commit("spec: generate spec.md for task");
        defer commit.deinit();

        if (!commit.success()) {
            try self.failTask(task, "manager produced no output", commit.stderr);
            return;
        }

        try self.db.updateTaskStatus(task.id, "qa");
        self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d}: spec ready, starting QA", .{task.id}));
    }

    fn runQaPhase(self: *Pipeline, task: db_mod.PipelineTask) !void {
        const wt_path = try self.worktreePath(task.repo_path, task.id);
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

        const resume_sid = if (task.session_id.len > 0) task.session_id else null;
        const result = self.spawnAgent(.qa, prompt_buf.items, wt_path, resume_sid) catch |err| {
            try self.failTask(task, "QA agent spawn failed", @errorName(err));
            return;
        };
        defer self.allocator.free(result.output);

        if (result.new_session_id) |sid| {
            self.db.setTaskSessionId(task.id, sid) catch {};
            self.allocator.free(sid);
        }

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
        const wt_path = try self.worktreePath(task.repo_path, task.id);
        defer self.allocator.free(wt_path);
        var wt_git = Git.init(self.allocator, wt_path);

        // Idempotency: if a previous run left passing code, skip the agent
        const test_cmd = self.config.getTestCmdForRepo(task.repo_path);
        if (self.runTestCommandForRepo(wt_path, test_cmd)) |pre_test| {
            defer self.allocator.free(pre_test.stdout);
            defer self.allocator.free(pre_test.stderr);
            if (pre_test.exit_code == 0) {
                try self.db.updateTaskStatus(task.id, "done");
                try self.db.enqueueForIntegration(task.id, task.branch, task.repo_path);
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

        const resume_sid = if (task.session_id.len > 0) task.session_id else null;
        const result = self.spawnAgent(.worker, prompt_buf.items, wt_path, resume_sid) catch |err| {
            try self.failTask(task, "worker agent spawn failed", @errorName(err));
            return;
        };
        defer self.allocator.free(result.output);

        if (result.new_session_id) |sid| {
            self.db.setTaskSessionId(task.id, sid) catch {};
            self.allocator.free(sid);
        }

        self.db.storeTaskOutput(task.id, "impl", result.output, 0) catch {};

        // Commit implementation in worktree
        var add = try wt_git.addAll();
        defer add.deinit();
        var commit = try wt_git.commit("impl: implementation from worker agent");
        defer commit.deinit();

        // Run tests in worktree
        const test_result = self.runTestCommandForRepo(wt_path, test_cmd) catch |err| {
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
            try self.db.enqueueForIntegration(task.id, task.branch, task.repo_path);
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
                std.log.warn("Task #{d} exhausted {d} attempts, recycling to backlog", .{ task.id, task.max_attempts });
                try self.recycleTask(task);
                self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} exhausted {d} attempts — recycling to backlog.", .{ task.id, task.max_attempts }));
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
        const wt_path = try self.worktreePath(task.repo_path, task.id);
        defer self.allocator.free(wt_path);

        const wt_exists = blk: {
            std.fs.accessAbsolute(wt_path, .{}) catch break :blk false;
            break :blk true;
        };
        if (!wt_exists) {
            var repo_git = Git.init(self.allocator, task.repo_path);
            const wt_dir = try std.fmt.allocPrint(self.allocator, "{s}/.worktrees", .{task.repo_path});
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

            const resume_sid = if (task.session_id.len > 0) task.session_id else null;
            const result = self.spawnAgent(.worker, prompt_buf.items, wt_path, resume_sid) catch |err| {
                try self.failTask(task, "rebase: worker agent failed", @errorName(err));
                return;
            };
            defer self.allocator.free(result.output);

            if (result.new_session_id) |sid| {
                self.db.setTaskSessionId(task.id, sid) catch {};
                self.allocator.free(sid);
            }

            self.db.storeTaskOutput(task.id, "rebase", result.output, 0) catch {};
        }

        // Run tests on the rebased branch
        const rebase_test_cmd = self.config.getTestCmdForRepo(task.repo_path);
        const test_result = self.runTestCommandForRepo(wt_path, rebase_test_cmd) catch |err| {
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
            try self.db.enqueueForIntegration(task.id, task.branch, task.repo_path);
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
                std.log.warn("Task #{d} exhausted {d} rebase attempts, recycling to backlog", .{ task.id, task.max_attempts });
                try self.recycleTask(task);
                self.notify(task.notify_chat, std.fmt.allocPrint(self.allocator, "Task #{d} exhausted {d} rebase attempts — recycling to backlog.", .{ task.id, task.max_attempts }) catch return);
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
        return self.runTestCommandForRepo(cwd, self.config.pipeline_test_cmd);
    }

    fn runTestCommandForRepo(self: *Pipeline, cwd: []const u8, test_cmd: []const u8) !TestResult {
        var child = std.process.Child.init(
            &.{ "/bin/sh", "-c", test_cmd },
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

        var ran_any = false;
        for (self.config.watched_repos) |repo| {
            var arena = std.heap.ArenaAllocator.init(self.allocator);
            defer arena.deinit();
            const queued = self.db.getQueuedBranchesForRepo(arena.allocator(), repo.path) catch continue;
            if (queued.len == 0) continue;

            if (self.graphite_available) {
                std.log.info("Graphite integration for {s}: {d} branches", .{ repo.path, queued.len });
                self.runGraphiteIntegration(queued, repo.path, repo.is_self) catch |err| {
                    std.log.err("Graphite integration error for {s}: {}", .{ repo.path, err });
                };
            } else {
                std.log.info("Release train for {s}: {d} branches", .{ repo.path, queued.len });
                self.runReleaseTrain(queued, repo.path, repo.is_self) catch |err| {
                    std.log.err("Release train error for {s}: {}", .{ repo.path, err });
                };
            }
            ran_any = true;
        }

        if (ran_any) {
            self.last_release_ts = std.time.timestamp();
        }
    }

    fn runGraphiteIntegration(self: *Pipeline, queued: []db_mod.QueueEntry, repo_path: []const u8, is_self: bool) !void {
        var git = Git.init(self.allocator, repo_path);
        var gt = Gt.init(self.allocator, repo_path);

        self.notify(self.config.pipeline_admin_chat, try self.allocator.dupe(u8, "Graphite integration starting..."));

        // 1. Sync: ensure main is up to date, restack existing branches
        var co = try git.checkout("main");
        defer co.deinit();
        var pull_r = try git.pull();
        defer pull_r.deinit();
        var restack = try gt.restack();
        defer restack.deinit();

        // 2. Submit the stack to create/update PRs
        var submit = try gt.submitStack();
        defer submit.deinit();
        if (submit.success()) {
            std.log.info("Graphite stack submitted ({d} queued branches)", .{queued.len});
        } else {
            std.log.warn("gt submit --stack: {s}", .{submit.stderr});
        }

        // 3. Merge consecutive ready branches bottom-up via gh pr merge
        var merged = std.ArrayList([]const u8).init(self.allocator);
        defer merged.deinit();

        for (queued) |entry| {
            try self.db.updateQueueStatus(entry.id, "merging", null);

            const merge_cmd = try std.fmt.allocPrint(
                self.allocator,
                "gh pr merge {s} --squash --delete-branch",
                .{entry.branch},
            );
            defer self.allocator.free(merge_cmd);
            const merge_result = self.runTestCommandForRepo(repo_path, merge_cmd) catch {
                std.log.err("gh pr merge failed for {s}", .{entry.branch});
                try self.db.updateQueueStatus(entry.id, "queued", null);
                break;
            };
            defer self.allocator.free(merge_result.stdout);
            defer self.allocator.free(merge_result.stderr);

            if (merge_result.exit_code != 0) {
                std.log.warn("gh pr merge {s} failed: {s}", .{ entry.branch, merge_result.stderr });
                try self.db.updateQueueStatus(entry.id, "queued", null);
                break;
            }

            try self.db.updateQueueStatus(entry.id, "merged", null);
            try self.db.updateTaskStatus(entry.task_id, "merged");
            try merged.append(entry.branch);

            if (self.db.getPipelineTask(self.allocator, entry.task_id) catch null) |task| {
                self.notify(task.notify_chat, std.fmt.allocPrint(self.allocator, "Task #{d} \"{s}\" merged via PR.", .{ task.id, task.title }) catch continue);
            }
        }

        // 4. Sync after merges
        if (merged.items.len > 0) {
            var sync = try gt.repoSync();
            defer sync.deinit();
            var pull2 = try git.pull();
            defer pull2.deinit();
        }

        // 5. Self-update check
        if (is_self and merged.items.len > 0) self.checkSelfUpdate(repo_path);

        // 6. Notify
        if (merged.items.len > 0) {
            const digest = try self.generateDigest(merged.items, &.{});
            self.notify(self.config.pipeline_admin_chat, digest);
            std.log.info("Graphite integration complete: {d} merged", .{merged.items.len});
        } else {
            self.notify(self.config.pipeline_admin_chat, try self.allocator.dupe(u8, "Graphite integration: no branches merged (PRs may need CI)."));
        }
    }

    fn runReleaseTrain(self: *Pipeline, queued: []db_mod.QueueEntry, repo_path: []const u8, is_self: bool) !void {
        var git = Git.init(self.allocator, repo_path);

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
                        std.log.warn("Task #{d} exhausted merge attempts, recycling to backlog", .{task.id});
                        self.recycleTask(task) catch {};
                        self.notify(task.notify_chat, std.fmt.allocPrint(self.allocator, "Task #{d} \"{s}\" exhausted merge attempts — recycling to backlog.", .{ task.id, task.title }) catch continue);
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
            const rt_test_cmd = self.config.getTestCmdForRepo(repo_path);
            const test_result = self.runTestCommandForRepo(repo_path, rt_test_cmd) catch {
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
                        std.log.warn("Task #{d} exhausted integration test attempts, recycling to backlog", .{task.id});
                        self.recycleTask(task) catch {};
                        self.notify(task.notify_chat, std.fmt.allocPrint(self.allocator, "Task #{d} \"{s}\" exhausted integration test attempts — recycling to backlog.", .{ task.id, task.title }) catch continue);
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

        // 5b. Self-update: only for the primary (self) repo
        if (is_self) self.checkSelfUpdate(repo_path);

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

    /// Periodically fetch origin and pull if the self repo has new commits.
    fn checkRemoteUpdates(self: *Pipeline) void {
        const now = std.time.timestamp();
        if (now - self.last_remote_check_ts < REMOTE_CHECK_INTERVAL_S) return;
        self.last_remote_check_ts = now;

        // Only check the primary (self) repo
        for (self.config.watched_repos) |repo| {
            if (!repo.is_self) continue;

            var git = Git.init(self.allocator, repo.path);

            var fetch_result = git.fetch("origin") catch return;
            defer fetch_result.deinit();
            if (!fetch_result.success()) return;

            const local = git.revParseHead() catch return;
            const remote = git.revParse("origin/main") catch return;

            if (std.mem.eql(u8, &local, &remote)) return;

            std.log.info("Remote update detected on {s}, pulling...", .{repo.path});
            var pull_result = git.pull() catch return;
            defer pull_result.deinit();

            if (!pull_result.success()) {
                std.log.err("Remote pull failed: {s}", .{pull_result.stderr});
                return;
            }

            self.checkSelfUpdate(repo.path);
            return;
        }
    }

    fn checkSelfUpdate(self: *Pipeline, repo_path: []const u8) void {
        var git = Git.init(self.allocator, repo_path);
        const current_head = git.revParseHead() catch return;

        const startup = self.startup_heads.get(repo_path) orelse return;
        if (std.mem.eql(u8, &current_head, &startup)) return;
        if (std.mem.eql(u8, &startup, &([_]u8{0} ** 40))) return;

        std.log.info("Self-update: main HEAD changed, rebuilding...", .{});
        self.notify(self.config.pipeline_admin_chat, self.allocator.dupe(u8, "Self-update: new commits detected, rebuilding...") catch return);

        // Run zig build in the repo
        var child = std.process.Child.init(
            &.{ "zig", "build" },
            self.allocator,
        );
        child.cwd = repo_path;
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

    fn spawnAgent(self: *Pipeline, persona: AgentPersona, prompt: []const u8, workdir: []const u8, resume_session: ?[]const u8) !agent_mod.AgentResult {
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
        if (resume_session) |sid| {
            if (sid.len > 0) {
                const esc_sid = try json_mod.escapeString(tmp, sid);
                try input.writer().print("{{\"prompt\":\"{s}\",\"systemPrompt\":\"{s}\",\"model\":\"{s}\",\"allowedTools\":\"{s}\",\"workdir\":\"/workspace/repo\",\"resumeSessionId\":\"{s}\"}}", .{
                    esc_prompt, esc_sys, self.config.model, allowed_tools, esc_sid,
                });
            } else {
                try input.writer().print("{{\"prompt\":\"{s}\",\"systemPrompt\":\"{s}\",\"model\":\"{s}\",\"allowedTools\":\"{s}\",\"workdir\":\"/workspace/repo\"}}", .{
                    esc_prompt, esc_sys, self.config.model, allowed_tools,
                });
            }
        } else {
            try input.writer().print("{{\"prompt\":\"{s}\",\"systemPrompt\":\"{s}\",\"model\":\"{s}\",\"allowedTools\":\"{s}\",\"workdir\":\"/workspace/repo\"}}", .{
                esc_prompt, esc_sys, self.config.model, allowed_tools,
            });
        }

        // Container name
        var name_buf: [128]u8 = undefined;
        const seq = struct {
            var counter = std.atomic.Value(u32).init(0);
        };
        const n = seq.counter.fetchAdd(1, .monotonic);
        const container_name = try std.fmt.bufPrint(&name_buf, "borg-{s}-{d}-{d}", .{
            @tagName(persona), std.time.timestamp(), n,
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
        try self.db.incrementTaskAttempt(task.id);
        try self.db.updateTaskError(task.id, detail[0..@min(detail.len, 4000)]);

        if (task.attempt + 1 >= task.max_attempts) {
            std.log.warn("Task #{d} failed ({s}), exhausted {d} attempts — shelving", .{ task.id, reason, task.max_attempts });
            try self.db.updateTaskStatus(task.id, "failed");
            self.cleanupWorktree(task);
            self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} failed: {s} — gave up after {d} attempts", .{ task.id, reason, task.max_attempts }));
        } else {
            std.log.warn("Task #{d} failed ({s}), recycling to backlog ({d}/{d}): {s}", .{ task.id, reason, task.attempt + 1, task.max_attempts, detail[0..@min(detail.len, 200)] });
            try self.recycleTask(task);
            self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} failed: {s} — retry {d}/{d}", .{ task.id, reason, task.attempt + 1, task.max_attempts }));
        }
    }

    fn recycleTask(self: *Pipeline, task: db_mod.PipelineTask) !void {
        try self.db.updateTaskStatus(task.id, "backlog");
        self.cleanupWorktree(task);
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

test {
    _ = @import("pipeline_shutdown_test.zig");
}
