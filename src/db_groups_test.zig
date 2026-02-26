// Tests for spec #69: Add tests for db.zig registered_groups CRUD
//
// Covers: registerGroup, getAllGroups, unregisterGroup
//
// The `requires_trigger` field is stored as SQLite INTEGER (0/1) and must
// round-trip correctly as a Zig bool.  All allocations use an ArenaAllocator
// so string cleanup is handled automatically — no need to free individual
// RegisteredGroup fields in these tests.
//
// To include in the build, add to the test block in src/db.zig:
//   _ = @import("db_groups_test.zig");

const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;
const RegisteredGroup = db_mod.RegisteredGroup;

// =============================================================================
// AC1 — requires_trigger=true round-trips as true
// =============================================================================

test "AC1: registerGroup with requires_trigger=true is retrieved as true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.registerGroup("jid:1", "Group One", "folder-one", "@Borg", true);

    const groups = try db.getAllGroups(arena.allocator());
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    try std.testing.expect(groups[0].requires_trigger == true);
}

// =============================================================================
// AC2 — requires_trigger=false round-trips as false
// =============================================================================

test "AC2: registerGroup with requires_trigger=false is retrieved as false" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.registerGroup("jid:2", "Group Two", "folder-two", "@Borg", false);

    const groups = try db.getAllGroups(arena.allocator());
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    try std.testing.expect(groups[0].requires_trigger == false);
}

// =============================================================================
// AC3 — Upsert semantics: re-registering the same JID updates, not duplicates
// =============================================================================

test "AC3: re-registering same JID produces exactly one row" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.registerGroup("jid:dup", "OldName", "folder-dup", "@Borg", true);
    try db.registerGroup("jid:dup", "NewName", "folder-dup", "@Borg", false);

    const groups = try db.getAllGroups(arena.allocator());
    try std.testing.expectEqual(@as(usize, 1), groups.len);
}

test "AC3: upsert updates name field to the new value" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.registerGroup("jid:dup", "OldName", "folder-dup", "@Borg", true);
    try db.registerGroup("jid:dup", "NewName", "folder-dup", "@Borg", false);

    const groups = try db.getAllGroups(arena.allocator());
    try std.testing.expectEqualStrings("NewName", groups[0].name);
}

test "AC3: upsert updates requires_trigger to the new value" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.registerGroup("jid:dup", "OldName", "folder-dup", "@Borg", true);
    try db.registerGroup("jid:dup", "NewName", "folder-dup", "@Borg", false);

    const groups = try db.getAllGroups(arena.allocator());
    try std.testing.expect(groups[0].requires_trigger == false);
}

// =============================================================================
// AC4 — getAllGroups returns an empty slice on a fresh database
// =============================================================================

test "AC4: getAllGroups returns empty slice when no groups registered" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    const groups = try db.getAllGroups(arena.allocator());
    try std.testing.expectEqual(@as(usize, 0), groups.len);
}

// =============================================================================
// AC5 — unregisterGroup removes the target and leaves others untouched
// =============================================================================

test "AC5: unregisterGroup reduces group count by one" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.registerGroup("g1", "Group 1", "folder1", "@Borg", true);
    try db.registerGroup("g2", "Group 2", "folder2", "@Borg", true);
    try db.unregisterGroup("g1");

    const groups = try db.getAllGroups(arena.allocator());
    try std.testing.expectEqual(@as(usize, 1), groups.len);
}

test "AC5: unregisterGroup leaves the non-deleted entry intact" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.registerGroup("g1", "Group 1", "folder1", "@Borg", true);
    try db.registerGroup("g2", "Group 2", "folder2", "@Borg", true);
    try db.unregisterGroup("g1");

    const groups = try db.getAllGroups(arena.allocator());
    try std.testing.expectEqualStrings("g2", groups[0].jid);
}

// =============================================================================
// AC6 — Default trigger value "@Borg" is stored and retrieved correctly
// =============================================================================

