// Tests for spec #35: Add tests for db.zig proposal lifecycle operations
//
// Covers: createProposal, getProposal, getProposals, updateProposalStatus
//
// All allocations use an ArenaAllocator so string cleanup is handled
// automatically — no need to free individual Proposal fields in these tests.
//
// To include in the build, add to the test block in src/db.zig:
//   _ = @import("db_proposal_test.zig");

const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;
const Proposal = db_mod.Proposal;

// =============================================================================
// AC1 — createProposal returns a positive, auto-incremented ID
// =============================================================================

test "AC1: createProposal returns a positive ID" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createProposal("/repo", "Title", "Desc", "Rationale");
    try std.testing.expect(id > 0);
}

test "AC1: createProposal IDs are strictly increasing" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id1 = try db.createProposal("/repo", "First",  "D1", "R1");
    const id2 = try db.createProposal("/repo", "Second", "D2", "R2");
    try std.testing.expect(id2 > id1);
}

// =============================================================================
// AC2 — getProposal returns null for a nonexistent ID
// =============================================================================

test "AC2: getProposal returns null on empty DB" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const result = try db.getProposal(arena.allocator(), 9999);
    try std.testing.expect(result == null);
}

// =============================================================================
// AC3 — getProposal returns a correctly populated Proposal
// =============================================================================

test "AC3: getProposal maps all seven fields correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createProposal("/repo", "T", "D", "R");
    const p = (try db.getProposal(arena.allocator(), id)).?;

    try std.testing.expectEqual(id,        p.id);
    try std.testing.expectEqualStrings("/repo",   p.repo_path);
    try std.testing.expectEqualStrings("T",       p.title);
    try std.testing.expectEqualStrings("D",       p.description);
    try std.testing.expectEqualStrings("R",       p.rationale);
    try std.testing.expectEqualStrings("proposed", p.status);
    try std.testing.expect(p.created_at.len > 0);
}

// =============================================================================
// AC4 — New proposals default to "proposed" status
// =============================================================================

test "AC4: new proposal has status 'proposed' by default" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createProposal("/repo", "My Proposal", "desc", "why");
    const p = (try db.getProposal(arena.allocator(), id)).?;
    try std.testing.expectEqualStrings("proposed", p.status);
}

// =============================================================================
// AC5 — getProposals(null, limit) returns all proposals up to limit
// =============================================================================

test "AC5: getProposals returns all 3 proposals when limit is generous" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createProposal("/r", "P1", "d", "r");
    _ = try db.createProposal("/r", "P2", "d", "r");
    _ = try db.createProposal("/r", "P3", "d", "r");

    const list = try db.getProposals(arena.allocator(), null, 10);
    try std.testing.expectEqual(@as(usize, 3), list.len);
}

test "AC5: getProposals respects limit parameter" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createProposal("/r", "A", "d", "r");
    _ = try db.createProposal("/r", "B", "d", "r");
    _ = try db.createProposal("/r", "C", "d", "r");
    _ = try db.createProposal("/r", "D", "d", "r");
    _ = try db.createProposal("/r", "E", "d", "r");

    const list = try db.getProposals(arena.allocator(), null, 2);
    try std.testing.expectEqual(@as(usize, 2), list.len);
}

test "AC5: getProposals returns empty slice on empty DB" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const list = try db.getProposals(arena.allocator(), null, 10);
    try std.testing.expectEqual(@as(usize, 0), list.len);
}

// =============================================================================
// AC6 — getProposals with status_filter returns only matching rows
// =============================================================================

test "AC6: getProposals filters by 'proposed' status" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createProposal("/r", "P1", "d", "r");
    _ = try db.createProposal("/r", "P2", "d", "r");
    const id3 = try db.createProposal("/r", "P3", "d", "r");
    try db.updateProposalStatus(id3, "approved");

    const proposed = try db.getProposals(arena.allocator(), "proposed", 10);
    try std.testing.expectEqual(@as(usize, 2), proposed.len);
    for (proposed) |p| {
        try std.testing.expectEqualStrings("proposed", p.status);
    }
}

