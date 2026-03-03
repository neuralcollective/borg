/**
 * Tests for apiFetchJson helper: deduplicates the apiFetch+ok-guard+json pattern
 * that previously appeared inline across 17+ functions in api.ts.
 */

import { describe, test, expect, beforeAll } from "bun:test";
import { readFileSync } from "fs";
import { join } from "path";

const SRC_PATH = join(import.meta.dir, "api.ts");
let src: string;

function extractFunctionBody(source: string, marker: string): string {
  const start = source.indexOf(marker);
  if (start === -1) throw new Error(`Marker not found: ${marker}`);

  // Pass 1: find the opening { of the function body, skipping <> in the signature
  let angleDepth = 0, parenDepth = 0, bodyStart = -1;
  for (let i = start; i < source.length; i++) {
    const ch = source[i];
    if (ch === "(" && angleDepth === 0) parenDepth++;
    else if (ch === ")" && angleDepth === 0) parenDepth--;
    else if (ch === "<" && parenDepth === 0) angleDepth++;
    else if (ch === ">" && parenDepth === 0 && angleDepth > 0) angleDepth--;
    else if (ch === "{" && parenDepth === 0 && angleDepth === 0) { bodyStart = i; break; }
  }
  if (bodyStart === -1) throw new Error(`Could not find body start for: ${marker}`);

  // Pass 2: from the body {, count braces to find matching }
  let braceDepth = 0;
  for (let i = bodyStart; i < source.length; i++) {
    if (source[i] === "{") braceDepth++;
    else if (source[i] === "}") { braceDepth--; if (braceDepth === 0) return source.slice(start, i + 1); }
  }
  throw new Error(`Could not find closing brace for marker: ${marker}`);
}

beforeAll(() => {
  src = readFileSync(SRC_PATH, "utf-8");
});

// ---------------------------------------------------------------------------
// AC1 — apiFetchJson helper exists and is well-formed
// ---------------------------------------------------------------------------

describe("AC1: apiFetchJson is defined as an async generic helper", () => {
  test("apiFetchJson<T> is declared in api.ts", () => {
    expect(/async function apiFetchJson<T>/.test(src)).toBe(true);
  });

  test("apiFetchJson body calls apiFetch", () => {
    const body = extractFunctionBody(src, "async function apiFetchJson");
    expect(/apiFetch\(/.test(body)).toBe(true);
  });

  test("apiFetchJson body checks res.ok and throws on failure", () => {
    const body = extractFunctionBody(src, "async function apiFetchJson");
    expect(/res\.ok/.test(body)).toBe(true);
    expect(/throw new Error/.test(body)).toBe(true);
  });

  test("apiFetchJson body calls and returns res.json()", () => {
    const body = extractFunctionBody(src, "async function apiFetchJson");
    expect(/res\.json\(\)/.test(body)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// AC2 — inline two-liner is eliminated from all call sites
// ---------------------------------------------------------------------------

describe("AC2: inline ok-guard+json two-liner is eliminated from call sites", () => {
  test("the two-liner does not appear in apiFetch-based call sites", () => {
    // Remove both private helper bodies (fetchJson and apiFetchJson) from consideration
    const srcWithoutHelpers = src
      .replace(extractFunctionBody(src, "async function fetchJson"), "")
      .replace(extractFunctionBody(src, "async function apiFetchJson"), "");

    // Match the consecutive ok-guard + json return pattern
    const pattern = /if\s*\(!res\.ok\)\s*throw new Error\(`\$\{res\.status\}`\);\s*\n\s*return res\.json\(\)/g;
    const matches = srcWithoutHelpers.match(pattern) ?? [];

    // Only legitimate exceptions: uploadProjectFiles and uploadKnowledgeFile use raw
    // fetch() (not apiFetch) for multipart form uploads and cannot use apiFetchJson.
    expect(matches.length).toBeLessThanOrEqual(2);
  });
});

// ---------------------------------------------------------------------------
// AC3 — key functions now delegate to apiFetchJson
// ---------------------------------------------------------------------------

describe("AC3: converted functions delegate to apiFetchJson", () => {
  const cases: Array<[string, string]> = [
    ["saveCustomMode", "export async function saveCustomMode"],
    ["removeCustomMode", "export async function removeCustomMode"],
    ["updateSettings", "export async function updateSettings"],
    ["approveProposal", "export async function approveProposal"],
    ["triageProposals", "export async function triageProposals"],
    ["approveTask", "export async function approveTask"],
    ["requestRevision", "export async function requestRevision"],
    ["verifyTaskCitations", "export async function verifyTaskCitations"],
    ["createTask", "export async function createTask"],
    ["createProject", "export async function createProject"],
    ["createDeadline", "export async function createDeadline"],
    ["sendProjectChat", "export async function sendProjectChat"],
    ["updateKnowledgeFile", "export async function updateKnowledgeFile"],
    ["deleteKnowledgeFile", "export async function deleteKnowledgeFile"],
    ["deleteCacheVolume", "export async function deleteCacheVolume"],
  ];

  for (const [name, marker] of cases) {
    test(`${name} uses apiFetchJson (no inline ok-guard)`, () => {
      const body = extractFunctionBody(src, marker);
      expect(/apiFetchJson/.test(body)).toBe(true);
      expect(/if\s*\(!res\.ok\)/.test(body)).toBe(false);
    });
  }
});

// ---------------------------------------------------------------------------
// AC4 — functions that should NOT be changed are untouched
// ---------------------------------------------------------------------------

describe("AC4: special-case functions are not changed", () => {
  test("checkConflicts still returns [] on error (custom error handling)", () => {
    const body = extractFunctionBody(src, "export async function checkConflicts");
    // It returns [] on !ok, not throws — must not use apiFetchJson
    expect(/return \[\]/.test(body)).toBe(true);
  });

  test("fetchProjectFileContent still returns arrayBuffer (not JSON)", () => {
    const body = extractFunctionBody(src, "export async function fetchProjectFileContent");
    expect(/arrayBuffer/.test(body)).toBe(true);
  });
});
