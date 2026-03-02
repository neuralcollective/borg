/**
 * Tests for Task #19: Cancel pending tokenReady promise on component unmount
 * to prevent EventSource leak.
 *
 * Static-analysis tests: read source text of api.ts and chat-panel.tsx and
 * verify that the cancelled-flag guard pattern is present inside the
 * tokenReady.then() callback for useLogs and ChatPanel's connect function.
 */

import { describe, test, expect, beforeAll } from "bun:test";
import { readFileSync } from "fs";
import { join } from "path";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const API_PATH = join(import.meta.dir, "api.ts");
const CHAT_PANEL_PATH = join(import.meta.dir, "../components/chat-panel.tsx");

let apiSrc: string;
let chatPanelSrc: string;
let useLogsBody: string;

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
      if (depth === 0) return source.slice(start, i + 1);
    }
    i++;
  }
  throw new Error(`Could not find closing brace for '${name}'`);
}

beforeAll(() => {
  apiSrc = readFileSync(API_PATH, "utf-8");
  chatPanelSrc = readFileSync(CHAT_PANEL_PATH, "utf-8");
  useLogsBody = extractFunctionBody(apiSrc, "useLogs");
});

// ---------------------------------------------------------------------------
// useLogs — cancelledRef declared
// ---------------------------------------------------------------------------

