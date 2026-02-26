/**
 * Tests for Task #76: Scope SSE log cache invalidation to affected query keys.
 *
 * These are static-analysis tests: they read the compiled source text of
 * api.ts and verify the correct call patterns are present / absent inside
 * the useLogs function body.  They use only Bun built-ins so no extra
 * dependencies are required.
 *
 * Current state: FAILING (bare invalidateQueries() with no filter exists).
 * Expected state after fix: all tests pass.
 */

import { describe, test, expect, beforeAll } from "bun:test";
import { readFileSync } from "fs";
import { join } from "path";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const SRC_PATH = join(import.meta.dir, "api.ts");
let src: string;
let useLogsBody: string;

/**
 * Extracts the text of the first top-level function whose declaration begins
 * with `export function <name>` by tracking brace depth.
 */
function extractFunctionBody(source: string, name: string): string {
  const marker = `export function ${name}(`;
  const start = source.indexOf(marker);
  if (start === -1) throw new Error(`Function '${name}' not found in source`);

  let depth = 0;
  let opened = false;
  let i = start;
  while (i < source.length) {
    if (source[i] === "{") {
      depth++;
      opened = true;
    } else if (source[i] === "}" && opened) {
      depth--;
      if (depth === 0) {
        return source.slice(start, i + 1);
      }
    }
    i++;
  }
  throw new Error(`Could not find closing brace for '${name}'`);
}

beforeAll(() => {
  src = readFileSync(SRC_PATH, "utf-8");
  useLogsBody = extractFunctionBody(src, "useLogs");
});

// ---------------------------------------------------------------------------
// Acceptance criterion 1 — no bare invalidateQueries() in useLogs
// ---------------------------------------------------------------------------

