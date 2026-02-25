// Tests for Pipeline.isTestFileError classification logic.
//
// isTestFileError gates whether a failing test run routes to the recoverable
// qa_fix state or stays in retry/failed. It inspects both stderr and stdout
// independently across four detection branches:
//   1. _test.zig  + error:
//   2. /tests/    + error:
//   3. Segmentation fault  (no second keyword required)
//   4. panicked   + _test
//
// To include in the build, pipeline.zig must:
//   - declare isTestFileError as `pub fn isTestFileError(...)`
//   - add `_ = @import("pipeline_isTestFileError_test.zig");` to its test block

const std = @import("std");
const pipeline = @import("pipeline.zig");

const isTestFileError = pipeline.Pipeline.isTestFileError;

// =============================================================================
// AC1: _test.zig + error: present in stderr
// =============================================================================

test "AC1: _test.zig with error: in stderr returns true" {
    try std.testing.expect(isTestFileError(
        "src/foo_test.zig:12:5: error: undeclared identifier",
        "",
    ));
}

// =============================================================================
// AC2: _test.zig + error: present in stdout (function checks both streams)
// =============================================================================

test "AC2: _test.zig with error: in stdout returns true" {
    try std.testing.expect(isTestFileError(
        "",
        "src/foo_test.zig:12:5: error: undeclared identifier",
    ));
}

// =============================================================================
// AC3: /tests/ + error: present in stderr
// =============================================================================

test "AC3: /tests/ with error: in stderr returns true" {
    try std.testing.expect(isTestFileError(
        "/repo/tests/foo.zig:3:1: error: expected ';'",
        "",
    ));
}

// =============================================================================
// AC4: /tests/ + error: present in stdout
// =============================================================================

test "AC4: /tests/ with error: in stdout returns true" {
    try std.testing.expect(isTestFileError(
        "",
        "/repo/tests/foo.zig:3:1: error: expected ';'",
    ));
}

// =============================================================================
// AC5: Segmentation fault in stderr (no second keyword required)
// =============================================================================

test "AC5: Segmentation fault in stderr returns true" {
    try std.testing.expect(isTestFileError(
        "Segmentation fault (core dumped)",
        "",
    ));
}

// =============================================================================
// AC6: Segmentation fault in stdout
// =============================================================================

test "AC6: Segmentation fault in stdout returns true" {
    try std.testing.expect(isTestFileError(
        "",
        "Segmentation fault at address 0x00",
    ));
}

// =============================================================================
// AC7: panicked + _test in stderr
// =============================================================================

test "AC7: panicked with _test in stderr returns true" {
    try std.testing.expect(isTestFileError(
        "thread 1 panicked: index out of bounds in foo_test at ...",
        "",
    ));
}

// =============================================================================
// AC8: panicked + _test in stdout
// =============================================================================

test "AC8: panicked with _test in stdout returns true" {
    try std.testing.expect(isTestFileError(
        "",
        "panicked: assertion failed in bar_test",
    ));
}

// =============================================================================
// AC9: plain error: without any test-file marker returns false
// =============================================================================

test "AC9: plain error: without test-file marker returns false" {
    try std.testing.expect(!isTestFileError(
        "error: compilation failed in src/main.zig",
        "",
    ));
}

// =============================================================================
// AC10: _test.zig without error: returns false
// =============================================================================

test "AC10: _test.zig without error: returns false" {
    try std.testing.expect(!isTestFileError(
        "running foo_test.zig... ok",
        "",
    ));
}

// =============================================================================
// AC11: /tests/ without error: returns false
// =============================================================================

test "AC11: /tests/ without error: returns false" {
    try std.testing.expect(!isTestFileError(
        "/repo/tests/foo.zig: all tests passed",
        "",
    ));
}

// =============================================================================
// AC12: panicked without _test returns false
// =============================================================================

test "AC12: panicked without _test returns false" {
    try std.testing.expect(!isTestFileError(
        "panicked: out of memory in src/main.zig",
        "",
    ));
}

// =============================================================================
// AC13: both inputs empty returns false
// =============================================================================

test "AC13: both inputs empty returns false" {
    try std.testing.expect(!isTestFileError("", ""));
}

// =============================================================================
// AC14: both inputs non-empty but no matching patterns returns false
// =============================================================================

test "AC14: non-matching content in both streams returns false" {
    try std.testing.expect(!isTestFileError("build succeeded", "all ok"));
}

// =============================================================================
// E1: cross-stream split — _test.zig in stderr, error: only in stdout → false
// Each output slice is checked independently; both substrings must appear in
// the same stream to fire the branch.
// =============================================================================

test "E1: _test.zig in stderr and error: only in stdout returns false" {
    try std.testing.expect(!isTestFileError(
        "running foo_test.zig",
        "error: something went wrong in main.zig",
    ));
}

// =============================================================================
// E2: cross-stream split — panicked in stderr, _test only in stdout → false
// =============================================================================

test "E2: panicked in stderr and _test only in stdout returns false" {
    try std.testing.expect(!isTestFileError(
        "panicked: assertion failed",
        "output from bar_test",
    ));
}

// =============================================================================
// E3: minimal exact substrings — "_test.zig" and "error:" with no context
// =============================================================================

test "E3: minimal _test.zig and error: substrings return true" {
    try std.testing.expect(isTestFileError("_test.zig error:", ""));
}

// =============================================================================
// E4: Segmentation fault as a substring deep inside a longer line
// =============================================================================

test "E4: Segmentation fault embedded in a longer line returns true" {
    try std.testing.expect(isTestFileError(
        "Process terminated: Segmentation fault occurred at pc=0xdeadbeef",
        "",
    ));
}

// =============================================================================
// E5: /tests/ in a container-style absolute path with error:
// The function does not anchor on repo root.
// =============================================================================

test "E5: /tests/ in container absolute path with error: returns true" {
    try std.testing.expect(isTestFileError(
        "/workspace/tests/runner.zig:1:1: error: use of undeclared identifier",
        "",
    ));
}

// =============================================================================
// E6: _test.zig satisfies the panicked + _test branch because _test is a
// substring of _test.zig
// =============================================================================

test "E6: panicked with _test.zig (superstring of _test) returns true" {
    try std.testing.expect(isTestFileError(
        "panicked: index out of bounds in src/foo_test.zig",
        "",
    ));
}

// =============================================================================
// E7: whitespace-only stderr, empty stdout — no patterns match → false
// (output.len > 0 so the guard does not skip, but no substrings match)
// =============================================================================

test "E7: whitespace-only stderr and empty stdout returns false" {
    try std.testing.expect(!isTestFileError("   \n\t  ", ""));
}
