// Tests for parseNdjson session_id extraction precedence.
//
// Covers three scenarios that were previously untested:
//   AC1 — session_id present only in a system message
//   AC2 — session_id present only in a result message
//   AC3 — session_id present in both; result message must win
//
// To include these tests in the build, add the following to agent.zig
// (inside the existing test section):
//
//   test {
//       _ = @import("agent_session_id_test.zig");
//   }

const std = @import("std");
const agent = @import("agent.zig");

// =============================================================================
// AC1: session_id from system message only
//
// The stream contains a system message with a session_id and a result message
// that does NOT carry a session_id field.  The system-sourced value must be
// preserved — the result message must not silently clear it.
// =============================================================================

test "parseNdjson session_id from system message only" {
    const alloc = std.testing.allocator;
    const data =
        \\{"type":"system","subtype":"init","session_id":"sys-only-id"}
        \\{"type":"result","subtype":"success","result":"done"}
    ;
    const result = try agent.parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    // Output comes from the result message
    try std.testing.expectEqualStrings("done", result.output);
    // new_session_id must be non-null and equal to the system message value
    try std.testing.expect(result.new_session_id != null);
    try std.testing.expectEqualStrings("sys-only-id", result.new_session_id.?);
}

// =============================================================================
// AC2: session_id from result message only
//
// The stream contains a system message WITHOUT a session_id field and a result
// message WITH a session_id.  Only the result value should be captured.
// =============================================================================

test "parseNdjson session_id from result message only" {
    const alloc = std.testing.allocator;
    const data =
        \\{"type":"system","subtype":"init"}
        \\{"type":"result","subtype":"success","result":"done","session_id":"result-only-id"}
    ;
    const result = try agent.parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("done", result.output);
    try std.testing.expect(result.new_session_id != null);
    try std.testing.expectEqualStrings("result-only-id", result.new_session_id.?);
}

// =============================================================================
// AC3: result session_id overrides system session_id
//
// Both the system message and the result message carry a session_id, but with
// DIFFERENT values.  Because the result message appears later in the stream,
// its value must take precedence and be returned as new_session_id.
// =============================================================================

test "parseNdjson result session_id overrides system session_id" {
    const alloc = std.testing.allocator;
    const data =
        \\{"type":"system","subtype":"init","session_id":"sys-session"}
        \\{"type":"result","subtype":"success","result":"done","session_id":"result-session"}
    ;
    const result = try agent.parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("done", result.output);
    try std.testing.expect(result.new_session_id != null);
    // "result-session" must win over "sys-session"
    try std.testing.expectEqualStrings("result-session", result.new_session_id.?);
}
