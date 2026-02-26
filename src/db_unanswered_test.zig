// Tests for Task #71: getUnansweredMessages logic in db.zig.
//
// Covers the following acceptance criteria from spec.md:
//   AC1 — A group with a recent user message and no bot reply is returned.
//   AC2 — A group where bot_ts >= user_ts is NOT returned.
//   AC3 — A group whose latest user message is older than max_age_s is excluded.
//   AC4 — A group with no messages at all is excluded.
//   AC5 — Multiple groups with independent state are all evaluated in one call.
//
// Edge cases:
//   E1 — Empty registered_groups table → empty result, no error.
//   E2 — user_ts == bot_ts (strictly-greater check) → not returned.
//   E3 — Group with only bot messages (no user messages) → not returned.
//   E4 — max_age_s = 0 excludes all pre-inserted fixed timestamps.
//   E5 — last_user_ts round-trips the stored timestamp string exactly.
//
// To include in the build, the following line must appear in the `test {}` block
// in src/db.zig:
//   _ = @import("db_unanswered_test.zig");

const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;

// =============================================================================
// AC1 — Group with recent user message and no bot reply is returned
// =============================================================================

test "AC1: group with recent user message and no bot reply appears in result" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("g1", "Group One", "folder1", "@Bot", false);
    // Insert a user message timestamped right now so the age filter passes.
    try db.sqlite_db.exec(
        "INSERT INTO messages (id, chat_jid, sender, sender_name, content, timestamp, is_from_me, is_bot_message) " ++
            "VALUES ('m1', 'g1', 'user', 'User', 'hello', datetime('now'), 0, 0)",
    );

    const results = try db.getUnansweredMessages(alloc, 86400);

    try std.testing.expectEqual(@as(usize, 1), results.len);
    try std.testing.expectEqualStrings("g1", results[0].jid);
}

test "AC1: returned entry has a non-empty last_user_ts" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("g1", "Group One", "folder1", "@Bot", false);
    try db.sqlite_db.exec(
        "INSERT INTO messages (id, chat_jid, sender, sender_name, content, timestamp, is_from_me, is_bot_message) " ++
            "VALUES ('m1', 'g1', 'user', 'User', 'ping', datetime('now'), 0, 0)",
    );

    const results = try db.getUnansweredMessages(alloc, 86400);

    try std.testing.expect(results.len > 0);
    try std.testing.expect(results[0].last_user_ts.len > 0);
}

// =============================================================================
// AC2 — Group where bot message is newer than (or equal to) user message
//        is NOT returned
// =============================================================================

test "AC2: group where bot_ts is newer than user_ts is not returned" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("g2", "Group Two", "folder2", "@Bot", false);
    // User message at 10:00, bot reply at 11:00.
    try db.storeMessage(.{
        .id = "u1",
        .chat_jid = "g2",
        .sender = "user",
        .sender_name = "User",
        .content = "question",
        .timestamp = "2024-01-01T10:00:00",
        .is_from_me = false,
        .is_bot_message = false,
    });
    try db.storeMessage(.{
        .id = "b1",
        .chat_jid = "g2",
        .sender = "bot",
        .sender_name = "Bot",
        .content = "answer",
        .timestamp = "2024-01-01T11:00:00",
        .is_from_me = true,
        .is_bot_message = true,
    });

    // Use a very large max_age_s so the age filter is not a factor.
    const results = try db.getUnansweredMessages(alloc, 86400 * 365 * 10);

    try std.testing.expectEqual(@as(usize, 0), results.len);
}

test "AC2: group where bot_ts equals user_ts is not returned" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("g2eq", "Group Two Equal", "folder2eq", "@Bot", false);
    // Identical timestamps — lexicographic comparison is .eq, not .gt.
    try db.storeMessage(.{
        .id = "u1",
        .chat_jid = "g2eq",
        .sender = "user",
        .sender_name = "User",
        .content = "hi",
        .timestamp = "2024-06-01T09:00:00",
        .is_from_me = false,
        .is_bot_message = false,
    });
    try db.storeMessage(.{
        .id = "b1",
        .chat_jid = "g2eq",
        .sender = "bot",
        .sender_name = "Bot",
        .content = "hi back",
        .timestamp = "2024-06-01T09:00:00",
        .is_from_me = true,
        .is_bot_message = true,
    });

    const results = try db.getUnansweredMessages(alloc, 86400 * 365 * 10);

    try std.testing.expectEqual(@as(usize, 0), results.len);
}

test "AC2: group where user_ts is older than bot_ts is not returned even with multiple messages" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("g2m", "Group Multi", "folder2m", "@Bot", false);
    // Two user messages; bot reply is newer than both.
    try db.storeMessage(.{ .id = "u1", .chat_jid = "g2m", .sender = "u", .sender_name = "U", .content = "a", .timestamp = "2024-03-01T08:00:00", .is_from_me = false, .is_bot_message = false });
    try db.storeMessage(.{ .id = "u2", .chat_jid = "g2m", .sender = "u", .sender_name = "U", .content = "b", .timestamp = "2024-03-01T09:00:00", .is_from_me = false, .is_bot_message = false });
    try db.storeMessage(.{ .id = "b1", .chat_jid = "g2m", .sender = "bot", .sender_name = "Bot", .content = "ok", .timestamp = "2024-03-01T10:00:00", .is_from_me = true, .is_bot_message = true });

    const results = try db.getUnansweredMessages(alloc, 86400 * 365 * 10);

    try std.testing.expectEqual(@as(usize, 0), results.len);
}

