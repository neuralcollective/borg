// Tests for Task #62: backoffSeconds formula (exponential backoff).
//
// Covers acceptance criterion AC1 from spec.md:
//   backoffSeconds(0)  =   60  (2^0 = 1 minute)
//   backoffSeconds(1)  =  120  (2^1 = 2 minutes)
//   backoffSeconds(2)  =  240  (2^2 = 4 minutes)
//   backoffSeconds(3)  =  480  (2^3 = 8 minutes)
//   backoffSeconds(4)  =  960  (2^4 = 16 minutes)
//   backoffSeconds(5)  = 1920  (2^5 = 32 minutes)
//   backoffSeconds(6)  = 3600  (2^6 = 64 > 60 → capped to 60 minutes)
//   backoffSeconds(10) = 3600  (still capped)
//
// These tests FAIL to compile until pipeline.zig declares:
//   pub fn backoffSeconds(attempt: i64) i64
// as a pub method on the Pipeline struct (no self receiver).
//
// To include in the build, add to the test block in src/pipeline.zig:
//   _ = @import("pipeline_backoff_test.zig");

const std = @import("std");
const pipeline = @import("pipeline.zig");

const backoffSeconds = pipeline.Pipeline.backoffSeconds;

// =============================================================================
// AC1 — Exact values for each meaningful attempt count
// =============================================================================

test "AC1: backoffSeconds(0) = 60 (2^0 = 1 minute)" {
    try std.testing.expectEqual(@as(i64, 60), backoffSeconds(0));
}

test "AC1: backoffSeconds(1) = 120 (2^1 = 2 minutes)" {
    try std.testing.expectEqual(@as(i64, 120), backoffSeconds(1));
}

test "AC1: backoffSeconds(2) = 240 (2^2 = 4 minutes)" {
    try std.testing.expectEqual(@as(i64, 240), backoffSeconds(2));
}

test "AC1: backoffSeconds(3) = 480 (2^3 = 8 minutes)" {
    try std.testing.expectEqual(@as(i64, 480), backoffSeconds(3));
}

test "AC1: backoffSeconds(4) = 960 (2^4 = 16 minutes)" {
    try std.testing.expectEqual(@as(i64, 960), backoffSeconds(4));
}

test "AC1: backoffSeconds(5) = 1920 (2^5 = 32 minutes)" {
    try std.testing.expectEqual(@as(i64, 1920), backoffSeconds(5));
}

test "AC1: backoffSeconds(6) = 3600 (2^6=64 exceeds 60-minute cap)" {
    try std.testing.expectEqual(@as(i64, 3600), backoffSeconds(6));
}

test "AC1: backoffSeconds(7) = 3600 (still at 60-minute cap)" {
    try std.testing.expectEqual(@as(i64, 3600), backoffSeconds(7));
}

test "AC1: backoffSeconds(10) = 3600 (capped per spec)" {
    try std.testing.expectEqual(@as(i64, 3600), backoffSeconds(10));
}

test "AC1: backoffSeconds(100) = 3600 (large attempt, still capped)" {
    try std.testing.expectEqual(@as(i64, 3600), backoffSeconds(100));
}

// =============================================================================
// Edge — first failure always waits (never zero)
// =============================================================================

test "Edge: backoffSeconds(0) is positive — first failure always delays" {
    try std.testing.expect(backoffSeconds(0) > 0);
}

// =============================================================================
// Edge — strictly increasing up to the cap
// =============================================================================

test "Edge: backoff doubles for each attempt from 0 to 5" {
    // Verify the doubling pattern holds for all pre-cap values
    const expected = [_]i64{ 60, 120, 240, 480, 960, 1920 };
    for (expected, 0..) |exp, a| {
        const got = backoffSeconds(@intCast(a));
        try std.testing.expectEqual(exp, got);
    }
}

test "Edge: backoff is strictly increasing from attempt 0 to 5" {
    var prev = backoffSeconds(0);
    var a: i64 = 1;
    while (a <= 5) : (a += 1) {
        const curr = backoffSeconds(a);
        try std.testing.expect(curr > prev);
        prev = curr;
    }
}

// =============================================================================
// Edge — flat (capped) at 60 minutes from attempt 6 onwards
// =============================================================================

test "Edge: backoff is flat at 3600 from attempt 6 to 11" {
    const cap = backoffSeconds(6);
    try std.testing.expectEqual(@as(i64, 3600), cap);
    var a: i64 = 7;
    while (a <= 11) : (a += 1) {
        try std.testing.expectEqual(cap, backoffSeconds(a));
    }
}

// =============================================================================
// Edge — maximum cap is exactly 3600 seconds
// =============================================================================

test "Edge: maximum backoff across attempts 0-19 is exactly 3600 seconds" {
    var max_seen: i64 = 0;
    var a: i64 = 0;
    while (a < 20) : (a += 1) {
        const d = backoffSeconds(a);
        if (d > max_seen) max_seen = d;
    }
    try std.testing.expectEqual(@as(i64, 3600), max_seen);
}

// =============================================================================
// Edge — values are multiples of 60 (whole minutes)
// =============================================================================

test "Edge: all backoff values are multiples of 60 seconds" {
    var a: i64 = 0;
    while (a < 12) : (a += 1) {
        const d = backoffSeconds(a);
        try std.testing.expectEqual(@as(i64, 0), @rem(d, 60));
    }
}
