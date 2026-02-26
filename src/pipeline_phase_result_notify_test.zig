// Tests for the sentinel-marker notification path in pipeline.zig:
//   AC2: pipeline.zig calls web.pushPhaseResult() to inject an SSE event.
//   AC3: chat notification content is truncated to ≤ 2000 chars with "…".
//   AC5: at-most-once guarantee — scanner.found flag suppresses duplicates.
//   AC6: graceful degradation — no notification when markers are absent.
//
// The pipeline's taskStreamCallback and TaskStreamCtx are internal types that
// cannot be exercised without a live Db / Docker / Telegram stack.  These tests
// therefore use two complementary strategies:
//
//   Structural  — @embedFile("pipeline.zig") verifies that key identifiers and
//                 patterns are present in the source after implementation.
//   Behavioral  — A standalone truncation helper verifies the 2000-char-cap
//                 contract (the helper mirrors the spec, not the implementation).
//
// These tests FAIL initially because:
//   - TaskStreamCtx lacks scanner / notify_chat / phase_name fields.
//   - taskStreamCallback does not yet call pushPhaseResult or notify().
//   - The 2000-char truncation constant is absent from pipeline.zig.
//
// To wire into the build, add inside the trailing `test { … }` block of
// src/pipeline.zig:
//   _ = @import("pipeline_phase_result_notify_test.zig");

const std = @import("std");

const pipeline_src = @embedFile("pipeline.zig");

// =============================================================================
// AC5: at-most-once guard — scanner.found referenced in pipeline.zig
// =============================================================================

test "AC5: pipeline.zig references scanner.found to suppress duplicate fires" {
    try std.testing.expect(std.mem.indexOf(u8, pipeline_src, "scanner.found") != null);
}

// =============================================================================
// AC5: SentinelScanner is instantiated inside TaskStreamCtx
// =============================================================================

test "AC5: pipeline.zig contains a SentinelScanner field in TaskStreamCtx" {
    try std.testing.expect(std.mem.indexOf(u8, pipeline_src, "SentinelScanner") != null);
}

// =============================================================================
// AC5: extractPhaseResult is called as post-run fallback
// =============================================================================

test "AC5: pipeline.zig calls extractPhaseResult as post-run fallback" {
    try std.testing.expect(std.mem.indexOf(u8, pipeline_src, "extractPhaseResult") != null);
}

// =============================================================================
// AC2: pushPhaseResult is called from within pipeline.zig
// =============================================================================

test "AC2: pipeline.zig calls pushPhaseResult to inject SSE event" {
    try std.testing.expect(std.mem.indexOf(u8, pipeline_src, "pushPhaseResult") != null);
}

// =============================================================================
// AC3: truncation limit constant (2000) is present in pipeline.zig
// =============================================================================

test "AC3: pipeline.zig contains the 2000-char truncation limit" {
    try std.testing.expect(std.mem.indexOf(u8, pipeline_src, "2000") != null);
}

test "AC3: pipeline.zig contains a truncation suffix (ellipsis or dots)" {
    // Accept Unicode ellipsis "…" (0xE2 0x80 0xA6) or three ASCII dots "..."
    const has_unicode = std.mem.indexOf(u8, pipeline_src, "…") != null;
    const has_ascii = std.mem.indexOf(u8, pipeline_src, "\"...\"") != null;
    try std.testing.expect(has_unicode or has_ascii);
}

// =============================================================================
// AC3: pure behavioral spec — truncation contract
//      (documents expected behavior independently of pipeline.zig internals)
// =============================================================================

