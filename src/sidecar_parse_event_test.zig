// Tests for Sidecar.parseEvent: field extraction and null-return paths.
//
// To include in the build, apply these two changes to src/sidecar.zig:
//   1. Change `fn parseEvent` to `pub fn parseEvent`
//   2. Inside the test section add: test { _ = @import("sidecar_parse_event_test.zig"); }
//
// Tests are written against the public interface only. All returned
// SidecarMessage string fields are allocator-owned copies; tests use an
// ArenaAllocator so every allocation is freed in bulk on deinit, keeping
// std.testing.allocator's leak detector clean.

const std = @import("std");
const sidecar = @import("sidecar.zig");
const Sidecar = sidecar.Sidecar;
const SidecarEvent = sidecar.SidecarEvent;

fn testSidecar(alloc: std.mem.Allocator) Sidecar {
    return Sidecar.init(alloc, "TestBot");
}

// ── AC1: missing `source` field returns null ────────────────────────────

test "AC1: missing source field returns null" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    const result = s.parseEvent(alloc, "{\"event\":\"message\",\"text\":\"hi\"}");
    try std.testing.expect(result == null);
}

// ── AC2: missing `event` field returns null ─────────────────────────────

test "AC2: missing event field returns null" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    const result = s.parseEvent(alloc, "{\"source\":\"discord\",\"text\":\"hi\"}");
    try std.testing.expect(result == null);
}

// ── AC3: unknown `source` string returns null ───────────────────────────

test "AC3: unknown source string returns null" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    const result = s.parseEvent(alloc, "{\"source\":\"slack\",\"event\":\"message\",\"text\":\"hi\"}");
    try std.testing.expect(result == null);
}

// ── AC4: unknown `event` type string returns null ───────────────────────

test "AC4: unknown event type returns null" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    const result = s.parseEvent(alloc, "{\"source\":\"discord\",\"event\":\"heartbeat\"}");
    try std.testing.expect(result == null);
}

// ── AC5: Discord is_dm:true → is_group:false ────────────────────────────

test "AC5: discord is_dm true maps to is_group false" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    const line =
        \\{"source":"discord","event":"message","is_dm":true,
        \\"message_id":"1","channel_id":"c","sender_id":"s",
        \\"sender_name":"Alice","text":"hi","timestamp":0,"mentions_bot":false}
    ;
    const result = s.parseEvent(alloc, line);
    try std.testing.expect(result != null);
    try std.testing.expect(result.? == .message);
    try std.testing.expect(result.?.message.is_group == false);
}

// ── AC6: Discord is_dm:false → is_group:true ────────────────────────────

test "AC6: discord is_dm false maps to is_group true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    const line =
        \\{"source":"discord","event":"message","is_dm":false,
        \\"message_id":"1","channel_id":"c","sender_id":"s",
        \\"sender_name":"Alice","text":"hi","timestamp":0,"mentions_bot":false}
    ;
    const result = s.parseEvent(alloc, line);
    try std.testing.expect(result != null);
    try std.testing.expect(result.? == .message);
    try std.testing.expect(result.?.message.is_group == true);
}

// ── AC7: Discord is_dm absent → is_group:true ───────────────────────────

test "AC7: discord is_dm absent defaults to is_group true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    // No is_dm key at all; getBool returns null, orelse false → !false = true
    const line =
        \\{"source":"discord","event":"message",
        \\"message_id":"1","channel_id":"c","sender_id":"s",
        \\"sender_name":"Alice","text":"hi","timestamp":0,"mentions_bot":false}
    ;
    const result = s.parseEvent(alloc, line);
    try std.testing.expect(result != null);
    try std.testing.expect(result.? == .message);
    try std.testing.expect(result.?.message.is_group == true);
}

// ── AC8: WhatsApp is_group:true → is_group:true ─────────────────────────

test "AC8: whatsapp is_group true maps directly to is_group true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    const line =
        \\{"source":"whatsapp","event":"message","id":"1","jid":"j",
        \\"sender":"s","sender_name":"Bob","text":"hi","timestamp":0,
        \\"is_group":true,"mentions_bot":false}
    ;
    const result = s.parseEvent(alloc, line);
    try std.testing.expect(result != null);
    try std.testing.expect(result.? == .message);
    try std.testing.expect(result.?.message.is_group == true);
}

// ── AC9: WhatsApp is_group:false → is_group:false ───────────────────────

test "AC9: whatsapp is_group false maps directly to is_group false" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    const line =
        \\{"source":"whatsapp","event":"message","id":"1","jid":"j",
        \\"sender":"s","sender_name":"Bob","text":"hi","timestamp":0,
        \\"is_group":false,"mentions_bot":false}
    ;
    const result = s.parseEvent(alloc, line);
    try std.testing.expect(result != null);
    try std.testing.expect(result.? == .message);
    try std.testing.expect(result.?.message.is_group == false);
}