test "AC6: trigger value '@Borg' survives the round-trip" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.registerGroup("jid:borg", "Borg Group", "borg-folder", "@Borg", true);

    const groups = try db.getAllGroups(arena.allocator());
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    try std.testing.expectEqualStrings("@Borg", groups[0].trigger);
}

// =============================================================================
// E1 — unregisterGroup on a nonexistent JID is a no-op (no error)
// =============================================================================

test "E1: unregisterGroup on nonexistent JID does not error" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    // Must not return an error — DELETE with zero matching rows is success.
    try db.unregisterGroup("does-not-exist");
}

test "E1: unregisterGroup on nonexistent JID leaves DB empty" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.unregisterGroup("does-not-exist");

    const groups = try db.getAllGroups(arena.allocator());
    try std.testing.expectEqual(@as(usize, 0), groups.len);
}

// =============================================================================
// E2 — requires_trigger=false is stored as integer 0 (not merely non-1)
// =============================================================================

test "E2: requires_trigger=false is stored as 0 and read back as false" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.registerGroup("jid:zero", "Zero Group", "zero-folder", "@Borg", false);

    const groups = try db.getAllGroups(arena.allocator());
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    // Explicitly check the field value is false, not just "not true".
    try std.testing.expectEqual(false, groups[0].requires_trigger);
}

test "E2: requires_trigger=true is stored as 1 and read back as true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.registerGroup("jid:one", "One Group", "one-folder", "@Borg", true);

    const groups = try db.getAllGroups(arena.allocator());
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    try std.testing.expectEqual(true, groups[0].requires_trigger);
}

// =============================================================================
// E3 — All five fields round-trip without truncation or corruption
// =============================================================================

test "E3: all five RegisteredGroup fields survive the round-trip" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.registerGroup("jid:full", "Full Name", "full-folder", "!custom", false);

    const groups = try db.getAllGroups(arena.allocator());
    try std.testing.expectEqual(@as(usize, 1), groups.len);

    const g = groups[0];
    try std.testing.expectEqualStrings("jid:full",    g.jid);
    try std.testing.expectEqualStrings("Full Name",   g.name);
    try std.testing.expectEqualStrings("full-folder", g.folder);
    try std.testing.expectEqualStrings("!custom",     g.trigger);
    try std.testing.expectEqual(false,                g.requires_trigger);
}

// =============================================================================
// E4 — Multiple independent groups coexist; getAllGroups returns all
// =============================================================================

test "E4: three registered groups are all returned by getAllGroups" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.registerGroup("jid:a", "Alpha",   "folder-a", "@Borg", true);
    try db.registerGroup("jid:b", "Beta",    "folder-b", "@Borg", false);
    try db.registerGroup("jid:c", "Gamma",   "folder-c", "@Borg", true);

    const groups = try db.getAllGroups(arena.allocator());
    try std.testing.expectEqual(@as(usize, 3), groups.len);
}

// =============================================================================
// E5 — Upsert preserves folder and trigger from the replacement call
// =============================================================================

test "E5: upsert updates folder and trigger to the new values" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.registerGroup("jid:upd", "Name", "old-folder", "!old", true);
    try db.registerGroup("jid:upd", "Name", "new-folder", "!new", false);

    const groups = try db.getAllGroups(arena.allocator());
    try std.testing.expectEqual(@as(usize, 1), groups.len);
    try std.testing.expectEqualStrings("new-folder", groups[0].folder);
    try std.testing.expectEqualStrings("!new",       groups[0].trigger);
}

test "E5: upsert does not retain any old field values" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.registerGroup("jid:upd", "OldName", "old-folder", "!old", true);
    try db.registerGroup("jid:upd", "NewName", "new-folder", "!new", false);

    const groups = try db.getAllGroups(arena.allocator());
    const g = groups[0];

    // None of the old values must appear.
    try std.testing.expect(!std.mem.eql(u8, g.name,   "OldName"));
    try std.testing.expect(!std.mem.eql(u8, g.folder, "old-folder"));
    try std.testing.expect(!std.mem.eql(u8, g.trigger, "!old"));
    try std.testing.expect(g.requires_trigger == false);
}
