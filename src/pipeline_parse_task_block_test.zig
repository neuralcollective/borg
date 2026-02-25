// Tests for Pipeline.parseTaskBlock — the helper that replaces raw pointer
// arithmetic with std.mem.indexOf when extracting the DESCRIPTION: content
// from a TASK block.
//
// A "block" is the text between TASK_START … TASK_END after outer whitespace
// trimming. Expected format:
//
//   TITLE: some title
//   DESCRIPTION: body text that may
//   span multiple lines
//
// parseTaskBlock returns:
//   null                          — when no TITLE: line is present
//   .{ .title, .description }     — on success; description falls back to
//                                    title when no DESCRIPTION: line exists
//
// To include in the build, pipeline.zig must:
//   - declare `pub fn parseTaskBlock(block: []const u8) ?...` accessible as
//     Pipeline.parseTaskBlock
//   - add `_ = @import("pipeline_parse_task_block_test.zig");` to its test block
//
// All tests FAIL before the implementation is applied because parseTaskBlock
// does not yet exist as a pub function.

const std = @import("std");
const pipeline = @import("pipeline.zig");

const parseTaskBlock = pipeline.Pipeline.parseTaskBlock;

// =============================================================================
// AC1: No @intFromPtr subtraction for DESCRIPTION offset
//
// After the fix, pipeline.zig must not contain the raw pointer subtraction
// pattern used to compute desc_start.
// =============================================================================

test "AC1: pipeline.zig no longer contains @intFromPtr pointer subtraction for DESCRIPTION" {
    const source = @embedFile("pipeline.zig");
    const bad_pattern = "@intFromPtr(trimmed.ptr) - @intFromPtr(block.ptr)";
    const found = std.mem.indexOf(u8, source, bad_pattern);
    try std.testing.expect(found == null);
}

// =============================================================================
// AC2: Both sites use std.mem.indexOf to locate DESCRIPTION content
//
// Verified by confirming the canonical needle appears at least twice in the
// source (one per call site).
// =============================================================================

test "AC2: pipeline.zig uses std.mem.indexOf for DESCRIPTION at both call sites" {
    const source = @embedFile("pipeline.zig");
    const needle = "std.mem.indexOf(u8, block, \"DESCRIPTION:\")";
    const first = std.mem.indexOf(u8, source, needle);
    try std.testing.expect(first != null);
    const second = std.mem.indexOfPos(u8, source, first.? + needle.len, needle);
    try std.testing.expect(second != null);
}

// =============================================================================
// AC3: Normal block with TITLE and DESCRIPTION
//
// Both fields must be extracted; description is the trimmed content after the
// DESCRIPTION: prefix, not the title.
// =============================================================================

test "AC3: block with TITLE and DESCRIPTION extracts both correctly" {
    const block = "TITLE: Fix the widget\nDESCRIPTION: The widget crashes on empty input.";
    const result = parseTaskBlock(block);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("Fix the widget", result.?.title);
    try std.testing.expectEqualStrings("The widget crashes on empty input.", result.?.description);
}

test "AC3: multi-line description is trimmed but preserved" {
    const block = "TITLE: Add logging\nDESCRIPTION: Log every request.\nInclude timestamps.";
    const result = parseTaskBlock(block);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("Add logging", result.?.title);
    // Description begins after "DESCRIPTION:" and is outer-trimmed; interior
    // newlines are preserved as-is.
    try std.testing.expect(std.mem.startsWith(u8, result.?.description, "Log every request."));
}

test "AC3: title and description do not bleed into each other" {
    const block = "TITLE: Short\nDESCRIPTION: Long body here.";
    const result = parseTaskBlock(block);
    try std.testing.expect(result != null);
    try std.testing.expect(!std.mem.eql(u8, result.?.title, result.?.description));
    try std.testing.expectEqualStrings("Short", result.?.title);
    try std.testing.expectEqualStrings("Long body here.", result.?.description);
}

// =============================================================================
// AC4: Block with TITLE only — description falls back to title
// =============================================================================