// ── AC10: malformed JSON returns null without panic ──────────────────────

test "AC10: malformed JSON returns null" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    const result = s.parseEvent(alloc, "not json at all");
    try std.testing.expect(result == null);
}

// ── AC11: both source and event missing returns null ────────────────────

test "AC11: both source and event missing returns null" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    const result = s.parseEvent(alloc, "{\"text\":\"hi\"}");
    try std.testing.expect(result == null);
}

// ── EC1: empty string input returns null ────────────────────────────────

test "EC1: empty string input returns null" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    const result = s.parseEvent(alloc, "");
    try std.testing.expect(result == null);
}

// ── EC2: unknown source with valid event returns null ───────────────────

test "EC2: unknown source with valid event returns null" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    const result = s.parseEvent(alloc, "{\"source\":\"signal\",\"event\":\"message\"}");
    try std.testing.expect(result == null);
}

// ── EC3: valid source with unknown event returns null ───────────────────

test "EC3: valid source with unknown event returns null" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    const result = s.parseEvent(alloc, "{\"source\":\"whatsapp\",\"event\":\"typing\"}");
    try std.testing.expect(result == null);
}

// ── EC4: is_dm wrong type (string) treated as absent → is_group:true ────

test "EC4: discord is_dm wrong type treated as absent, is_group true" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    // getBool returns null for a string value; orelse false → !false = true
    const line =
        \\{"source":"discord","event":"message","is_dm":"yes",
        \\"message_id":"1","channel_id":"c","sender_id":"s",
        \\"sender_name":"Alice","text":"hi","timestamp":0,"mentions_bot":false}
    ;
    const result = s.parseEvent(alloc, line);
    try std.testing.expect(result != null);
    try std.testing.expect(result.? == .message);
    try std.testing.expect(result.?.message.is_group == true);
}

// ── EC5: WhatsApp is_group absent defaults to is_group:false ────────────

test "EC5: whatsapp is_group absent defaults to is_group false" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    // No is_group key; getBool returns null, orelse false → false
    const line =
        \\{"source":"whatsapp","event":"message","id":"1","jid":"j",
        \\"sender":"s","sender_name":"Bob","text":"hi","timestamp":0,
        \\"mentions_bot":false}
    ;
    const result = s.parseEvent(alloc, line);
    try std.testing.expect(result != null);
    try std.testing.expect(result.? == .message);
    try std.testing.expect(result.?.message.is_group == false);
}

// ── EC6: source field on returned message is correct ────────────────────

test "EC6: discord message carries source discord" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    const line =
        \\{"source":"discord","event":"message","is_dm":false,
        \\"message_id":"42","channel_id":"ch1","sender_id":"u1",
        \\"sender_name":"Eve","text":"hello","timestamp":1000,"mentions_bot":true}
    ;
    const result = s.parseEvent(alloc, line);
    try std.testing.expect(result != null);
    const msg = result.?.message;
    try std.testing.expect(msg.source == .discord);
    try std.testing.expectEqualStrings("42", msg.id);
    try std.testing.expectEqualStrings("ch1", msg.chat_id);
    try std.testing.expectEqualStrings("u1", msg.sender);
    try std.testing.expectEqualStrings("Eve", msg.sender_name);
    try std.testing.expectEqualStrings("hello", msg.text);
    try std.testing.expectEqual(@as(i64, 1000), msg.timestamp);
    try std.testing.expect(msg.mentions_bot == true);
    try std.testing.expect(msg.is_group == true); // is_dm:false → !false = true
}

test "EC6: whatsapp message carries source whatsapp" {
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var s = testSidecar(alloc);

    const line =
        \\{"source":"whatsapp","event":"message","id":"w99","jid":"g1@g.us",
        \\"sender":"phone1","sender_name":"Carlos","text":"hey","timestamp":2000,
        \\"is_group":true,"mentions_bot":false}
    ;
    const result = s.parseEvent(alloc, line);
    try std.testing.expect(result != null);
    const msg = result.?.message;
    try std.testing.expect(msg.source == .whatsapp);
    try std.testing.expectEqualStrings("w99", msg.id);
    try std.testing.expectEqualStrings("g1@g.us", msg.chat_id);
    try std.testing.expectEqualStrings("phone1", msg.sender);
    try std.testing.expectEqualStrings("Carlos", msg.sender_name);
    try std.testing.expectEqualStrings("hey", msg.text);
    try std.testing.expectEqual(@as(i64, 2000), msg.timestamp);
    try std.testing.expect(msg.mentions_bot == false);
    try std.testing.expect(msg.is_group == true);
}