test "AC6: getProposals filters by 'approved' status" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createProposal("/r", "P1", "d", "r");
    _ = try db.createProposal("/r", "P2", "d", "r");
    const id3 = try db.createProposal("/r", "P3", "d", "r");
    try db.updateProposalStatus(id3, "approved");

    const approved = try db.getProposals(arena.allocator(), "approved", 10);
    try std.testing.expectEqual(@as(usize, 1), approved.len);
    try std.testing.expectEqualStrings("approved", approved[0].status);
}

test "AC6: getProposals with 'dismissed' filter returns empty when none dismissed" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createProposal("/r", "P1", "d", "r");
    _ = try db.createProposal("/r", "P2", "d", "r");

    const dismissed = try db.getProposals(arena.allocator(), "dismissed", 10);
    try std.testing.expectEqual(@as(usize, 0), dismissed.len);
}

// =============================================================================
// AC7 — getProposals(null, limit) returns proposals newest-first
// =============================================================================

test "AC7: getProposals returns proposals in newest-first order" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id_a = try db.createProposal("/r", "Older", "d", "r");
    const id_b = try db.createProposal("/r", "Newer", "d", "r");

    // Force distinct timestamps so the ORDER BY is deterministic.
    try db.sqlite_db.execute(
        "UPDATE proposals SET created_at = ?1 WHERE id = ?2",
        .{ "2024-01-01 10:00:00", id_a },
    );
    try db.sqlite_db.execute(
        "UPDATE proposals SET created_at = ?1 WHERE id = ?2",
        .{ "2024-01-01 11:00:00", id_b },
    );

    const list = try db.getProposals(arena.allocator(), null, 10);
    try std.testing.expectEqual(@as(usize, 2), list.len);
    // Newest (B) must be first.
    try std.testing.expectEqual(id_b, list[0].id);
    try std.testing.expectEqual(id_a, list[1].id);
}

// =============================================================================
// AC8 — updateProposalStatus: proposed → approved
// =============================================================================

test "AC8: updateProposalStatus transitions proposed to approved" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createProposal("/r", "P", "d", "r");
    try db.updateProposalStatus(id, "approved");

    const p = (try db.getProposal(arena.allocator(), id)).?;
    try std.testing.expectEqualStrings("approved", p.status);
}

// =============================================================================
// AC9 — updateProposalStatus: proposed → dismissed
// =============================================================================

test "AC9: updateProposalStatus transitions proposed to dismissed" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createProposal("/r", "P", "d", "r");
    try db.updateProposalStatus(id, "dismissed");

    const p = (try db.getProposal(arena.allocator(), id)).?;
    try std.testing.expectEqualStrings("dismissed", p.status);
}

// =============================================================================
// AC10 — updateProposalStatus does not affect other proposals
// =============================================================================

test "AC10: updateProposalStatus only changes the targeted proposal" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id_a = try db.createProposal("/r", "A", "d", "r");
    const id_b = try db.createProposal("/r", "B", "d", "r");

    try db.updateProposalStatus(id_a, "approved");

    const b = (try db.getProposal(arena.allocator(), id_b)).?;
    try std.testing.expectEqualStrings("proposed", b.status);
}

// =============================================================================
// AC11 — updateProposalStatus is idempotent
// =============================================================================

test "AC11: updateProposalStatus called twice with same value does not error" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createProposal("/r", "P", "d", "r");
    try db.updateProposalStatus(id, "approved");
    try db.updateProposalStatus(id, "approved"); // second call must not error

    const p = (try db.getProposal(arena.allocator(), id)).?;
    try std.testing.expectEqualStrings("approved", p.status);
}

// =============================================================================
// AC12 — Full lifecycle: create → retrieve → transition → re-retrieve
// =============================================================================