test "AC4: block with TITLE only makes description equal to title" {
    const block = "TITLE: Refactor database layer";
    const result = parseTaskBlock(block);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("Refactor database layer", result.?.title);
    try std.testing.expectEqualStrings(result.?.title, result.?.description);
}

test "AC4: title-only block with trailing whitespace still falls back" {
    const block = "TITLE: Write tests   ";
    const result = parseTaskBlock(block);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("Write tests", result.?.title);
    try std.testing.expectEqualStrings(result.?.title, result.?.description);
}

// =============================================================================
// AC5: DESCRIPTION: before TITLE: in the block
//
// TITLE is still required. The function must not crash or produce a wrong
// result; the exact return value (null vs. valid result) depends on whether
// the implementation scans the full block or breaks at DESCRIPTION.
// Either way, description content must not be corrupt or out-of-bounds.
// =============================================================================

test "AC5: block with DESCRIPTION before TITLE does not crash or corrupt memory" {
    const block = "DESCRIPTION: body text here\nTITLE: My task";
    // The result may be null (single-pass) or a valid struct (two-pass).
    // What must NOT happen: a panic, out-of-bounds read, or wrong pointer.
    const result = parseTaskBlock(block);
    if (result) |r| {
        // If the implementation handles reordered fields, both must be correct.
        try std.testing.expectEqualStrings("My task", r.title);
        try std.testing.expect(std.mem.indexOf(u8, r.description, "body text here") != null);
    }
    // null is also an acceptable outcome for a single-pass implementation.
}

test "AC5: TITLE is required — block with only DESCRIPTION returns null" {
    const block = "DESCRIPTION: some body without any title line";
    const result = parseTaskBlock(block);
    try std.testing.expect(result == null);
}

// =============================================================================
// AC6: DESCRIPTION: line with leading whitespace
//
// std.mem.indexOf finds "DESCRIPTION:" regardless of leading spaces; the
// extracted description equals the text after the colon, trimmed.
// =============================================================================

test "AC6: DESCRIPTION line with leading spaces is found and trimmed correctly" {
    const block = "TITLE: My task\n  DESCRIPTION: body after spaces";
    const result = parseTaskBlock(block);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("My task", result.?.title);
    try std.testing.expectEqualStrings("body after spaces", result.?.description);
}

test "AC6: DESCRIPTION line with leading tabs is found and trimmed correctly" {
    const block = "TITLE: task\t\n\tDESCRIPTION: tabbed body";
    const result = parseTaskBlock(block);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("task", result.?.title);
    try std.testing.expectEqualStrings("tabbed body", result.?.description);
}

test "AC6: description content itself is not corrupted by whitespace stripping" {
    const block = "TITLE: t\n   DESCRIPTION:   leading spaces in value too";
    const result = parseTaskBlock(block);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("leading spaces in value too", result.?.description);
}

// =============================================================================
// AC7: DESCRIPTION: as the very first line of the block
//
// std.mem.indexOf returns 0; desc_start becomes "DESCRIPTION:".len (12).
// The description content is extracted correctly.
// =============================================================================

test "AC7: DESCRIPTION at byte offset 0 yields desc_start equal to prefix length" {
    // Block with DESCRIPTION: at the very beginning.
    // A two-pass implementation finds TITLE after DESCRIPTION and returns a
    // valid result; a single-pass implementation returns null.
    const block = "DESCRIPTION: first line body\nTITLE: task title";
    const result = parseTaskBlock(block);
    if (result) |r| {
        try std.testing.expectEqualStrings("task title", r.title);
        try std.testing.expectEqualStrings("first line body", r.description);
    }
    // null is acceptable for a single-pass implementation; the key assertion
    // is that indexOf(block, "DESCRIPTION:") == 0 is handled without panic.
}

test "AC7: DESCRIPTION at offset 0 with no TITLE returns null" {
    const block = "DESCRIPTION: only description no title";
    const result = parseTaskBlock(block);
    try std.testing.expect(result == null);
}

// =============================================================================
// E1: DESCRIPTION: appears exactly once
//
// indexOf returns the first (and only) match; result is identical to the
// old pointer-arithmetic approach for every well-formed block.
// =============================================================================