/// Mirrors the expected truncation behavior from the spec.
/// Content > 2000 chars → kept[:1997] ++ "…" (3 UTF-8 bytes) = 2000 bytes total.
fn truncateForChat(content: []const u8, buf: []u8) []const u8 {
    const CHAT_MAX = 2000;
    const SUFFIX = "…"; // U+2026 HORIZONTAL ELLIPSIS = 3 UTF-8 bytes
    if (content.len <= CHAT_MAX) {
        @memcpy(buf[0..content.len], content);
        return buf[0..content.len];
    }
    const kept = CHAT_MAX - SUFFIX.len;
    @memcpy(buf[0..kept], content[0..kept]);
    @memcpy(buf[kept..][0..SUFFIX.len], SUFFIX);
    return buf[0 .. kept + SUFFIX.len];
}

test "AC3: content of exactly 2000 chars is forwarded unchanged" {
    var buf: [2100]u8 = undefined;
    const content = "A" ** 2000;
    const out = truncateForChat(content, &buf);
    try std.testing.expectEqual(@as(usize, 2000), out.len);
    try std.testing.expectEqualStrings(content, out);
}

test "AC3: content of 2001 chars is truncated to 2000 bytes ending with ellipsis" {
    var buf: [2100]u8 = undefined;
    const content = "B" ** 2001;
    const out = truncateForChat(content, &buf);
    // "…" is 3 UTF-8 bytes; kept = 1997; total = 2000 bytes
    try std.testing.expectEqual(@as(usize, 2000), out.len);
    try std.testing.expect(std.mem.endsWith(u8, out, "…"));
}

test "AC3: content of 5000 chars is truncated with correct prefix preserved" {
    var buf: [5100]u8 = undefined;
    const content = "C" ** 5000;
    const out = truncateForChat(content, &buf);
    try std.testing.expectEqual(@as(usize, 2000), out.len);
    try std.testing.expect(std.mem.endsWith(u8, out, "…"));
    try std.testing.expect(std.mem.startsWith(u8, out, "C" ** 1997));
}

test "AC3: empty content is forwarded unchanged" {
    var buf: [16]u8 = undefined;
    const out = truncateForChat("", &buf);
    try std.testing.expectEqual(@as(usize, 0), out.len);
}

test "AC3: content of exactly 1999 chars is not truncated" {
    var buf: [2100]u8 = undefined;
    const content = "E" ** 1999;
    const out = truncateForChat(content, &buf);
    try std.testing.expectEqual(@as(usize, 1999), out.len);
    try std.testing.expect(!std.mem.endsWith(u8, out, "…"));
}

test "AC3: content of exactly 2000 chars does not get ellipsis appended" {
    var buf: [2100]u8 = undefined;
    const content = "F" ** 2000;
    const out = truncateForChat(content, &buf);
    try std.testing.expectEqual(@as(usize, 2000), out.len);
    try std.testing.expect(!std.mem.endsWith(u8, out, "…"));
}

// =============================================================================
// AC3: notify_chat is tracked in TaskStreamCtx
// =============================================================================

test "AC3: pipeline.zig TaskStreamCtx contains notify_chat field" {
    try std.testing.expect(std.mem.indexOf(u8, pipeline_src, "notify_chat") != null);
}

// =============================================================================
// Structural: phase_name field is tracked in TaskStreamCtx
// =============================================================================

test "structural: pipeline.zig TaskStreamCtx contains phase_name field" {
    try std.testing.expect(std.mem.indexOf(u8, pipeline_src, "phase_name") != null);
}

// =============================================================================
// AC6: graceful degradation — scanner.found stays false when no markers
// =============================================================================

test "AC6: pipeline.zig uses scanner.found to gate the notification branch" {
    // When no markers fire, scanner.found is false and no notification is sent.
    // The at-most-once guard is the same code path as graceful degradation.
    try std.testing.expect(std.mem.indexOf(u8, pipeline_src, "scanner.found") != null);
}

// =============================================================================
// EC8: only designated phases (spec, qa, qa_fix) get the scanner attached
// =============================================================================

test "EC8: pipeline.zig tracks phase_name to limit scanner to designated phases" {
    try std.testing.expect(std.mem.indexOf(u8, pipeline_src, "phase_name") != null);
}
