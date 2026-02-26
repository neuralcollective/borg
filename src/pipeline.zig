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
const modes = @import("modes.zig");
const agent_mod = @import("agent.zig");
const Config = @import("config.zig").Config;
const web_mod = @import("web.zig");

const AGENT_TIMEOUT_S_FALLBACK = 600;


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
    last_triage_ts: i64,
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
            .last_triage_ts = 0,
            .startup_heads = heads,
            .active_agents = std.atomic.Value(u32).init(0),
        };
    }

    pub fn run(self: *Pipeline) void {
        std.log.info("Pipeline thread started for {d} repo(s)", .{self.config.watched_repos.len});

        // Clear stale dispatched_at from previous instance (ACID recovery)
        self.db.clearAllDispatched() catch |e| std.log.warn("clearAllDispatched: {}", .{e});
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
            self.maybeAutoTriage();
            self.maybeAutoPromoteProposals();
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

                _ = self.db.createPipelineTask(title, description, repo.path, "backlog", self.config.pipeline_admin_chat, repo.mode) catch continue;
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
                self.db.clearTaskDispatched(task.id) catch |e| std.log.warn("clearTaskDispatched #{d}: {}", .{ task.id, e });
                continue;
            };
            dispatched[i] = true;
        }

        // Free strings for tasks not dispatched to threads
        for (tasks, 0..) |task, i| {
            if (!dispatched[i]) task.deinit(self.allocator);
        }
    }

    fn getModeForTask(_: *Pipeline, task: db_mod.PipelineTask) *const modes.PipelineMode {
        return modes.getMode(task.mode) orelse &modes.swe_mode;
    }

    fn processTaskThread(self: *Pipeline, task: db_mod.PipelineTask) void {
        defer {
            task.deinit(self.allocator);
            _ = self.active_agents.fetchSub(1, .acq_rel);
            self.db.clearTaskDispatched(task.id) catch |e| std.log.warn("clearTaskDispatched #{d}: {}", .{ task.id, e });
        }

        // Only run tasks for the primary (self) repo — delete stray tasks from other repos
        if (!self.isSelfRepo(task.repo_path)) {
            std.log.warn("Task #{d} targets non-primary repo {s}, deleting", .{ task.id, task.repo_path });
            self.db.deletePipelineTask(task.id) catch |e| std.log.warn("deletePipelineTask #{d}: {}", .{ task.id, e });
            return;
        }

        const mode = self.getModeForTask(task);
        const phase = mode.getPhase(task.status) orelse {
            std.log.err("Task #{d} has unknown phase '{s}' for mode '{s}'", .{ task.id, task.status, mode.name });
            return;
        };

        switch (phase.phase_type) {
            .setup => self.setupBranch(task) catch |err| {
                std.log.err("Task #{d} setup error: {}", .{ task.id, err });
            },
            .agent => self.runAgentPhase(task, phase, mode) catch |err| {
                std.log.err("Task #{d} {s} error: {}", .{ task.id, phase.name, err });
            },
            .rebase => self.runRebasePhase(task, phase) catch |err| {
                std.log.err("Task #{d} rebase error: {}", .{ task.id, err });
            },
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

        // Find primary repo and its mode
        var primary_repo: ?@import("config.zig").RepoConfig = null;
        for (self.config.watched_repos) |repo| {
            if (repo.is_self) {
                primary_repo = repo;
                break;
            }
        }
        const repo = primary_repo orelse return;
        const mode = modes.getMode(repo.mode) orelse &modes.swe_mode;
        if (mode.seed_modes.len == 0) return;

        // Rotate seed mode index within this mode's seed_modes
        const seed_idx = blk: {
            const mode_str = self.db.getState(self.allocator, "seed_mode") catch null;
            const prev: u32 = if (mode_str) |s| std.fmt.parseInt(u32, s, 10) catch 0 else 0;
            if (mode_str) |s| self.allocator.free(s);
            const next = (prev + 1) % @as(u32, @intCast(mode.seed_modes.len));
            var next_buf: [4]u8 = undefined;
            const next_str = std.fmt.bufPrint(&next_buf, "{d}", .{next}) catch "0";
            self.db.setState("seed_mode", next_str) catch {};
            break :blk next;
        };

        const seed_config = mode.seed_modes[seed_idx];
        self.last_seed_ts = now;
        self.config.refreshOAuthToken();
        std.log.info("Seed scan starting ({s}: {s})", .{ mode.name, seed_config.label });

        var total_created: u32 = 0;
        const active_u32: u32 = @intCast(@max(active, 0));

        if (active_u32 + total_created < self.config.pipeline_max_backlog) {
            const created = self.seedRepo(repo.path, seed_config, mode, active_u32 + total_created);
            total_created += created;
        }

        // Cross-pollinate: analyze watched repos for ideas to bring into primary
        for (self.config.watched_repos) |watched| {
            if (watched.is_self) continue;
            if (active_u32 + total_created >= self.config.pipeline_max_backlog) break;
            const created = self.seedCrossPollinate(watched.path, repo.path);
            total_created += created;
        }

        if (total_created > 0) {
            std.log.info("Seed scan ({s}: {s}): created {d} task(s)/proposal(s)", .{ mode.name, seed_config.label, total_created });
            self.notify(self.config.pipeline_admin_chat, std.fmt.allocPrint(self.allocator, "Seed scan ({s}: {s}): created {d} task(s)/proposal(s)", .{ mode.name, seed_config.label, total_created }) catch return);
        } else {
            std.log.info("Seed scan ({s}: {s}): no results", .{ mode.name, seed_config.label });
        }
    }

    fn appendRepoContext(self: *Pipeline, buf: *std.ArrayList(u8), repo_path: []const u8) void {
        const w = buf.writer();

        // Read CLAUDE.md for project description
        const claude_md_path = std.fmt.allocPrint(self.allocator, "{s}/CLAUDE.md", .{repo_path}) catch return;
        defer self.allocator.free(claude_md_path);
        if (std.fs.cwd().readFileAlloc(self.allocator, claude_md_path, 32 * 1024)) |content| {
            defer self.allocator.free(content);
            w.writeAll("## Project Documentation (CLAUDE.md)\n\n") catch return;
            w.writeAll(content[0..@min(content.len, 8000)]) catch return;
            w.writeAll("\n\n---\n\n") catch return;
        } else |_| {}

        // Include file listing
        var git = Git.init(self.allocator, repo_path);
        var ls = git.exec(&.{ "ls-files", "--full-name" }) catch return;
        defer ls.deinit();
        if (ls.stdout.len > 0) {
            w.writeAll("## Repository Files\n\n```\n") catch return;
            w.writeAll(ls.stdout[0..@min(ls.stdout.len, 4000)]) catch return;
            w.writeAll("\n```\n\n---\n\n") catch return;
        }
    }

    fn seedRepo(self: *Pipeline, repo_path: []const u8, seed_config: modes.SeedConfig, mode: *const modes.PipelineMode, current_count: u32) u32 {
        var prompt_buf = std.ArrayList(u8).init(self.allocator);
        defer prompt_buf.deinit();
        const w = prompt_buf.writer();

        // Include CLAUDE.md, file listing, and exploration instructions
        self.appendRepoContext(&prompt_buf, repo_path);
        w.writeAll(prompts.seed_explore_preamble) catch return 0;
        w.writeAll(seed_config.prompt) catch return 0;

        if (seed_config.output_type == .proposal) {
            w.writeAll(prompts.seed_proposal_suffix) catch return 0;
            return self.seedRepoProposals(repo_path, repo_path, prompt_buf.items);
        }

        w.writeAll(prompts.seed_task_suffix) catch return 0;

        // Use first agent phase's system prompt for seed agents
        const first_agent = blk: {
            for (mode.phases) |*p| {
                if (p.phase_type == .agent) break :blk p;
            }
            break :blk &mode.phases[0];
        };
        const result = self.spawnAgent(first_agent.system_prompt, first_agent.allowed_tools, prompt_buf.items, repo_path, null, 0) catch |err| {
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
                mode.name,
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
        // Use a generic read-only tool set for proposal generation
        const result = self.spawnAgent("You are an analyst reviewing a codebase.", "Read,Glob,Grep,Write", prompt, source_repo, null, 0) catch |err| {
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

    fn runAgentPhase(self: *Pipeline, task: db_mod.PipelineTask, phase: *const modes.PhaseConfig, mode: *const modes.PipelineMode) !void {
        const wt_path = if (mode.uses_git_worktrees) try self.worktreePath(task.repo_path, task.id) else try self.allocator.dupe(u8, task.repo_path);
        defer self.allocator.free(wt_path);
        var wt_git = Git.init(self.allocator, wt_path);

        // Idempotency: if runs_tests and a previous run left passing code, skip the agent
        if (phase.runs_tests and mode.uses_test_cmd) {
            const test_cmd = self.config.getTestCmdForRepo(task.repo_path);
            if (self.runTestCommandForRepo(wt_path, test_cmd)) |pre_test| {
                defer self.allocator.free(pre_test.stdout);
                defer self.allocator.free(pre_test.stderr);
                if (pre_test.exit_code == 0) {
                    var diff_check = try wt_git.exec(&.{ "diff", "--stat", "origin/main..HEAD" });
                    defer diff_check.deinit();
                    const has_changes = diff_check.success() and std.mem.trim(u8, diff_check.stdout, " \t\r\n").len > 0;

                    if (has_changes) {
                        try self.db.updateTaskStatus(task.id, "done");
                        if (mode.integration == .git_pr) {
                            try self.db.enqueueForIntegration(task.id, task.branch, task.repo_path);
                            self.cleanupWorktree(task);
                        }
                        std.log.info("Task #{d} tests already pass, queued for integration", .{task.id});
                        self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} passed all tests! Queued for integration.", .{task.id}));
                    } else {
                        try self.db.updateTaskStatus(task.id, "merged");
                        if (mode.uses_git_worktrees) self.cleanupWorktree(task);
                        std.log.info("Task #{d} tests already pass with no changes, marking as merged", .{task.id});
                    }
                    return;
                }
            } else |_| {}
        }

        // Build prompt
        var prompt_buf = std.ArrayList(u8).init(self.allocator);
        defer prompt_buf.deinit();
        const w = prompt_buf.writer();

        if (phase.include_task_context) {
            try w.print("Task #{d}: {s}\n\nDescription:\n{s}\n\n", .{ task.id, task.title, task.description });
        }

        if (phase.include_file_listing and mode.uses_git_worktrees) {
            var ls = try wt_git.exec(&.{ "ls-files", "--full-name" });
            defer ls.deinit();
            try w.writeAll("## Repository Files\n\n```\n");
            try w.writeAll(ls.stdout[0..@min(ls.stdout.len, 4000)]);
            try w.writeAll("\n```\n\n");
        }

        try w.writeAll(phase.instruction);

        // Append error context if available
        if (task.last_error.len > 0 and phase.error_instruction.len > 0) {
            const err_tail = if (task.last_error.len > 3000) task.last_error[task.last_error.len - 3000 ..] else task.last_error;
            try modes.substituteError(w, phase.error_instruction, err_tail);
        }

        // Session handling
        const resume_sid = if (phase.fresh_session) null else if (task.session_id.len > 0) task.session_id else null;

        // Spawn agent (Docker or host)
        const result = if (phase.use_docker)
            self.spawnAgent(phase.system_prompt, phase.allowed_tools, prompt_buf.items, wt_path, resume_sid, task.id) catch |err| {
                try self.failTask(task, "agent spawn failed", @errorName(err));
                return;
            }
        else
            self.spawnAgentHost(phase.system_prompt, phase.allowed_tools, prompt_buf.items, wt_path, resume_sid, task.id) catch |err| {
                try self.failTask(task, "agent spawn failed", @errorName(err));
                return;
            };
        defer self.allocator.free(result.output);
        defer self.allocator.free(result.raw_stream);

        // Store session
        if (result.new_session_id) |sid| {
            self.db.setTaskSessionId(task.id, sid) catch |e| std.log.warn("setTaskSessionId #{d}: {}", .{ task.id, e });
            self.allocator.free(sid);
        }

        self.db.storeTaskOutputFull(task.id, phase.name, result.output, result.raw_stream, 0) catch |e| std.log.warn("storeTaskOutput #{d} {s}: {}", .{ task.id, phase.name, e });

        // Check artifact if required
        if (phase.check_artifact) |artifact| {
            const artifact_path = try std.fmt.allocPrint(self.allocator, "{s}/{s}", .{ wt_path, artifact });
            defer self.allocator.free(artifact_path);
            const exists = blk: {
                std.fs.accessAbsolute(artifact_path, .{}) catch break :blk false;
                break :blk true;
            };
            if (exists) {
                const content = std.fs.cwd().readFileAlloc(self.allocator, artifact_path, 64 * 1024) catch null;
                if (content) |c| {
                    defer self.allocator.free(c);
                    const diff_name = try std.fmt.allocPrint(self.allocator, "{s}_diff", .{phase.name});
                    defer self.allocator.free(diff_name);
                    self.db.storeTaskOutput(task.id, diff_name, c, 0) catch |e| std.log.warn("storeTaskOutput #{d} artifact: {}", .{ task.id, e });
                }
            }
            if (!exists and result.output.len == 0) {
                const reason = try std.fmt.allocPrint(self.allocator, "agent produced no output (missing {s})", .{artifact});
                defer self.allocator.free(reason);
                try self.failTask(task, reason, "empty result and artifact not found");
                return;
            }
        }

        // Commit if configured
        if (phase.commits and mode.uses_git_worktrees) {
            var add = try wt_git.addAll();
            defer add.deinit();
            var commit = try wt_git.commitWithAuthor(phase.commit_message, self.config.git_author);
            defer commit.deinit();

            if (commit.success()) {
                var diff = try wt_git.exec(&.{ "diff", "HEAD~1" });
                defer diff.deinit();
                if (diff.success()) {
                    const diff_name = try std.fmt.allocPrint(self.allocator, "{s}_diff", .{phase.name});
                    defer self.allocator.free(diff_name);
                    self.db.storeTaskOutput(task.id, diff_name, diff.stdout, 0) catch |e| std.log.warn("storeTaskOutput #{d} diff: {}", .{ task.id, e });
                }
            } else if (phase.check_artifact == null and !phase.allow_no_changes) {
                try self.failTask(task, "agent produced no changes", commit.stderr);
                return;
            }
        }

        // Run tests if configured
        if (phase.runs_tests and mode.uses_test_cmd) {
            const test_cmd = self.config.getTestCmdForRepo(task.repo_path);
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
                    self.db.storeTaskOutput(task.id, "test", tc, @intCast(test_result.exit_code)) catch |e| std.log.warn("storeTaskOutput #{d} test: {}", .{ task.id, e });
                }
            }

            if (test_result.exit_code == 0) {
                try self.db.updateTaskStatus(task.id, phase.next);
                if (mode.integration == .git_pr and std.mem.eql(u8, phase.next, "done")) {
                    try self.db.enqueueForIntegration(task.id, task.branch, task.repo_path);
                    self.cleanupWorktree(task);
                }
                std.log.info("Task #{d} passed tests, advancing to {s}", .{ task.id, phase.next });
                self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} passed all tests! Advancing to {s}.", .{ task.id, phase.next }));
            } else {
                const combined = combineTestOutput(self.allocator, test_result.stdout, test_result.stderr, 2000);
                defer if (combined.len > 0) self.allocator.free(combined);
                try self.db.updateTaskError(task.id, combined[0..@min(combined.len, 4000)]);

                if (task.attempt + 1 >= task.max_attempts) {
                    std.log.warn("Task #{d} exhausted {d} attempts — marking failed", .{ task.id, task.max_attempts });
                    try self.db.updateTaskStatus(task.id, "failed");
                    if (mode.uses_git_worktrees) self.cleanupWorktree(task);
                    self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} exhausted {d} attempts — failed.", .{ task.id, task.max_attempts }));
                } else {
                    try self.db.incrementTaskAttempt(task.id);

                    if (phase.has_qa_fix_routing and task.attempt >= 1 and isTestFileError(test_result.stderr, test_result.stdout)) {
                        try self.db.updateTaskStatus(task.id, "qa_fix");
                        self.db.setTaskSessionId(task.id, "") catch |e| std.log.warn("setTaskSessionId #{d}: {}", .{ task.id, e });
                        std.log.info("Task #{d} test error in test files, routing to QA fix ({d}/{d})", .{ task.id, task.attempt + 1, task.max_attempts });
                        self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} test code has bugs — sending back to QA for fix ({d}/{d})", .{ task.id, task.attempt + 1, task.max_attempts }));
                    } else {
                        // Stay on current phase for retry
                        std.log.info("Task #{d} test failed, retry {d}/{d}", .{ task.id, task.attempt + 1, task.max_attempts });
                    }
                }
            }
            return;
        }

        // No tests — advance to next phase
        try self.db.updateTaskStatus(task.id, phase.next);
        std.log.info("Task #{d} {s} complete, advancing to {s}", .{ task.id, phase.name, phase.next });
        self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d}: {s} complete, advancing to {s}", .{ task.id, phase.label, phase.next }));
    }

    fn runRebasePhase(self: *Pipeline, task: db_mod.PipelineTask, phase: *const modes.PhaseConfig) !void {
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

            try w.writeAll(phase.instruction);

            if (task.last_error.len > 0 and phase.error_instruction.len > 0) {
                const err_tail = if (task.last_error.len > 2000) task.last_error[task.last_error.len - 2000 ..] else task.last_error;
                try modes.substituteError(w, phase.error_instruction, err_tail);
            }

            // Run on host (not Docker) — rebase needs full git repo access
            const result = self.spawnAgentHost(phase.system_prompt, phase.allowed_tools, prompt_buf.items, wt_path, null, task.id) catch |err| {
                try self.failTask(task, "rebase: worker agent failed", @errorName(err));
                return;
            };
            defer self.allocator.free(result.output);
            defer self.allocator.free(result.raw_stream);

            if (result.new_session_id) |sid| {
                self.db.setTaskSessionId(task.id, sid) catch |e| std.log.warn("setTaskSessionId #{d}: {}", .{ task.id, e });
                self.allocator.free(sid);
            }

            self.db.storeTaskOutputFull(task.id, "rebase", result.output, result.raw_stream, 0) catch |e| std.log.warn("storeTaskOutput #{d} rebase: {}", .{ task.id, e });
        }

        // Verify the agent actually completed the rebase before doing anything else.
        // Agents exit 0 even when they fail; without this check we push the old tip.
        var rb_verify = try wt_git.exec(&.{ "merge-base", "--is-ancestor", "origin/main", task.branch });
        defer rb_verify.deinit();
        if (!rb_verify.success()) {
            std.log.warn("Task #{d}: branch still not rebased after agent ({d}/{d})", .{ task.id, task.attempt + 1, task.max_attempts });
            if (task.attempt + 1 >= task.max_attempts) {
                std.log.warn("Task #{d} exhausted rebase attempts — marking failed", .{task.id});
                try self.db.updateTaskStatus(task.id, "failed");
                self.cleanupWorktree(task);
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
            // Tests fail after clean rebase — spawn agent to fix
            const combined_err = combineTestOutput(self.allocator, test_result.stdout, test_result.stderr, 2000);
            defer if (combined_err.len > 0) self.allocator.free(combined_err);
            try self.db.updateTaskError(task.id, combined_err[0..@min(combined_err.len, 4000)]);

            if (task.attempt + 1 >= task.max_attempts) {
                std.log.warn("Task #{d} exhausted {d} rebase attempts — marking failed", .{ task.id, task.max_attempts });
                try self.db.updateTaskStatus(task.id, "failed");
                self.cleanupWorktree(task);
                self.notify(task.notify_chat, std.fmt.allocPrint(self.allocator, "Task #{d} exhausted {d} rebase attempts — failed.", .{ task.id, task.max_attempts }) catch return);
            } else {
                std.log.info("Task #{d} rebase tests failed, spawning fix agent ({d}/{d})", .{ task.id, task.attempt + 1, task.max_attempts });

                var fix_prompt = std.ArrayList(u8).init(self.allocator);
                defer fix_prompt.deinit();
                const fw = fix_prompt.writer();
                try fw.writeAll(phase.fix_instruction);
                const err_tail = if (combined_err.len > 3000) combined_err[combined_err.len - 3000 ..] else combined_err;
                if (phase.fix_error_instruction.len > 0) {
                    try modes.substituteError(fw, phase.fix_error_instruction, err_tail);
                }

                const fix_result = self.spawnAgentHost(phase.system_prompt, phase.allowed_tools, fix_prompt.items, wt_path, null, task.id) catch |err| {
                    try self.failTask(task, "rebase: fix agent failed", @errorName(err));
                    return;
                };
                defer self.allocator.free(fix_result.output);
                defer self.allocator.free(fix_result.raw_stream);
                self.db.storeTaskOutputFull(task.id, "rebase_fix", fix_result.output, fix_result.raw_stream, 0) catch |e| std.log.warn("storeTaskOutput #{d} rebase_fix: {}", .{ task.id, e });

                // Re-run tests after fix agent
                const retest = self.runTestCommandForRepo(wt_path, rebase_test_cmd) catch |err| {
                    try self.failTask(task, "rebase: retest failed", @errorName(err));
                    return;
                };
                defer self.allocator.free(retest.stdout);
                defer self.allocator.free(retest.stderr);

                if (retest.exit_code == 0) {
                    // Fixed! Push and queue
                    var push_r2 = try wt_git.exec(&.{ "push", "--force", "origin", task.branch });
                    defer push_r2.deinit();
                    if (push_r2.success()) {
                        try self.db.updateTaskStatus(task.id, "done");
                        try self.db.enqueueForIntegration(task.id, task.branch, task.repo_path);
                        self.cleanupWorktree(task);
                        std.log.info("Task #{d} rebase fix succeeded, re-queued for integration", .{task.id});
                        self.notify(task.notify_chat, std.fmt.allocPrint(self.allocator, "Task #{d} rebase fix succeeded, re-queued for release.", .{task.id}) catch return);
                    } else {
                        try self.db.incrementTaskAttempt(task.id);
                        std.log.warn("Task #{d} rebase fix: push failed", .{task.id});
                    }
                } else {
                    try self.db.incrementTaskAttempt(task.id);
                    std.log.info("Task #{d} rebase fix agent didn't fully resolve tests, retry {d}/{d}", .{ task.id, task.attempt + 1, task.max_attempts });
                }
            }
        }
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
                    self.db.updateQueueStatus(entry.id, "merged", null) catch |e| std.log.warn("updateQueueStatus #{d}: {}", .{ entry.id, e });
                    self.db.updateTaskStatus(entry.task_id, "merged") catch |e| std.log.warn("updateTaskStatus #{d}: {}", .{ entry.task_id, e });
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
                    self.db.updateQueueStatus(entry.id, "merged", null) catch |e| std.log.warn("updateQueueStatus #{d}: {}", .{ entry.id, e });
                    self.db.updateTaskStatus(entry.task_id, "merged") catch |e| std.log.warn("updateTaskStatus #{d}: {}", .{ entry.task_id, e });
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

    fn repoMode(self: *Pipeline, repo_path: []const u8) []const u8 {
        for (self.config.watched_repos) |repo| {
            if (std.mem.eql(u8, repo.path, repo_path)) return repo.mode;
        }
        return "sweborg";
    }

    const AUTO_PROMOTE_SCORE: i64 = 7;

    fn maybeAutoPromoteProposals(self: *Pipeline) void {
        const active = self.db.getActivePipelineTaskCount() catch return;
        if (active >= self.config.pipeline_max_backlog) return;

        const slots = @as(i64, @intCast(self.config.pipeline_max_backlog)) - active;

        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        const proposals = self.db.getTopScoredProposals(alloc, AUTO_PROMOTE_SCORE, slots) catch return;
        for (proposals) |p| {
            const mode = self.repoMode(p.repo_path);
            const task_id = self.db.createPipelineTask(p.title, p.description, p.repo_path, "proposal", "", mode) catch |e| {
                std.log.warn("Auto-promote proposal #{d}: {}", .{ p.id, e });
                continue;
            };
            self.db.updateProposalStatus(p.id, "approved") catch |e| std.log.warn("updateProposalStatus #{d}: {}", .{ p.id, e });
            std.log.info("Auto-promoted proposal #{d} (score={d}) → task #{d}: {s}", .{ p.id, p.triage_score, task_id, p.title });
        }
    }

    const TRIAGE_INTERVAL_S: i64 = 6 * 3600; // 6 hours

    fn maybeAutoTriage(self: *Pipeline) void {
        const now = std.time.timestamp();
        if (now - self.last_triage_ts < TRIAGE_INTERVAL_S) return;

        // Only run if there are unscored proposals
        const unscored = self.db.countUnscoredProposals();
        if (unscored == 0) return;

        self.last_triage_ts = now;
        std.log.info("Auto-triage: {d} unscored proposals, running triage", .{unscored});

        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const alloc = arena.allocator();

        const proposals = self.db.getProposals(alloc, "proposed", 100) catch return;
        if (proposals.len == 0) return;

        const merged_tasks = self.db.getRecentMergedTasks(alloc, 50) catch &[0]db_mod.PipelineTask{};

        var prompt_buf = std.ArrayList(u8).init(alloc);
        const pw = prompt_buf.writer();
        pw.writeAll(
            \\Rate each proposal on 4 dimensions (1-5 scale), and flag proposals
            \\that should be auto-dismissed.
            \\
            \\Dimensions:
            \\- impact: How much value does this deliver? (5 = critical fix/feature, 1 = cosmetic)
            \\- feasibility: How likely is an AI agent to implement this correctly without human help? (5 = trivial, 1 = needs human)
            \\- risk: How likely to break existing functionality? (5 = very risky, 1 = safe)
            \\- effort: How many agent cycles will this need? (5 = massive multi-file, 1 = simple one-file)
            \\
            \\Overall score formula: (impact * 2 + feasibility * 2 - risk - effort) mapped to 1-10 scale.
            \\
            \\Set "dismiss": true if the proposal should be auto-closed for any of these reasons:
            \\- Already implemented (covered by a recently merged task)
            \\- Duplicate of another proposal in this list
            \\- Nonsensical, vague, or not actionable
            \\- Irrelevant to the project
            \\
            \\Reply with ONLY a JSON array, no markdown fences, no commentary:
            \\[{"id": <number>, "impact": <1-5>, "feasibility": <1-5>, "risk": <1-5>, "effort": <1-5>, "score": <1-10>, "reasoning": "<one sentence>", "dismiss": <true|false>}]
            \\
        ) catch return;

        if (merged_tasks.len > 0) {
            pw.writeAll("Recently merged tasks (for duplicate detection):\n") catch return;
            for (merged_tasks) |t| {
                pw.print("- {s}\n", .{t.title}) catch return;
            }
            pw.writeAll("\n") catch return;
        }

        pw.writeAll("Proposals to evaluate:\n\n") catch return;
        for (proposals) |p| {
            pw.print("- ID {d}: {s}\n  Description: {s}\n  Rationale: {s}\n\n", .{
                p.id,
                p.title,
                if (p.description.len > 0) p.description else "(none)",
                if (p.rationale.len > 0) p.rationale else "(none)",
            }) catch return;
        }

        self.config.refreshOAuthToken();
        var argv = std.ArrayList([]const u8).init(alloc);
        argv.appendSlice(&.{ "claude", "--print", "--model", "haiku", "--permission-mode", "bypassPermissions" }) catch return;
        var child = std.process.Child.init(argv.items, alloc);
        child.stdin_behavior = .Pipe;
        child.stdout_behavior = .Pipe;
        child.stderr_behavior = .Ignore;

        var env = std.process.getEnvMap(alloc) catch return;
        env.put("CLAUDE_CODE_OAUTH_TOKEN", self.config.oauth_token) catch return;
        child.env_map = &env;

        child.spawn() catch return;

        if (child.stdin) |stdin| {
            stdin.writeAll(prompt_buf.items) catch |e| std.log.warn("Auto-triage: stdin write failed: {}", .{e});
            stdin.close();
            child.stdin = null;
        }

        var stdout_buf = std.ArrayList(u8).init(alloc);
        if (child.stdout) |stdout| {
            var read_buf: [8192]u8 = undefined;
            while (true) {
                const n = stdout.read(&read_buf) catch break;
                if (n == 0) break;
                stdout_buf.appendSlice(read_buf[0..n]) catch break;
            }
        }
        _ = child.wait() catch {};

        const output = stdout_buf.items;
        const arr_start = std.mem.indexOf(u8, output, "[") orelse {
            std.log.warn("Auto-triage: no JSON array in output ({d} bytes)", .{output.len});
            return;
        };
        const arr_end_idx = std.mem.lastIndexOf(u8, output, "]") orelse return;
        const json_slice = output[arr_start .. arr_end_idx + 1];

        var parsed = json_mod.parse(alloc, json_slice) catch {
            std.log.warn("Auto-triage: JSON parse failed", .{});
            return;
        };
        defer parsed.deinit();

        const items = switch (parsed.value) {
            .array => |a| a.items,
            else => return,
        };

        var scored: u32 = 0;
        var dismissed: u32 = 0;
        for (items) |item| {
            const p_id = json_mod.getInt(item, "id") orelse continue;
            const impact = json_mod.getInt(item, "impact") orelse continue;
            const feasibility = json_mod.getInt(item, "feasibility") orelse continue;
            const risk = json_mod.getInt(item, "risk") orelse continue;
            const effort = json_mod.getInt(item, "effort") orelse continue;
            const score = json_mod.getInt(item, "score") orelse continue;
            const reasoning = json_mod.getString(item, "reasoning") orelse "";
            const should_dismiss = json_mod.getBool(item, "dismiss") orelse false;

            self.db.updateProposalTriage(p_id, score, impact, feasibility, risk, effort, reasoning) catch continue;
            scored += 1;

            if (should_dismiss) {
                self.db.updateProposalStatus(p_id, "auto_dismissed") catch continue;
                dismissed += 1;
                std.log.info("Auto-triage: auto-dismissed proposal #{d}: {s}", .{ p_id, reasoning });
            }
        }

        std.log.info("Auto-triage: scored {d}/{d} proposals, auto-dismissed {d}", .{ scored, proposals.len, dismissed });
    }

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

        _ = self.db.createPipelineTask(title, desc, repo_path, "health-check", "", "sweborg") catch return;
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

    fn spawnAgentHost(self: *Pipeline, system_prompt: []const u8, allowed_tools: []const u8, prompt: []const u8, workdir: []const u8, resume_session: ?[]const u8, task_id: i64) !agent_mod.AgentResult {
        self.config.refreshOAuthToken();

        const suffix = self.config.getSystemPromptSuffix(self.allocator);
        defer if (suffix.len > 0) self.allocator.free(suffix);

        // Combine phase system prompt with config suffix
        var sys_buf = std.ArrayList(u8).init(self.allocator);
        defer sys_buf.deinit();
        try sys_buf.appendSlice(system_prompt);
        try sys_buf.appendSlice(suffix);

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
            .allowed_tools = allowed_tools,
            .system_prompt = sys_buf.items,
        }, prompt, cb);
    }

    fn spawnAgent(self: *Pipeline, system_prompt: []const u8, allowed_tools: []const u8, prompt: []const u8, workdir: []const u8, resume_session: ?[]const u8, task_id: i64) !agent_mod.AgentResult {
        var arena = std.heap.ArenaAllocator.init(self.allocator);
        defer arena.deinit();
        const tmp = arena.allocator();

        self.config.refreshOAuthToken();

        // Append config-driven instructions to system prompt
        const suffix = self.config.getSystemPromptSuffix(tmp);
        var sys_buf = std.ArrayList(u8).init(tmp);
        try sys_buf.appendSlice(system_prompt);
        try sys_buf.appendSlice(suffix);
        const full_system_prompt = sys_buf.items;

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
        const esc_sys = try json_mod.escapeString(tmp, full_system_prompt);
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
        const container_name = try std.fmt.bufPrint(&name_buf, "borg-agent-{d}-{d}", .{
            std.time.timestamp(), n,
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

        std.log.info("Spawning agent: {s}", .{container_name});

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
            std.log.warn("Agent returned empty output (exit={d}) — likely auth or API issue", .{run_result.exit_code});
        } else {
            std.log.info("Agent done (exit={d}, {d} bytes)", .{ run_result.exit_code, run_result.stdout.len });
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
            docker.killContainer(name) catch |e| std.log.warn("killContainer {s}: {}", .{ name, e });
        }
    }

    // --- Helpers ---

    fn isSelfRepo(self: *Pipeline, repo_path: []const u8) bool {
        for (self.config.watched_repos) |repo| {
            if (repo.is_self and std.mem.eql(u8, repo.path, repo_path)) return true;
        }
        return false;
    }

    fn combineTestOutput(allocator: std.mem.Allocator, stdout: []const u8, stderr: []const u8, max: usize) []const u8 {
        const out = stdout[0..@min(stdout.len, max)];
        const err = stderr[0..@min(stderr.len, max)];
        if (out.len > 0 and err.len > 0)
            return std.fmt.allocPrint(allocator, "stdout:\n{s}\nstderr:\n{s}", .{ out, err }) catch ""
        else if (err.len > 0)
            return std.fmt.allocPrint(allocator, "{s}", .{err}) catch ""
        else if (out.len > 0)
            return std.fmt.allocPrint(allocator, "{s}", .{out}) catch ""
        else
            return "";
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
            std.log.warn("Task #{d} failed ({s}), exhausted {d} attempts — marking failed", .{ task.id, reason, task.max_attempts });
            try self.db.updateTaskStatus(task.id, "failed");
            self.cleanupWorktree(task);
            self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} failed: {s} — gave up after {d} attempts", .{ task.id, reason, task.max_attempts }));
        } else {
            std.log.warn("Task #{d} failed ({s}), will retry ({d}/{d}): {s}", .{ task.id, reason, task.attempt + 1, task.max_attempts, detail[0..@min(detail.len, 200)] });
            self.notify(task.notify_chat, try std.fmt.allocPrint(self.allocator, "Task #{d} failed: {s} — retry {d}/{d}", .{ task.id, reason, task.attempt + 1, task.max_attempts }));
        }
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

test "mode phases have correct tools" {
    const spec = modes.swe_mode.getPhase("spec").?;
    const qa = modes.swe_mode.getPhase("qa").?;
    const impl = modes.swe_mode.getPhase("impl").?;

    // Spec and QA should not have Bash or Edit
    try std.testing.expect(std.mem.indexOf(u8, spec.allowed_tools, "Bash") == null);
    try std.testing.expect(std.mem.indexOf(u8, qa.allowed_tools, "Bash") == null);
    try std.testing.expect(std.mem.indexOf(u8, qa.allowed_tools, "Edit") == null);

    // Impl has Bash and Edit
    try std.testing.expect(std.mem.indexOf(u8, impl.allowed_tools, "Bash") != null);
    try std.testing.expect(std.mem.indexOf(u8, impl.allowed_tools, "Edit") != null);
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
