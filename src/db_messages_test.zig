// Tests for Task #70: storeMessage / getMessagesSince coverage.
//
// Covers: storeMessage, getMessagesSince
//
// All allocations use an ArenaAllocator over :memory: so string cleanup is
// handled automatically — no need to free individual Message fields.
//
// To include in the build, add to the test block in src/db.zig:
//   _ = @import("db_messages_test.zig");

const std = @import("std");
const db_mod = @import("db.zig");
const Db = db_mod.Db;
const Message = db_mod.Message;

// =============================================================================
// AC1 — Basic store and retrieve
// =============================================================================

test "AC1: store a message and retrieve it with getMessagesSince(since=before)" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "msg-001",
        .chat_jid = "chat-1",
        .sender = "user-a",
        .sender_name = "Alice",
        .content = "Hello world",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "chat-1", "2024-06-01T09:00:00Z");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expectEqualStrings("msg-001",              msgs[0].id);
    try std.testing.expectEqualStrings("chat-1",              msgs[0].chat_jid);
    try std.testing.expectEqualStrings("user-a",              msgs[0].sender);
    try std.testing.expectEqualStrings("Alice",               msgs[0].sender_name);
    try std.testing.expectEqualStrings("Hello world",         msgs[0].content);
    try std.testing.expectEqualStrings("2024-06-01T10:00:00Z", msgs[0].timestamp);
    try std.testing.expect(!msgs[0].is_from_me);
    try std.testing.expect(!msgs[0].is_bot_message);
}

// =============================================================================
// AC2 — `since` cutoff excludes earlier messages
// =============================================================================

test "AC2: message with timestamp equal to since is not returned" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "msg-eq",
        .chat_jid = "chat-1",
        .sender = "u",
        .sender_name = "U",
        .content = "Exact match",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    // since == timestamp: query uses >, so this message must NOT appear
    const msgs = try db.getMessagesSince(alloc, "chat-1", "2024-06-01T10:00:00Z");
    try std.testing.expectEqual(@as(usize, 0), msgs.len);
}

test "AC2: message with timestamp strictly before since is not returned" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "msg-before",
        .chat_jid = "chat-1",
        .sender = "u",
        .sender_name = "U",
        .content = "Old message",
        .timestamp = "2024-06-01T09:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "chat-1", "2024-06-01T10:00:00Z");
    try std.testing.expectEqual(@as(usize, 0), msgs.len);
}

// =============================================================================
// AC3 — INSERT OR IGNORE duplicate suppression
// =============================================================================

test "AC3: duplicate chat_jid+id is silently dropped, no error returned" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "msg-1",
        .chat_jid = "g1",
        .sender = "u",
        .sender_name = "U",
        .content = "Original",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    // Same chat_jid+id, different content — must not error
    try db.storeMessage(.{
        .id = "msg-1",
        .chat_jid = "g1",
        .sender = "u",
        .sender_name = "U",
        .content = "Duplicate",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "g1", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    // First write wins
    try std.testing.expectEqualStrings("Original", msgs[0].content);
}

// =============================================================================
// AC4 — is_from_me bool round-trip
// =============================================================================

test "AC4: is_from_me=true survives store/retrieve round-trip" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "msg-from-me",
        .chat_jid = "chat-1",
        .sender = "me",
        .sender_name = "Me",
        .content = "Sent by me",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = true,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "chat-1", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expect(msgs[0].is_from_me);
}

test "AC4: is_from_me=false survives store/retrieve round-trip" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "msg-not-from-me",
        .chat_jid = "chat-1",
        .sender = "other",
        .sender_name = "Other",
        .content = "Not from me",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "chat-1", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expect(!msgs[0].is_from_me);
}

// =============================================================================
// AC5 — is_bot_message bool round-trip
// =============================================================================

test "AC5: is_bot_message=true survives store/retrieve round-trip" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "msg-bot",
        .chat_jid = "chat-1",
        .sender = "bot",
        .sender_name = "Bot",
        .content = "Bot reply",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = false,
        .is_bot_message = true,
    });

    const msgs = try db.getMessagesSince(alloc, "chat-1", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expect(msgs[0].is_bot_message);
}

test "AC5: is_bot_message=false survives store/retrieve round-trip" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "msg-human",
        .chat_jid = "chat-1",
        .sender = "human",
        .sender_name = "Human",
        .content = "Human message",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "chat-1", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expect(!msgs[0].is_bot_message);
}