// =============================================================================
// AC3 — Group where user message is older than max_age_s is excluded
// =============================================================================

test "AC3: group with user message older than max_age_s is excluded" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("g3", "Group Three", "folder3", "@Bot", false);
    // Year-2000 timestamp — far outside any reasonable max_age_s window.
    try db.storeMessage(.{
        .id = "u1",
        .chat_jid = "g3",
        .sender = "user",
        .sender_name = "User",
        .content = "old message",
        .timestamp = "2000-01-01T00:00:00",
        .is_from_me = false,
        .is_bot_message = false,
    });

    // max_age_s = 3600 (1 hour) — the year-2000 message is far too old.
    const results = try db.getUnansweredMessages(alloc, 3600);

    try std.testing.expectEqual(@as(usize, 0), results.len);
}

test "AC3: group with old user message is excluded regardless of bot reply absence" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("g3b", "Group Three B", "folder3b", "@Bot", false);
    try db.storeMessage(.{
        .id = "u1",
        .chat_jid = "g3b",
        .sender = "user",
        .sender_name = "User",
        .content = "ancient",
        .timestamp = "2000-06-15T12:00:00",
        .is_from_me = false,
        .is_bot_message = false,
    });
    // No bot message: unanswered = true, but age filter should still reject it.

    const results = try db.getUnansweredMessages(alloc, 86400); // 24 h max age

    try std.testing.expectEqual(@as(usize, 0), results.len);
}

// =============================================================================
// AC4 — Group with no messages at all is excluded
// =============================================================================

test "AC4: registered group with no messages is not returned" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("g4", "Group Four", "folder4", "@Bot", false);
    // No messages inserted.

    const results = try db.getUnansweredMessages(alloc, 86400);

    try std.testing.expectEqual(@as(usize, 0), results.len);
}

// =============================================================================
// AC5 — Multiple groups handled independently in a single call
// =============================================================================

test "AC5: only the unanswered recent group appears when four groups have different states" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // gA: recent user message, no bot reply — SHOULD appear.
    try db.registerGroup("gA", "Group A", "folderA", "@Bot", false);
    try db.sqlite_db.exec(
        "INSERT INTO messages (id, chat_jid, sender, sender_name, content, timestamp, is_from_me, is_bot_message) " ++
            "VALUES ('mA1', 'gA', 'user', 'User', 'need help', datetime('now'), 0, 0)",
    );

    // gB: user message older than bot reply — should NOT appear.
    try db.registerGroup("gB", "Group B", "folderB", "@Bot", false);
    try db.storeMessage(.{ .id = "mB1", .chat_jid = "gB", .sender = "u", .sender_name = "U", .content = "q", .timestamp = "2024-02-10T08:00:00", .is_from_me = false, .is_bot_message = false });
    try db.storeMessage(.{ .id = "mB2", .chat_jid = "gB", .sender = "bot", .sender_name = "Bot", .content = "a", .timestamp = "2024-02-10T09:00:00", .is_from_me = true, .is_bot_message = true });

    // gC: very old user message, no bot reply — should NOT appear (age filter).
    try db.registerGroup("gC", "Group C", "folderC", "@Bot", false);
    try db.storeMessage(.{ .id = "mC1", .chat_jid = "gC", .sender = "u", .sender_name = "U", .content = "old", .timestamp = "2000-01-01T00:00:00", .is_from_me = false, .is_bot_message = false });

    // gD: no messages — should NOT appear.
    try db.registerGroup("gD", "Group D", "folderD", "@Bot", false);

    // max_age_s = 3600 (1 hour): only gA's datetime('now') message passes.
    const results = try db.getUnansweredMessages(alloc, 3600);

    try std.testing.expectEqual(@as(usize, 1), results.len);
    try std.testing.expectEqualStrings("gA", results[0].jid);
}

test "AC5: two groups both unanswered and recent — both appear" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("gX", "Group X", "folderX", "@Bot", false);
    try db.registerGroup("gY", "Group Y", "folderY", "@Bot", false);

    try db.sqlite_db.exec(
        "INSERT INTO messages (id, chat_jid, sender, sender_name, content, timestamp, is_from_me, is_bot_message) " ++
            "VALUES ('mX1', 'gX', 'user', 'User', 'msg', datetime('now'), 0, 0)",
    );
    try db.sqlite_db.exec(
        "INSERT INTO messages (id, chat_jid, sender, sender_name, content, timestamp, is_from_me, is_bot_message) " ++
            "VALUES ('mY1', 'gY', 'user', 'User', 'msg', datetime('now'), 0, 0)",
    );

    const results = try db.getUnansweredMessages(alloc, 86400);

    try std.testing.expectEqual(@as(usize, 2), results.len);
    // Both jids must be present (order may vary).
    var found_x = false;
    var found_y = false;
    for (results) |r| {
        if (std.mem.eql(u8, r.jid, "gX")) found_x = true;
        if (std.mem.eql(u8, r.jid, "gY")) found_y = true;
    }
    try std.testing.expect(found_x);
    try std.testing.expect(found_y);
}