test "AC12: full proposal lifecycle create→proposed→approved→dismissed" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    // Create
    const id = try db.createProposal("/lifecycle/repo", "Lifecycle Proposal", "Full desc", "Full rationale");

    // Verify initial state via getProposal
    {
        const p = (try db.getProposal(arena.allocator(), id)).?;
        try std.testing.expectEqualStrings("proposed", p.status);
    }

    // Transition to approved
    try db.updateProposalStatus(id, "approved");

    // Verify via both getProposal and getProposals
    {
        const p = (try db.getProposal(arena.allocator(), id)).?;
        try std.testing.expectEqualStrings("approved", p.status);
    }
    {
        const list = try db.getProposals(arena.allocator(), null, 10);
        try std.testing.expectEqual(@as(usize, 1), list.len);
        try std.testing.expectEqualStrings("approved", list[0].status);
    }

    // Transition to dismissed
    try db.updateProposalStatus(id, "dismissed");

    // Verify via getProposal
    {
        const p = (try db.getProposal(arena.allocator(), id)).?;
        try std.testing.expectEqualStrings("dismissed", p.status);
    }

    // Appears in dismissed filter, absent from proposed and approved filters
    {
        const dismissed_list = try db.getProposals(arena.allocator(), "dismissed", 10);
        try std.testing.expectEqual(@as(usize, 1), dismissed_list.len);
        try std.testing.expectEqual(id, dismissed_list[0].id);
    }
    {
        const proposed_list = try db.getProposals(arena.allocator(), "proposed", 10);
        try std.testing.expectEqual(@as(usize, 0), proposed_list.len);
    }
    {
        const approved_list = try db.getProposals(arena.allocator(), "approved", 10);
        try std.testing.expectEqual(@as(usize, 0), approved_list.len);
    }
}

// =============================================================================
// E1 — Multiple proposals for the same repo_path
// =============================================================================

test "E1: two proposals with the same repo_path get distinct IDs" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id1 = try db.createProposal("/same/repo", "First",  "d1", "r1");
    const id2 = try db.createProposal("/same/repo", "Second", "d2", "r2");

    try std.testing.expect(id1 != id2);

    const list = try db.getProposals(arena.allocator(), null, 10);
    try std.testing.expectEqual(@as(usize, 2), list.len);
}

// =============================================================================
// E2 — getProposals with limit=0 returns an empty slice
// =============================================================================

test "E2: getProposals with limit=0 returns empty even when proposals exist" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createProposal("/r", "P", "d", "r");

    const list = try db.getProposals(arena.allocator(), null, 0);
    try std.testing.expectEqual(@as(usize, 0), list.len);
}

// =============================================================================
// E3 — Long description and rationale stored without truncation
// =============================================================================

test "E3: long description and rationale survive round-trip without truncation" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const long_desc = "D" ** 4000;
    const long_rat  = "R" ** 4000;

    const id = try db.createProposal("/r", "Long Fields", long_desc, long_rat);
    const p = (try db.getProposal(arena.allocator(), id)).?;

    try std.testing.expectEqual(@as(usize, 4000), p.description.len);
    try std.testing.expectEqual(@as(usize, 4000), p.rationale.len);
    try std.testing.expectEqualStrings(long_desc, p.description);
    try std.testing.expectEqualStrings(long_rat,  p.rationale);
}

// =============================================================================
// E4 — getProposal after deletion returns null
// =============================================================================

test "E4: getProposal returns null after the row is deleted" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createProposal("/r", "P", "d", "r");
    try db.sqlite_db.execute("DELETE FROM proposals WHERE id = ?1", .{id});

    const result = try db.getProposal(arena.allocator(), id);
    try std.testing.expect(result == null);
}

// =============================================================================
// E5 — updateProposalStatus for a nonexistent ID is a no-op (no error)
// =============================================================================

test "E5: updateProposalStatus on nonexistent ID does not error" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    // Must not return an error — SQLite UPDATE with zero matching rows is success.
    try db.updateProposalStatus(99999, "approved");
}

// =============================================================================
// E6 — getProposals with status_filter matching no rows returns empty slice
// =============================================================================

test "E6: getProposals with non-matching status filter returns empty slice" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createProposal("/r", "P1", "d", "r");
    _ = try db.createProposal("/r", "P2", "d", "r");

    const list = try db.getProposals(arena.allocator(), "dismissed", 10);
    try std.testing.expectEqual(@as(usize, 0), list.len);
}

// =============================================================================
// E7 — created_at is DB-generated and non-empty
// =============================================================================

test "E7: getProposal always returns a non-empty created_at" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const id = try db.createProposal("/r", "P", "d", "r");
    const p = (try db.getProposal(arena.allocator(), id)).?;
    try std.testing.expect(p.created_at.len > 0);
}

test "E7: getProposals always returns non-empty created_at for each row" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    _ = try db.createProposal("/r", "P1", "d", "r");
    _ = try db.createProposal("/r", "P2", "d", "r");

    const list = try db.getProposals(arena.allocator(), null, 10);
    for (list) |p| {
        try std.testing.expect(p.created_at.len > 0);
    }
}