describe("AC1: invalidateQueries is never called without a queryKey filter inside useLogs", () => {
  test("bare queryClient.invalidateQueries() call is absent", () => {
    // Matches calls with no arguments at all: invalidateQueries()
    const bareNoArgs = /queryClient\.invalidateQueries\s*\(\s*\)/;
    expect(bareNoArgs.test(useLogsBody)).toBe(false);
  });

  test("bare queryClient.invalidateQueries({}) call (empty object) is absent", () => {
    // Matches calls with an empty object: invalidateQueries({})
    const bareEmptyObj = /queryClient\.invalidateQueries\s*\(\s*\{\s*\}\s*\)/;
    expect(bareEmptyObj.test(useLogsBody)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Acceptance criterion 2 — targeted calls for ["tasks"] and ["status"]
// ---------------------------------------------------------------------------

describe('AC2: invalidateQueries called with { queryKey: ["tasks"] } and { queryKey: ["status"] }', () => {
  test('invalidateQueries is called with { queryKey: ["tasks"] }', () => {
    const pattern =
      /queryClient\.invalidateQueries\s*\(\s*\{\s*queryKey\s*:\s*\[\s*["']tasks["']\s*\]\s*\}\s*\)/;
    expect(pattern.test(useLogsBody)).toBe(true);
  });

  test('invalidateQueries is called with { queryKey: ["status"] }', () => {
    const pattern =
      /queryClient\.invalidateQueries\s*\(\s*\{\s*queryKey\s*:\s*\[\s*["']status["']\s*\]\s*\}\s*\)/;
    expect(pattern.test(useLogsBody)).toBe(true);
  });

  test("exactly 2 invalidateQueries calls exist inside useLogs", () => {
    const matches = useLogsBody.match(/queryClient\.invalidateQueries/g) ?? [];
    expect(matches.length).toBe(2);
  });
});

// ---------------------------------------------------------------------------
// Acceptance criterion 3 — queries that must NOT be invalidated
// ---------------------------------------------------------------------------

describe("AC3: invalidateQueries is NOT called for queue, proposals, or modes", () => {
  test('invalidateQueries is NOT called with queryKey ["queue"]', () => {
    const pattern =
      /queryClient\.invalidateQueries\s*\(\s*\{\s*queryKey\s*:\s*\[\s*["']queue["']\s*\]\s*\}\s*\)/;
    expect(pattern.test(useLogsBody)).toBe(false);
  });

  test('invalidateQueries is NOT called with queryKey ["proposals"]', () => {
    const pattern =
      /queryClient\.invalidateQueries\s*\(\s*\{\s*queryKey\s*:\s*\[\s*["']proposals["']\s*\]\s*\}\s*\)/;
    expect(pattern.test(useLogsBody)).toBe(false);
  });

  test('invalidateQueries is NOT called with queryKey ["modes"]', () => {
    const pattern =
      /queryClient\.invalidateQueries\s*\(\s*\{\s*queryKey\s*:\s*\[\s*["']modes["']\s*\]\s*\}\s*\)/;
    expect(pattern.test(useLogsBody)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Acceptance criterion 4 — debounce behaviour preserved
// ---------------------------------------------------------------------------

describe("AC4: debounce behaviour (at most once per 1000 ms) is preserved", () => {
  test("a setTimeout with 1000 ms delay is present inside useLogs", () => {
    // Must still have the 1-second debounce timer.
    // The callback body contains () calls so we cannot use [^)]* — instead
    // match the closing ", 1000)" that terminates the setTimeout invocation.
    const hasSetTimeout = /setTimeout\s*\(/.test(useLogsBody);
    const has1000Delay = /},\s*1000\s*\)/.test(useLogsBody);
    expect(hasSetTimeout).toBe(true);
    expect(has1000Delay).toBe(true);
  });

  test("the invalidateTimer guard (if !invalidateTimer.current) is still present", () => {
    // The guard prevents setting a second timer while one is pending
    const pattern = /if\s*\(\s*!invalidateTimer\.current\s*\)/;
    expect(pattern.test(useLogsBody)).toBe(true);
  });

  test("invalidateTimer.current is reset to null inside the setTimeout callback", () => {
    // After firing, the timer ref must be cleared so the next event can re-arm it
    const pattern = /invalidateTimer\.current\s*=\s*null/;
    expect(pattern.test(useLogsBody)).toBe(true);
  });

  test("both targeted invalidateQueries calls live inside the setTimeout callback", () => {
    // Locate the first setTimeout in useLogs and check both calls appear after it,
    // before the matching closing parenthesis of setTimeout's first argument.
    const setTimeoutIdx = useLogsBody.indexOf("setTimeout");
    expect(setTimeoutIdx).toBeGreaterThan(-1);

    // Everything from setTimeout onward (conservative: just check both patterns
    // appear in the slice that starts at the setTimeout invocation)
    const afterTimeout = useLogsBody.slice(setTimeoutIdx);

    const tasksInCb =
      /queryClient\.invalidateQueries\s*\(\s*\{\s*queryKey\s*:\s*\[\s*["']tasks["']\s*\]\s*\}\s*\)/.test(
        afterTimeout
      );
    const statusInCb =
      /queryClient\.invalidateQueries\s*\(\s*\{\s*queryKey\s*:\s*\[\s*["']status["']\s*\]\s*\}\s*\)/.test(
        afterTimeout
      );

    expect(tasksInCb).toBe(true);
    expect(statusInCb).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Acceptance criterion 5 — hook return type unchanged
// ---------------------------------------------------------------------------

describe("AC5: useLogs hook return type is unchanged", () => {
  test("useLogs returns an object containing both logs and connected", () => {
    // Matches: return { logs, connected } with optional whitespace/trailing comma
    const pattern = /return\s*\{\s*logs\s*,\s*connected\s*,?\s*\}/;
    expect(pattern.test(useLogsBody)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Edge case: ["task", id] detail queries are not invalidated from useLogs
// ---------------------------------------------------------------------------

describe("Edge case: task detail queries are not invalidated from useLogs", () => {
  test('invalidateQueries is NOT called with queryKey starting with "task" (detail variant)', () => {
    // The detail query key is ["task", id]; it must not be invalidated here
    const pattern =
      /queryClient\.invalidateQueries\s*\(\s*\{\s*queryKey\s*:\s*\[\s*["']task["']/;
    expect(pattern.test(useLogsBody)).toBe(false);
  });
});
