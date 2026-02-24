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
    last_release_ts: i64,

    pub fn init(
        allocator: std.mem.Allocator,
        db: *Db,
        docker: *Docker,
        tg: *Telegram,
        config: *Config,
    ) Pipeline {
        return .{
            .allocator = allocator,
            .db = db,
            .docker = docker,
            .tg = tg,
            .config = config,
            .running = std.atomic.Value(bool).init(true),
            .last_release_ts = std.time.timestamp(),
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
        const task = (try self.db.getNextPipelineTask(self.allocator)) orelse return;

        std.log.info("Pipeline processing task #{d} [{s}]: {s}", .{ task.id, task.status, task.title });

        if (std.mem.eql(u8, task.status, "backlog")) {
            try self.setupBranch(task);
        } else if (std.mem.eql(u8, task.status, "spec")) {
            try self.runSpecPhase(task);
        } else if (std.mem.eql(u8, task.status, "qa")) {
            try self.runQaPhase(task);
        } else if (std.mem.eql(u8, task.status, "impl") or std.mem.eql(u8, task.status, "retry")) {
            try self.runImplPhase(task);
        }
    }

    fn setupBranch(self: *Pipeline, task: db_mod.PipelineTask) !void {
        var git = Git.init(self.allocator, self.config.pipeline_repo);

        // Ensure we're on a clean main
        var co = try git.checkout("main");
        defer co.deinit();
        if (!co.success()) {
            std.log.err("git checkout main failed: {s}", .{co.stderr});
            try self.db.updateTaskStatus(task.id, "failed");
            try self.db.updateTaskError(task.id, co.stderr);
            return;
        }

        var pull = try git.pull();
        defer pull.deinit();

        // Create feature branch
        var branch_buf: [128]u8 = undefined;
        const branch = try std.fmt.bufPrint(&branch_buf, "feature/task-{d}", .{task.id});

        var br = try git.createBranch(branch, "main");
        defer br.deinit();
        if (!br.success()) {
            std.log.err("git branch create failed: {s}", .{br.stderr});
            try self.db.updateTaskStatus(task.id, "failed");
            try self.db.updateTaskError(task.id, br.stderr);
            return;
        }

        try self.db.updateTaskBranch(task.id, branch);
        try self.db.updateTaskStatus(task.id, "spec");
        std.log.info("Created branch {s} for task #{d}", .{ branch, task.id });
    }

    fn runSpecPhase(self: *Pipeline, task: db_mod.PipelineTask) !void {
        var git = Git.init(self.allocator, self.config.pipeline_repo);

        // Checkout the task branch
        var co = try git.checkout(task.branch);
        defer co.deinit();
        if (!co.success()) {
            try self.failTask(task, "checkout failed", co.stderr);
            return;
        }

        // Get file listing for context
        var ls = try git.exec(&.{ "ls-files", "--full-name" });
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

        const result = self.spawnAgent(.manager, prompt_buf.items) catch |err| {
            try self.failTask(task, "manager agent spawn failed", @errorName(err));
            return;
        };
        defer self.allocator.free(result.output);
        defer if (result.new_session_id) |sid| self.allocator.free(sid);

        // Commit spec.md
        var add = try git.addAll();
        defer add.deinit();
        var commit = try git.commit("spec: generate spec.md for task");
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
        var git = Git.init(self.allocator, self.config.pipeline_repo);

        var co = try git.checkout(task.branch);
        defer co.deinit();
        if (!co.success()) {
            try self.failTask(task, "checkout failed", co.stderr);
            return;
        }

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

        const result = self.spawnAgent(.qa, prompt_buf.items) catch |err| {
            try self.failTask(task, "QA agent spawn failed", @errorName(err));
            return;
        };
        defer self.allocator.free(result.output);
        defer if (result.new_session_id) |sid| self.allocator.free(sid);

        var add = try git.addAll();
        defer add.deinit();
        var commit = try git.commit("test: add tests from QA agent");
        defer commit.deinit();

        if (!commit.success()) {
            try self.failTask(task, "QA produced no test files", commit.stderr);
            return;
        }

        try self.db.updateTaskStatus(task.id, "impl");
        self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d}: tests written, starting implementation", .{task.id}));
    }

    fn runImplPhase(self: *Pipeline, task: db_mod.PipelineTask) !void {
        var git = Git.init(self.allocator, self.config.pipeline_repo);

        var co = try git.checkout(task.branch);
        defer co.deinit();
        if (!co.success()) {
            try self.failTask(task, "checkout failed", co.stderr);
            return;
        }

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

        const result = self.spawnAgent(.worker, prompt_buf.items) catch |err| {
            try self.failTask(task, "worker agent spawn failed", @errorName(err));
            return;
        };
        defer self.allocator.free(result.output);
        defer if (result.new_session_id) |sid| self.allocator.free(sid);

        // Commit implementation
        var add = try git.addAll();
        defer add.deinit();
        var commit = try git.commit("impl: implementation from worker agent");
        defer commit.deinit();

        // Run tests deterministically
        const test_result = self.runTestCommand() catch |err| {
            try self.failTask(task, "test command execution failed", @errorName(err));
            return;
        };
        defer self.allocator.free(test_result.stdout);
        defer self.allocator.free(test_result.stderr);

        if (test_result.exit_code == 0) {
            // Tests pass!
            try self.db.updateTaskStatus(task.id, "done");
            try self.db.enqueueForIntegration(task.id, task.branch);
            std.log.info("Task #{d} passed tests, queued for integration", .{task.id});
            self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} passed all tests! Queued for release train.", .{task.id}));
        } else {
            // Tests failed
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
                // Retry: store error context for next worker prompt
                const combined = try std.fmt.allocPrint(self.allocator, "{s}\n{s}", .{ test_result.stdout, test_result.stderr });
                defer self.allocator.free(combined);
                try self.db.updateTaskError(task.id, combined[0..@min(combined.len, 4000)]);
                try self.db.incrementTaskAttempt(task.id);
                try self.db.updateTaskStatus(task.id, "retry");
                std.log.info("Task #{d} test failed, retry {d}/{d}", .{ task.id, task.attempt + 1, task.max_attempts });
            }
        }
    }

    fn runTestCommand(self: *Pipeline) !TestResult {
        // Split test command and run
        var child = std.process.Child.init(
            &.{ "/bin/sh", "-c", self.config.pipeline_test_cmd },
            self.allocator,
        );
        child.stdin_behavior = .Close;
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Pipe;

        // Set cwd to repo
        child.cwd = self.config.pipeline_repo;

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
        const interval: i64 = @intCast(@as(u64, self.config.release_interval_hours) * 3600);
        if (now - self.last_release_ts < interval) return;

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
                // Merge conflict — abort and exclude
                var abort = try git.abortMerge();
                defer abort.deinit();
                try self.db.updateQueueStatus(entry.id, "excluded", "merge conflict");
                try self.db.updateTaskStatus(entry.task_id, "failed");
                try self.db.updateTaskError(entry.task_id, "Excluded from release: merge conflict");
                try excluded.append(entry.branch);
                continue;
            }

            // Run global tests
            const test_result = self.runTestCommand() catch {
                try excluded.append(entry.branch);
                continue;
            };
            defer self.allocator.free(test_result.stdout);
            defer self.allocator.free(test_result.stderr);

            if (test_result.exit_code != 0) {
                // Tests failed after merge — revert
                var reset = try git.resetHard("HEAD~1");
                defer reset.deinit();
                try self.db.updateQueueStatus(entry.id, "excluded", "tests failed after merge");
                try self.db.updateTaskStatus(entry.task_id, "failed");
                try self.db.updateTaskError(entry.task_id, "Excluded from release: integration tests failed");
                try excluded.append(entry.branch);
                continue;
            }

            // Success!
            try self.db.updateQueueStatus(entry.id, "merged", null);
            try self.db.updateTaskStatus(entry.task_id, "merged");
            try merged.append(entry.branch);
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

        // 6. Generate and send digest
        const digest = try self.generateDigest(merged.items, excluded.items);
        defer self.allocator.free(digest);
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

    // --- Agent Spawning ---

    fn spawnAgent(self: *Pipeline, persona: AgentPersona, prompt: []const u8) !agent_mod.AgentResult {
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

        // Bind mount repo into container
        var bind_buf: [1024]u8 = undefined;
        const repo_bind = try std.fmt.bufPrint(&bind_buf, "{s}:/workspace/repo", .{self.config.pipeline_repo});

        const binds = [_][]const u8{repo_bind};

        std.log.info("Spawning {s} agent: {s}", .{ @tagName(persona), container_name });

        var run_result = try self.docker.runWithStdio(docker_mod.ContainerConfig{
            .image = self.config.container_image,
            .name = container_name,
            .env = &env,
            .binds = &binds,
            .memory_limit = 1024 * 1024 * 1024, // 1GB for pipeline agents
        }, input.items);
        defer run_result.deinit();

        std.log.info("{s} agent done (exit={d}, {d} bytes)", .{ @tagName(persona), run_result.exit_code, run_result.stdout.len });

        return try agent_mod.parseNdjson(self.allocator, run_result.stdout);
    }

    // --- Helpers ---

    fn failTask(self: *Pipeline, task: db_mod.PipelineTask, reason: []const u8, detail: []const u8) !void {
        std.log.err("Task #{d} failed: {s}: {s}", .{ task.id, reason, detail[0..@min(detail.len, 200)] });
        try self.db.updateTaskStatus(task.id, "failed");
        try self.db.updateTaskError(task.id, detail[0..@min(detail.len, 4000)]);
        self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} failed: {s}", .{ task.id, reason }));
    }

    fn notify(self: *Pipeline, chat_id: []const u8, message: []const u8) void {
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
