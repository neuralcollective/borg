// Tests for is_bot_message fix: verifies that is_bot_message is independent of is_from_me.
//
// To include in the build, add to build.zig or reference from an existing module:
//   test { _ = @import("is_bot_message_test.zig"); }
//
// All tests below should FAIL before the fix is applied (the Message struct
// lacks the is_bot_message field, causing compile errors).

const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;

// =============================================================================
// AC1: Struct field exists — Message has is_bot_message: bool
// =============================================================================

test "AC1: Message struct has is_bot_message field of type bool" {
    // Constructing a Message with is_bot_message proves the field exists.
    // If the field is missing, this is a compile error.
    const msg = db_mod.Message{
        .id = "test",
        .chat_jid = "chat",
        .sender = "s",
        .sender_name = "n",
        .content = "c",
        .timestamp = "t",
        .is_from_me = false,
        .is_bot_message = false,
    };
    // Verify the field type is bool at comptime
    try std.testing.expect(@TypeOf(msg.is_bot_message) == bool);
}

test "AC1: is_bot_message can be set to true" {
    const msg = db_mod.Message{
        .id = "test",
        .chat_jid = "chat",
        .sender = "s",
        .sender_name = "n",
        .content = "c",
        .timestamp = "t",
        .is_from_me = false,
        .is_bot_message = true,
    };
    try std.testing.expect(msg.is_bot_message == true);
}

// =============================================================================
// AC2: Independent SQL binding — is_bot_message bound to msg.is_bot_message
// =============================================================================

test "AC2: storeMessage writes is_from_me=1, is_bot_message=0 when fields differ" {
    // This is the key bug-detection test. Before the fix, both columns
    // mirror is_from_me, so is_bot_message would incorrectly be 1.
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "msg-ac2a",
        .chat_jid = "chat:ac2",
        .sender = "human",
        .sender_name = "Human",
        .content = "sent from bot account by human",
        .timestamp = "2024-01-01T00:00:00Z",
        .is_from_me = true,
        .is_bot_message = false,
    });

    // Verify raw database values to ensure the SQL binding is correct
    var rows = try db.sqlite_db.query(
        alloc,
        "SELECT is_from_me, is_bot_message FROM messages WHERE id = ?1 AND chat_jid = ?2",
        .{ @as([]const u8, "msg-ac2a"), @as([]const u8, "chat:ac2") },
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    // is_from_me must be 1
    try std.testing.expectEqual(@as(i64, 1), rows.items[0].getInt(0).?);
    // is_bot_message must be 0 (NOT mirroring is_from_me)
    try std.testing.expectEqual(@as(i64, 0), rows.items[0].getInt(1).?);
}

test "AC2: storeMessage writes is_from_me=0, is_bot_message=1 when fields differ" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "msg-ac2b",
        .chat_jid = "chat:ac2",
        .sender = "relay",
        .sender_name = "Relay",
        .content = "relayed bot message",
        .timestamp = "2024-01-01T00:00:01Z",
        .is_from_me = false,
        .is_bot_message = true,
    });

    var rows = try db.sqlite_db.query(
        alloc,
        "SELECT is_from_me, is_bot_message FROM messages WHERE id = ?1 AND chat_jid = ?2",
        .{ @as([]const u8, "msg-ac2b"), @as([]const u8, "chat:ac2") },
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    // is_from_me must be 0
    try std.testing.expectEqual(@as(i64, 0), rows.items[0].getInt(0).?);
    // is_bot_message must be 1 (NOT mirroring is_from_me)
    try std.testing.expectEqual(@as(i64, 1), rows.items[0].getInt(1).?);
}

test "AC2: storeMessage writes is_from_me=1, is_bot_message=1 when both true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "msg-both-true",
        .chat_jid = "chat:ac2",
        .sender = "bot",
        .sender_name = "Bot",
        .content = "bot response",
        .timestamp = "2024-01-01T00:00:02Z",
        .is_from_me = true,
        .is_bot_message = true,
    });

    var rows = try db.sqlite_db.query(
        alloc,
        "SELECT is_from_me, is_bot_message FROM messages WHERE id = ?1 AND chat_jid = ?2",
        .{ @as([]const u8, "msg-both-true"), @as([]const u8, "chat:ac2") },
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqual(@as(i64, 1), rows.items[0].getInt(0).?);
    try std.testing.expectEqual(@as(i64, 1), rows.items[0].getInt(1).?);
}

