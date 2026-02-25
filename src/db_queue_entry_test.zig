// Tests for spec #16: Extract rowToQueueEntry helper in db.zig
//
// Covers acceptance criteria from spec.md:
//   AC3 — Inline field-mapping block appears exactly once in db.zig
//   AC4 — rowToQueueEntry is private (not pub) and is defined
//   AC6 — Public signatures of getQueuedBranches / getQueuedBranchesForRepo
//          are unchanged and return correctly populated QueueEntry values
//   Edge cases from spec §5
//
// Structural tests (AC3, AC4) use @embedFile to read db.zig source at
// compile time so they are independent of the working directory at runtime.
//
// To include in the build, add to the test block in db.zig:
//   _ = @import("db_queue_entry_test.zig");

const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;
const QueueEntry = db_mod.QueueEntry;

/// db.zig source embedded at compile time — used by structural tests.
const db_source = @embedFile("db.zig");

// =============================================================================
// AC3 — The inline column-mapping block appears exactly ONCE in db.zig
//
// Before the refactor both getQueuedBranches and getQueuedBranchesForRepo
// contain the full struct literal, so the distinctive patterns appear twice.
// After the refactor they live only inside rowToQueueEntry → count must be 1.
// These tests FAIL before the refactor is applied.
// =============================================================================

fn countOccurrences(haystack: []const u8, needle: []const u8) usize {
    var count: usize = 0;
    var rest: []const u8 = haystack;
    while (std.mem.indexOf(u8, rest, needle)) |idx| {
        count += 1;
        rest = rest[idx + needle.len ..];
    }
    return count;
}

test "AC3: '.task_id = row.getInt(1)' appears exactly once in db.zig" {
    // Distinctive pattern from column index 1 of the QueueEntry mapping.
    // Count = 2 before refactor (one per function), 1 after.
    const n = countOccurrences(db_source, ".task_id = row.getInt(1)");
    try std.testing.expectEqual(@as(usize, 1), n);
}

test "AC3: '.pr_number = row.getInt(6)' appears exactly once in db.zig" {
    // Distinctive pattern from column index 6 of the QueueEntry mapping.
    const n = countOccurrences(db_source, ".pr_number = row.getInt(6)");
    try std.testing.expectEqual(@as(usize, 1), n);
}

test "AC3: QueueEntry struct literal not inside getQueuedBranches body" {
    // Locate the getQueuedBranches function body and verify it does NOT
    // contain the inline field mapping.
    const fn_start = std.mem.indexOf(u8, db_source, "pub fn getQueuedBranches(") orelse
        return error.FunctionNotFound;
    // Find the next pub fn after it to delimit the body.
    const body_start = fn_start + "pub fn getQueuedBranches(".len;
    const body = blk: {
        // Grab text until the next top-level "pub fn" or end-of-file.
        if (std.mem.indexOf(u8, db_source[body_start..], "\n    pub fn ")) |rel| {
            break :blk db_source[body_start .. body_start + rel];
        }
        break :blk db_source[body_start..];
    };
    const contains_inline = std.mem.indexOf(u8, body, ".task_id = row.getInt(1)") != null;
    try std.testing.expect(!contains_inline);
}

test "AC3: QueueEntry struct literal not inside getQueuedBranchesForRepo body" {
    const fn_start = std.mem.indexOf(u8, db_source, "pub fn getQueuedBranchesForRepo(") orelse
        return error.FunctionNotFound;
    const body_start = fn_start + "pub fn getQueuedBranchesForRepo(".len;
    const body = blk: {
        if (std.mem.indexOf(u8, db_source[body_start..], "\n    pub fn ")) |rel| {
            break :blk db_source[body_start .. body_start + rel];
        }
        break :blk db_source[body_start..];
    };
    const contains_inline = std.mem.indexOf(u8, body, ".task_id = row.getInt(1)") != null;
    try std.testing.expect(!contains_inline);
}

// =============================================================================
// AC4 — rowToQueueEntry must be defined and must NOT be pub
//
// "defined" test FAILS before the refactor (function does not exist yet).
// "not pub" test passes both before and after the refactor (it would only
// fail if someone incorrectly marks the helper pub).
// =============================================================================

test "AC4: rowToQueueEntry function is defined in db.zig" {
    const found = std.mem.indexOf(u8, db_source, "fn rowToQueueEntry(") != null;
    try std.testing.expect(found);
}

test "AC4: rowToQueueEntry is not declared pub" {
    const is_pub = std.mem.indexOf(u8, db_source, "pub fn rowToQueueEntry(") != null;
    try std.testing.expect(!is_pub);
}

