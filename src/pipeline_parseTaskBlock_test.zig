// Tests for the deduplicated parseNextTaskBlock helper in pipeline.zig.
//
// Every acceptance criterion from spec.md is covered here. These tests will
// FAIL until parseNextTaskBlock (and ParsedTask) are added to pipeline.zig and
// the test block there includes:
//   _ = @import("pipeline_parseTaskBlock_test.zig");

const std = @import("std");
const pipeline = @import("pipeline.zig");

const parseNextTaskBlock = pipeline.parseNextTaskBlock;
const ParsedTask = pipeline.ParsedTask;

// =============================================================================
// AC1: Single valid block → correct title and description returned
// =============================================================================

test "AC1: single valid block returns correct title and description" {
    var remaining: []const u8 =
        \\TASK_START
        \\TITLE: Fix the bug
        \\DESCRIPTION: Something is broken and needs fixing.
        \\TASK_END
    ;
    const task = parseNextTaskBlock(&remaining);
    try std.testing.expect(task != null);
    try std.testing.expectEqualStrings("Fix the bug", task.?.title);
    try std.testing.expect(std.mem.indexOf(u8, task.?.description, "Something is broken") != null);
}

// =============================================================================
// AC2: Two valid blocks → iterate correctly, third call returns null
// =============================================================================

test "AC2: two blocks — iterates both then returns null" {
    var remaining: []const u8 =
        \\TASK_START
        \\TITLE: First task
        \\DESCRIPTION: Details of first.
        \\TASK_END
        \\TASK_START
        \\TITLE: Second task
        \\DESCRIPTION: Details of second.
        \\TASK_END
    ;

    const t1 = parseNextTaskBlock(&remaining);
    try std.testing.expect(t1 != null);
    try std.testing.expectEqualStrings("First task", t1.?.title);

    const t2 = parseNextTaskBlock(&remaining);
    try std.testing.expect(t2 != null);
    try std.testing.expectEqualStrings("Second task", t2.?.title);

    const t3 = parseNextTaskBlock(&remaining);
    try std.testing.expect(t3 == null);
}

// =============================================================================
// AC3: Block with TITLE: but no DESCRIPTION: → description falls back to title
// =============================================================================

test "AC3: block with TITLE but no DESCRIPTION — description equals title" {
    var remaining: []const u8 =
        \\TASK_START
        \\TITLE: Only a title here
        \\TASK_END
    ;
    const task = parseNextTaskBlock(&remaining);
    try std.testing.expect(task != null);
    try std.testing.expectEqualStrings("Only a title here", task.?.title);
    try std.testing.expectEqualStrings(task.?.title, task.?.description);
}

// =============================================================================
// AC4: Block with whitespace-only TITLE: → skipped; next valid block returned
// =============================================================================

test "AC4a: whitespace-only TITLE block is skipped; next valid block returned" {
    var remaining: []const u8 =
        \\TASK_START
        \\TITLE:
        \\DESCRIPTION: Should be skipped.
        \\TASK_END
        \\TASK_START
        \\TITLE: Valid task
        \\DESCRIPTION: This one counts.
        \\TASK_END
    ;
    const task = parseNextTaskBlock(&remaining);
    try std.testing.expect(task != null);
    try std.testing.expectEqualStrings("Valid task", task.?.title);
}

test "AC4b: whitespace-only TITLE block with no successor returns null" {
    var remaining: []const u8 =
        \\TASK_START
        \\TITLE:
        \\DESCRIPTION: No valid task after this.
        \\TASK_END
    ;
    const task = parseNextTaskBlock(&remaining);
    try std.testing.expect(task == null);
}

// =============================================================================
// AC5: Block missing TITLE: line entirely → skipped
// =============================================================================

test "AC5: block with no TITLE line is skipped" {
    var remaining: []const u8 =
        \\TASK_START
        \\DESCRIPTION: There is no title here.
        \\TASK_END
    ;
    const task = parseNextTaskBlock(&remaining);
    try std.testing.expect(task == null);
}

// =============================================================================
// AC6: Empty input → returns null
// =============================================================================

test "AC6: empty input returns null" {
    var remaining: []const u8 = "";
    const task = parseNextTaskBlock(&remaining);
    try std.testing.expect(task == null);
}

// =============================================================================
// AC7: TASK_START with no TASK_END → returns null; remaining set to ""
// =============================================================================

test "AC7: TASK_START without TASK_END returns null and clears remaining" {
    var remaining: []const u8 =
        \\TASK_START
        \\TITLE: Incomplete block
        \\DESCRIPTION: No closing marker.
    ;
    const task = parseNextTaskBlock(&remaining);
    try std.testing.expect(task == null);
    try std.testing.expectEqualStrings("", remaining);
}

// =============================================================================
// AC8: Leading/trailing whitespace on TITLE: and DESCRIPTION: values is stripped
// =============================================================================

