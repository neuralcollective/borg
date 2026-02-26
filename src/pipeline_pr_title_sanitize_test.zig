// Tests for PR title sanitization in pipeline.zig.
//
// These tests verify that '\n' and '\r' are filtered from PR titles before
// they are interpolated into the `gh pr create` shell command string, along
// with the pre-existing set of dangerous shell characters.
//
// All tests call pipeline.sanitizePrTitle, which the implementation agent
// must extract from the inline loop and expose as:
//
//   pub fn sanitizePrTitle(input: []const u8, max_len: usize, out: *std.ArrayList(u8)) !void
//
// These tests FAIL until that function is added to pipeline.zig and this
// file is imported from the pipeline.zig test block:
//   _ = @import("pipeline_pr_title_sanitize_test.zig");

const std = @import("std");
const pipeline = @import("pipeline.zig");

const sanitizePrTitle = pipeline.sanitizePrTitle;

// =============================================================================
// AC1 — LF stripped: '\n' is replaced with a space
// =============================================================================

test "AC1: LF in title is replaced with a space" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("Fix bug\nin handler", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\n") == null);
    // The byte that was '\n' becomes ' ', so the text around it is preserved
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "Fix bug") != null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "in handler") != null);
    // Replaced with a space, so there must be a space at that position
    try std.testing.expect(std.mem.indexOf(u8, buf.items, " ") != null);
}

test "AC1: LF-only title becomes a single space" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("\n", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\n") == null);
    try std.testing.expectEqualStrings(" ", buf.items);
}

test "AC1: multiple LFs all become spaces" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("a\nb\nc", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\n") == null);
    try std.testing.expectEqualStrings("a b c", buf.items);
}

// =============================================================================
// AC2 — CR stripped: '\r' is replaced with a space
// =============================================================================

test "AC2: CR in title is replaced with a space" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("Fix bug\rin handler", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\r") == null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "Fix bug") != null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "in handler") != null);
}

test "AC2: CR-only title becomes a single space" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("\r", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\r") == null);
    try std.testing.expectEqualStrings(" ", buf.items);
}

test "AC2: multiple CRs all become spaces" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("a\rb\rc", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\r") == null);
    try std.testing.expectEqualStrings("a b c", buf.items);
}

// =============================================================================
// AC3 — CRLF stripped: each of '\r' and '\n' is independently replaced
// =============================================================================

test "AC3: CRLF sequence produces two spaces" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("line1\r\nline2", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\r") == null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\n") == null);
    try std.testing.expectEqualStrings("line1  line2", buf.items);
}

test "AC3: multiple CRLF pairs are all stripped" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("a\r\nb\r\nc", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\r") == null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\n") == null);
    try std.testing.expectEqualStrings("a  b  c", buf.items);
}

test "AC3: title that is only CRLF becomes all spaces" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("\r\n", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\r") == null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\n") == null);
    try std.testing.expectEqualStrings("  ", buf.items);
}

// =============================================================================
// AC4 — Existing characters still stripped (regression guard)
// Each of '"', '\', '$', '`' must still be replaced with a space.
// =============================================================================

test "AC4: double-quote is replaced with a space" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("say \"hello\"", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\"") == null);
    try std.testing.expectEqualStrings("say  hello ", buf.items);
}

test "AC4: backslash is replaced with a space" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("path\\to\\file", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\\") == null);
    try std.testing.expectEqualStrings("path to file", buf.items);
}

test "AC4: dollar sign is replaced with a space" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("cost is $100", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "$") == null);
    try std.testing.expectEqualStrings("cost is  100", buf.items);
}

test "AC4: backtick is replaced with a space" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("use `cmd`", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "`") == null);
    try std.testing.expectEqualStrings("use  cmd ", buf.items);
}

