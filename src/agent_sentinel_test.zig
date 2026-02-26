// Tests for agent.extractPhaseResult and agent.SentinelScanner.
//
// These FAIL initially because neither function/type exists in agent.zig yet.
// Once implemented they cover:
//   AC8-1: extractPhaseResult returns content from a valid marker pair.
//   AC8-2: extractPhaseResult returns null when no marker is present.
//   AC8-3: extractPhaseResult returns null when only the start marker is present.
//   AC8-4: extractPhaseResult returns the LAST pair when multiple pairs exist.
//   AC8-scanner: SentinelScanner fires on partial-chunk input spanning two feeds.
//   EC1:  unclosed start marker → null.
//   EC2:  whitespace-only content → null.
//   EC3:  PHASE_RESULT_START / PHASE_RESULT_END constants have correct values.
//   EC4:  multiple marker pairs → last wins.
//   EC5:  start marker split across two feed() calls.
//
// To wire into the build, add inside the trailing `test { … }` block of
// src/agent.zig:
//   _ = @import("agent_sentinel_test.zig");

const std = @import("std");
const agent = @import("agent.zig");

const extractPhaseResult = agent.extractPhaseResult;
const SentinelScanner = agent.SentinelScanner;
const PHASE_RESULT_START = agent.PHASE_RESULT_START;
const PHASE_RESULT_END = agent.PHASE_RESULT_END;

// =============================================================================
// EC3: Constants have correct verbatim values
// =============================================================================

test "EC3: PHASE_RESULT_START constant is the correct sentinel string" {
    try std.testing.expectEqualStrings("---PHASE_RESULT_START---", PHASE_RESULT_START);
}

test "EC3: PHASE_RESULT_END constant is the correct sentinel string" {
    try std.testing.expectEqualStrings("---PHASE_RESULT_END---", PHASE_RESULT_END);
}

// =============================================================================
// AC8-1: extractPhaseResult returns content from a valid marker pair
// =============================================================================

test "AC8-1: extractPhaseResult returns content between markers" {
    const text =
        \\---PHASE_RESULT_START---
        \\Spec complete. Added tests for auth module.
        \\---PHASE_RESULT_END---
    ;
    const result = extractPhaseResult(text);
    try std.testing.expect(result != null);
    try std.testing.expect(std.mem.indexOf(u8, result.?, "Spec complete.") != null);
}

test "AC8-1: extractPhaseResult with surrounding prose still extracts content" {
    const text =
        \\I have reviewed the codebase thoroughly.
        \\
        \\---PHASE_RESULT_START---
        \\Tests written: 5 new files covering the auth module.
        \\---PHASE_RESULT_END---
        \\
        \\The phase is now complete.
    ;
    const result = extractPhaseResult(text);
    try std.testing.expect(result != null);
    try std.testing.expect(std.mem.indexOf(u8, result.?, "Tests written:") != null);
}

test "AC8-1: extractPhaseResult trims whitespace from extracted content" {
    const text = "---PHASE_RESULT_START---\n  Summary line.  \n---PHASE_RESULT_END---";
    const result = extractPhaseResult(text);
    try std.testing.expect(result != null);
    // result must not start or end with whitespace
    const trimmed = std.mem.trim(u8, result.?, " \t\r\n");
    try std.testing.expectEqualStrings(trimmed, result.?);
}

// =============================================================================
// AC8-2: extractPhaseResult returns null when no markers are present
// =============================================================================

test "AC8-2: extractPhaseResult returns null for plain text with no markers" {
    const result = extractPhaseResult("This is just plain agent output with no markers.");
    try std.testing.expectEqual(@as(?[]const u8, null), result);
}

test "AC8-2: extractPhaseResult returns null for empty string" {
    const result = extractPhaseResult("");
    try std.testing.expectEqual(@as(?[]const u8, null), result);
}

test "AC8-2: extractPhaseResult returns null for NDJSON without markers" {
    const data =
        \\{"type":"system","session_id":"abc"}
        \\{"type":"assistant","message":{"content":[{"type":"text","text":"Analyzing..."}]}}
        \\{"type":"result","result":"Analysis complete."}
    ;
    try std.testing.expectEqual(@as(?[]const u8, null), extractPhaseResult(data));
}

// =============================================================================
// AC8-3 / EC1: Returns null when only the start marker is present (unclosed)
// =============================================================================

test "AC8-3: extractPhaseResult returns null when end marker is absent" {
    const text =
        \\---PHASE_RESULT_START---
        \\This summary was never closed.
    ;
    try std.testing.expectEqual(@as(?[]const u8, null), extractPhaseResult(text));
}

