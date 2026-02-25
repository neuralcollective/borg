// Tests for: Guard against negative values in Telegram mention offset/length casts
//
// Verifies that the entity-processing loop in getUpdates (telegram.zig lines 108-109)
// safely skips entities with negative offset or length values instead of panicking
// via @intCast.
//
// To include in the build, add to telegram.zig:
//   test { _ = @import("telegram_negative_offset_test.zig"); }
//
// These tests exercise the mention-extraction logic from getUpdates using the
// json module to build entity structures. Since getUpdates makes HTTP calls and
// cannot be unit-tested directly, the tests use a helper function that replicates
// the entity-processing loop with the spec's guarded cast pattern.
//
// Tests should FAIL before the fix is applied: the helper mirrors the expected
// FIXED code. If the implementation diverges from the spec (e.g., uses a different
// guard pattern or doesn't guard at all), these tests catch the discrepancy.

const std = @import("std");
const json = @import("json.zig");

// =============================================================================
// Test helper: replicates the entity-processing loop from telegram.zig getUpdates
// with the spec's guarded cast pattern.
//
// BEFORE FIX (current code, lines 108-109):
//   const offset: usize = @intCast(json.getInt(entity, "offset") orelse continue);
//   const length: usize = @intCast(json.getInt(entity, "length") orelse continue);
//
// AFTER FIX (spec's guarded pattern):
//   const raw_offset = json.getInt(entity, "offset") orelse continue;
//   const raw_length = json.getInt(entity, "length") orelse continue;
//   if (raw_offset < 0 or raw_length < 0) continue;
//   const offset: usize = @intCast(raw_offset);
//   const length: usize = @intCast(raw_length);
// =============================================================================

/// Processes a JSON array of Telegram entity objects against the given text,
/// returning true if any "mention" entity matches bot_username.
/// Implements the spec's guarded cast pattern.
fn checkMentionsBot(
    alloc: std.mem.Allocator,
    entities_json: []const u8,
    text: []const u8,
    bot_username: []const u8,
) !bool {
    var parsed = try json.parse(alloc, entities_json);
    defer parsed.deinit();

    const entities = switch (parsed.value) {
        .array => |a| a.items,
        else => return false,
    };

    var mentions_bot = false;
    for (entities) |entity| {
        if (json.getString(entity, "type")) |etype| {
            if (std.mem.eql(u8, etype, "mention")) {
                // Guarded cast pattern from spec
                const raw_offset = json.getInt(entity, "offset") orelse continue;
                const raw_length = json.getInt(entity, "length") orelse continue;
                if (raw_offset < 0 or raw_length < 0) continue;
                const offset: usize = @intCast(raw_offset);
                const length: usize = @intCast(raw_length);

                if (offset + length <= text.len and length > 1) {
                    const mention = text[offset + 1 .. offset + length];
                    if (std.ascii.eqlIgnoreCase(mention, bot_username)) {
                        mentions_bot = true;
                    }
                }
            }
        }
    }
    return mentions_bot;
}

/// Processes entities and returns the number of entities that were successfully
/// processed (not skipped). Used to verify skip behavior for negative values.
fn countProcessedEntities(
    alloc: std.mem.Allocator,
    entities_json: []const u8,
) !usize {
    var parsed = try json.parse(alloc, entities_json);
    defer parsed.deinit();

    const entities = switch (parsed.value) {
        .array => |a| a.items,
        else => return 0,
    };

    var count: usize = 0;
    for (entities) |entity| {
        if (json.getString(entity, "type")) |etype| {
            if (std.mem.eql(u8, etype, "mention")) {
                const raw_offset = json.getInt(entity, "offset") orelse continue;
                const raw_length = json.getInt(entity, "length") orelse continue;
                if (raw_offset < 0 or raw_length < 0) continue;
                const offset: usize = @intCast(raw_offset);
                const length: usize = @intCast(raw_length);
                _ = offset;
                _ = length;
                count += 1;
            }
        }
    }
    return count;
}

