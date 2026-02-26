// Tests for the phase_result_instruction constant and its injection into
// agent phase configurations (spec, qa, qa_fix).
//
// These FAIL initially because:
//   - prompts.phase_result_instruction does not yet exist.
//   - The marker directive has not yet been injected into phase instructions.
//
// Once implemented they cover:
//   AC1: phase_result_instruction constant is declared in prompts.zig.
//   AC1: The constant contains both PHASE_RESULT_START and PHASE_RESULT_END marker strings.
//   AC1: spec, qa, and qa_fix phase instructions include the marker directive.
//   AC1: rebase phase instruction does NOT include the marker directive.
//
// To wire into the build, add inside the trailing `test { … }` block of
// src/prompts.zig:
//   _ = @import("prompts_phase_result_test.zig");

const std = @import("std");
const prompts = @import("prompts.zig");
const modes = @import("modes.zig");

// =============================================================================
// AC1: phase_result_instruction constant exists in prompts.zig
// =============================================================================

test "AC1: prompts.phase_result_instruction is declared" {
    // FAILS until phase_result_instruction is added to prompts.zig
    try std.testing.expect(@hasDecl(prompts, "phase_result_instruction"));
}

test "AC1: phase_result_instruction is a non-empty string" {
    try std.testing.expect(prompts.phase_result_instruction.len > 0);
}

// =============================================================================
// AC1: The constant contains both sentinel marker strings
// =============================================================================

test "AC1: phase_result_instruction contains the PHASE_RESULT_START marker" {
    try std.testing.expect(
        std.mem.indexOf(u8, prompts.phase_result_instruction, "---PHASE_RESULT_START---") != null,
    );
}

test "AC1: phase_result_instruction contains the PHASE_RESULT_END marker" {
    try std.testing.expect(
        std.mem.indexOf(u8, prompts.phase_result_instruction, "---PHASE_RESULT_END---") != null,
    );
}

// =============================================================================
// AC1: spec phase instruction includes the phase_result directive
// =============================================================================

test "AC1: spec phase instruction includes the phase_result marker directive" {
    const phase = modes.swe_mode.getPhase("spec") orelse {
        // If spec phase does not exist the pipeline is broken — fail clearly.
        try std.testing.expect(false);
        return;
    };
    // The directive may appear in either the instruction or the system_prompt.
    const in_instruction = std.mem.indexOf(u8, phase.instruction, "---PHASE_RESULT_START---") != null;
    const in_system = std.mem.indexOf(u8, phase.system_prompt, "---PHASE_RESULT_START---") != null;
    try std.testing.expect(in_instruction or in_system);
}

// =============================================================================
// AC1: qa phase instruction includes the phase_result directive
// =============================================================================

test "AC1: qa phase instruction includes the phase_result marker directive" {
    const phase = modes.swe_mode.getPhase("qa") orelse {
        try std.testing.expect(false);
        return;
    };
    const in_instruction = std.mem.indexOf(u8, phase.instruction, "---PHASE_RESULT_START---") != null;
    const in_system = std.mem.indexOf(u8, phase.system_prompt, "---PHASE_RESULT_START---") != null;
    try std.testing.expect(in_instruction or in_system);
}

// =============================================================================
// AC1: qa_fix phase instruction includes the phase_result directive
// =============================================================================

test "AC1: qa_fix phase instruction includes the phase_result marker directive" {
    const phase = modes.swe_mode.getPhase("qa_fix") orelse {
        try std.testing.expect(false);
        return;
    };
    const in_instruction = std.mem.indexOf(u8, phase.instruction, "---PHASE_RESULT_START---") != null;
    const in_system = std.mem.indexOf(u8, phase.system_prompt, "---PHASE_RESULT_START---") != null;
    try std.testing.expect(in_instruction or in_system);
}

// =============================================================================
// AC1: rebase phase does NOT include the phase_result directive
// =============================================================================

test "AC1: rebase phase instruction does NOT include the marker directive" {
    const phase = modes.swe_mode.getPhase("rebase") orelse {
        try std.testing.expect(false);
        return;
    };
    const in_instruction = std.mem.indexOf(u8, phase.instruction, "---PHASE_RESULT_START---") != null;
    const in_system = std.mem.indexOf(u8, phase.system_prompt, "---PHASE_RESULT_START---") != null;
    const in_fix = std.mem.indexOf(u8, phase.fix_instruction, "---PHASE_RESULT_START---") != null;
    try std.testing.expect(!in_instruction and !in_system and !in_fix);
}

// =============================================================================
// AC1: Structural check — prompts.zig source contains the marker strings
//      (catches accidental deletion)
// =============================================================================

test "AC1: prompts.zig source file contains PHASE_RESULT_START literal" {
    const src = @embedFile("prompts.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "PHASE_RESULT_START") != null);
}

test "AC1: prompts.zig source file contains PHASE_RESULT_END literal" {
    const src = @embedFile("prompts.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "PHASE_RESULT_END") != null);
}

// =============================================================================
// AC1: Structural check — modes.zig references the phase_result_instruction
// =============================================================================

test "AC1: modes.zig source references phase_result_instruction" {
    const src = @embedFile("modes.zig");
    try std.testing.expect(std.mem.indexOf(u8, src, "phase_result_instruction") != null);
}