test "AC2: storeMessage writes is_from_me=0, is_bot_message=0 when both false" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "msg-both-false",
        .chat_jid = "chat:ac2",
        .sender = "user",
        .sender_name = "User",
        .content = "user message",
        .timestamp = "2024-01-01T00:00:03Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    var rows = try db.sqlite_db.query(
        alloc,
        "SELECT is_from_me, is_bot_message FROM messages WHERE id = ?1 AND chat_jid = ?2",
        .{ @as([]const u8, "msg-both-false"), @as([]const u8, "chat:ac2") },
    );
    defer rows.deinit();

    try std.testing.expectEqual(@as(usize, 1), rows.items.len);
    try std.testing.expectEqual(@as(i64, 0), rows.items[0].getInt(0).?);
    try std.testing.expectEqual(@as(i64, 0), rows.items[0].getInt(1).?);
}

// =============================================================================
// AC3: Round-trip read — getMessagesSince returns is_bot_message from DB
// =============================================================================

test "AC3: getMessagesSince returns is_bot_message=true for bot messages" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "rt-1",
        .chat_jid = "chat:rt",
        .sender = "bot",
        .sender_name = "Bot",
        .content = "bot says hi",
        .timestamp = "2024-01-01T00:00:00Z",
        .is_from_me = true,
        .is_bot_message = true,
    });

    const msgs = try db.getMessagesSince(alloc, "chat:rt", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expect(msgs[0].is_bot_message == true);
    try std.testing.expect(msgs[0].is_from_me == true);
}

test "AC3: getMessagesSince returns is_bot_message=false for user messages" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "rt-2",
        .chat_jid = "chat:rt2",
        .sender = "user",
        .sender_name = "User",
        .content = "user says hi",
        .timestamp = "2024-01-01T00:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "chat:rt2", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expect(msgs[0].is_bot_message == false);
    try std.testing.expect(msgs[0].is_from_me == false);
}

test "AC3: getMessagesSince round-trips divergent is_from_me and is_bot_message" {
    // Store message where the two fields diverge, read back, confirm independence.
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "rt-div",
        .chat_jid = "chat:rtdiv",
        .sender = "human",
        .sender_name = "Human",
        .content = "human on bot account",
        .timestamp = "2024-01-01T00:00:00Z",
        .is_from_me = true,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "chat:rtdiv", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expect(msgs[0].is_from_me == true);
    try std.testing.expect(msgs[0].is_bot_message == false);
}

// =============================================================================
// AC4 & AC5: Correct field values for user messages and bot responses
// These test the expected field combinations at the DB level.
// =============================================================================

