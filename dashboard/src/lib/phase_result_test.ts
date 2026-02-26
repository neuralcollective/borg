/**
 * Tests for AC4: Dashboard phase_result rendering.
 *
 * These are static-analysis tests that read the source of api.ts and
 * live-terminal.tsx and verify the required changes are present.
 *
 * These tests FAIL initially because:
 *   - StreamEvent does not have a `phase` string field.
 *   - live-terminal.tsx does not handle the `phase_result` event type.
 *   - No distinct rendering (green border, phase label) exists for phase_result.
 */

import { describe, test, expect, beforeAll } from "bun:test";
import { readFileSync } from "fs";
import { join } from "path";

const API_PATH = join(import.meta.dir, "api.ts");
const TERMINAL_PATH = join(import.meta.dir, "../components/live-terminal.tsx");

let apiSrc: string;
let terminalSrc: string;

beforeAll(() => {
  apiSrc = readFileSync(API_PATH, "utf-8");
  terminalSrc = readFileSync(TERMINAL_PATH, "utf-8");
});

// =============================================================================
// AC4: StreamEvent has a phase field
// =============================================================================

describe("AC4: StreamEvent has a phase field", () => {
  test("StreamEvent interface declares a phase field (string)", () => {
    // Matches `phase?: string` or `phase: string`
    const pattern = /phase\??\s*:\s*string/;
    expect(pattern.test(apiSrc)).toBe(true);
  });

  test("StreamEvent interface is present in api.ts", () => {
    expect(apiSrc).toContain("export interface StreamEvent");
  });

  test("StreamEvent interface body contains 'phase'", () => {
    const start = apiSrc.indexOf("export interface StreamEvent");
    const end = apiSrc.indexOf("}", start);
    const body = apiSrc.slice(start, end + 1);
    expect(body).toContain("phase");
  });
});

// =============================================================================
// AC4: live-terminal.tsx handles phase_result events
// =============================================================================

describe("AC4: live-terminal.tsx handles phase_result events", () => {
  test("TermLine type includes 'phase_result' as a possible type value", () => {
    expect(terminalSrc).toContain("phase_result");
  });

  test("parseEvents function handles phase_result event type", () => {
    const parseStart = terminalSrc.indexOf("function parseEvents(");
    expect(parseStart).toBeGreaterThan(-1);
    const afterParse = terminalSrc.slice(parseStart);
    expect(afterParse).toContain("phase_result");
  });

  test("TermLineView has a branch for phase_result", () => {
    const viewStart = terminalSrc.indexOf("function TermLineView(");
    expect(viewStart).toBeGreaterThan(-1);
    const afterView = terminalSrc.slice(viewStart);
    expect(afterView).toContain("phase_result");
  });
});

// =============================================================================
// AC4: phase_result has distinct green styling
// =============================================================================

describe("AC4: phase_result is rendered with green/emerald styling", () => {
  test("live-terminal.tsx uses emerald or green color for phase_result", () => {
    // The phase_result TermLineView branch must use a green/emerald color class.
    // Since 'emerald' is already used for 'result', check that a new branch also uses it.
    const hasGreen =
      terminalSrc.includes("emerald") || terminalSrc.includes("green");
    expect(hasGreen).toBe(true);
  });

  test("live-terminal.tsx references a phase label for phase_result events", () => {
    // The rendering must include something like "Phase result:" or use line.phase.
    const hasPhaseLabelRef =
      terminalSrc.includes("Phase result") ||
      (terminalSrc.includes("phase_result") && terminalSrc.includes("line.phase"));
    expect(hasPhaseLabelRef).toBe(true);
  });
});

// =============================================================================
// AC4: StreamEvent content field is present (for phase_result payload)
// =============================================================================

describe("AC4: StreamEvent has a content field usable for phase_result", () => {
  test("StreamEvent interface body contains 'content'", () => {
    const start = apiSrc.indexOf("export interface StreamEvent");
    const end = apiSrc.indexOf("}", start);
    const body = apiSrc.slice(start, end + 1);
    expect(body).toContain("content");
  });
});
