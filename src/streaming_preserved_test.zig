// Tests for spec Task #73: Fix Failing Build on Main — streaming-callback preservation.
//
// AC23 — After the process.zig / subprocess.zig refactor, Docker.runWithStdio
//        must remain callable AND the StreamCallback must still be invoked for
//        each chunk of stdout produced by the container.  The same requirement
//        applies to agent.runDirect.
//
// Design rationale
// ────────────────
// The refactor replaces inline `read_buf: [8192]u8` drain loops and inline
// `.Exited => |code| code` switches with calls to process.drainPipe /
// subprocess.collectOutput and process.exitCode.  During that edit there is a
// real risk of accidentally dropping the `stream_cb.call(chunk)` invocation
// that feeds the live-stream dashboard.  These tests guard against that.
//
// Two kinds of assertions are used:
//
//   Structural  – @embedFile inspection: verifies that the source text still
//                 contains the callback invocation site.  This is the only
//                 practical approach since runWithStdio requires a live Docker
//                 daemon and runDirect requires a live Claude CLI.
//
//   Type-level  – @hasDecl / @typeInfo: verifies that public types and
//                 functions remain callable so that callers compile unchanged.
//
// Failure modes
// ─────────────
// Before implementation  : the structural tests that check `@import("process.zig")`
//                          inside docker.zig / agent.zig FAIL because neither
//                          file imports that module yet.
// After implementation   : all tests pass — the refactor has been applied and
//                          the streaming sites are preserved.
//
// To include in the build, add to docker.zig's trailing test section:
//   _ = @import("streaming_preserved_test.zig");

const std = @import("std");
const docker = @import("docker.zig");
const agent = @import("agent.zig");

// ── AC23 structural: stream_cb.call() is still present in docker.zig ──────────

test "AC23: docker.zig source still contains stream_cb.call() after refactor" {
    const src = @embedFile("docker.zig");
    // If this fails the implementation accidentally removed the streaming site.
    try std.testing.expect(std.mem.indexOf(u8, src, "stream_cb.call(") != null);
}

test "AC23: docker.zig source still contains stream_cb parameter in runWithStdio" {
    const src = @embedFile("docker.zig");
    // runWithStdio's signature must still accept a stream_cb argument.
    try std.testing.expect(std.mem.indexOf(u8, src, "stream_cb: agent_mod.StreamCallback") != null);
}

// ── AC23 structural: stream_cb.call() is still present in agent.zig ───────────

test "AC23: agent.zig source still contains stream_cb.call() in runDirect" {
    const src = @embedFile("agent.zig");
    // If this fails the implementation accidentally removed the streaming site.
    try std.testing.expect(std.mem.indexOf(u8, src, "stream_cb.call(") != null);
}

test "AC23: agent.zig source still contains stream_cb parameter in runDirect" {
    const src = @embedFile("agent.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "stream_cb: StreamCallback") != null);
}

// ── AC23 type-level: runWithStdio is still declared on Docker ─────────────────

test "AC23: Docker.runWithStdio is still declared" {
    try std.testing.expect(@hasDecl(docker.Docker, "runWithStdio"));
}

// ── AC23 type-level: runDirect is still declared in agent ─────────────────────

test "AC23: agent.runDirect is still declared" {
    try std.testing.expect(@hasDecl(agent, "runDirect"));
}

// ── AC23 type-level: StreamCallback type unchanged ────────────────────────────

test "AC23: agent.StreamCallback still has call method" {
    try std.testing.expect(@hasDecl(agent.StreamCallback, "call"));
}

test "AC23: agent.StreamCallback.call accepts (context, []const u8)" {
    // Verify call is a method on StreamCallback — inlineable, takes self + data.
    const cb = agent.StreamCallback{};
    // Calling with empty slice must not crash (on_data is null by default).
    cb.call("test chunk");
}

// ── AC21 regression: public result types are unchanged ────────────────────────
//
// After the refactor, no fields may be added, removed, or renamed on the result
// structs that callers depend on.

test "AC21: docker.RunResult still has stdout, exit_code, allocator fields" {
    const info = @typeInfo(docker.RunResult);
    const fields = info.@"struct".fields;
    const expected = [_][]const u8{ "stdout", "exit_code", "allocator" };
    for (expected) |name| {
        var found = false;
        for (fields) |f| {
            if (std.mem.eql(u8, f.name, name)) {
                found = true;
                break;
            }
        }
        try std.testing.expect(found);
    }
}

test "AC21: agent.AgentResult still has output, raw_stream, new_session_id fields" {
    const info = @typeInfo(agent.AgentResult);
    const fields = info.@"struct".fields;
    const expected = [_][]const u8{ "output", "raw_stream", "new_session_id" };
    for (expected) |name| {
        var found = false;
        for (fields) |f| {
            if (std.mem.eql(u8, f.name, name)) {
                found = true;
                break;
            }
        }
        try std.testing.expect(found);
    }
}

// ── Process.zig import checks (fail before implementation) ────────────────────
//
// These tests FAIL before the refactor because docker.zig and agent.zig do not
// yet import process.zig.  They pass once the implementation is applied.

test "AC23 pre-condition: docker.zig imports process.zig for exit-code extraction" {
    const src = @embedFile("docker.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "@import(\"process.zig\")") != null);
}

test "AC23 pre-condition: agent.zig imports process.zig for exit-code extraction" {
    const src = @embedFile("agent.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "@import(\"process.zig\")") != null);
}

// ── Edge: docker.zig exit-code switch is gone (old pattern removed) ───────────
//
// FAILS before implementation (switch still present), passes after.

test "Edge: docker.zig no longer has inline .Exited => |code| code switch" {
    const src = @embedFile("docker.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, ".Exited => |code| code") == null);
}

// ── Edge: agent.zig exit-code switch is gone (old pattern removed) ────────────

test "Edge: agent.zig no longer has inline .Exited => |code| code switch" {
    const src = @embedFile("agent.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, ".Exited => |code| code") == null);
}