test "AC4: user message pattern has is_from_me=false and is_bot_message=false" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Simulate what main.zig should do for incoming user messages
    try db.storeMessage(.{
        .id = "user-msg-1",
        .chat_jid = "chat:main",
        .sender = "u1",
        .sender_name = "Alice",
        .content = "Hi bot",
        .timestamp = "2024-01-01T00:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "chat:main", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expect(msgs[0].is_from_me == false);
    try std.testing.expect(msgs[0].is_bot_message == false);

    // Also verify raw DB
    var rows = try db.sqlite_db.query(
        alloc,
        "SELECT is_from_me, is_bot_message FROM messages WHERE id = ?1 AND chat_jid = ?2",
        .{ @as([]const u8, "user-msg-1"), @as([]const u8, "chat:main") },
    );
    defer rows.deinit();
    try std.testing.expectEqual(@as(i64, 0), rows.items[0].getInt(0).?);
    try std.testing.expectEqual(@as(i64, 0), rows.items[0].getInt(1).?);
}

test "AC5: bot response pattern has is_from_me=true and is_bot_message=true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Simulate what main.zig should do for bot responses
    try db.storeMessage(.{
        .id = "bot-resp-1",
        .chat_jid = "chat:main",
        .sender = "borg",
        .sender_name = "Borg",
        .content = "Hello! How can I help?",
        .timestamp = "2024-01-01T00:00:01Z",
        .is_from_me = true,
        .is_bot_message = true,
    });

    const msgs = try db.getMessagesSince(alloc, "chat:main", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expect(msgs[0].is_from_me == true);
    try std.testing.expect(msgs[0].is_bot_message == true);

    // Also verify raw DB
    var rows = try db.sqlite_db.query(
        alloc,
        "SELECT is_from_me, is_bot_message FROM messages WHERE id = ?1 AND chat_jid = ?2",
        .{ @as([]const u8, "bot-resp-1"), @as([]const u8, "chat:main") },
    );
    defer rows.deinit();
    try std.testing.expectEqual(@as(i64, 1), rows.items[0].getInt(0).?);
    try std.testing.expectEqual(@as(i64, 1), rows.items[0].getInt(1).?);
}

// =============================================================================
// AC7: Independence test — store mixed combos, read back, assert independent
// =============================================================================

test "AC7: is_from_me and is_bot_message are fully independent across multiple messages" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Message A: is_from_me=true, is_bot_message=false (human on bot account)
    try db.storeMessage(.{
        .id = "ind-a",
        .chat_jid = "chat:ind",
        .sender = "human",
        .sender_name = "Human",
        .content = "from me but not bot",
        .timestamp = "2024-01-01T00:00:00Z",
        .is_from_me = true,
        .is_bot_message = false,
    });

    // Message B: is_from_me=false, is_bot_message=true (bot via relay)
    try db.storeMessage(.{
        .id = "ind-b",
        .chat_jid = "chat:ind",
        .sender = "relay",
        .sender_name = "Relay",
        .content = "not from me but is bot",
        .timestamp = "2024-01-01T00:00:01Z",
        .is_from_me = false,
        .is_bot_message = true,
    });

    const msgs = try db.getMessagesSince(alloc, "chat:ind", "");
    try std.testing.expectEqual(@as(usize, 2), msgs.len);

    // Message A assertions
    try std.testing.expectEqualStrings("ind-a", msgs[0].id);
    try std.testing.expect(msgs[0].is_from_me == true);
    try std.testing.expect(msgs[0].is_bot_message == false);

    // Message B assertions
    try std.testing.expectEqualStrings("ind-b", msgs[1].id);
    try std.testing.expect(msgs[1].is_from_me == false);
    try std.testing.expect(msgs[1].is_bot_message == true);

    // Cross-verify: the two fields must not be equal for either message
    try std.testing.expect(msgs[0].is_from_me != msgs[0].is_bot_message);
    try std.testing.expect(msgs[1].is_from_me != msgs[1].is_bot_message);
}

// =============================================================================
// Edge Case 1: Existing rows default to is_bot_message=0
// =============================================================================

test "Edge1: rows inserted without is_bot_message default to 0" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Simulate a pre-fix row inserted directly via SQL (no is_bot_message column)
    try db.sqlite_db.exec(
        "INSERT INTO messages (id, chat_jid, sender, sender_name, content, timestamp, is_from_me) VALUES ('legacy-1', 'chat:legacy', 'u1', 'Old User', 'old msg', '2023-06-01T00:00:00Z', 1)"
    );

    // Read back via getMessagesSince — is_bot_message should default to false (0)
    const msgs = try db.getMessagesSince(alloc, "chat:legacy", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expect(msgs[0].is_from_me == true);
    try std.testing.expect(msgs[0].is_bot_message == false);
}

// =============================================================================
// Edge Case 2: Message deduplication preserves original is_bot_message
// =============================================================================