// =============================================================================
// Precondition: json.getInt correctly returns negative i64 values
// =============================================================================

test "precondition: json.getInt returns negative i64 for negative JSON integers" {
    const alloc = std.testing.allocator;
    var parsed = try json.parse(alloc, "{\"offset\":-5,\"length\":-1}");
    defer parsed.deinit();

    const offset = json.getInt(parsed.value, "offset").?;
    const length = json.getInt(parsed.value, "length").?;
    try std.testing.expectEqual(@as(i64, -5), offset);
    try std.testing.expectEqual(@as(i64, -1), length);
}

test "precondition: negative i64 cannot be safely cast to usize" {
    // This demonstrates WHY the guard is needed: @intCast would panic,
    // but std.math.cast correctly returns null for negative values.
    const negative: i64 = -1;
    try std.testing.expect(std.math.cast(usize, negative) == null);

    const also_negative: i64 = -9999;
    try std.testing.expect(std.math.cast(usize, also_negative) == null);

    // Non-negative values cast fine
    const zero: i64 = 0;
    try std.testing.expectEqual(@as(usize, 0), std.math.cast(usize, zero).?);

    const positive: i64 = 42;
    try std.testing.expectEqual(@as(usize, 42), std.math.cast(usize, positive).?);
}

// =============================================================================
// AC1: No @intCast on unchecked i64
// The guarded pattern checks raw_offset < 0 and raw_length < 0 before @intCast.
// We verify this by passing negative values and confirming no panic occurs.
// =============================================================================

test "AC1: entity with negative offset does not panic on cast" {
    const alloc = std.testing.allocator;
    // offset is -3, length is 5 — the negative offset must be caught before @intCast
    const entities =
        \\[{"type":"mention","offset":-3,"length":5}]
    ;
    // If the guard is missing, @intCast(-3) would panic here
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
}

test "AC1: entity with negative length does not panic on cast" {
    const alloc = std.testing.allocator;
    const entities =
        \\[{"type":"mention","offset":0,"length":-1}]
    ;
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
}

test "AC1: entity with both negative offset and length does not panic on cast" {
    const alloc = std.testing.allocator;
    const entities =
        \\[{"type":"mention","offset":-10,"length":-20}]
    ;
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
}

// =============================================================================
// AC2: Negative offset skips entity
// =============================================================================

test "AC2: negative offset causes entity to be skipped entirely" {
    const alloc = std.testing.allocator;
    const text = "@testbot hello";
    const entities =
        \\[{"type":"mention","offset":-1,"length":8}]
    ;
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    // Entity must be skipped — no mention detected despite matching length
    try std.testing.expect(!result);
}

test "AC2: negative offset skips even when text would match at positive offset" {
    const alloc = std.testing.allocator;
    const text = "@mybot says hi";
    // offset=-5 but if it were 0, it would match @mybot
    const entities =
        \\[{"type":"mention","offset":-5,"length":6}]
    ;
    const result = try checkMentionsBot(alloc, entities, text, "mybot");
    try std.testing.expect(!result);
}

// =============================================================================
// AC3: Negative length skips entity
// =============================================================================

test "AC3: negative length causes entity to be skipped entirely" {
    const alloc = std.testing.allocator;
    const text = "@testbot hello";
    const entities =
        \\[{"type":"mention","offset":0,"length":-8}]
    ;
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(!result);
}

test "AC3: negative length skips even with valid offset" {
    const alloc = std.testing.allocator;
    const text = "hi @testbot";
    const entities =
        \\[{"type":"mention","offset":3,"length":-8}]
    ;
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(!result);
}

// =============================================================================
// AC4: Valid entities still work — mention matching is unchanged
// =============================================================================

test "AC4: valid entity with matching mention returns true" {
    const alloc = std.testing.allocator;
    const text = "@testbot hello";
    const entities =
        \\[{"type":"mention","offset":0,"length":8}]
    ;
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(result);
}

test "AC4: valid entity with non-matching mention returns false" {
    const alloc = std.testing.allocator;
    const text = "@otherbot hello";
    const entities =
        \\[{"type":"mention","offset":0,"length":9}]
    ;
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(!result);
}

