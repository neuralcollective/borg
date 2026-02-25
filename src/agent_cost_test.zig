// Tests for cost_usd extraction in parseNdjson.
//
// Covers: AC6 from spec.md (Task #56) — AgentResult.cost_usd is populated
// from the total_cost_usd field of the Claude NDJSON result event.
//
// These tests FAIL until the implementation adds:
//   - cost_usd: f64 field to AgentResult
//   - extraction of total_cost_usd from the result event in parseNdjson
//
// To include in the build, add to agent.zig's test block:
//   _ = @import("agent_cost_test.zig");

const std = @import("std");
const agent_mod = @import("agent.zig");
const parseNdjson = agent_mod.parseNdjson;

// =============================================================================
// AC6 — cost_usd extracted from total_cost_usd in result event
// =============================================================================

test "AC6: parseNdjson extracts total_cost_usd from result event" {
    const alloc = std.testing.allocator;
    const data =
        \\{"type":"result","subtype":"success","result":"done","total_cost_usd":0.125}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    // 0.125 is exactly representable in IEEE 754
    try std.testing.expectApproxEqAbs(@as(f64, 0.125), result.cost_usd, 1e-9);
}

test "AC6: parseNdjson cost_usd defaults to 0.0 when total_cost_usd absent" {
    const alloc = std.testing.allocator;
    const data =
        \\{"type":"result","subtype":"success","result":"done"}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqual(@as(f64, 0.0), result.cost_usd);
}

test "AC6: parseNdjson cost_usd is 0.0 when no result event present" {
    const alloc = std.testing.allocator;
    const data =
        \\{"type":"assistant","message":{"content":[{"type":"text","text":"hello"}]}}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqual(@as(f64, 0.0), result.cost_usd);
}

test "AC6: parseNdjson cost_usd is 0.0 when total_cost_usd is 0" {
    const alloc = std.testing.allocator;
    const data =
        \\{"type":"result","subtype":"success","result":"done","total_cost_usd":0}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqual(@as(f64, 0.0), result.cost_usd);
}

test "AC6: parseNdjson cost_usd with integer-valued total_cost_usd" {
    const alloc = std.testing.allocator;
    const data =
        \\{"type":"result","subtype":"success","result":"r","total_cost_usd":2}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectApproxEqAbs(@as(f64, 2.0), result.cost_usd, 1e-9);
}

test "AC6: parseNdjson cost_usd with fractional total_cost_usd" {
    const alloc = std.testing.allocator;
    // Use 0.5 — exact in IEEE 754
    const data =
        \\{"type":"result","subtype":"success","result":"r","total_cost_usd":0.5}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectApproxEqAbs(@as(f64, 0.5), result.cost_usd, 1e-9);
}

// =============================================================================
// Last result event wins (cost from last result line)
// =============================================================================

test "last result event's total_cost_usd is used when multiple result events" {
    const alloc = std.testing.allocator;
    const data =
        \\{"type":"result","result":"first","total_cost_usd":0.25}
        \\{"type":"result","result":"second","total_cost_usd":0.75}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    // Last result wins for output text
    try std.testing.expectEqualStrings("second", result.output);
    // Last result wins for cost too
    try std.testing.expectApproxEqAbs(@as(f64, 0.75), result.cost_usd, 1e-9);
}

test "cost_usd is 0.0 when second result event has no total_cost_usd" {
    const alloc = std.testing.allocator;
    const data =
        \\{"type":"result","result":"first","total_cost_usd":0.5}
        \\{"type":"result","result":"second"}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    // Second result has no cost, so it resets to 0.0
    try std.testing.expectEqualStrings("second", result.output);
    try std.testing.expectEqual(@as(f64, 0.0), result.cost_usd);
}

// =============================================================================
// cost_usd does not interfere with other AgentResult fields
// =============================================================================

test "cost_usd field is independent of output and session_id extraction" {
    const alloc = std.testing.allocator;
    const data =
        \\{"type":"system","subtype":"init","session_id":"sess-xyz"}
        \\{"type":"result","subtype":"success","result":"answer","session_id":"sess-xyz","total_cost_usd":0.25}
    ;
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("answer", result.output);
    try std.testing.expectEqualStrings("sess-xyz", result.new_session_id.?);
    try std.testing.expectApproxEqAbs(@as(f64, 0.25), result.cost_usd, 1e-9);
}

test "empty NDJSON stream yields cost_usd=0.0" {
    const alloc = std.testing.allocator;
    const result = try parseNdjson(alloc, "");
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqual(@as(f64, 0.0), result.cost_usd);
}

test "invalid-JSON lines do not affect cost_usd extraction" {
    const alloc = std.testing.allocator;
    const data = "\nnot json\n{\"type\":\"result\",\"result\":\"ok\",\"total_cost_usd\":0.125}\n";
    const result = try parseNdjson(alloc, data);
    defer alloc.free(result.output);
    defer alloc.free(result.raw_stream);
    defer if (result.new_session_id) |sid| alloc.free(sid);

    try std.testing.expectEqualStrings("ok", result.output);
    try std.testing.expectApproxEqAbs(@as(f64, 0.125), result.cost_usd, 1e-9);
}

// =============================================================================
// AgentResult struct has cost_usd field
// =============================================================================

test "AgentResult struct has cost_usd field of type f64" {
    // Verify the struct can be initialized with cost_usd
    const r = agent_mod.AgentResult{
        .output = "out",
        .raw_stream = "raw",
        .new_session_id = null,
        .cost_usd = 1.25,
    };
    try std.testing.expectApproxEqAbs(@as(f64, 1.25), r.cost_usd, 1e-9);
}
