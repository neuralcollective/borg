// Tests for removing the unreachable else branch in the seed_mode switch.
//
// The switch on seed_mode in Pipeline.seedScan (pipeline.zig ~line 337) had an
// `else => "refactoring"` arm that can never execute because seed_mode is
// always in [0,4] (computed as `(prev + 1) % 5`).  The fix replaces that arm
// with `else => unreachable` so Zig's safety-checked builds trap any future
// regression and the misleading silent fallback is gone.
//
// All tests here are structural: they embed the pipeline.zig source at compile
// time and assert properties about its text.  This means:
//   - They FAIL before the fix  (bad code is still present).
//   - They PASS after the fix   (unreachable replaces the string literal).
//
// To include in the build, pipeline.zig must add to its test block:
//   _ = @import("pipeline_seed_mode_switch_test.zig");

const std = @import("std");

// Embed the full source of pipeline.zig at compile time.
const pipeline_src = @embedFile("pipeline.zig");

// =============================================================================
// AC1: The arm `else => "refactoring"` must not exist in the switch.
// Before the fix this test fails because the literal is present.
// =============================================================================

test "AC1: pipeline.zig does not contain 'else => \"refactoring\"' in seed_mode switch" {
    // The forbidden pattern is the exact dead-branch text that must be removed.
    const forbidden = "else => \"refactoring\"";
    try std.testing.expect(std.mem.indexOf(u8, pipeline_src, forbidden) == null);
}

// =============================================================================
// AC2: The else arm must use `unreachable`, not a string literal.
// Before the fix `else => unreachable` is absent; after the fix it is present.
// =============================================================================

test "AC2: pipeline.zig contains 'else => unreachable' in seed_mode switch" {
    const required = "else => unreachable";
    try std.testing.expect(std.mem.indexOf(u8, pipeline_src, required) != null);
}

// =============================================================================
// AC3: "refactoring" appears exactly once in the switch (for arm 0 =>),
// not twice.  Before the fix it appears in both arm 0 and the else arm.
// =============================================================================

test "AC3: \"refactoring\" appears exactly once in pipeline.zig switch" {
    // Count occurrences of the string "refactoring" in the entire source.
    // The only legitimate occurrence is: `0 => "refactoring"`.
    const needle = "\"refactoring\"";
    var count: usize = 0;
    var i: usize = 0;
    while (i + needle.len <= pipeline_src.len) {
        if (std.mem.eql(u8, pipeline_src[i .. i + needle.len], needle)) {
            count += 1;
            i += needle.len;
        } else {
            i += 1;
        }
    }
    try std.testing.expectEqual(@as(usize, 1), count);
}

// =============================================================================
// AC4: The five mode-label arms (0–4) are all still present and unchanged.
// Removing the else arm must not accidentally remove a real arm.
// =============================================================================

test "AC4: all five seed_mode labels are present" {
    const labels = [_][]const u8{
        "\"refactoring\"",
        "\"bug audit\"",
        "\"test coverage\"",
        "\"feature discovery\"",
        "\"architecture review\"",
    };
    for (labels) |label| {
        try std.testing.expect(
            std.mem.indexOf(u8, pipeline_src, label) != null,
        );
    }
}

// =============================================================================
// Edge case: verify arm 0 is mapped to "refactoring" (correct association).
// The substring `0 => "refactoring"` (or `0 => "refactoring",`) must exist.
// =============================================================================

test "Edge: arm 0 maps to refactoring" {
    // Accept either `0 => "refactoring"` or `0 => "refactoring",`
    const arm = "0 => \"refactoring\"";
    try std.testing.expect(std.mem.indexOf(u8, pipeline_src, arm) != null);
}

// =============================================================================
// Edge case: verify there is no `else => "refactoring",` variant either.
// (With or without trailing comma — both are forbidden.)
// =============================================================================

test "Edge: else arm with trailing comma also absent" {
    const forbidden_comma = "else => \"refactoring\",";
    try std.testing.expect(std.mem.indexOf(u8, pipeline_src, forbidden_comma) == null);
}
