// Tests for spec #51: expireSessions time-offset string formatting in db.zig
//
// Covers acceptance criteria from spec.md:
//   AC1 — expireSessions(0) on empty DB succeeds without error
//   AC2 — expireSessions(0) deletes all past-dated sessions
//   AC3 — expireSessions(999999) deletes no current sessions
//   AC4 — expireSessions(mid-range) deletes only stale sessions
//   AC5 — expireSessions(999999) on empty DB succeeds without error
//   Edge cases E1–E6 from spec §5
//
// Sessions that must be reliably deleted by expireSessions(0) are inserted
// via raw SQL with an explicit past timestamp to avoid the same-SQLite-second
// ambiguity described in edge case E5.
//
// To include in the build, add to the test block in db.zig:
//   _ = @import("db_expire_sessions_test.zig");

const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;

// =============================================================================
// AC1 / E1 — expireSessions(0) on an empty DB returns no error
// =============================================================================

test "AC1: expireSessions(0) on empty DB succeeds without error" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    // Must not return an error
    try db.expireSessions(0);

    // Table is still empty — any folder lookup returns null
    const result = try db.getSession(arena.allocator(), "nonexistent");
    try std.testing.expect(result == null);
}

// =============================================================================
// AC2 / E3 — expireSessions(0) deletes all past-dated sessions
//
// Sessions are inserted with datetime('now', '-1 hour') so they are
// guaranteed to be older than datetime('now', '-0 hours') = datetime('now').
// =============================================================================

test "AC2: expireSessions(0) deletes single past-dated session" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.sqlite_db.exec(
        "INSERT INTO sessions (folder, session_id, created_at) VALUES ('old', 'sess-old', datetime('now', '-1 hour'))"
    );

    // Session is visible before expiry
    try std.testing.expect((try db.getSession(arena.allocator(), "old")) != null);

    try db.expireSessions(0);

    // Session must be gone after expiry
    try std.testing.expect((try db.getSession(arena.allocator(), "old")) == null);
}

test "AC2/E3: expireSessions(0) deletes multiple past-dated sessions" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.sqlite_db.exec(
        "INSERT INTO sessions (folder, session_id, created_at) VALUES ('f1', 's1', datetime('now', '-1 hour'))"
    );
    try db.sqlite_db.exec(
        "INSERT INTO sessions (folder, session_id, created_at) VALUES ('f2', 's2', datetime('now', '-2 hours'))"
    );
    try db.sqlite_db.exec(
        "INSERT INTO sessions (folder, session_id, created_at) VALUES ('f3', 's3', datetime('now', '-48 hours'))"
    );

    // All visible before expiry
    try std.testing.expect((try db.getSession(arena.allocator(), "f1")) != null);
    try std.testing.expect((try db.getSession(arena.allocator(), "f2")) != null);
    try std.testing.expect((try db.getSession(arena.allocator(), "f3")) != null);

    try db.expireSessions(0);

    // All must be gone
    try std.testing.expect((try db.getSession(arena.allocator(), "f1")) == null);
    try std.testing.expect((try db.getSession(arena.allocator(), "f2")) == null);
    try std.testing.expect((try db.getSession(arena.allocator(), "f3")) == null);
}

// =============================================================================
// AC3 / E4 / E6 — expireSessions(999999) deletes no current sessions
//
// 999999 hours ≈ 114 years in the past — no datetime('now') row satisfies
// created_at < datetime('now', '-999999 hours').
// Also verifies that the 14-char string "-999999 hours" fits within the
// 64-byte buffer (E4) and that session values are readable after the no-op (E6).
// =============================================================================

test "AC3/E4/E6: expireSessions(999999) keeps all current sessions intact" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    try db.setSession("alpha", "sess-alpha");
    try db.setSession("beta", "sess-beta");
    try db.setSession("gamma", "sess-gamma");

    try db.expireSessions(999999);

    // All sessions must still be present (E6: verify by value, not just non-null)
    const a = try db.getSession(arena.allocator(), "alpha");
    try std.testing.expect(a != null);
    try std.testing.expectEqualStrings("sess-alpha", a.?);

    const b = try db.getSession(arena.allocator(), "beta");
    try std.testing.expect(b != null);
    try std.testing.expectEqualStrings("sess-beta", b.?);

    const c = try db.getSession(arena.allocator(), "gamma");
    try std.testing.expect(c != null);
    try std.testing.expectEqualStrings("sess-gamma", c.?);
}

// =============================================================================
// AC4 — expireSessions(mid-range) deletes only stale sessions
//
// One session inserted 25 hours in the past (stale for a 4-hour threshold),
// one inserted at datetime('now') (fresh).
// =============================================================================

test "AC4: expireSessions(4) removes 25-hour-old session but keeps fresh one" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    // Old session: 25 hours in the past, outside the 4-hour window
    try db.sqlite_db.exec(
        "INSERT INTO sessions (folder, session_id, created_at) VALUES ('old', 'old-sess', datetime('now', '-25 hours'))"
    );
    // Fresh session: current time, inside the 4-hour window
    try db.setSession("fresh", "fresh-sess");

    // Both visible before expiry
    try std.testing.expect((try db.getSession(arena.allocator(), "old")) != null);
    try std.testing.expect((try db.getSession(arena.allocator(), "fresh")) != null);

    try db.expireSessions(4);

    // Old session is expired
    try std.testing.expect((try db.getSession(arena.allocator(), "old")) == null);
    // Fresh session survives (E6: verify by value)
    const fresh = try db.getSession(arena.allocator(), "fresh");
    try std.testing.expect(fresh != null);
    try std.testing.expectEqualStrings("fresh-sess", fresh.?);
}

// =============================================================================
// AC5 / E2 — expireSessions(999999) on an empty DB returns no error
// =============================================================================

test "AC5/E2: expireSessions(999999) on empty DB succeeds without error" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    // Must not return an error on an empty table
    try db.expireSessions(999999);
}

// =============================================================================
// E5 — same-second boundary: session created at datetime('now') is NOT deleted
//      by expireSessions(0) because the condition is strict less-than
// =============================================================================

test "E5: session created at datetime('now') survives expireSessions(0)" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    // Insert at datetime('now') — same second as the expiry threshold
    try db.setSession("same-second", "ss-sess");

    try db.expireSessions(0);

    // The session must NOT be deleted (strict < means equal timestamps survive)
    const result = try db.getSession(arena.allocator(), "same-second");
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("ss-sess", result.?);
}

// =============================================================================
// Additional: mixed old/fresh with expireSessions(0) — only past-dated rows go
// =============================================================================

test "expireSessions(0) deletes only past-dated rows when mixed with current" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    var db = try Db.init(arena.allocator(), ":memory:");
    defer db.deinit();

    // Past-dated (will be deleted)
    try db.sqlite_db.exec(
        "INSERT INTO sessions (folder, session_id, created_at) VALUES ('past', 'past-sess', datetime('now', '-1 hour'))"
    );
    // Current (must survive)
    try db.setSession("current", "cur-sess");

    try db.expireSessions(0);

    try std.testing.expect((try db.getSession(arena.allocator(), "past")) == null);

    const cur = try db.getSession(arena.allocator(), "current");
    try std.testing.expect(cur != null);
    try std.testing.expectEqualStrings("cur-sess", cur.?);
}
