import { describe, test, expect, beforeAll } from "bun:test";
import { readFileSync } from "fs";
import { join } from "path";

const PANEL_PATH = join(import.meta.dir, "../mode-creator-panel.tsx");
const REDUCER_PATH = join(import.meta.dir, "reducer.ts");
const DETAIL_PATH = join(import.meta.dir, "phase-detail.tsx");

let panelSrc = "";
let reducerSrc = "";
let detailSrc = "";

beforeAll(() => {
  panelSrc = readFileSync(PANEL_PATH, "utf-8");
  reducerSrc = readFileSync(REDUCER_PATH, "utf-8");
  detailSrc = readFileSync(DETAIL_PATH, "utf-8");
});

describe("Compliance Phase UX", () => {
  test("Mode creator exposes UK and US quick-add actions", () => {
    expect(panelSrc).toContain("+ UK SRA Check");
    expect(panelSrc).toContain("+ US Ethics Check");
  });

  test("Reducer supports ADD_COMPLIANCE_PHASE action", () => {
    expect(reducerSrc).toContain("ADD_COMPLIANCE_PHASE");
    expect(reducerSrc).toContain("compliance_profile");
    expect(reducerSrc).toContain("compliance_enforcement");
  });

  test("Phase detail supports compliance_check type", () => {
    expect(detailSrc).toContain("compliance_check");
    expect(detailSrc).toContain("Compliance Check");
  });
});