// =============================================================================
// E1 — Empty registered_groups table returns empty slice without error
// =============================================================================

test "E1: empty registered_groups table returns empty result" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();
    // No groups, no messages.

    const results = try db.getUnansweredMessages(alloc, 86400);

    try std.testing.expectEqual(@as(usize, 0), results.len);
}

// =============================================================================
// E2 — user_ts == bot_ts: strictly-greater check means group is NOT returned
// =============================================================================

test "E2: group with user_ts equal to bot_ts is not returned" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("gEq", "Equal TS Group", "folderEq", "@Bot", false);
    const ts = "2024-09-20T15:30:00";
    try db.storeMessage(.{ .id = "u1", .chat_jid = "gEq", .sender = "u", .sender_name = "U", .content = "msg", .timestamp = ts, .is_from_me = false, .is_bot_message = false });
    try db.storeMessage(.{ .id = "b1", .chat_jid = "gEq", .sender = "bot", .sender_name = "Bot", .content = "reply", .timestamp = ts, .is_from_me = true, .is_bot_message = true });

    const results = try db.getUnansweredMessages(alloc, 86400 * 365 * 10);

    try std.testing.expectEqual(@as(usize, 0), results.len);
}

// =============================================================================
// E3 — Group with only bot messages (no user messages) is not returned
// =============================================================================

test "E3: group with only bot messages and no user messages is excluded" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("gBot", "Bot-Only Group", "folderBot", "@Bot", false);
    // Only a bot message; is_bot_message = true → user_ts query returns nothing.
    try db.storeMessage(.{
        .id = "b1",
        .chat_jid = "gBot",
        .sender = "bot",
        .sender_name = "Bot",
        .content = "scheduled announcement",
        .timestamp = "2024-07-04T12:00:00",
        .is_from_me = true,
        .is_bot_message = true,
    });

    const results = try db.getUnansweredMessages(alloc, 86400 * 365 * 10);

    try std.testing.expectEqual(@as(usize, 0), results.len);
}

// =============================================================================
// E4 — max_age_s = 0 excludes all pre-inserted fixed timestamps
// =============================================================================

test "E4: max_age_s=0 excludes all messages with pre-inserted timestamps" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("gZ", "Zero Age Group", "folderZ", "@Bot", false);
    // Insert a message with a fixed past timestamp.
    try db.storeMessage(.{
        .id = "u1",
        .chat_jid = "gZ",
        .sender = "user",
        .sender_name = "User",
        .content = "hi",
        .timestamp = "2024-01-15T14:30:00",
        .is_from_me = false,
        .is_bot_message = false,
    });

    // max_age_s = 0: the age window is datetime('now') ± 0, so any past
    // fixed timestamp fails the age check.
    const results = try db.getUnansweredMessages(alloc, 0);

    try std.testing.expectEqual(@as(usize, 0), results.len);
}

// =============================================================================
// E5 — last_user_ts round-trips the stored timestamp string exactly
// =============================================================================

test "E5: last_user_ts matches the stored timestamp string exactly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("gTs", "Timestamp Group", "folderTs", "@Bot", false);
    // Use a fixed, known timestamp that is recent enough under a 10-year window.
    const known_ts = "2024-06-15T10:00:00";
    try db.storeMessage(.{
        .id = "u1",
        .chat_jid = "gTs",
        .sender = "user",
        .sender_name = "User",
        .content = "check timestamp",
        .timestamp = known_ts,
        .is_from_me = false,
        .is_bot_message = false,
    });

    // 10-year window: any 2024 timestamp is well within range of 2026 'now'.
    const results = try db.getUnansweredMessages(alloc, 86400 * 365 * 10);

    try std.testing.expectEqual(@as(usize, 1), results.len);
    try std.testing.expectEqualStrings(known_ts, results[0].last_user_ts);
}

test "E5: latest user message timestamp is returned when multiple user messages exist" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.registerGroup("gLast", "Latest TS Group", "folderLast", "@Bot", false);
    // Two user messages; the later one should be in last_user_ts.
    try db.storeMessage(.{ .id = "u1", .chat_jid = "gLast", .sender = "u", .sender_name = "U", .content = "first", .timestamp = "2024-05-01T08:00:00", .is_from_me = false, .is_bot_message = false });
    try db.storeMessage(.{ .id = "u2", .chat_jid = "gLast", .sender = "u", .sender_name = "U", .content = "second", .timestamp = "2024-05-01T09:00:00", .is_from_me = false, .is_bot_message = false });

    const results = try db.getUnansweredMessages(alloc, 86400 * 365 * 10);

    try std.testing.expectEqual(@as(usize, 1), results.len);
    try std.testing.expectEqualStrings("2024-05-01T09:00:00", results[0].last_user_ts);
}
