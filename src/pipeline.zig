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
const prompts = @import("prompts.zig");
const agent_mod = @import("agent.zig");
const Config = @import("config.zig").Config;
const web_mod = @import("web.zig");

const AGENT_TIMEOUT_S_FALLBACK = 600;

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
    force_restart: std.atomic.Value(bool),
    last_release_ts: i64,
    last_seed_ts: i64,
    last_remote_check_ts: i64,
    last_self_update_ts: i64, // non-zero = a new build is ready to deploy
    last_health_ts: i64,
    startup_heads: std.StringHashMap([40]u8),

    // Pipelining: concurrent phase processing
    active_agents: std.atomic.Value(u32),

    // Web server for live streaming
    web: ?*web_mod.WebServer = null,

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
            .force_restart = std.atomic.Value(bool).init(false),
            .last_release_ts = std.time.timestamp(),
            .last_seed_ts = 0,
            .last_remote_check_ts = 0,
            .last_self_update_ts = 0,
            .last_health_ts = 0,
            .startup_heads = heads,
            .active_agents = std.atomic.Value(u32).init(0),
        };
    }

    pub fn run(self: *Pipeline) void {
        std.log.info("Pipeline thread started for {d} repo(s)", .{self.config.watched_repos.len});

        // Clear stale dispatched_at from previous instance (ACID recovery)
        self.db.clearAllDispatched() catch {};
        self.killOrphanedContainers();

        self.processBacklogFiles();

        while (self.running.load(.acquire)) {
            self.tick() catch |err| {
                std.log.err("Pipeline tick error: {}", .{err});
            };

            self.checkIntegration() catch |err| {
                std.log.err("Integration error: {}", .{err});
            };

            self.checkRemoteUpdates();
            self.checkHealth();
            self.maybeApplySelfUpdate();

            std.time.sleep(self.config.pipeline_tick_s * std.time.ns_per_s);
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

    fn processBacklogFiles(self: *Pipeline) void {
        for (self.config.watched_repos) |repo| {
            if (!repo.is_self) continue;
            // Only import once per repo — track via DB state
            const state_key = std.fmt.allocPrint(self.allocator, "backlog_imported:{s}", .{repo.path}) catch continue;
            defer self.allocator.free(state_key);
            const already = self.db.getState(self.allocator, state_key) catch null;
            if (already) |v| { self.allocator.free(v); continue; }

            const backlog_path = std.fmt.allocPrint(self.allocator, "{s}/BACKLOG.md", .{repo.path}) catch continue;
            defer self.allocator.free(backlog_path);

            const content = std.fs.cwd().readFileAlloc(self.allocator, backlog_path, 128 * 1024) catch continue;
            defer self.allocator.free(content);
            if (std.mem.indexOf(u8, content, "TASK_START") == null) continue;

            // Parse TASK_START/TASK_END blocks directly — no LLM needed
            var created: u32 = 0;
            var remaining = content;
            while (std.mem.indexOf(u8, remaining, "TASK_START")) |start_pos| {
                remaining = remaining[start_pos + "TASK_START".len ..];
                const end_pos = std.mem.indexOf(u8, remaining, "TASK_END") orelse break;
                const block = std.mem.trim(u8, remaining[0..end_pos], &[_]u8{ ' ', '\t', '\n', '\r' });
                remaining = remaining[end_pos + "TASK_END".len ..];

                var title: []const u8 = "";
                var desc_start: usize = 0;
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
                const description = if (desc_start < block.len)
                    std.mem.trim(u8, block[desc_start..block.len], &[_]u8{ ' ', '\t', '\n', '\r' })
                else
                    title;

                _ = self.db.createPipelineTask(title, description, repo.path, "backlog", self.config.pipeline_admin_chat) catch continue;
                created += 1;
            }

            if (created == 0) continue;

            // Mark imported so we don't re-load on next restart
            self.db.setState(state_key, "1") catch {};

            std.log.info("Loaded {d} tasks from BACKLOG.md in {s}", .{ created, repo.path });
            self.notify(self.config.pipeline_admin_chat, std.fmt.allocPrint(
                self.allocator,
                "Loaded {d} tasks from BACKLOG.md",
                .{created},
            ) catch return);
        }
    }

    // Called after integration merges complete. If all backlog tasks are done and
    // BACKLOG.md still exists, open a PR to remove it.
    fn maybeCleanupBacklog(self: *Pipeline, repo_path: []const u8) void {
        const remaining = self.db.getUnmergedBacklogCount() catch return;
        if (remaining > 0) return;

        const backlog_path = std.fmt.allocPrint(self.allocator, "{s}/BACKLOG.md", .{repo_path}) catch return;
        defer self.allocator.free(backlog_path);
        std.fs.accessAbsolute(backlog_path, .{}) catch return; // file doesn't exist

        var git = Git.init(self.allocator, repo_path);
        var co = git.checkout("main") catch return;
        defer co.deinit();
        var pull = git.pull() catch return;
        defer pull.deinit();

        const branch = "remove-backlog-md";
        var cb = git.exec(&.{ "checkout", "-b", branch }) catch return;
        defer cb.deinit();
        if (!cb.success()) return;

        std.fs.deleteFileAbsolute(backlog_path) catch {};
        var add = git.exec(&.{ "add", "BACKLOG.md" }) catch return;
        defer add.deinit();
        var commit = git.exec(&.{ "commit", "-m", "chore: remove BACKLOG.md — all tasks implemented" }) catch return;
        defer commit.deinit();
        if (!commit.success()) {
            _ = git.checkout("main") catch {};
            return;
        }
        var push = git.exec(&.{ "push", "origin", branch }) catch return;
        defer push.deinit();

        const pr_cmd = "gh pr create --title \"chore: remove BACKLOG.md\" --body \"All backlog tasks have been implemented and merged. Removing BACKLOG.md.\" --base main";
        const pr = self.runTestCommandForRepo(repo_path, pr_cmd) catch {
            _ = git.checkout("main") catch {};
            return;
        };
        defer self.allocator.free(pr.stdout);
        defer self.allocator.free(pr.stderr);

        _ = git.checkout("main") catch {};

        if (pr.exit_code == 0) {
            std.log.info("Opened PR to remove BACKLOG.md: {s}", .{std.mem.trim(u8, pr.stdout, &[_]u8{ ' ', '\n' })});
        }
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

        // Track which tasks were handed off to threads (thread takes ownership)
        var dispatched = [_]bool{false} ** 20;

        for (tasks, 0..) |task, i| {
            if (self.active_agents.load(.acquire) >= self.config.pipeline_max_agents) break;

            // Skip if already in-flight (persisted in DB)
            if (self.db.isTaskDispatched(task.id)) continue;
            self.db.markTaskDispatched(task.id) catch continue;

            _ = self.active_agents.fetchAdd(1, .acq_rel);
            std.log.info("Pipeline dispatching task #{d} [{s}] in {s}: {s}", .{ task.id, task.status, task.repo_path, task.title });

            _ = std.Thread.spawn(.{}, processTaskThread, .{ self, task }) catch {
                _ = self.active_agents.fetchSub(1, .acq_rel);
                self.db.clearTaskDispatched(task.id) catch {};
                continue;
            };
            dispatched[i] = true;
        }

        // Free strings for tasks not dispatched to threads
        for (tasks, 0..) |task, i| {
            if (!dispatched[i]) task.deinit(self.allocator);
        }
    }

    fn processTaskThread(self: *Pipeline, task: db_mod.PipelineTask) void {
        defer {
            task.deinit(self.allocator);
            _ = self.active_agents.fetchSub(1, .acq_rel);
            self.db.clearTaskDispatched(task.id) catch {};
        }

        // Only run tasks for the primary (self) repo — delete stray tasks from other repos
        if (!self.isSelfRepo(task.repo_path)) {
            std.log.warn("Task #{d} targets non-primary repo {s}, deleting", .{ task.id, task.repo_path });
            self.db.deletePipelineTask(task.id) catch {};
            return;
        }

        if (std.mem.eql(u8, task.status, "backlog")) {
            self.setupBranch(task) catch |err| {
                std.log.err("Task #{d} backlog error: {}", .{ task.id, err });
            };
        } else if (std.mem.eql(u8, task.status, "spec")) {
            self.runSpecPhase(task) catch |err| {
                std.log.err("Task #{d} spec error: {}", .{ task.id, err });
            };
        } else if (std.mem.eql(u8, task.status, "qa") or std.mem.eql(u8, task.status, "qa_fix")) {
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
        const cooldown: i64 = if (self.config.continuous_mode) 1800 else self.config.pipeline_seed_cooldown_s;
        if (now - self.last_seed_ts < cooldown) return;

        // Don't seed if there are already active tasks
        const active = try self.db.getActivePipelineTaskCount();
        if (active >= self.config.pipeline_max_backlog) return;

        // Don't seed while tasks are queued for integration — wait for them to merge first
        const pending_integration = try self.db.getQueuedIntegrationCount();
        if (pending_integration > 0) return;

        // Rotate seed mode: 0=refactoring, 1=bug audit, 2=test coverage, 3=features, 4=architecture
        // Modes 3-4 produce proposals (require approval); 0-2 produce tasks (auto-execute)
        const seed_mode = blk: {
            const mode_str = self.db.getState(self.allocator, "seed_mode") catch null;
            const prev: u32 = if (mode_str) |s| std.fmt.parseInt(u32, s, 10) catch 0 else 0;
            if (mode_str) |s| self.allocator.free(s);
            const next = (prev + 1) % 5;
            var next_buf: [4]u8 = undefined;
            const next_str = std.fmt.bufPrint(&next_buf, "{d}", .{next}) catch "0";
            self.db.setState("seed_mode", next_str) catch {};
            break :blk next;
        };

        const mode_label: []const u8 = switch (seed_mode) {
            0 => "refactoring",
            1 => "bug audit",
            2 => "test coverage",
            3 => "feature discovery",
            4 => "architecture review",
            else => "refactoring",
        };
        self.last_seed_ts = now;
        self.config.refreshOAuthToken();
        std.log.info("Seed scan starting ({s} mode)", .{mode_label});

        // Seed primary repo directly, then cross-pollinate from watched repos
        var total_created: u32 = 0;
        const active_u32: u32 = @intCast(@max(active, 0));
        var primary_path: []const u8 = "";
        for (self.config.watched_repos) |repo| {
            if (repo.is_self) {
                primary_path = repo.path;
                if (active_u32 + total_created >= self.config.pipeline_max_backlog) break;
                const created = self.seedRepo(repo.path, seed_mode, active_u32 + total_created);
                total_created += created;
                break;
            }
        }
        // Cross-pollinate: analyze watched repos for ideas to bring into primary
        if (primary_path.len > 0) {
            for (self.config.watched_repos) |repo| {
                if (repo.is_self) continue;
                if (active_u32 + total_created >= self.config.pipeline_max_backlog) break;
                const created = self.seedCrossPollinate(repo.path, primary_path);
                total_created += created;
            }
        }

        if (total_created > 0) {
            std.log.info("Seed scan ({s}): created {d} task(s)/proposal(s)", .{ mode_label, total_created });
            self.notify(self.config.pipeline_admin_chat, std.fmt.allocPrint(self.allocator, "Seed scan ({s}): created {d} task(s)/proposal(s)", .{ mode_label, total_created }) catch return);
        } else {
            std.log.info("Seed scan ({s}): no results (agents may have returned empty output)", .{mode_label});
        }
    }

    fn seedRepo(self: *Pipeline, repo_path: []const u8, seed_mode: u32, current_count: u32) u32 {
        var prompt_buf = std.ArrayList(u8).init(self.allocator);
        defer prompt_buf.deinit();
        const w = prompt_buf.writer();

        switch (seed_mode) {
            0 => w.writeAll(prompts.seed_refactor) catch return 0,
            1 => w.writeAll(prompts.seed_security) catch return 0,
            2 => w.writeAll(prompts.seed_tests) catch return 0,
            3 => {
                w.writeAll(prompts.seed_features) catch return 0;
                w.writeAll(prompts.seed_proposal_suffix) catch return 0;
                return self.seedRepoProposals(repo_path, repo_path, prompt_buf.items);
            },
            4 => {
                w.writeAll(prompts.seed_architecture) catch return 0;
                w.writeAll(prompts.seed_proposal_suffix) catch return 0;
                return self.seedRepoProposals(repo_path, repo_path, prompt_buf.items);
            },
            else => w.writeAll(prompts.seed_refactor) catch return 0,
        }

        w.writeAll(prompts.seed_task_suffix) catch return 0;

        const result = self.spawnAgent(.manager, prompt_buf.items, repo_path, null, 0) catch |err| {
            std.log.err("Seed agent failed for {s}: {}", .{ repo_path, err });
            return 0;
        };
        defer self.allocator.free(result.output);
        defer self.allocator.free(result.raw_stream);
        defer if (result.new_session_id) |sid| self.allocator.free(sid);

        self.db.storeTaskOutputFull(0, "seed", result.output, result.raw_stream, 0) catch {};

        if (result.output.len == 0) {
            std.log.warn("Seed agent returned empty output for {s} ({d} raw bytes)", .{ repo_path, result.raw_stream.len });
            return 0;
        }

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
            if (current_count + created >= self.config.pipeline_max_backlog) break;
        }

        return created;
    }

    fn seedRepoProposals(self: *Pipeline, source_repo: []const u8, target_repo: []const u8, prompt: []const u8) u32 {
        const result = self.spawnAgent(.manager, prompt, source_repo, null, 0) catch |err| {
            std.log.err("Seed proposal agent failed for {s}: {}", .{ source_repo, err });
            return 0;
        };
        defer self.allocator.free(result.output);
        defer self.allocator.free(result.raw_stream);
        defer if (result.new_session_id) |sid| self.allocator.free(sid);

        self.db.storeTaskOutputFull(0, "seed_proposals", result.output, result.raw_stream, 0) catch {};

        if (result.output.len == 0) {
            std.log.warn("Seed proposal agent returned empty output for {s} ({d} raw bytes)", .{ source_repo, result.raw_stream.len });
            return 0;
        }

        var created: u32 = 0;
        var remaining = result.output;
        while (std.mem.indexOf(u8, remaining, "PROPOSAL_START")) |start_pos| {
            remaining = remaining[start_pos + "PROPOSAL_START".len ..];
            const end_pos = std.mem.indexOf(u8, remaining, "PROPOSAL_END") orelse break;
            const block = std.mem.trim(u8, remaining[0..end_pos], &[_]u8{ ' ', '\t', '\n', '\r' });
            remaining = remaining[end_pos + "PROPOSAL_END".len ..];

            var title: []const u8 = "";
            var description: []const u8 = "";
            var rationale: []const u8 = "";

            var lines = std.mem.splitScalar(u8, block, '\n');
            while (lines.next()) |line| {
                const trimmed = std.mem.trim(u8, line, &[_]u8{ ' ', '\t', '\r' });
                if (std.mem.startsWith(u8, trimmed, "TITLE:")) {
                    title = std.mem.trim(u8, trimmed["TITLE:".len..], &[_]u8{ ' ', '\t' });
                } else if (std.mem.startsWith(u8, trimmed, "DESCRIPTION:")) {
                    description = std.mem.trim(u8, trimmed["DESCRIPTION:".len..], &[_]u8{ ' ', '\t' });
                } else if (std.mem.startsWith(u8, trimmed, "RATIONALE:")) {
                    rationale = std.mem.trim(u8, trimmed["RATIONALE:".len..], &[_]u8{ ' ', '\t' });
                }
            }

            if (title.len == 0) continue;

            _ = self.db.createProposal(target_repo, title, description, rationale) catch |err| {
                std.log.err("Failed to create proposal: {}", .{err});
                continue;
            };
            created += 1;
            std.log.info("Proposal: {s}", .{title});
        }

        if (created > 0) {
            std.log.info("Created {d} proposal(s) for {s} (from {s})", .{ created, target_repo, source_repo });
        }
        return 0; // proposals don't count toward task backlog
    }

    fn seedCrossPollinate(self: *Pipeline, watched_repo: []const u8, primary_repo: []const u8) u32 {
        var prompt_buf = std.ArrayList(u8).init(self.allocator);
        defer prompt_buf.deinit();
        const w = prompt_buf.writer();

        w.writeAll(prompts.seed_cross_pollinate) catch return 0;

        // Describe the target project (primary repo) so the agent knows what to suggest for
        const primary_name = std.fs.path.basename(primary_repo);
        w.print("Project: {s} (at {s})\n\n", .{ primary_name, primary_repo }) catch return 0;
        w.writeAll(prompts.seed_proposal_suffix) catch return 0;

        return self.seedRepoProposals(watched_repo, primary_repo, prompt_buf.items);
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
        // Always delete the directory and prune to clear corrupted worktree entries
        // (old Graphite code left full .git dirs instead of .git files)
        std.fs.deleteTreeAbsolute(wt_path) catch {};
        var prune = try git.exec(&.{ "worktree", "prune" });
        defer prune.deinit();
        var del_branch = try git.exec(&.{ "branch", "-D", branch });
        defer del_branch.deinit();

        var wt = try git.exec(&.{ "worktree", "add", wt_path, "-b", branch, "origin/main" });
        defer wt.deinit();
        if (!wt.success()) {
            std.log.err("git worktree add failed for task #{d}: {s}", .{ task.id, wt.stderr });
            try self.db.updateTaskError(task.id, wt.stderr);
            return;
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
        // Clean up per-task session dir
        const sess_path = std.fmt.allocPrint(self.allocator, "store/sessions/task-{d}", .{task.id}) catch return;
        defer self.allocator.free(sess_path);
        std.fs.cwd().deleteTree(sess_path) catch {};
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

        try w.print(prompts.spec_phase, .{ task.id, task.title, task.description });
        try w.writeAll(ls.stdout[0..@min(ls.stdout.len, 4000)]);
        try w.writeAll(prompts.spec_phase_suffix);

        const resume_sid = if (task.session_id.len > 0) task.session_id else null;
        const result = self.spawnAgent(.manager, prompt_buf.items, wt_path, resume_sid, task.id) catch |err| {
            try self.failTask(task, "manager agent spawn failed", @errorName(err));
            return;
        };
        defer self.allocator.free(result.output);
        defer self.allocator.free(result.raw_stream);

        // Store session for next phase
        if (result.new_session_id) |sid| {
            self.db.setTaskSessionId(task.id, sid) catch {};
            self.allocator.free(sid);
        }

        self.db.storeTaskOutputFull(task.id, "spec", result.output, result.raw_stream, 0) catch {};

        // Check spec.md was actually written
        const spec_path = try std.fmt.allocPrint(self.allocator, "{s}/spec.md", .{wt_path});
        defer self.allocator.free(spec_path);
        const spec_exists = blk: {
            std.fs.accessAbsolute(spec_path, .{}) catch break :blk false;
            break :blk true;
        };
        if (!spec_exists and result.output.len == 0) {
            try self.failTask(task, "manager produced no output", "no spec.md and empty result");
            return;
        }

        // Store spec.md content as diff (spec.md stays gitignored, not committed)
        if (spec_exists) {
            const spec_content = std.fs.cwd().readFileAlloc(self.allocator, spec_path, 64 * 1024) catch null;
            if (spec_content) |content| {
                defer self.allocator.free(content);
                self.db.storeTaskOutput(task.id, "spec_diff", content, 0) catch {};
            }
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

        try w.writeAll(prompts.qa_phase);

        if (std.mem.eql(u8, task.status, "qa_fix") and task.last_error.len > 0) {
            const err_tail = if (task.last_error.len > 3000) task.last_error[task.last_error.len - 3000 ..] else task.last_error;
            try w.print(prompts.qa_fix_fmt, .{err_tail});
        }

        // qa_fix gets a fresh session since the impl agent overwrote the QA session
        const resume_sid = if (std.mem.eql(u8, task.status, "qa_fix")) null else if (task.session_id.len > 0) task.session_id else null;
        const result = self.spawnAgent(.qa, prompt_buf.items, wt_path, resume_sid, task.id) catch |err| {
            try self.failTask(task, "QA agent spawn failed", @errorName(err));
            return;
        };
        defer self.allocator.free(result.output);
        defer self.allocator.free(result.raw_stream);

        if (result.new_session_id) |sid| {
            self.db.setTaskSessionId(task.id, sid) catch {};
            self.allocator.free(sid);
        }

        self.db.storeTaskOutputFull(task.id, "qa", result.output, result.raw_stream, 0) catch {};

        var add = try wt_git.addAll();
        defer add.deinit();
        const is_fix = std.mem.eql(u8, task.status, "qa_fix");
        const commit_msg = if (is_fix) "test: fix tests from QA agent" else "test: add tests from QA agent";
        var commit = try wt_git.commitWithAuthor(commit_msg, self.config.git_author);
        defer commit.deinit();

        if (!commit.success()) {
            if (is_fix) {
                try self.failTask(task, "QA fix produced no changes", commit.stderr);
            } else {
                try self.failTask(task, "QA produced no test files", commit.stderr);
            }
            return;
        }

        const diff_phase = if (is_fix) "qa_fix_diff" else "qa_diff";
        var qa_diff = try wt_git.exec(&.{ "diff", "HEAD~1" });
        defer qa_diff.deinit();
        if (qa_diff.success()) self.db.storeTaskOutput(task.id, diff_phase, qa_diff.stdout, 0) catch {};

        try self.db.updateTaskStatus(task.id, "impl");
        if (is_fix) {
            self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d}: QA fixed tests, retrying implementation", .{task.id}));
        } else {
            self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d}: tests written, starting implementation", .{task.id}));
        }
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
                // Check if the branch has any changes vs main
                var diff_check = try wt_git.exec(&.{ "diff", "--stat", "origin/main..HEAD" });
                defer diff_check.deinit();
                const has_changes = diff_check.success() and std.mem.trim(u8, diff_check.stdout, " \t\r\n").len > 0;

                if (has_changes) {
                    try self.db.updateTaskStatus(task.id, "done");
                    try self.db.enqueueForIntegration(task.id, task.branch, task.repo_path);
                    self.cleanupWorktree(task);
                    std.log.info("Task #{d} tests already pass, queued for integration", .{task.id});
                    self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} passed all tests! Queued for integration.", .{task.id}));
                } else {
                    try self.db.updateTaskStatus(task.id, "merged");
                    self.cleanupWorktree(task);
                    std.log.info("Task #{d} tests already pass with no changes, marking as merged", .{task.id});
                }
                return;
            }
        } else |_| {}

        // Build prompt with error context for retries
        var prompt_buf = std.ArrayList(u8).init(self.allocator);
        defer prompt_buf.deinit();
        const w = prompt_buf.writer();

        try w.writeAll(prompts.impl_phase);

        if (std.mem.eql(u8, task.status, "retry") and task.last_error.len > 0) {
            const err_tail = if (task.last_error.len > 3000) task.last_error[task.last_error.len - 3000 ..] else task.last_error;
            try w.print(prompts.impl_retry_fmt, .{err_tail});
        }

        const resume_sid = if (task.session_id.len > 0) task.session_id else null;
        const result = self.spawnAgent(.worker, prompt_buf.items, wt_path, resume_sid, task.id) catch |err| {
            try self.failTask(task, "worker agent spawn failed", @errorName(err));
            return;
        };
        defer self.allocator.free(result.output);
        defer self.allocator.free(result.raw_stream);

        if (result.new_session_id) |sid| {
            self.db.setTaskSessionId(task.id, sid) catch {};
            self.allocator.free(sid);
        }

        self.db.storeTaskOutputFull(task.id, "impl", result.output, result.raw_stream, 0) catch {};

        // Commit implementation in worktree
        var add = try wt_git.addAll();
        defer add.deinit();
        var commit = try wt_git.commitWithAuthor("impl: implementation from worker agent", self.config.git_author);
        defer commit.deinit();

        if (commit.success()) {
            var impl_diff = try wt_git.exec(&.{ "diff", "HEAD~1" });
            defer impl_diff.deinit();
            if (impl_diff.success()) self.db.storeTaskOutput(task.id, "impl_diff", impl_diff.stdout, 0) catch {};
        }

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
            self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} passed all tests! Queued for integration.", .{task.id}));
        } else {
            if (task.attempt + 1 >= task.max_attempts) {
                const out = test_result.stdout[0..@min(test_result.stdout.len, 2000)];
                const err = test_result.stderr[0..@min(test_result.stderr.len, 2000)];
                const combined = if (out.len > 0 and err.len > 0)
                    try std.fmt.allocPrint(self.allocator, "stdout:\n{s}\nstderr:\n{s}", .{ out, err })
                else if (out.len > 0)
                    try std.fmt.allocPrint(self.allocator, "{s}", .{out})
                else if (err.len > 0)
                    try std.fmt.allocPrint(self.allocator, "{s}", .{err})
                else
                    try std.fmt.allocPrint(self.allocator, "tests failed (no output)", .{});
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

                // After 2+ impl attempts, check if the error is in test files themselves
                if (task.attempt >= 1 and isTestFileError(test_result.stderr, test_result.stdout)) {
                    try self.db.updateTaskStatus(task.id, "qa_fix");
                    self.db.setTaskSessionId(task.id, "") catch {};
                    std.log.info("Task #{d} test error appears to be in test files, routing to QA fix ({d}/{d})", .{ task.id, task.attempt + 1, task.max_attempts });
                    self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} test code has bugs — sending back to QA for fix ({d}/{d})", .{ task.id, task.attempt + 1, task.max_attempts }));
                } else {
                    try self.db.updateTaskStatus(task.id, "retry");
                    std.log.info("Task #{d} test failed, retry {d}/{d}", .{ task.id, task.attempt + 1, task.max_attempts });
                }
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

        // Valid worktree has .git as a FILE (gitdir pointer), not a directory.
        // A .git directory means the agent ran git init and corrupted it.
        const wt_valid = blk: {
            std.fs.accessAbsolute(wt_path, .{}) catch break :blk false;
            const git_sub = try std.fmt.allocPrint(self.allocator, "{s}/.git", .{wt_path});
            defer self.allocator.free(git_sub);
            var d = std.fs.openDirAbsolute(git_sub, .{}) catch break :blk true; // not a dir = valid
            d.close();
            std.log.warn("Task #{d}: worktree {s} has corrupted .git dir, rebuilding", .{ task.id, wt_path });
            break :blk false;
        };
        if (!wt_valid) {
            var repo_git = Git.init(self.allocator, task.repo_path);
            const wt_dir = try std.fmt.allocPrint(self.allocator, "{s}/.worktrees", .{task.repo_path});
            defer self.allocator.free(wt_dir);
            std.fs.makeDirAbsolute(wt_dir) catch {};

            // Clear any stale/corrupted worktree entries before creating a new one
            std.fs.deleteTreeAbsolute(wt_path) catch {};
            var prune = try repo_git.exec(&.{ "worktree", "prune" });
            defer prune.deinit();

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

            try w.writeAll(prompts.rebase_phase);

            if (task.last_error.len > 0) {
                const err_tail = if (task.last_error.len > 2000) task.last_error[task.last_error.len - 2000 ..] else task.last_error;
                try w.print(prompts.rebase_error_fmt, .{err_tail});
            }

            // Run on host (not Docker) — rebase needs full git repo access
            // Don't pass Docker session ID — host agent can't resume Docker sessions
            // (different HOME and project path hash). It will start fresh.
            const result = self.spawnAgentHost(prompt_buf.items, wt_path, null, task.id) catch |err| {
                try self.failTask(task, "rebase: worker agent failed", @errorName(err));
                return;
            };
            defer self.allocator.free(result.output);
            defer self.allocator.free(result.raw_stream);

            if (result.new_session_id) |sid| {
                self.db.setTaskSessionId(task.id, sid) catch {};
                self.allocator.free(sid);
            }

            self.db.storeTaskOutputFull(task.id, "rebase", result.output, result.raw_stream, 0) catch {};
        }

        // Verify the agent actually completed the rebase before doing anything else.
        // Agents exit 0 even when they fail; without this check we push the old tip.
        var rb_verify = try wt_git.exec(&.{ "merge-base", "--is-ancestor", "origin/main", task.branch });
        defer rb_verify.deinit();
        if (!rb_verify.success()) {
            std.log.warn("Task #{d}: branch still not rebased after agent ({d}/{d})", .{ task.id, task.attempt + 1, task.max_attempts });
            if (task.attempt + 1 >= task.max_attempts) {
                std.log.warn("Task #{d} exhausted rebase attempts, recycling to backlog", .{task.id});
                try self.recycleTask(task);
            } else {
                try self.db.incrementTaskAttempt(task.id);
            }
            return;
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
            // Push the rebased branch — use --force since rebase rewrites history
            var push_r = try wt_git.exec(&.{ "push", "--force", "origin", task.branch });
            defer push_r.deinit();
            if (!push_r.success()) {
                // "cannot lock ref" — delete remote branch and retry
                if (std.mem.indexOf(u8, push_r.stderr, "cannot lock ref") != null) {
                    std.log.info("Task #{d}: cannot lock ref, deleting remote branch and retrying", .{task.id});
                    var del = try wt_git.exec(&.{ "push", "origin", "--delete", task.branch });
                    defer del.deinit();
                    var push2 = try wt_git.exec(&.{ "push", "--force", "origin", task.branch });
                    defer push2.deinit();
                    if (push2.success()) {
                        try self.db.updateTaskStatus(task.id, "done");
                        try self.db.enqueueForIntegration(task.id, task.branch, task.repo_path);
                        self.cleanupWorktree(task);
                        std.log.info("Task #{d} rebased and re-queued for integration (after ref fix)", .{task.id});
                        return;
                    }
                }
                std.log.err("Task #{d} rebase push failed: {s}", .{ task.id, push_r.stderr[0..@min(push_r.stderr.len, 200)] });
                return;
            }

            try self.db.updateTaskStatus(task.id, "done");
            try self.db.enqueueForIntegration(task.id, task.branch, task.repo_path);
            self.cleanupWorktree(task);
            std.log.info("Task #{d} rebased and re-queued for integration", .{task.id});
            self.notify(task.notify_chat, std.fmt.allocPrint(self.allocator, "Task #{d} \"{s}\" rebased successfully, re-queued for release.", .{ task.id, task.title }) catch return);
        } else {
            // Tests still fail after rebase — retry if attempts remain
            if (task.attempt + 1 >= task.max_attempts) {
                const out = test_result.stdout[0..@min(test_result.stdout.len, 2000)];
                const err = test_result.stderr[0..@min(test_result.stderr.len, 2000)];
                const combined = if (out.len > 0 and err.len > 0)
                    try std.fmt.allocPrint(self.allocator, "stdout:\n{s}\nstderr:\n{s}", .{ out, err })
                else if (out.len > 0)
                    try std.fmt.allocPrint(self.allocator, "{s}", .{out})
                else if (err.len > 0)
                    try std.fmt.allocPrint(self.allocator, "{s}", .{err})
                else
                    try std.fmt.allocPrint(self.allocator, "tests failed (no output)", .{});
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

    // --- Macro Loop: Integration ---

    fn checkIntegration(self: *Pipeline) !void {
        const now = std.time.timestamp();
        const min_interval: i64 = 60; // never fire more than once per minute
        if (!self.config.continuous_mode) {
            const interval: i64 = @intCast(@as(u64, self.config.release_interval_mins) * 60);
            if (now - self.last_release_ts < interval) return;
        } else if (now - self.last_release_ts < min_interval) {
            return;
        }

        var ran_any = false;
        for (self.config.watched_repos) |repo| {
            var arena = std.heap.ArenaAllocator.init(self.allocator);
            defer arena.deinit();
            const queued = self.db.getQueuedBranchesForRepo(arena.allocator(), repo.path) catch continue;
            if (queued.len == 0) continue;

            std.log.info("Integration: {d} branches for {s}", .{ queued.len, repo.path });
            self.runIntegration(queued, repo.path, repo.auto_merge) catch |err| {
                std.log.err("Integration error for {s}: {}", .{ repo.path, err });
            };
            ran_any = true;
        }

        if (ran_any) {
            self.last_release_ts = std.time.timestamp();
        }
    }

    fn runIntegration(self: *Pipeline, queued: []db_mod.QueueEntry, repo_path: []const u8, auto_merge: bool) !void {
        var git = Git.init(self.allocator, repo_path);

        // 1. Ensure main is checked out and up to date
        var co = try git.checkout("main");
        defer co.deinit();
        var pull_r = try git.pull();
        defer pull_r.deinit();

        // 2. Filter stale queue entries whose branches no longer exist locally
        var live = std.ArrayList(db_mod.QueueEntry).init(self.allocator);
        defer live.deinit();
        for (queued) |entry| {
            var check = git.exec(&.{ "rev-parse", "--verify", entry.branch }) catch {
                try self.db.updateQueueStatus(entry.id, "excluded", "branch not found");
                continue;
            };
            defer check.deinit();
            if (!check.success()) {
                std.log.warn("Excluding {s} from integration: branch not found", .{entry.branch});
                try self.db.updateQueueStatus(entry.id, "excluded", "branch not found");
                continue;
            }
            try live.append(entry);
        }
        if (live.items.len == 0) return;

        // 3. Sort by task_id ascending (oldest first)
        std.mem.sort(db_mod.QueueEntry, live.items, {}, struct {
            fn lt(_: void, a: db_mod.QueueEntry, b: db_mod.QueueEntry) bool {
                return a.task_id < b.task_id;
            }
        }.lt);

        // 4. Push each branch to origin and create PR if one doesn't exist yet.
        // Track which entries were excluded or freshly pushed so step 5 handles them.
        var excluded_ids = std.AutoHashMap(i64, void).init(self.allocator);
        defer excluded_ids.deinit();
        var freshly_pushed = std.AutoHashMap(i64, void).init(self.allocator);
        defer freshly_pushed.deinit();

        for (live.items) |entry| {
            // Check if PR is already merged on GitHub (avoids pointless rebase cycles)
            const state_cmd = try std.fmt.allocPrint(self.allocator, "gh pr view {s} --json state --jq .state 2>/dev/null", .{entry.branch});
            defer self.allocator.free(state_cmd);
            if (self.runTestCommandForRepo(repo_path, state_cmd) catch null) |state_result| {
                defer self.allocator.free(state_result.stdout);
                defer self.allocator.free(state_result.stderr);
                const state = std.mem.trim(u8, state_result.stdout, " \t\r\n");
                if (std.mem.eql(u8, state, "MERGED")) {
                    std.log.info("Task #{d} {s}: PR already merged, cleaning up", .{ entry.task_id, entry.branch });
                    self.db.updateQueueStatus(entry.id, "merged", null) catch {};
                    self.db.updateTaskStatus(entry.task_id, "merged") catch {};
                    excluded_ids.put(entry.id, {}) catch {};
                    continue;
                }
            }

            // Reject branches that aren't rebased on top of current main
            var rb_check = git.exec(&.{ "merge-base", "--is-ancestor", "origin/main", entry.branch }) catch null;
            if (rb_check) |*r| {
                defer r.deinit();
                if (!r.success()) {
                    std.log.info("Task #{d}: {s} not rebased on main, sending back to rebase", .{ entry.task_id, entry.branch });
                    try self.db.updateQueueStatus(entry.id, "excluded", "branch not rebased on main");
                    try self.db.updateTaskStatus(entry.task_id, "rebase");
                    excluded_ids.put(entry.id, {}) catch {};
                    continue;
                }
            }

            // Push branch — after rebase, use --force to overwrite the old remote
            var push = try git.exec(&.{ "push", "--force", "origin", entry.branch });
            defer push.deinit();
            if (!push.success()) {
                // "cannot lock ref" — delete remote branch and retry
                if (std.mem.indexOf(u8, push.stderr, "cannot lock ref") != null) {
                    var del = try git.exec(&.{ "push", "origin", "--delete", entry.branch });
                    defer del.deinit();
                    var push2 = try git.exec(&.{ "push", "--force", "origin", entry.branch });
                    defer push2.deinit();
                    if (push2.success()) {
                        std.log.info("Pushed {s} after deleting stale remote ref", .{entry.branch});
                    } else {
                        std.log.warn("Failed to push {s} after ref fix: {s}", .{ entry.branch, push2.stderr[0..@min(push2.stderr.len, 200)] });
                        continue;
                    }
                } else {
                    std.log.warn("Failed to push {s}: {s}", .{ entry.branch, push.stderr[0..@min(push.stderr.len, 200)] });
                    continue;
                }
            }

            // Check if PR already exists
            const view_cmd = try std.fmt.allocPrint(self.allocator, "gh pr view {s} --json number --jq .number 2>/dev/null", .{entry.branch});
            defer self.allocator.free(view_cmd);
            const view_result = self.runTestCommandForRepo(repo_path, view_cmd) catch continue;
            defer self.allocator.free(view_result.stdout);
            defer self.allocator.free(view_result.stderr);
            if (view_result.exit_code == 0 and std.mem.trim(u8, view_result.stdout, &[_]u8{ ' ', '\n' }).len > 0) {
                // PR exists — check if push actually changed something
                if (std.mem.indexOf(u8, push.stderr, "Everything up-to-date") == null) {
                    // Branch was updated, give GitHub time to recompute mergeability
                    freshly_pushed.put(entry.id, {}) catch {};
                }
                continue;
            }

            // Get task title for PR (sanitized for shell double-quote context)
            var title_buf = std.ArrayList(u8).init(self.allocator);
            defer title_buf.deinit();
            try title_buf.appendSlice(entry.branch);
            if (self.db.getPipelineTask(self.allocator, entry.task_id) catch null) |task| {
                defer task.deinit(self.allocator);
                title_buf.clearRetainingCapacity();
                for (task.title[0..@min(task.title.len, 100)]) |c| {
                    switch (c) {
                        '"', '\\', '$', '`' => try title_buf.append(' '),
                        else => try title_buf.append(c),
                    }
                }
            }

            const create_cmd = try std.fmt.allocPrint(
                self.allocator,
                "gh pr create --base main --head {s} --title \"{s}\" --body \"Automated implementation.\"",
                .{ entry.branch, title_buf.items },
            );
            defer self.allocator.free(create_cmd);
            const create_result = self.runTestCommandForRepo(repo_path, create_cmd) catch continue;
            defer self.allocator.free(create_result.stdout);
            defer self.allocator.free(create_result.stderr);
            if (create_result.exit_code != 0) {
                const err_text = create_result.stderr[0..@min(create_result.stderr.len, 300)];
                if (std.mem.indexOf(u8, err_text, "No commits between") != null) {
                    std.log.info("Task #{d} {s}: no commits vs main, marking as merged", .{ entry.task_id, entry.branch });
                    self.db.updateQueueStatus(entry.id, "merged", null) catch {};
                    self.db.updateTaskStatus(entry.task_id, "merged") catch {};
                    excluded_ids.put(entry.id, {}) catch {};
                    continue;
                }
                std.log.warn("gh pr create {s}: {s}", .{ entry.branch, err_text });
            } else {
                std.log.info("Created PR for {s}", .{entry.branch});
                // New PR — GitHub needs time to compute mergeability
                freshly_pushed.put(entry.id, {}) catch {};
            }
        }

        // 5. Merge ready PRs in task_id order (skip when manual merge mode)
        var merged = std.ArrayList([]const u8).init(self.allocator);
        defer merged.deinit();

        if (!auto_merge) {
            // Manual merge mode: PRs are created and kept rebased, but not merged
            for (live.items) |entry| {
                if (excluded_ids.contains(entry.id)) continue;
                try self.db.updateQueueStatus(entry.id, "pending_review", null);
                std.log.info("Task #{d} {s}: PR ready for manual review", .{ entry.task_id, entry.branch });
            }
        } else {
            for (live.items) |entry| {
                if (excluded_ids.contains(entry.id)) continue;
                // Skip freshly pushed branches — GitHub needs time to compute mergeability
                if (freshly_pushed.contains(entry.id)) {
                    std.log.info("Task #{d} {s}: skipping merge check (just pushed), will check next tick", .{ entry.task_id, entry.branch });
                    continue;
                }
                // Check PR state — detect already-merged PRs
                const view_cmd = try std.fmt.allocPrint(self.allocator, "gh pr view {s} --json number,state --jq '.state'", .{entry.branch});
                defer self.allocator.free(view_cmd);
                const view_result = self.runTestCommandForRepo(repo_path, view_cmd) catch continue;
                defer self.allocator.free(view_result.stdout);
                defer self.allocator.free(view_result.stderr);
                if (view_result.exit_code != 0) continue;
                const pr_state = std.mem.trim(u8, view_result.stdout, " \t\r\n");

                if (std.mem.eql(u8, pr_state, "MERGED")) {
                    std.log.info("Task #{d} {s}: PR already merged on GitHub", .{ entry.task_id, entry.branch });
                    try self.db.updateQueueStatus(entry.id, "merged", null);
                    try self.db.updateTaskStatus(entry.task_id, "merged");
                    try merged.append(entry.branch);
                    continue;
                }

                // Check GitHub's async mergeability before attempting merge
                var force_merge = false;
                const mb_cmd = try std.fmt.allocPrint(self.allocator, "gh pr view {s} --json mergeable --jq .mergeable", .{entry.branch});
                defer self.allocator.free(mb_cmd);
                const mb_result = self.runTestCommandForRepo(repo_path, mb_cmd) catch continue;
                defer self.allocator.free(mb_result.stdout);
                defer self.allocator.free(mb_result.stderr);
                const mb_status = std.mem.trim(u8, mb_result.stdout, " \t\r\n");
                if (std.mem.eql(u8, mb_status, "UNKNOWN")) {
                    const retries = self.db.getUnknownRetries(entry.id);
                    if (retries >= 5) {
                        std.log.warn("Task #{d} {s}: mergeability UNKNOWN after {d} retries, attempting merge anyway", .{ entry.task_id, entry.branch, retries });
                        self.db.resetUnknownRetries(entry.id) catch {};
                        force_merge = true;
                    } else {
                        self.db.incrementUnknownRetries(entry.id) catch {};
                        std.log.info("Task #{d} {s}: mergeability UNKNOWN ({d}/5), retrying next tick", .{ entry.task_id, entry.branch, retries + 1 });
                        continue;
                    }
                }
                if (!force_merge and !std.mem.eql(u8, mb_status, "MERGEABLE")) {
                    std.log.info("Task #{d} {s}: mergeable={s}, sending back to rebase", .{ entry.task_id, entry.branch, mb_status });
                    try self.db.updateQueueStatus(entry.id, "excluded", "merge conflict with main");
                    try self.db.updateTaskStatus(entry.task_id, "rebase");
                    continue;
                }

                try self.db.updateQueueStatus(entry.id, "merging", null);
                const merge_cmd = try std.fmt.allocPrint(self.allocator, "gh pr merge {s} --squash --delete-branch", .{entry.branch});
                defer self.allocator.free(merge_cmd);
                const merge_result = self.runTestCommandForRepo(repo_path, merge_cmd) catch {
                    try self.db.updateQueueStatus(entry.id, "queued", null);
                    continue;
                };
                defer self.allocator.free(merge_result.stdout);
                defer self.allocator.free(merge_result.stderr);

                if (merge_result.exit_code != 0) {
                    std.log.warn("gh pr merge {s}: {s}", .{ entry.branch, merge_result.stderr[0..@min(merge_result.stderr.len, 200)] });
                    const needs_rebase = std.mem.indexOf(u8, merge_result.stderr, "not mergeable") != null or
                        std.mem.indexOf(u8, merge_result.stderr, "cannot be cleanly created") != null;
                    if (needs_rebase) {
                        try self.db.updateQueueStatus(entry.id, "excluded", "merge conflict with main");
                        try self.db.updateTaskStatus(entry.task_id, "rebase");
                        std.log.info("Task #{d} has conflicts with main, sent back to rebase", .{entry.task_id});
                    } else {
                        try self.db.updateQueueStatus(entry.id, "queued", null);
                    }
                    continue;
                }

                try self.db.updateQueueStatus(entry.id, "merged", null);
                try self.db.updateTaskStatus(entry.task_id, "merged");
                try merged.append(entry.branch);

                if (self.db.getPipelineTask(self.allocator, entry.task_id) catch null) |task| {
                    defer task.deinit(self.allocator);
                    self.notify(task.notify_chat, std.fmt.allocPrint(self.allocator, "Task #{d} \"{s}\" merged via PR.", .{ task.id, task.title }) catch continue);
                }
            }
        }

        // 6. Pull after merges
        if (merged.items.len > 0) {
            var pull2 = try git.pull();
            defer pull2.deinit();
        }

        // 7. Check if backlog is fully done
        if (merged.items.len > 0) self.maybeCleanupBacklog(repo_path);

        // 8. Notify
        if (merged.items.len > 0) {
            const digest = try self.generateDigest(merged.items);
            self.notify(self.config.pipeline_admin_chat, digest);
            std.log.info("Integration complete: {d} merged", .{merged.items.len});
        }
    }

    fn generateDigest(self: *Pipeline, merged: [][]const u8) ![]const u8 {
        var buf = std.ArrayList(u8).init(self.allocator);
        const w = buf.writer();

        try w.print("*{d} PR(s) merged*\n", .{merged.len});
        for (merged) |branch| {
            try w.print("  + {s}\n", .{branch});
        }

        return buf.toOwnedSlice();
    }

    // --- Health Check ---

    const HEALTH_INTERVAL_S: i64 = 1800; // 30 minutes

    fn checkHealth(self: *Pipeline) void {
        const now = std.time.timestamp();
        if (now - self.last_health_ts < HEALTH_INTERVAL_S) return;
        self.last_health_ts = now;

        for (self.config.watched_repos) |repo| {
            if (!repo.is_self) continue;
            // Pull latest main before testing
            var git = Git.init(self.allocator, repo.path);
            var co = git.checkout("main") catch continue;
            defer co.deinit();
            var pull = git.pull() catch continue;
            defer pull.deinit();

            // Run build (zig build / make build)
            const build_cmd = blk: {
                if (std.mem.indexOf(u8, repo.test_cmd, "zig build")) |_| {
                    break :blk "zig build";
                } else if (std.mem.startsWith(u8, repo.test_cmd, "make")) {
                    break :blk "make";
                } else {
                    break :blk repo.test_cmd; // fallback: just run the test cmd
                }
            };

            const build_result = self.runTestCommandForRepo(repo.path, build_cmd) catch continue;
            defer self.allocator.free(build_result.stdout);
            defer self.allocator.free(build_result.stderr);

            if (build_result.exit_code != 0) {
                std.log.warn("Health: build failed for {s}", .{repo.path});
                self.createHealthTask(repo.path, "build", build_result.stderr);
                continue;
            }

            // Run tests
            const test_result = self.runTestCommandForRepo(repo.path, repo.test_cmd) catch continue;
            defer self.allocator.free(test_result.stdout);
            defer self.allocator.free(test_result.stderr);

            if (test_result.exit_code != 0) {
                std.log.warn("Health: tests failed for {s}", .{repo.path});
                self.createHealthTask(repo.path, "tests", test_result.stderr);
            } else {
                std.log.info("Health: {s} OK", .{repo.path});
            }
        }
    }

    fn createHealthTask(self: *Pipeline, repo_path: []const u8, kind: []const u8, stderr: []const u8) void {
        // Don't create duplicates — check if a health fix task already exists
        const tasks = self.db.getActivePipelineTasks(self.allocator, 50) catch return;
        defer {
            for (tasks) |t| t.deinit(self.allocator);
            self.allocator.free(tasks);
        }
        for (tasks) |t| {
            if (std.mem.startsWith(u8, t.title, "Fix failing ") and std.mem.eql(u8, t.repo_path, repo_path)) return;
        }

        const tail = if (stderr.len > 500) stderr[stderr.len - 500 ..] else stderr;
        const desc = std.fmt.allocPrint(self.allocator, "Health check detected {s} failure on main branch.\n\nError output:\n```\n{s}\n```", .{ kind, tail }) catch return;
        defer self.allocator.free(desc);

        const title = std.fmt.allocPrint(self.allocator, "Fix failing {s} on main", .{kind}) catch return;
        defer self.allocator.free(title);

        _ = self.db.createPipelineTask(title, desc, repo_path, "health-check", "") catch return;
        std.log.info("Health: created fix task for {s} {s} failure", .{ repo_path, kind });
        self.notify(self.config.pipeline_admin_chat, std.fmt.allocPrint(self.allocator, "Health check: {s} failing for {s}, created fix task", .{ kind, repo_path }) catch return);
    }

    // --- Self-Update ---

    /// Periodically fetch origin and pull if the self repo has new commits.
    fn checkRemoteUpdates(self: *Pipeline) void {
        const now = std.time.timestamp();
        if (now - self.last_remote_check_ts < self.config.remote_check_interval_s) return;
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

        std.log.info("Self-update: build succeeded, restart scheduled (3h or director)", .{});
        self.notify(self.config.pipeline_admin_chat, self.allocator.dupe(u8, "Self-update: new build ready. Will restart in 3h or on director command.") catch return);
        self.last_self_update_ts = std.time.timestamp();
    }

    fn maybeApplySelfUpdate(self: *Pipeline) void {
        if (self.last_self_update_ts == 0) return;
        const now = std.time.timestamp();
        const forced = self.force_restart.load(.acquire);
        if (!forced and now - self.last_self_update_ts < 3 * 3600) return;

        std.log.info("Self-update: applying restart (forced={}, age={}s)", .{ forced, now - self.last_self_update_ts });
        self.notify(self.config.pipeline_admin_chat, self.allocator.dupe(u8, "Self-update: restarting now...") catch return);
        self.update_ready.store(true, .release);
        self.running.store(false, .release);
    }

    // --- Agent Spawning ---

    // Run claude directly on the host (no Docker) — use when the agent needs full git access.
    const TaskStreamCtx = struct {
        web: *web_mod.WebServer,
        task_id: i64,
    };

    fn taskStreamCallback(ctx: ?*anyopaque, data: []const u8) void {
        if (ctx) |c| {
            const tsc: *TaskStreamCtx = @ptrCast(@alignCast(c));
            tsc.web.broadcastTaskStream(tsc.task_id, data);
        }
    }

    fn spawnAgentHost(self: *Pipeline, prompt: []const u8, workdir: []const u8, resume_session: ?[]const u8, task_id: i64) !agent_mod.AgentResult {
        self.config.refreshOAuthToken();

        // Same session dir as Docker (store/sessions/task-{id}/) so host
        // agents can resume from Docker sessions and vice versa
        const session_home = try std.fmt.allocPrint(self.allocator, "store/sessions/task-{d}", .{task_id});
        defer self.allocator.free(session_home);
        const claude_dir = try std.fmt.allocPrint(self.allocator, "{s}/.claude", .{session_home});
        defer self.allocator.free(claude_dir);
        std.fs.cwd().makePath(claude_dir) catch {};
        const abs_session_home = try std.fs.cwd().realpathAlloc(self.allocator, session_home);
        defer self.allocator.free(abs_session_home);

        // Set up live streaming
        var stream_ctx: TaskStreamCtx = undefined;
        var cb = agent_mod.StreamCallback{};
        if (self.web) |web| {
            stream_ctx = .{ .web = web, .task_id = task_id };
            cb = .{ .context = @ptrCast(&stream_ctx), .on_data = taskStreamCallback };
            web.startTaskStream(task_id);
        }
        defer if (self.web) |web| web.endTaskStream(task_id);

        std.log.info("Spawning host agent in {s}", .{workdir});
        return agent_mod.runDirect(self.allocator, .{
            .model = self.config.model,
            .oauth_token = self.config.oauth_token,
            .session_id = resume_session,
            .session_dir = abs_session_home,
            .assistant_name = "",
            .workdir = workdir,
            .allowed_tools = prompts.getAllowedTools(.worker),
        }, prompt, cb);
    }

    fn spawnAgent(self: *Pipeline, persona: AgentPersona, prompt: []const u8, workdir: []const u8, resume_session: ?[]const u8, task_id: i64) !agent_mod.AgentResult {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const tmp = arena.allocator();

        self.config.refreshOAuthToken();

        const base_system_prompt = prompts.getSystemPrompt(persona);
        const allowed_tools = prompts.getAllowedTools(persona);

        // Append config-driven instructions to system prompt
        const suffix = self.config.getSystemPromptSuffix(tmp);
        var sys_buf = std.ArrayList(u8).init(tmp);
        try sys_buf.appendSlice(base_system_prompt);
        try sys_buf.appendSlice(suffix);
        const system_prompt = sys_buf.items;

        // Inject per-repo prompt if configured (via prompt_file or .borg/prompt.md)
        var effective_prompt = prompt;
        if (self.config.getRepoPrompt(workdir)) |repo_prompt| {
            defer self.allocator.free(repo_prompt);
            var combined = std.ArrayList(u8).init(tmp);
            try combined.writer().print("## Project Context\n\n{s}\n\n---\n\n{s}", .{ repo_prompt, prompt });
            effective_prompt = combined.items;
        }

        // Per-task session dir — persists Claude sessions across container runs
        const session_dir = try std.fmt.allocPrint(tmp, "store/sessions/task-{d}/.claude", .{task_id});
        std.fs.cwd().makePath(session_dir) catch |err| {
            std.log.warn("Failed to create session dir {s}: {}", .{ session_dir, err });
        };
        const abs_session_dir = try std.fs.cwd().realpathAlloc(tmp, session_dir);

        // Build JSON input
        var input = std.ArrayList(u8).init(tmp);
        const esc_prompt = try json_mod.escapeString(tmp, effective_prompt);
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

        var env_list = std.ArrayList([]const u8).init(tmp);
        try env_list.appendSlice(&.{
            oauth_env,
            model_env,
            "HOME=/home/bun",
            "NODE_OPTIONS=--max-old-space-size=384",
        });

        // Pass git author env vars to container based on GIT_AUTHOR_MODE
        var git_aname_buf: [256]u8 = undefined;
        var git_aemail_buf: [256]u8 = undefined;
        var git_cname_buf: [256]u8 = undefined;
        var git_cemail_buf: [256]u8 = undefined;
        if (self.config.git_author_name.len > 0) {
            const name = if (self.config.git_via_borg)
                try std.fmt.bufPrint(&git_aname_buf, "GIT_AUTHOR_NAME={s} (via Borg)", .{self.config.git_author_name})
            else
                try std.fmt.bufPrint(&git_aname_buf, "GIT_AUTHOR_NAME={s}", .{self.config.git_author_name});
            try env_list.append(name);
        }
        if (self.config.git_author_email.len > 0) {
            const email = try std.fmt.bufPrint(&git_aemail_buf, "GIT_AUTHOR_EMAIL={s}", .{self.config.git_author_email});
            try env_list.append(email);
        }
        // Committer: use explicit committer fields, or fall back to author fields
        const cname = if (self.config.git_committer_name.len > 0) self.config.git_committer_name else self.config.git_author_name;
        const cemail = if (self.config.git_committer_email.len > 0) self.config.git_committer_email else self.config.git_author_email;
        if (cname.len > 0) {
            const cn = try std.fmt.bufPrint(&git_cname_buf, "GIT_COMMITTER_NAME={s}", .{cname});
            try env_list.append(cn);
        }
        if (cemail.len > 0) {
            const ce = try std.fmt.bufPrint(&git_cemail_buf, "GIT_COMMITTER_EMAIL={s}", .{cemail});
            try env_list.append(ce);
        }

        const env = env_list.items;

        // Bind mounts: worktree + persistent session dir + optional setup script
        var bind_buf: [1024]u8 = undefined;
        const repo_bind = try std.fmt.bufPrint(&bind_buf, "{s}:/workspace/repo", .{workdir});
        var sess_bind_buf: [1024]u8 = undefined;
        const sess_bind = try std.fmt.bufPrint(&sess_bind_buf, "{s}:/home/bun/.claude", .{abs_session_dir});

        var binds_list = std.ArrayList([]const u8).init(tmp);
        try binds_list.appendSlice(&.{ repo_bind, sess_bind });

        var setup_bind_buf: [1024]u8 = undefined;
        if (self.config.container_setup.len > 0) {
            const abs_setup = std.fs.cwd().realpathAlloc(tmp, self.config.container_setup) catch null;
            if (abs_setup) |setup_path| {
                const setup_bind = try std.fmt.bufPrint(&setup_bind_buf, "{s}:/workspace/setup.sh:ro", .{setup_path});
                try binds_list.append(setup_bind);
            }
        }

        const binds = binds_list.items;

        std.log.info("Spawning {s} agent: {s}", .{ @tagName(persona), container_name });

        // Set up live streaming
        var stream_ctx: TaskStreamCtx = undefined;
        var cb = agent_mod.StreamCallback{};
        if (self.web) |web| {
            stream_ctx = .{ .web = web, .task_id = task_id };
            cb = .{ .context = @ptrCast(&stream_ctx), .on_data = taskStreamCallback };
            web.startTaskStream(task_id);
        }
        defer if (self.web) |web| web.endTaskStream(task_id);

        // Start timeout watchdog
        var agent_done = std.atomic.Value(bool).init(false);
        const name_for_watchdog = try self.allocator.dupe(u8, container_name);
        const watchdog = std.Thread.spawn(.{}, agentTimeoutWatchdog, .{
            &agent_done, self.docker, name_for_watchdog, self.config.agent_timeout_s,
        }) catch null;

        var run_result = try self.docker.runWithStdio(docker_mod.ContainerConfig{
            .image = self.config.container_image,
            .name = container_name,
            .env = env,
            .binds = binds,
            .memory_limit = self.config.container_memory_mb * 1024 * 1024,
        }, input.items, cb);
        defer run_result.deinit();

        // Cancel watchdog
        agent_done.store(true, .release);
        if (watchdog) |w| w.join();
        self.allocator.free(name_for_watchdog);

        if (run_result.stdout.len == 0) {
            std.log.warn("{s} agent returned empty output (exit={d}) — likely auth or API issue", .{ @tagName(persona), run_result.exit_code });
        } else {
            std.log.info("{s} agent done (exit={d}, {d} bytes)", .{ @tagName(persona), run_result.exit_code, run_result.stdout.len });
        }

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

    fn isSelfRepo(self: *Pipeline, repo_path: []const u8) bool {
        for (self.config.watched_repos) |repo| {
            if (repo.is_self and std.mem.eql(u8, repo.path, repo_path)) return true;
        }
        return false;
    }

    fn isTestFileError(stderr: []const u8, stdout: []const u8) bool {
        const outputs = [_][]const u8{ stderr, stdout };
        for (&outputs) |output| {
            if (output.len == 0) continue;
            // Compile errors referencing test files (e.g. "src/foo_test.zig:12:5: error:")
            if (std.mem.indexOf(u8, output, "_test.zig") != null and
                std.mem.indexOf(u8, output, "error:") != null) return true;
            if (std.mem.indexOf(u8, output, "/tests/") != null and
                std.mem.indexOf(u8, output, "error:") != null) return true;
            // Segfault during test execution — often test setup (use-after-free, wrong allocator)
            if (std.mem.indexOf(u8, output, "Segmentation fault") != null) return true;
            // Zig panic in test code
            if (std.mem.indexOf(u8, output, "panicked") != null and
                std.mem.indexOf(u8, output, "_test") != null) return true;
        }
        return false;
    }

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
        self.db.setTaskSessionId(task.id, "") catch {};
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

    /// Kill any borg-* containers left over from a previous instance
    fn killOrphanedContainers(self: *Pipeline) void {
        var argv = [_][]const u8{ "docker", "ps", "-q", "--filter", "name=borg-" };
        var child = std.process.Child.init(&argv, self.allocator);
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Ignore;
        child.spawn() catch return;
        const stdout = child.stdout.?.reader().readAllAlloc(self.allocator, 64 * 1024) catch {
            _ = child.wait() catch {};
            return;
        };
        defer self.allocator.free(stdout);
        _ = child.wait() catch {};

        const trimmed = std.mem.trim(u8, stdout, " \t\r\n");
        if (trimmed.len == 0) return;

        var count: u32 = 0;
        var it = std.mem.splitScalar(u8, trimmed, '\n');
        while (it.next()) |_| count += 1;

        std.log.warn("Killing {d} orphaned container(s) from previous run", .{count});
        var it2 = std.mem.splitScalar(u8, trimmed, '\n');
        while (it2.next()) |id| {
            if (id.len == 0) continue;
            self.docker.killContainer(id) catch {};
        }
    }
};

const TestResult = struct {
    stdout: []const u8,
    stderr: []const u8,
    exit_code: u8,
};

// ── Tests ──────────────────────────────────────────────────────────────

test "prompts: system prompts non-empty for all personas" {
    try std.testing.expect(prompts.getSystemPrompt(.manager).len > 0);
    try std.testing.expect(prompts.getSystemPrompt(.qa).len > 0);
    try std.testing.expect(prompts.getSystemPrompt(.worker).len > 0);
}

test "getAllowedTools restricts manager and qa" {
    const mgr = prompts.getAllowedTools(.manager);
    const qa = prompts.getAllowedTools(.qa);
    const wrk = prompts.getAllowedTools(.worker);

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

    const merged = [_][]const u8{ "task-1", "task-2" };

    try w.print("*{d} PR(s) merged*\n", .{merged.len});
    for (merged) |branch| {
        try w.print("  + {s}\n", .{branch});
    }

    const result = buf.items;
    try std.testing.expect(std.mem.indexOf(u8, result, "2 PR(s) merged") != null);
    try std.testing.expect(std.mem.indexOf(u8, result, "task-1") != null);
    try std.testing.expect(std.mem.indexOf(u8, result, "task-2") != null);
}

test {
    _ = @import("pipeline_stats_test.zig");
}