// =============================================================================
// AC6 — Combined bool fields
// =============================================================================

test "AC6: is_from_me=true and is_bot_message=true both round-trip correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "msg-both-true",
        .chat_jid = "chat-1",
        .sender = "bot-me",
        .sender_name = "BotMe",
        .content = "I am the bot",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = true,
        .is_bot_message = true,
    });

    const msgs = try db.getMessagesSince(alloc, "chat-1", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expect(msgs[0].is_from_me);
    try std.testing.expect(msgs[0].is_bot_message);
}

test "AC6: is_from_me=false and is_bot_message=false both round-trip correctly" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "msg-both-false",
        .chat_jid = "chat-1",
        .sender = "user",
        .sender_name = "User",
        .content = "Plain user message",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "chat-1", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expect(!msgs[0].is_from_me);
    try std.testing.expect(!msgs[0].is_bot_message);
}

// =============================================================================
// AC7 — Ascending timestamp order
// =============================================================================

test "AC7: getMessagesSince returns results ordered by timestamp ascending" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    // Insert in reverse order to confirm the result is sorted, not insertion-ordered
    try db.storeMessage(.{
        .id = "msg-c",
        .chat_jid = "chat-order",
        .sender = "u",
        .sender_name = "U",
        .content = "Third",
        .timestamp = "2024-06-01T12:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });
    try db.storeMessage(.{
        .id = "msg-a",
        .chat_jid = "chat-order",
        .sender = "u",
        .sender_name = "U",
        .content = "First",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });
    try db.storeMessage(.{
        .id = "msg-b",
        .chat_jid = "chat-order",
        .sender = "u",
        .sender_name = "U",
        .content = "Second",
        .timestamp = "2024-06-01T11:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "chat-order", "");
    try std.testing.expectEqual(@as(usize, 3), msgs.len);
    try std.testing.expectEqualStrings("First",  msgs[0].content);
    try std.testing.expectEqualStrings("Second", msgs[1].content);
    try std.testing.expectEqualStrings("Third",  msgs[2].content);
    // Timestamps must be non-decreasing
    try std.testing.expect(std.mem.order(u8, msgs[0].timestamp, msgs[1].timestamp) != .gt);
    try std.testing.expect(std.mem.order(u8, msgs[1].timestamp, msgs[2].timestamp) != .gt);
}

// =============================================================================
// AC8 — chat_jid isolation
// =============================================================================

test "AC8: getMessagesSince only returns messages for the queried chat_jid" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "a-1",
        .chat_jid = "chat-A",
        .sender = "u",
        .sender_name = "U",
        .content = "A first",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });
    try db.storeMessage(.{
        .id = "a-2",
        .chat_jid = "chat-A",
        .sender = "u",
        .sender_name = "U",
        .content = "A second",
        .timestamp = "2024-06-01T10:01:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });
    try db.storeMessage(.{
        .id = "b-1",
        .chat_jid = "chat-B",
        .sender = "u",
        .sender_name = "U",
        .content = "B only",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    const a_msgs = try db.getMessagesSince(alloc, "chat-A", "");
    try std.testing.expectEqual(@as(usize, 2), a_msgs.len);
    for (a_msgs) |m| {
        try std.testing.expectEqualStrings("chat-A", m.chat_jid);
    }

    const b_msgs = try db.getMessagesSince(alloc, "chat-B", "");
    try std.testing.expectEqual(@as(usize, 1), b_msgs.len);
    try std.testing.expectEqualStrings("chat-B", b_msgs[0].chat_jid);
}

// =============================================================================
// AC9 — Empty result on no matching chat_jid
// =============================================================================

test "AC9: getMessagesSince returns empty slice when chat_jid has no messages" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const msgs = try db.getMessagesSince(alloc, "nonexistent", "");
    try std.testing.expectEqual(@as(usize, 0), msgs.len);
}

// =============================================================================
// E1 — since="" returns all messages for the jid
// =============================================================================

test "E1: since=empty string returns all messages for the jid" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "e1-1",
        .chat_jid = "chat-e1",
        .sender = "u",
        .sender_name = "U",
        .content = "Early",
        .timestamp = "2000-01-01T00:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });
    try db.storeMessage(.{
        .id = "e1-2",
        .chat_jid = "chat-e1",
        .sender = "u",
        .sender_name = "U",
        .content = "Late",
        .timestamp = "2099-12-31T23:59:59Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "chat-e1", "");
    try std.testing.expectEqual(@as(usize, 2), msgs.len);
}

// =============================================================================
// E2 — Two messages with the same timestamp but different id both returned
// =============================================================================

test "E2: two messages with identical timestamp but different id are both returned" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    const ts = "2024-06-01T10:00:00Z";
    try db.storeMessage(.{
        .id = "twin-1",
        .chat_jid = "chat-twin",
        .sender = "u",
        .sender_name = "U",
        .content = "Twin A",
        .timestamp = ts,
        .is_from_me = false,
        .is_bot_message = false,
    });
    try db.storeMessage(.{
        .id = "twin-2",
        .chat_jid = "chat-twin",
        .sender = "u",
        .sender_name = "U",
        .content = "Twin B",
        .timestamp = ts,
        .is_from_me = false,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "chat-twin", "");
    try std.testing.expectEqual(@as(usize, 2), msgs.len);
}

// =============================================================================
// E3 — is_from_me=true combined with is_bot_message=false round-trips
// =============================================================================

test "E3: is_from_me=true and is_bot_message=false are independent" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "e3-msg",
        .chat_jid = "chat-e3",
        .sender = "me",
        .sender_name = "Me",
        .content = "I am me, not a bot",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = true,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "chat-e3", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expect(msgs[0].is_from_me);
    try std.testing.expect(!msgs[0].is_bot_message);
}

// =============================================================================
// E4 — Duplicate insert with differing fields leaves first row untouched
// =============================================================================

test "E4: duplicate insert with differing is_from_me leaves original row" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "dup-id",
        .chat_jid = "chat-dup",
        .sender = "u",
        .sender_name = "U",
        .content = "First",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });
    // Same primary key, flipped bools — must not error, must not overwrite
    try db.storeMessage(.{
        .id = "dup-id",
        .chat_jid = "chat-dup",
        .sender = "u",
        .sender_name = "U",
        .content = "Second",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = true,
        .is_bot_message = true,
    });

    const msgs = try db.getMessagesSince(alloc, "chat-dup", "");
    try std.testing.expectEqual(@as(usize, 1), msgs.len);
    try std.testing.expectEqualStrings("First", msgs[0].content);
    try std.testing.expect(!msgs[0].is_from_me);
    try std.testing.expect(!msgs[0].is_bot_message);
}