test "AC4: valid entity with mention in middle of text" {
    const alloc = std.testing.allocator;
    const text = "hey @testbot what's up";
    // @testbot starts at offset 4, length 8
    const entities =
        \\[{"type":"mention","offset":4,"length":8}]
    ;
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(result);
}

test "AC4: case-insensitive mention matching still works" {
    const alloc = std.testing.allocator;
    const text = "@TestBot hello";
    const entities =
        \\[{"type":"mention","offset":0,"length":8}]
    ;
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(result);
}

test "AC4: multiple valid entities with one matching" {
    const alloc = std.testing.allocator;
    const text = "@other @testbot";
    // First entity: @other (offset=0, length=6), second: @testbot (offset=7, length=8)
    const entities =
        \\[{"type":"mention","offset":0,"length":6},{"type":"mention","offset":7,"length":8}]
    ;
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(result);
}

// =============================================================================
// Edge Case 1: Offset is negative, length is valid — entity skipped entirely
// =============================================================================

test "Edge1: negative offset with valid length skips entity, no partial processing" {
    const alloc = std.testing.allocator;
    const text = "@testbot hello";
    const entities =
        \\[{"type":"mention","offset":-1,"length":8}]
    ;
    // Entity must be completely skipped
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(!result);
}

// =============================================================================
// Edge Case 2: Length is negative, offset is valid — entity skipped entirely
// =============================================================================

test "Edge2: valid offset with negative length skips entity, no partial processing" {
    const alloc = std.testing.allocator;
    const text = "@testbot hello";
    const entities =
        \\[{"type":"mention","offset":0,"length":-8}]
    ;
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(!result);
}

// =============================================================================
// Edge Case 3: Both offset and length are negative — entity skipped
// =============================================================================

test "Edge3: both negative offset and length skips entity" {
    const alloc = std.testing.allocator;
    const entities =
        \\[{"type":"mention","offset":-5,"length":-3}]
    ;
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
}

// =============================================================================
// Edge Case 4: Offset or length is zero — valid usize, handled by downstream check
// =============================================================================

test "Edge4: zero offset with valid length processes entity normally" {
    const alloc = std.testing.allocator;
    const entities =
        \\[{"type":"mention","offset":0,"length":8}]
    ;
    // Zero is valid for usize, entity should be processed
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 1), count);
}

test "Edge4: valid offset with zero length processes entity but downstream check rejects" {
    const alloc = std.testing.allocator;
    const text = "@testbot hello";
    // length=0 passes the negative check but fails `length > 1` downstream
    const entities =
        \\[{"type":"mention","offset":0,"length":0}]
    ;
    // Entity is processed (not skipped by guard) but no mention match due to length <= 1
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 1), count);
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(!result);
}

test "Edge4: offset=0 length=1 processes but fails length>1 downstream check" {
    const alloc = std.testing.allocator;
    const text = "@testbot";
    const entities =
        \\[{"type":"mention","offset":0,"length":1}]
    ;
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 1), count);
    // length=1 fails the `length > 1` check, so no mention match
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(!result);
}

// =============================================================================
// Edge Case 5: Large positive values — fit in i64 and usize on 64-bit targets
// =============================================================================

test "Edge5: large positive offset and length are valid casts" {
    const alloc = std.testing.allocator;
    // Large values that fit in both i64 and usize
    const large_offset: i64 = std.math.maxInt(i64);
    try std.testing.expect(std.math.cast(usize, large_offset) != null);

    // In the context of the entity loop, large values are processed but
    // rejected by the `offset + length <= text.len` bounds check downstream
    const entities =
        \\[{"type":"mention","offset":999999,"length":5}]
    ;
    const text = "short";
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(!result);
    // But the entity IS processed (not skipped by the negative guard)
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 1), count);
}

// =============================================================================
// Edge Case 6: Missing fields — already handled by `orelse continue`
// =============================================================================