test "Edge2: INSERT OR IGNORE preserves original is_bot_message on duplicate" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Store original with is_bot_message=false
    try db.storeMessage(.{
        .id = "dup-1",
        .chat_jid = "chat:dup",
        .sender = "u1",
        .sender_name = "User",
        .content = "original",
        .timestamp = "2024-01-01T00:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    // Attempt to store duplicate with is_bot_message=true (should be silently ignored)
    try db.storeMessage(.{
        .id = "dup-1",
        .chat_jid = "chat:dup",
        .sender = "u1",
        .sender_name = "User",
        .content = "duplicate",
        .timestamp = "2024-01-01T00:00:00Z",
        .is_from_me = false,
        .is_bot_message = true,
    });

    const msgs = try db.getMessagesSince(alloc, "chat:dup", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    // Original content preserved
    try std.testing.expectEqualStrings("original", msgs[0].content);
    // Original is_bot_message=false preserved (not overwritten by duplicate)
    try std.testing.expect(msgs[0].is_bot_message == false);
}

// =============================================================================
// Edge Case 3: is_from_me=true, is_bot_message=false is a valid combination
// (human user sending from the same account the bot runs on)
// =============================================================================

test "Edge3: is_from_me=true with is_bot_message=false is valid and stored correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "edge3-1",
        .chat_jid = "chat:edge3",
        .sender = "me",
        .sender_name = "Me",
        .content = "I am the human behind the bot account",
        .timestamp = "2024-01-01T00:00:00Z",
        .is_from_me = true,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "chat:edge3", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expect(msgs[0].is_from_me == true);
    try std.testing.expect(msgs[0].is_bot_message == false);
}

// =============================================================================
// Edge Case 4: is_from_me=false, is_bot_message=true is a valid combination
// (bot message relayed through a different sender identity)
// =============================================================================

test "Edge4: is_from_me=false with is_bot_message=true is valid and stored correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "edge4-1",
        .chat_jid = "chat:edge4",
        .sender = "relay-bot",
        .sender_name = "Relay Bot",
        .content = "bot message via relay",
        .timestamp = "2024-01-01T00:00:00Z",
        .is_from_me = false,
        .is_bot_message = true,
    });

    const msgs = try db.getMessagesSince(alloc, "chat:edge4", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expect(msgs[0].is_from_me == false);
    try std.testing.expect(msgs[0].is_bot_message == true);
}

// =============================================================================
// Additional: All four boolean combinations in one chat, read back in order
// =============================================================================

test "all four is_from_me x is_bot_message combinations in a single chat" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();

    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // (false, false) — normal user message
    try db.storeMessage(.{
        .id = "combo-ff",
        .chat_jid = "chat:combo",
        .sender = "user",
        .sender_name = "User",
        .content = "user msg",
        .timestamp = "2024-01-01T00:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    // (true, true) — normal bot response
    try db.storeMessage(.{
        .id = "combo-tt",
        .chat_jid = "chat:combo",
        .sender = "bot",
        .sender_name = "Bot",
        .content = "bot reply",
        .timestamp = "2024-01-01T00:00:01Z",
        .is_from_me = true,
        .is_bot_message = true,
    });

    // (true, false) — human on bot account
    try db.storeMessage(.{
        .id = "combo-tf",
        .chat_jid = "chat:combo",
        .sender = "me",
        .sender_name = "Me",
        .content = "human on bot acct",
        .timestamp = "2024-01-01T00:00:02Z",
        .is_from_me = true,
        .is_bot_message = false,
    });

    // (false, true) — relayed bot message
    try db.storeMessage(.{
        .id = "combo-ft",
        .chat_jid = "chat:combo",
        .sender = "relay",
        .sender_name = "Relay",
        .content = "relayed bot",
        .timestamp = "2024-01-01T00:00:03Z",
        .is_from_me = false,
        .is_bot_message = true,
    });

    const msgs = try db.getMessagesSince(alloc, "chat:combo", "");
    try std.testing.expectEqual(@as(usize, 4), msgs.len);

    // combo-ff: (false, false)
    try std.testing.expectEqualStrings("combo-ff", msgs[0].id);
    try std.testing.expect(msgs[0].is_from_me == false);
    try std.testing.expect(msgs[0].is_bot_message == false);

    // combo-tt: (true, true)
    try std.testing.expectEqualStrings("combo-tt", msgs[1].id);
    try std.testing.expect(msgs[1].is_from_me == true);
    try std.testing.expect(msgs[1].is_bot_message == true);

    // combo-tf: (true, false)
    try std.testing.expectEqualStrings("combo-tf", msgs[2].id);
    try std.testing.expect(msgs[2].is_from_me == true);
    try std.testing.expect(msgs[2].is_bot_message == false);

    // combo-ft: (false, true)
    try std.testing.expectEqualStrings("combo-ft", msgs[3].id);
    try std.testing.expect(msgs[3].is_from_me == false);
    try std.testing.expect(msgs[3].is_bot_message == true);
}
