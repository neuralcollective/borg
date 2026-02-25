// Tests for parseNdjson assistant-message concatenation fallback.
//
// Covers: multi-message join order, non-text type skipping, mixed content,
// result-field priority, empty-result fallback activation, all-tool_use
// (empty output), and three-message ordering.
//
// To include in the build, add to agent.zig's test block:
//   _ = @import("agent_ndjson_fallback_test.zig");

const std = @import("std");
const agent_mod = @import("agent.zig");
const parseNdjson = agent_mod.parseNdjson;

// ── AC-1: multi-message concatenation order ────────────────────────────

test "parseNdjson fallback concatenates multiple assistant messages with newlines" {
    const alloc = std.testing.allocator;
    // Two assistant messages, each with a single text block. No result line.
    const data =
        \\{"type":"assistant","message":{"content":[{"type":"text","text":"first"}]}}
        \\{"type":"assistant","message":{"content":[{"type":"text","text":"second"}]}}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("first\nsecond", result.output);
    try std.testing.expect(result.new_session_id == null);
}

// ── AC-2: non-text content types skipped ──────────────────────────────

test "parseNdjson fallback skips non-text content types" {
    const alloc = std.testing.allocator;
    // Content array: tool_use, then text, then image. No result line.
    const data =
        \\{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu1","name":"bash","input":{}},{"type":"text","text":"only this"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"abc"}}]}}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("only this", result.output);
}

// ── AC-3: mixed content within a single message ────────────────────────

test "parseNdjson fallback mixed content single message" {
    const alloc = std.testing.allocator;
    // Interleaved tool_use and text blocks in one message. No result line.
    const data =
        \\{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu1","name":"read","input":{}},{"type":"text","text":"alpha"},{"type":"tool_use","id":"tu2","name":"write","input":{}},{"type":"text","text":"beta"}]}}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("alpha\nbeta", result.output);
}

// ── AC-4: result field takes priority over assistant fallback ──────────

test "parseNdjson result field takes priority over assistant fallback" {
    const alloc = std.testing.allocator;
    // Both an assistant text block and a non-empty result line present.
    const data =
        \\{"type":"assistant","message":{"content":[{"type":"text","text":"fallback text"}]}}
        \\{"type":"result","subtype":"success","result":"final answer"}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("final answer", result.output);
}

// ── AC-5: empty result field triggers assistant fallback ───────────────

test "parseNdjson empty result triggers assistant fallback" {
    const alloc = std.testing.allocator;
    // result field is present but empty string; assistant text should be used.
    const data =
        \\{"type":"assistant","message":{"content":[{"type":"text","text":"fallback"}]}}
        \\{"type":"result","subtype":"success","result":""}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("fallback", result.output);
}

// ── AC-6: no text content yields empty output ──────────────────────────

test "parseNdjson no text content yields empty output" {
    const alloc = std.testing.allocator;
    // Only tool_use blocks; no result line. Fallback collects nothing.
    const data =
        \\{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu1","name":"bash","input":{}},{"type":"tool_use","id":"tu2","name":"read","input":{}}]}}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("", result.output);
}

// ── AC-7: three assistant messages — order preserved ───────────────────

test "parseNdjson fallback three messages order preserved" {
    const alloc = std.testing.allocator;
    // Three messages in order a, b, c. No result line.
    const data =
        \\{"type":"assistant","message":{"content":[{"type":"text","text":"a"}]}}
        \\{"type":"assistant","message":{"content":[{"type":"text","text":"b"}]}}
        \\{"type":"assistant","message":{"content":[{"type":"text","text":"c"}]}}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("a\nb\nc", result.output);
}

// ── Edge cases ─────────────────────────────────────────────────────────

test "parseNdjson assistant message with no message key is skipped" {
    const alloc = std.testing.allocator;
    // assistant line has no "message" field — must not panic or error.
    const data =
        \\{"type":"assistant","other":"ignored"}
        \\{"type":"assistant","message":{"content":[{"type":"text","text":"ok"}]}}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    // Only the second message contributes.
    try std.testing.expectEqualStrings("ok", result.output);
}

test "parseNdjson assistant message with absent content array is skipped" {
    const alloc = std.testing.allocator;
    // message object exists but has no "content" array.
    const data =
        \\{"type":"assistant","message":{"role":"assistant"}}
        \\{"type":"assistant","message":{"content":[{"type":"text","text":"valid"}]}}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("valid", result.output);
}

test "parseNdjson content item with no type key is skipped" {
    const alloc = std.testing.allocator;
    // One item lacks "type"; only the item with type="text" counts.
    const data =
        \\{"type":"assistant","message":{"content":[{"text":"invisible"},{"type":"text","text":"visible"}]}}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("visible", result.output);
}

test "parseNdjson content item type text with no text key is skipped" {
    const alloc = std.testing.allocator;
    // type="text" but "text" field is absent — item contributes nothing.
    const data =
        \\{"type":"assistant","message":{"content":[{"type":"text"},{"type":"text","text":"present"}]}}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("present", result.output);
}

test "parseNdjson separator between fallback text blocks is exactly one newline" {
    const alloc = std.testing.allocator;
    // Verify the separator is "\n" not "\n\n" or " ".
    const data =
        \\{"type":"assistant","message":{"content":[{"type":"text","text":"x"}]}}
        \\{"type":"assistant","message":{"content":[{"type":"text","text":"y"}]}}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    // Must be exactly "x\ny" — neither "x\n\ny" nor "x y".
    try std.testing.expectEqualStrings("x\ny", result.output);
    try std.testing.expectEqual(@as(usize, 3), result.output.len);
}