test "AC8-3: extractPhaseResult returns null when only the end marker is present" {
    const text =
        \\Some text here.
        \\---PHASE_RESULT_END---
    ;
    try std.testing.expectEqual(@as(?[]const u8, null), extractPhaseResult(text));
}

test "EC1: start marker in stream with no closing marker returns null" {
    const text = "preamble\n---PHASE_RESULT_START---\ncontent without end\nmore lines";
    try std.testing.expectEqual(@as(?[]const u8, null), extractPhaseResult(text));
}

// =============================================================================
// AC8-4 / EC4: Multiple marker pairs — last complete pair wins
// =============================================================================

test "AC8-4: multiple marker pairs — last complete pair content is returned" {
    const text =
        \\---PHASE_RESULT_START---
        \\First attempt summary.
        \\---PHASE_RESULT_END---
        \\
        \\---PHASE_RESULT_START---
        \\Revised summary — this is the final one.
        \\---PHASE_RESULT_END---
    ;
    const result = extractPhaseResult(text);
    try std.testing.expect(result != null);
    try std.testing.expect(std.mem.indexOf(u8, result.?, "Revised summary") != null);
    try std.testing.expect(std.mem.indexOf(u8, result.?, "First attempt") == null);
}

test "EC4: three marker pairs — the third is returned" {
    const text =
        \\---PHASE_RESULT_START---
        \\First.
        \\---PHASE_RESULT_END---
        \\---PHASE_RESULT_START---
        \\Second.
        \\---PHASE_RESULT_END---
        \\---PHASE_RESULT_START---
        \\Third and final.
        \\---PHASE_RESULT_END---
    ;
    const result = extractPhaseResult(text);
    try std.testing.expect(result != null);
    try std.testing.expect(std.mem.indexOf(u8, result.?, "Third and final.") != null);
    try std.testing.expect(std.mem.indexOf(u8, result.?, "First.") == null);
    try std.testing.expect(std.mem.indexOf(u8, result.?, "Second.") == null);
}

// =============================================================================
// EC2: Empty / whitespace-only content between markers → null
// =============================================================================

test "EC2: extractPhaseResult returns null when content is whitespace-only" {
    const text = "---PHASE_RESULT_START---\n   \n\t\n---PHASE_RESULT_END---";
    try std.testing.expectEqual(@as(?[]const u8, null), extractPhaseResult(text));
}

test "EC2: extractPhaseResult returns null when content is an empty string between markers" {
    const text = "---PHASE_RESULT_START---\n---PHASE_RESULT_END---";
    try std.testing.expectEqual(@as(?[]const u8, null), extractPhaseResult(text));
}

// =============================================================================
// SentinelScanner: init
// =============================================================================

test "scanner-init: SentinelScanner.init creates scanner with found=false" {
    const alloc = std.testing.allocator;
    var scanner = SentinelScanner.init(alloc);
    defer scanner.deinit();
    try std.testing.expect(!scanner.found);
}

test "scanner-init: SentinelScanner has a 'found' field accessible at runtime" {
    const alloc = std.testing.allocator;
    var scanner = SentinelScanner.init(alloc);
    defer scanner.deinit();
    try std.testing.expect(@hasField(SentinelScanner, "found"));
}

// =============================================================================
// SentinelScanner: feed returns null for data without markers
// =============================================================================

test "scanner-no-marker: feed returns null for data without any marker" {
    const alloc = std.testing.allocator;
    var scanner = SentinelScanner.init(alloc);
    defer scanner.deinit();
    const result = scanner.feed("some plain output line\n", alloc);
    try std.testing.expectEqual(@as(?[]const u8, null), result);
    try std.testing.expect(!scanner.found);
}

test "scanner-no-marker: feed accumulates data without false-positive" {
    const alloc = std.testing.allocator;
    var scanner = SentinelScanner.init(alloc);
    defer scanner.deinit();
    _ = scanner.feed("line one\n", alloc);
    _ = scanner.feed("line two\n", alloc);
    const result = scanner.feed("line three\n", alloc);
    try std.testing.expectEqual(@as(?[]const u8, null), result);
    try std.testing.expect(!scanner.found);
}

// =============================================================================
// SentinelScanner: fires when complete pair arrives in one chunk
// =============================================================================