describe("useLogs: cancelledRef is declared", () => {
  test("cancelledRef is declared as a useRef(false)", () => {
    const pattern = /const\s+cancelledRef\s*=\s*useRef\s*\(\s*false\s*\)/;
    expect(pattern.test(useLogsBody)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// useLogs — guard inside tokenReady.then()
// ---------------------------------------------------------------------------

describe("useLogs: cancelled guard inside tokenReady.then()", () => {
  test("tokenReady.then() is present in useLogs connect", () => {
    expect(useLogsBody.includes("tokenReady.then(")).toBe(true);
  });

  test("cancelledRef.current is checked after tokenReady resolves", () => {
    // The guard must appear inside the .then() callback
    const thenIdx = useLogsBody.indexOf("tokenReady.then(");
    expect(thenIdx).toBeGreaterThan(-1);
    const afterThen = useLogsBody.slice(thenIdx);
    const pattern = /if\s*\(\s*cancelledRef\.current\s*\)\s*return/;
    expect(pattern.test(afterThen)).toBe(true);
  });

  test("the guard appears before EventSource construction", () => {
    const thenIdx = useLogsBody.indexOf("tokenReady.then(");
    const afterThen = useLogsBody.slice(thenIdx);
    const guardIdx = afterThen.search(/if\s*\(\s*cancelledRef\.current\s*\)\s*return/);
    const esIdx = afterThen.indexOf("new EventSource(");
    expect(guardIdx).toBeGreaterThan(-1);
    expect(esIdx).toBeGreaterThan(-1);
    expect(guardIdx).toBeLessThan(esIdx);
  });
});

// ---------------------------------------------------------------------------
// useLogs — cancelledRef.current set to false when connect starts
// ---------------------------------------------------------------------------

describe("useLogs: cancelledRef.current reset to false at start of connect", () => {
  test("cancelledRef.current = false appears before tokenReady.then()", () => {
    const connectIdx = useLogsBody.indexOf("const connect = useCallback(");
    expect(connectIdx).toBeGreaterThan(-1);
    const connectBody = useLogsBody.slice(connectIdx);
    const resetIdx = connectBody.indexOf("cancelledRef.current = false");
    const thenIdx = connectBody.indexOf("tokenReady.then(");
    expect(resetIdx).toBeGreaterThan(-1);
    expect(thenIdx).toBeGreaterThan(-1);
    expect(resetIdx).toBeLessThan(thenIdx);
  });
});

// ---------------------------------------------------------------------------
// useLogs — cleanup sets cancelledRef.current = true
// ---------------------------------------------------------------------------

describe("useLogs: cleanup sets cancelledRef.current = true", () => {
  test("cancelledRef.current = true is present in useLogs", () => {
    const pattern = /cancelledRef\.current\s*=\s*true/;
    expect(pattern.test(useLogsBody)).toBe(true);
  });

  test("cancelledRef.current = true appears after the connect() call in useEffect", () => {
    // The useEffect that calls connect() must set cancelled in its cleanup
    const connectCallIdx = useLogsBody.lastIndexOf("connect()");
    expect(connectCallIdx).toBeGreaterThan(-1);
    const afterConnectCall = useLogsBody.slice(connectCallIdx);
    const pattern = /cancelledRef\.current\s*=\s*true/;
    expect(pattern.test(afterConnectCall)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// ChatPanel — cancelledRef declared
// ---------------------------------------------------------------------------

describe("ChatPanel: cancelledRef is declared", () => {
  test("cancelledRef is declared as a useRef(false)", () => {
    const pattern = /const\s+cancelledRef\s*=\s*useRef\s*\(\s*false\s*\)/;
    expect(pattern.test(chatPanelSrc)).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// ChatPanel — guard inside connect's tokenReady.then()
// ---------------------------------------------------------------------------

describe("ChatPanel connect: cancelled guard inside tokenReady.then()", () => {
  test("tokenReady.then() is present in the connect callback", () => {
    // Locate the connect useCallback
    const connectIdx = chatPanelSrc.indexOf("const connect = useCallback(");
    expect(connectIdx).toBeGreaterThan(-1);
    const afterConnect = chatPanelSrc.slice(connectIdx);
    expect(afterConnect.includes("tokenReady.then(")).toBe(true);
  });

  test("cancelledRef.current guard appears inside the connect's tokenReady.then()", () => {
    const connectIdx = chatPanelSrc.indexOf("const connect = useCallback(");
    const afterConnect = chatPanelSrc.slice(connectIdx);
    const thenIdx = afterConnect.indexOf("tokenReady.then(");
    const afterThen = afterConnect.slice(thenIdx);
    const pattern = /if\s*\(\s*cancelledRef\.current\s*\)\s*return/;
    expect(pattern.test(afterThen)).toBe(true);
  });

  test("the guard appears before EventSource construction in connect", () => {
    const connectIdx = chatPanelSrc.indexOf("const connect = useCallback(");
    const afterConnect = chatPanelSrc.slice(connectIdx);
    const thenIdx = afterConnect.indexOf("tokenReady.then(");
    const afterThen = afterConnect.slice(thenIdx);
    const guardIdx = afterThen.search(/if\s*\(\s*cancelledRef\.current\s*\)\s*return/);
    const esIdx = afterThen.indexOf("new EventSource(");
    expect(guardIdx).toBeGreaterThan(-1);
    expect(esIdx).toBeGreaterThan(-1);
    expect(guardIdx).toBeLessThan(esIdx);
  });
});

// ---------------------------------------------------------------------------
// ChatPanel — cancelledRef.current reset to false at start of connect
// ---------------------------------------------------------------------------

describe("ChatPanel connect: cancelledRef.current reset to false", () => {
  test("cancelledRef.current = false appears before tokenReady.then() in connect", () => {
    const connectIdx = chatPanelSrc.indexOf("const connect = useCallback(");
    const afterConnect = chatPanelSrc.slice(connectIdx);
    const resetIdx = afterConnect.indexOf("cancelledRef.current = false");
    const thenIdx = afterConnect.indexOf("tokenReady.then(");
    expect(resetIdx).toBeGreaterThan(-1);
    expect(thenIdx).toBeGreaterThan(-1);
    expect(resetIdx).toBeLessThan(thenIdx);
  });
});

// ---------------------------------------------------------------------------
// ChatPanel — cleanup sets cancelledRef.current = true
// ---------------------------------------------------------------------------

describe("ChatPanel: cleanup sets cancelledRef.current = true", () => {
  test("cancelledRef.current = true is present in the SSE useEffect cleanup", () => {
    // The cleanup return () => { ... } that also closes esRef should set cancelled
    const pattern = /cancelledRef\.current\s*=\s*true/;
    expect(pattern.test(chatPanelSrc)).toBe(true);
  });
});