// =============================================================================
// E5 — since is after all stored messages → empty result
// =============================================================================

test "E5: since after all stored messages returns empty slice" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "e5-msg",
        .chat_jid = "chat-e5",
        .sender = "u",
        .sender_name = "U",
        .content = "Old",
        .timestamp = "2024-01-01T00:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    const msgs = try db.getMessagesSince(alloc, "chat-e5", "2099-01-01T00:00:00Z");
    try std.testing.expectEqual(@as(usize, 0), msgs.len);
}

// =============================================================================
// E6 — Third chat_jid in the DB does not leak into other queries
// =============================================================================

test "E6: messages for a third chat_jid do not appear in queries for other jids" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var db = try Db.init(alloc, ":memory:");
    defer db.deinit();

    try db.storeMessage(.{
        .id = "x-1",
        .chat_jid = "chat-X",
        .sender = "u",
        .sender_name = "U",
        .content = "X message",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });
    try db.storeMessage(.{
        .id = "y-1",
        .chat_jid = "chat-Y",
        .sender = "u",
        .sender_name = "U",
        .content = "Y message",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });
    try db.storeMessage(.{
        .id = "z-1",
        .chat_jid = "chat-Z",
        .sender = "u",
        .sender_name = "U",
        .content = "Z message",
        .timestamp = "2024-06-01T10:00:00Z",
        .is_from_me = false,
        .is_bot_message = false,
    });

    const x = try db.getMessagesSince(alloc, "chat-X", "");
    try std.testing.expectEqual(@as(usize, 1), x.len);
    try std.testing.expectEqualStrings("chat-X", x[0].chat_jid);

    const y = try db.getMessagesSince(alloc, "chat-Y", "");
    try std.testing.expectEqual(@as(usize, 1), y.len);
    try std.testing.expectEqualStrings("chat-Y", y[0].chat_jid);

    const z = try db.getMessagesSince(alloc, "chat-Z", "");
    try std.testing.expectEqual(@as(usize, 1), z.len);
    try std.testing.expectEqualStrings("chat-Z", z[0].chat_jid);
}