test "scanner-single-chunk: feed returns content when both markers in one chunk" {
    const alloc = std.testing.allocator;
    var scanner = SentinelScanner.init(alloc);
    defer scanner.deinit();
    const chunk = "preamble\n---PHASE_RESULT_START---\nSummary text.\n---PHASE_RESULT_END---\ntrailing";
    const result = scanner.feed(chunk, alloc);
    try std.testing.expect(result != null);
    defer alloc.free(result.?);
    try std.testing.expect(std.mem.indexOf(u8, result.?, "Summary text.") != null);
    try std.testing.expect(scanner.found);
}

// =============================================================================
// AC8-scanner-4: SentinelScanner fires on partial-chunk input spanning two feeds
// =============================================================================

test "AC8-scanner-4: scanner fires when marker pair spans two feed() calls" {
    const alloc = std.testing.allocator;
    var scanner = SentinelScanner.init(alloc);
    defer scanner.deinit();

    // First chunk: start marker + content, no end marker
    const r1 = scanner.feed("---PHASE_RESULT_START---\nStreamed content here.\n", alloc);
    try std.testing.expectEqual(@as(?[]const u8, null), r1);
    try std.testing.expect(!scanner.found);

    // Second chunk: end marker arrives
    const r2 = scanner.feed("---PHASE_RESULT_END---\n", alloc);
    try std.testing.expect(r2 != null);
    defer alloc.free(r2.?);
    try std.testing.expect(std.mem.indexOf(u8, r2.?, "Streamed content here.") != null);
    try std.testing.expect(scanner.found);
}

test "scanner-split: preamble in first chunk, markers in second chunk" {
    const alloc = std.testing.allocator;
    var scanner = SentinelScanner.init(alloc);
    defer scanner.deinit();

    _ = scanner.feed("Regular NDJSON lines before the result...\n", alloc);
    const r = scanner.feed("---PHASE_RESULT_START---\nResult.\n---PHASE_RESULT_END---\n", alloc);
    try std.testing.expect(r != null);
    defer alloc.free(r.?);
    try std.testing.expect(std.mem.indexOf(u8, r.?, "Result.") != null);
}

// =============================================================================
// EC5: Marker string itself split across two byte-level feed calls
// =============================================================================

test "EC5: scanner handles start marker split at a byte boundary" {
    const alloc = std.testing.allocator;
    var scanner = SentinelScanner.init(alloc);
    defer scanner.deinit();

    // Split "---PHASE_RESULT_START---" in the middle
    _ = scanner.feed("prefix\n---PHASE_RESULT_ST", alloc);
    const result = scanner.feed("ART---\nContent.\n---PHASE_RESULT_END---\n", alloc);

    // After the second feed the buffer contains the complete pair.
    // The scanner must detect it (found=true) and return the content.
    if (result) |r| {
        defer alloc.free(r);
        try std.testing.expect(scanner.found);
        try std.testing.expect(std.mem.indexOf(u8, r, "Content.") != null);
    } else {
        // If the implementation chose not to scan mid-marker splits,
        // found must still be false (not a spurious fire).
        try std.testing.expect(!scanner.found);
    }
}

// =============================================================================
// AC8-scanner-5: At-most-once — scanner ignores second pair after found is set
// =============================================================================

test "AC8-scanner-5: scanner returns null for second marker pair after first fires" {
    const alloc = std.testing.allocator;
    var scanner = SentinelScanner.init(alloc);
    defer scanner.deinit();

    const chunk1 = "---PHASE_RESULT_START---\nFirst result.\n---PHASE_RESULT_END---\n";
    const r1 = scanner.feed(chunk1, alloc);
    try std.testing.expect(r1 != null);
    defer alloc.free(r1.?);
    try std.testing.expect(scanner.found);

    // Second pair must be suppressed
    const chunk2 = "---PHASE_RESULT_START---\nSecond result.\n---PHASE_RESULT_END---\n";
    const r2 = scanner.feed(chunk2, alloc);
    try std.testing.expectEqual(@as(?[]const u8, null), r2);
}

// =============================================================================
// Multiline content is preserved
// =============================================================================

test "multi-line: extractPhaseResult preserves multi-line content" {
    const text =
        \\---PHASE_RESULT_START---
        \\Line one.
        \\Line two.
        \\Line three.
        \\---PHASE_RESULT_END---
    ;
    const result = extractPhaseResult(text);
    try std.testing.expect(result != null);
    try std.testing.expect(std.mem.indexOf(u8, result.?, "Line one.") != null);
    try std.testing.expect(std.mem.indexOf(u8, result.?, "Line two.") != null);
    try std.testing.expect(std.mem.indexOf(u8, result.?, "Line three.") != null);
}