// =============================================================================
// AC6 — getQueuedBranches: public signature unchanged, all 7 fields correct
// (regression tests — pass before and after the refactor)
// =============================================================================

test "AC6: getQueuedBranches returns empty slice when queue is empty" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const entries = try db.getQueuedBranches(arena.allocator());
    try std.testing.expectEqual(@as(usize, 0), entries.len);
}

test "AC6: getQueuedBranches maps all 7 QueueEntry fields correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const task_id = try db.createPipelineTask("Spec16 Task", "desc", "/myrepo", "", "");
    try db.enqueueForIntegration(task_id, "feature/spec16", "/myrepo");

    const entries = try db.getQueuedBranches(arena.allocator());
    try std.testing.expectEqual(@as(usize, 1), entries.len);

    const e = entries[0];
    try std.testing.expect(e.id > 0);                                    // id (col 0)
    try std.testing.expectEqual(task_id, e.task_id);                     // task_id (col 1)
    try std.testing.expectEqualStrings("feature/spec16", e.branch);      // branch (col 2)
    try std.testing.expectEqualStrings("/myrepo", e.repo_path);          // repo_path (col 3)
    try std.testing.expectEqualStrings("queued", e.status);              // status (col 4)
    try std.testing.expect(e.queued_at.len > 0);                         // queued_at (col 5)
    try std.testing.expectEqual(@as(i64, 0), e.pr_number);               // pr_number (col 6)
}

test "AC6: getQueuedBranches returns multiple entries ordered ASC by queued_at" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", "");
    const id3 = try db.createPipelineTask("T3", "d", "/repo", "", "");

    try db.enqueueForIntegration(id1, "branch-1", "/repo");
    try db.enqueueForIntegration(id2, "branch-2", "/repo");
    try db.enqueueForIntegration(id3, "branch-3", "/repo");

    const entries = try db.getQueuedBranches(arena.allocator());
    try std.testing.expectEqual(@as(usize, 3), entries.len);
    // Insertion order matches queue time order for an in-memory DB.
    try std.testing.expectEqual(id1, entries[0].task_id);
    try std.testing.expectEqual(id2, entries[1].task_id);
    try std.testing.expectEqual(id3, entries[2].task_id);
}

test "AC6: getQueuedBranches excludes entries with non-queued status" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id1 = try db.createPipelineTask("T1", "d", "/repo", "", "");
    const id2 = try db.createPipelineTask("T2", "d", "/repo", "", "");

    try db.enqueueForIntegration(id1, "branch-1", "/repo");
    try db.enqueueForIntegration(id2, "branch-2", "/repo");

    const all = try db.getQueuedBranches(arena.allocator());
    try std.testing.expectEqual(@as(usize, 2), all.len);

    try db.updateQueueStatus(all[0].id, "merged", null);

    const remaining = try db.getQueuedBranches(arena.allocator());
    try std.testing.expectEqual(@as(usize, 1), remaining.len);
    try std.testing.expectEqual(id2, remaining[0].task_id);
}

// =============================================================================
// AC6 — getQueuedBranchesForRepo: public signature unchanged, filters by repo
// =============================================================================

test "AC6: getQueuedBranchesForRepo returns empty slice when no entries for repo" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const entries = try db.getQueuedBranchesForRepo(arena.allocator(), "/nonexistent");
    try std.testing.expectEqual(@as(usize, 0), entries.len);
}

test "AC6: getQueuedBranchesForRepo maps all 7 QueueEntry fields correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const task_id = try db.createPipelineTask("T", "d", "/myrepo", "", "");
    try db.enqueueForIntegration(task_id, "fix/bug-42", "/myrepo");

    const entries = try db.getQueuedBranchesForRepo(arena.allocator(), "/myrepo");
    try std.testing.expectEqual(@as(usize, 1), entries.len);

    const e = entries[0];
    try std.testing.expect(e.id > 0);
    try std.testing.expectEqual(task_id, e.task_id);
    try std.testing.expectEqualStrings("fix/bug-42", e.branch);
    try std.testing.expectEqualStrings("/myrepo", e.repo_path);
    try std.testing.expectEqualStrings("queued", e.status);
    try std.testing.expect(e.queued_at.len > 0);
    try std.testing.expectEqual(@as(i64, 0), e.pr_number);
}