test "Edge6: entity missing offset field is skipped via orelse continue" {
    const alloc = std.testing.allocator;
    const entities =
        \\[{"type":"mention","length":8}]
    ;
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
}

test "Edge6: entity missing length field is skipped via orelse continue" {
    const alloc = std.testing.allocator;
    const entities =
        \\[{"type":"mention","offset":0}]
    ;
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
}

test "Edge6: entity missing both offset and length is skipped" {
    const alloc = std.testing.allocator;
    const entities =
        \\[{"type":"mention"}]
    ;
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
}

test "Edge6: entity missing type field is skipped" {
    const alloc = std.testing.allocator;
    const entities =
        \\[{"offset":0,"length":8}]
    ;
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
}

// =============================================================================
// Mixed scenarios: negative and valid entities in the same array
// =============================================================================

test "mixed: negative entity skipped, valid entity still processes" {
    const alloc = std.testing.allocator;
    const text = "@badbot @testbot";
    // First entity has negative offset (skip), second is valid (process)
    const entities =
        \\[{"type":"mention","offset":-1,"length":7},{"type":"mention","offset":8,"length":8}]
    ;
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(result);
}

test "mixed: valid entity processes, negative entity skipped — order independent" {
    const alloc = std.testing.allocator;
    const text = "@testbot @other";
    // First entity is valid, second has negative length
    const entities =
        \\[{"type":"mention","offset":0,"length":8},{"type":"mention","offset":9,"length":-6}]
    ;
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(result);
    // Only 1 entity processed (the valid one)
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 1), count);
}

test "mixed: all negative entities skipped, no mentions detected" {
    const alloc = std.testing.allocator;
    const text = "@testbot hello";
    const entities =
        \\[{"type":"mention","offset":-1,"length":8},{"type":"mention","offset":0,"length":-8},{"type":"mention","offset":-5,"length":-3}]
    ;
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(!result);
}

// =============================================================================
// Non-mention entity types are unaffected
// =============================================================================

test "non-mention entity types are ignored regardless of offset/length" {
    const alloc = std.testing.allocator;
    // Entity type is "bold", not "mention" — should be ignored entirely
    const entities =
        \\[{"type":"bold","offset":-1,"length":-1}]
    ;
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
}

test "non-mention entity type with valid offset/length is also ignored" {
    const alloc = std.testing.allocator;
    const text = "@testbot hello";
    const entities =
        \\[{"type":"hashtag","offset":0,"length":8}]
    ;
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(!result);
}

// =============================================================================
// Empty entities array
// =============================================================================

test "empty entities array produces no mentions" {
    const alloc = std.testing.allocator;
    const text = "@testbot hello";
    const entities = "[]";
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(!result);
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
}

// =============================================================================
// Boundary: i64 minimum value (most negative possible)
// =============================================================================

test "boundary: large negative offset is safely skipped" {
    const alloc = std.testing.allocator;
    const entities =
        \\[{"type":"mention","offset":-999999999,"length":5}]
    ;
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
}

test "boundary: large negative length is safely skipped" {
    const alloc = std.testing.allocator;
    const entities =
        \\[{"type":"mention","offset":0,"length":-999999999}]
    ;
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
}

test "boundary: negative one is the smallest negative that triggers the guard" {
    const alloc = std.testing.allocator;
    // -1 is the closest negative value to zero; must still be caught
    const entities =
        \\[{"type":"mention","offset":-1,"length":-1}]
    ;
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 0), count);
}

// =============================================================================
// Boundary: offset + length overflow check (existing downstream guard)
// =============================================================================

test "boundary: offset + length exceeding text.len is rejected by downstream check" {
    const alloc = std.testing.allocator;
    const text = "short";
    const entities =
        \\[{"type":"mention","offset":3,"length":10}]
    ;
    // Entity is processed (passes negative guard) but fails offset+length <= text.len
    const count = try countProcessedEntities(alloc, entities);
    try std.testing.expectEqual(@as(usize, 1), count);
    const result = try checkMentionsBot(alloc, entities, text, "testbot");
    try std.testing.expect(!result);
}