test "AC8: TITLE and DESCRIPTION values are trimmed of surrounding whitespace" {
    var remaining: []const u8 =
        \\TASK_START
        \\TITLE:    Trim me
        \\DESCRIPTION:    Trim this too
        \\TASK_END
    ;
    const task = parseNextTaskBlock(&remaining);
    try std.testing.expect(task != null);
    try std.testing.expectEqualStrings("Trim me", task.?.title);
    // description should start with the trimmed content, not leading spaces
    try std.testing.expect(!std.mem.startsWith(u8, task.?.description, " "));
    try std.testing.expect(std.mem.indexOf(u8, task.?.description, "Trim this too") != null);
}

// =============================================================================
// AC9: remaining pointer advances past TASK_END so callers iterate all blocks
// =============================================================================

test "AC9: remaining advances correctly across three consecutive blocks" {
    var remaining: []const u8 =
        \\TASK_START
        \\TITLE: Alpha
        \\DESCRIPTION: First.
        \\TASK_END
        \\TASK_START
        \\TITLE: Beta
        \\DESCRIPTION: Second.
        \\TASK_END
        \\TASK_START
        \\TITLE: Gamma
        \\DESCRIPTION: Third.
        \\TASK_END
    ;

    var count: u32 = 0;
    while (parseNextTaskBlock(&remaining)) |_| {
        count += 1;
    }
    try std.testing.expectEqual(@as(u32, 3), count);
    // Entire input consumed — remaining is empty or contains only non-block text
    try std.testing.expect(std.mem.indexOf(u8, remaining, "TASK_START") == null);
}

// =============================================================================
// AC10: DESCRIPTION: content spanning multiple lines captured to end of block
// =============================================================================

test "AC10: multi-line DESCRIPTION is captured to end of block" {
    var remaining: []const u8 =
        \\TASK_START
        \\TITLE: Multi-line desc
        \\DESCRIPTION: Line one.
        \\Line two continues here.
        \\Line three as well.
        \\TASK_END
    ;
    const task = parseNextTaskBlock(&remaining);
    try std.testing.expect(task != null);
    try std.testing.expectEqualStrings("Multi-line desc", task.?.title);
    try std.testing.expect(std.mem.indexOf(u8, task.?.description, "Line one") != null);
    try std.testing.expect(std.mem.indexOf(u8, task.?.description, "Line two") != null);
    try std.testing.expect(std.mem.indexOf(u8, task.?.description, "Line three") != null);
}

// =============================================================================
// Edge case: TASK_END appearing before any TASK_START is ignored
// =============================================================================

test "E1: stray TASK_END before TASK_START is ignored" {
    var remaining: []const u8 =
        \\TASK_END
        \\Some random text
        \\TASK_START
        \\TITLE: Real task
        \\DESCRIPTION: Comes after stray marker.
        \\TASK_END
    ;
    const task = parseNextTaskBlock(&remaining);
    try std.testing.expect(task != null);
    try std.testing.expectEqualStrings("Real task", task.?.title);
}

// =============================================================================
// Edge case: whitespace-only block body (no TITLE match) → skipped
// =============================================================================

test "E2: whitespace-only block body is skipped" {
    var remaining: []const u8 = "TASK_START\n   \n\t\nTASK_END";
    const task = parseNextTaskBlock(&remaining);
    try std.testing.expect(task == null);
}

// =============================================================================
// Edge case: DESCRIPTION: appears before TITLE: → block skipped
// (matches existing behaviour: title still empty when DESCRIPTION: is seen)
// =============================================================================

test "E3: DESCRIPTION before TITLE causes block to be skipped" {
    var remaining: []const u8 =
        \\TASK_START
        \\DESCRIPTION: Comes first, no title yet.
        \\TITLE: Comes second
        \\TASK_END
    ;
    // Per spec §5: "title will be empty (no TITLE: seen yet) so the block is skipped."
    const task = parseNextTaskBlock(&remaining);
    try std.testing.expect(task == null);
}

// =============================================================================
// Edge case: duplicate TITLE: lines — first occurrence wins
// =============================================================================

test "E4: first TITLE line wins when there are two" {
    var remaining: []const u8 =
        \\TASK_START
        \\TITLE: First title
        \\DESCRIPTION: Body text.
        \\TITLE: Second title
        \\TASK_END
    ;
    const task = parseNextTaskBlock(&remaining);
    try std.testing.expect(task != null);
    try std.testing.expectEqualStrings("First title", task.?.title);
}

// =============================================================================
// Edge case: input contains only non-block text (no markers) → null
// =============================================================================

test "E5: input with no markers at all returns null" {
    var remaining: []const u8 = "Just some random text with no markers whatsoever.";
    const task = parseNextTaskBlock(&remaining);
    try std.testing.expect(task == null);
}