test "AC6: getQueuedBranchesForRepo filters entries to the requested repo only" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id_a = try db.createPipelineTask("TA", "d", "/repo-a", "", "");
    const id_b = try db.createPipelineTask("TB", "d", "/repo-b", "", "");
    const id_b2 = try db.createPipelineTask("TB2", "d", "/repo-b", "", "");

    try db.enqueueForIntegration(id_a, "branch-a", "/repo-a");
    try db.enqueueForIntegration(id_b, "branch-b1", "/repo-b");
    try db.enqueueForIntegration(id_b2, "branch-b2", "/repo-b");

    const a_entries = try db.getQueuedBranchesForRepo(arena.allocator(), "/repo-a");
    try std.testing.expectEqual(@as(usize, 1), a_entries.len);
    try std.testing.expectEqualStrings("/repo-a", a_entries[0].repo_path);

    const b_entries = try db.getQueuedBranchesForRepo(arena.allocator(), "/repo-b");
    try std.testing.expectEqual(@as(usize, 2), b_entries.len);
    for (b_entries) |e| {
        try std.testing.expectEqualStrings("/repo-b", e.repo_path);
    }
}

// =============================================================================
// AC6 — Consistency: both functions return identical field values for same row
// =============================================================================

test "AC6: getQueuedBranches and getQueuedBranchesForRepo agree on all fields" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const task_id = try db.createPipelineTask("Consistency", "d", "/shared-repo", "", "");
    try db.enqueueForIntegration(task_id, "feat/consistent", "/shared-repo");

    const all = try db.getQueuedBranches(arena.allocator());
    const by_repo = try db.getQueuedBranchesForRepo(arena.allocator(), "/shared-repo");

    try std.testing.expectEqual(@as(usize, 1), all.len);
    try std.testing.expectEqual(@as(usize, 1), by_repo.len);

    const a = all[0];
    const b = by_repo[0];

    try std.testing.expectEqual(a.id, b.id);
    try std.testing.expectEqual(a.task_id, b.task_id);
    try std.testing.expectEqualStrings(a.branch, b.branch);
    try std.testing.expectEqualStrings(a.repo_path, b.repo_path);
    try std.testing.expectEqualStrings(a.status, b.status);
    try std.testing.expectEqualStrings(a.queued_at, b.queued_at);
    try std.testing.expectEqual(a.pr_number, b.pr_number);
}

// =============================================================================
// Edge case: pr_number is stored and read back correctly
// =============================================================================

test "Edge: pr_number stored via updateQueuePrNumber is visible in getQueuedBranches" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const task_id = try db.createPipelineTask("T", "d", "/repo", "", "");
    try db.enqueueForIntegration(task_id, "my-branch", "/repo");

    const before = try db.getQueuedBranches(arena.allocator());
    try std.testing.expectEqual(@as(i64, 0), before[0].pr_number);

    try db.updateQueuePrNumber(before[0].id, 99);

    const after = try db.getQueuedBranches(arena.allocator());
    try std.testing.expectEqual(@as(usize, 1), after.len);
    try std.testing.expectEqual(@as(i64, 99), after[0].pr_number);
}

test "Edge: pr_number visible via getQueuedBranchesForRepo after updateQueuePrNumber" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const task_id = try db.createPipelineTask("T", "d", "/repo", "", "");
    try db.enqueueForIntegration(task_id, "my-branch", "/repo");

    const before = try db.getQueuedBranchesForRepo(arena.allocator(), "/repo");
    try db.updateQueuePrNumber(before[0].id, 101);

    const after = try db.getQueuedBranchesForRepo(arena.allocator(), "/repo");
    try std.testing.expectEqual(@as(usize, 1), after.len);
    try std.testing.expectEqual(@as(i64, 101), after[0].pr_number);
}

// =============================================================================
// Edge case: status field defaults to "queued" for every returned entry
// =============================================================================

test "Edge: every entry returned by getQueuedBranches has status 'queued'" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    for (0..4) |_| {
        const tid = try db.createPipelineTask("T", "d", "/repo", "", "");
        try db.enqueueForIntegration(tid, "b", "/repo");
    }

    const entries = try db.getQueuedBranches(arena.allocator());
    try std.testing.expectEqual(@as(usize, 4), entries.len);
    for (entries) |e| {
        try std.testing.expectEqualStrings("queued", e.status);
    }
}

// =============================================================================
// Edge case: enqueueForIntegration replaces a pre-existing queued entry for
// the same task — only the latest entry survives
// =============================================================================

test "Edge: re-enqueuing the same task_id replaces the previous queue entry" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const task_id = try db.createPipelineTask("T", "d", "/repo", "", "");
    try db.enqueueForIntegration(task_id, "old-branch", "/repo");
    try db.enqueueForIntegration(task_id, "new-branch", "/repo");

    const entries = try db.getQueuedBranches(arena.allocator());
    try std.testing.expectEqual(@as(usize, 1), entries.len);
    try std.testing.expectEqualStrings("new-branch", entries[0].branch);
}