test "E1: single DESCRIPTION occurrence gives correct description" {
    const block = "TITLE: Single\nDESCRIPTION: exactly once";
    const result = parseTaskBlock(block);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("exactly once", result.?.description);
}

// =============================================================================
// E2: DESCRIPTION: substring inside the TITLE value
//
// Example: TITLE: Fix DESCRIPTION: handling
// indexOf(block, "DESCRIPTION:") naively finds the one inside TITLE first.
// The implementation must guard against this (e.g., search from the line
// offset, not from block[0]) so the real DESCRIPTION: line is used.
// =============================================================================

test "E2: DESCRIPTION substring in TITLE does not corrupt description" {
    const block = "TITLE: Fix DESCRIPTION: handling\nDESCRIPTION: actual body";
    const result = parseTaskBlock(block);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("Fix DESCRIPTION: handling", result.?.title);
    // The description must come from the real DESCRIPTION: line, not from
    // the middle of the TITLE value.
    try std.testing.expectEqualStrings("actual body", result.?.description);
}

// =============================================================================
// E3: Empty block
//
// No TITLE found; the function must return null without panicking.
// =============================================================================

test "E3: empty block returns null" {
    const result = parseTaskBlock("");
    try std.testing.expect(result == null);
}

test "E3: block with only whitespace returns null" {
    const result = parseTaskBlock("   \n\t  \n  ");
    try std.testing.expect(result == null);
}

// =============================================================================
// E4: DESCRIPTION: with no content after the colon
//
// block[desc_start..] is empty or whitespace-only; after trimming, description
// becomes "". The function must still return a struct (not null) because TITLE
// is present; description should be "" or fall back to title.
// =============================================================================

test "E4: empty description after colon returns title as fallback or empty string" {
    const block = "TITLE: My task\nDESCRIPTION:";
    const result = parseTaskBlock(block);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("My task", result.?.title);
    // description is either "" (no fallback) or the title (fallback); either
    // is acceptable as long as it is not an out-of-bounds slice.
    const desc = result.?.description;
    try std.testing.expect(desc.len == 0 or std.mem.eql(u8, desc, result.?.title));
}

test "E4: whitespace-only description after colon treated as empty or falls back" {
    const block = "TITLE: task\nDESCRIPTION:   \n   ";
    const result = parseTaskBlock(block);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("task", result.?.title);
    const desc = result.?.description;
    try std.testing.expect(desc.len == 0 or std.mem.eql(u8, desc, result.?.title));
}

// =============================================================================
// E5: Block with only a DESCRIPTION: line, no TITLE:
//
// The guard `if (title.len == 0)` must fire and the function must return null.
// =============================================================================

test "E5: DESCRIPTION-only block with no TITLE returns null" {
    const block = "DESCRIPTION: Some body text without any title at all.";
    const result = parseTaskBlock(block);
    try std.testing.expect(result == null);
}

test "E5: DESCRIPTION and junk but no TITLE returns null" {
    const block = "SOMETHING: else\nDESCRIPTION: content";
    const result = parseTaskBlock(block);
    try std.testing.expect(result == null);
}

// =============================================================================
// E6: Windows-style CRLF line endings
//
// The per-line trim strips '\r' before checking startsWith. std.mem.indexOf
// searches the raw block which may contain '\r' but "DESCRIPTION:" does not
// include '\r', so indexOf still finds the correct offset.
// =============================================================================

test "E6: CRLF line endings — description extracted correctly" {
    const block = "TITLE: Windows task\r\nDESCRIPTION: Windows body\r\n";
    const result = parseTaskBlock(block);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("Windows task", result.?.title);
    try std.testing.expectEqualStrings("Windows body", result.?.description);
}

test "E6: mixed CRLF and LF — description still correct" {
    const block = "TITLE: Mixed\r\nDESCRIPTION: body line\nnext line";
    const result = parseTaskBlock(block);
    try std.testing.expect(result != null);
    try std.testing.expectEqualStrings("Mixed", result.?.title);
    try std.testing.expect(std.mem.startsWith(u8, result.?.description, "body line"));
}
