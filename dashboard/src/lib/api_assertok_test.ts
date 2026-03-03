/**
 * Tests for assertOk() helper extraction in api.ts.
 *
 * Static-analysis tests: read api.ts source and verify the helper exists
 * and the repetitive inline pattern has been eliminated.
 */

import { describe, test, expect, beforeAll } from "bun:test";
import { readFileSync } from "fs";
import { join } from "path";

const SRC_PATH = join(import.meta.dir, "api.ts");
let src: string;

beforeAll(() => {
  src = readFileSync(SRC_PATH, "utf-8");
});

// ---------------------------------------------------------------------------
// AC1 — assertOk is defined
// ---------------------------------------------------------------------------

describe("AC1: assertOk function is defined in api.ts", () => {
  test("assertOk function declaration exists", () => {
    expect(/function assertOk\s*\(/.test(src)).toBe(true);
  });

  test("assertOk throws an Error using the response status", () => {
    // Extract the body of assertOk — handle return type annotation between ) and {
    const match = src.match(/function assertOk\s*\([^)]*\)[^{]*\{[^}]*\}/);
    expect(match).not.toBeNull();
    const body = match![0];
    expect(body).toContain("res.ok");
    expect(body).toContain("throw new Error");
    expect(body).toContain("res.status");
  });
});

// ---------------------------------------------------------------------------
// AC2 — old inline pattern is eliminated
// ---------------------------------------------------------------------------

describe("AC2: repetitive if-throw pattern is replaced", () => {
  test("no more than 2 occurrences of bare if(!res.ok) throw remain", () => {
    const matches = src.match(/if \(!res\.ok\) throw new Error/g) ?? [];
    // Allow 2: one inside assertOk itself, one in fetchProjectFileContent (custom message)
    expect(matches.length).toBeLessThanOrEqual(2);
  });
});

// ---------------------------------------------------------------------------
// AC3 — assertOk is used extensively
// ---------------------------------------------------------------------------

describe("AC3: assertOk(res) is called throughout the file", () => {
  test("assertOk(res) appears at least 30 times", () => {
    const matches = src.match(/assertOk\s*\(\s*res\s*\)/g) ?? [];
    expect(matches.length).toBeGreaterThanOrEqual(30);
  });
});

// ---------------------------------------------------------------------------
// AC4 — fetchJson uses assertOk
// ---------------------------------------------------------------------------

describe("AC4: fetchJson uses assertOk", () => {
  test("assertOk is called inside fetchJson", () => {
    const marker = "async function fetchJson";
    const start = src.indexOf(marker);
    expect(start).toBeGreaterThan(-1);

    // Find closing brace of fetchJson
    let depth = 0;
    let opened = false;
    let i = start;
    while (i < src.length) {
      if (src[i] === "{") { depth++; opened = true; }
      else if (src[i] === "}" && opened) {
        depth--;
        if (depth === 0) break;
      }
      i++;
    }
    const body = src.slice(start, i + 1);
    expect(body).toContain("assertOk(res)");
  });
});