test "AC4: all originally-stripped characters together are each replaced" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("\"\\$`", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\"") == null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\\") == null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "$") == null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "`") == null);
    try std.testing.expectEqualStrings("    ", buf.items);
}

// =============================================================================
// AC5 — Clean title unchanged: ordinary text passes through unmodified
// =============================================================================

test "AC5: clean alphanumeric title is unchanged" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("Fix login bug", 100, &buf);

    try std.testing.expectEqualStrings("Fix login bug", buf.items);
}

test "AC5: title with hyphens, colons, and parentheses is unchanged" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    const title = "feat(auth): add OAuth2 support (RFC-6749)";
    try sanitizePrTitle(title, 100, &buf);

    try std.testing.expectEqualStrings(title, buf.items);
}

test "AC5: title with spaces and digits is unchanged" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    const title = "Task 42 update config v2";
    try sanitizePrTitle(title, 100, &buf);

    try std.testing.expectEqualStrings(title, buf.items);
}

// =============================================================================
// AC6 — Truncation enforced: output is at most max_len bytes
// =============================================================================

test "AC6: title longer than max_len is truncated to max_len bytes" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    // 110 'a' characters — should be truncated to 100
    const long_title = "aaaaaaaaaa" ** 11; // 110 bytes
    try sanitizePrTitle(long_title, 100, &buf);

    try std.testing.expectEqual(@as(usize, 100), buf.items.len);
    // Content is all 'a', so equality check on first 100 chars
    try std.testing.expectEqualStrings(long_title[0..100], buf.items);
}

test "AC6: title exactly max_len bytes is not truncated" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    const exact_title = "a" ** 100;
    try sanitizePrTitle(exact_title, 100, &buf);

    try std.testing.expectEqual(@as(usize, 100), buf.items.len);
    try std.testing.expectEqualStrings(exact_title, buf.items);
}

test "AC6: title shorter than max_len is not padded" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    const short_title = "Short";
    try sanitizePrTitle(short_title, 100, &buf);

    try std.testing.expectEqual(@as(usize, 5), buf.items.len);
    try std.testing.expectEqualStrings(short_title, buf.items);
}

test "AC6: max_len=0 produces empty output regardless of input" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("anything", 0, &buf);

    try std.testing.expectEqual(@as(usize, 0), buf.items.len);
}

// =============================================================================
// AC7 — Newline at boundary: when the 100th byte is '\n' or '\r', it becomes
// a space (truncation and filtering are both applied to boundary byte)
// =============================================================================

test "AC7: LF at exactly the 100th byte position is replaced with a space" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    // 99 'x' bytes followed by '\n' followed by more text (>100 total)
    var title_buf: [110]u8 = undefined;
    @memset(title_buf[0..99], 'x');
    title_buf[99] = '\n';
    @memset(title_buf[100..110], 'y');

    try sanitizePrTitle(title_buf[0..110], 100, &buf);

    try std.testing.expectEqual(@as(usize, 100), buf.items.len);
    // The 100th byte (index 99) must not be '\n'
    try std.testing.expect(buf.items[99] != '\n');
    try std.testing.expectEqual(@as(u8, ' '), buf.items[99]);
    // No '\n' anywhere in the result
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\n") == null);
}

test "AC7: CR at exactly the 100th byte position is replaced with a space" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    // 99 'x' bytes followed by '\r' followed by more text
    var title_buf: [110]u8 = undefined;
    @memset(title_buf[0..99], 'x');
    title_buf[99] = '\r';
    @memset(title_buf[100..110], 'y');

    try sanitizePrTitle(title_buf[0..110], 100, &buf);

    try std.testing.expectEqual(@as(usize, 100), buf.items.len);
    try std.testing.expect(buf.items[99] != '\r');
    try std.testing.expectEqual(@as(u8, ' '), buf.items[99]);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\r") == null);
}

// =============================================================================
// Edge Case — empty title produces empty output
// =============================================================================

test "Edge: empty input produces empty output" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("", 100, &buf);

    try std.testing.expectEqual(@as(usize, 0), buf.items.len);
}

// =============================================================================
// Edge Case — title consisting entirely of newlines becomes all spaces
// =============================================================================

test "Edge: title of only LFs becomes all spaces" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("\n\n\n", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\n") == null);
    try std.testing.expectEqualStrings("   ", buf.items);
}

test "Edge: title of only CRs becomes all spaces" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("\r\r\r", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\r") == null);
    try std.testing.expectEqualStrings("   ", buf.items);
}

test "Edge: title of only CRLF pairs becomes all spaces" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("\r\n\r\n", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\r") == null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\n") == null);
    try std.testing.expectEqualStrings("    ", buf.items);
}

// =============================================================================
// Edge Case — mixed content: surrounding text is preserved exactly
// =============================================================================

test "Edge: mixed content with LF preserves surrounding text" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    // "Fix bug\nin config\r\nhandler" → "Fix bug in config  handler"
    try sanitizePrTitle("Fix bug\nin config\r\nhandler", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\n") == null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\r") == null);
    try std.testing.expectEqualStrings("Fix bug in config  handler", buf.items);
}

test "Edge: all six dangerous characters mixed with normal text" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("a\"b\\c$d`e\nf\rg", 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\"") == null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\\") == null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "$") == null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "`") == null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\n") == null);
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\r") == null);
    try std.testing.expectEqualStrings("a b c d e f g", buf.items);
}

// =============================================================================
// Edge Case — multi-byte UTF-8 sequences pass through unmodified
// No UTF-8 continuation byte has value 0x0A ('\n') or 0x0D ('\r'), so
// multi-byte sequences must not be mangled.
// =============================================================================

test "Edge: UTF-8 multi-byte sequences are preserved intact" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    // "café" — 'é' is U+00E9, encoded as 0xC3 0xA9 in UTF-8
    const title = "caf\xc3\xa9";
    try sanitizePrTitle(title, 100, &buf);

    try std.testing.expectEqualStrings(title, buf.items);
}

test "Edge: UTF-8 title with a newline: newline replaced, multi-byte preserved" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    // "über\nnacht" — 'ü' is U+00FC → 0xC3 0xBC
    const title = "\xc3\xbcber\nnacht";
    try sanitizePrTitle(title, 100, &buf);

    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\n") == null);
    // The two UTF-8 bytes for 'ü' must still be present
    try std.testing.expect(std.mem.indexOf(u8, buf.items, "\xc3\xbc") != null);
    try std.testing.expectEqualStrings("\xc3\xbcber nacht", buf.items);
}

// =============================================================================
// Edge Case — output buffer is appended to (not reset): repeated calls
// accumulate output, consistent with the ArrayList contract.
// =============================================================================

test "Edge: successive calls append to the output buffer" {
    var buf = std.ArrayList(u8).init(std.testing.allocator);
    defer buf.deinit();

    try sanitizePrTitle("hello", 100, &buf);
    try sanitizePrTitle(" world", 100, &buf);

    try std.testing.expectEqualStrings("hello world", buf.items);
}
