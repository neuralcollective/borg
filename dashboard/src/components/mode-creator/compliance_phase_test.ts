import { beforeAll, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const PANEL_PATH = join(import.meta.dir, "../mode-creator-panel.tsx");
const STRIP_PATH = join(import.meta.dir, "phase-strip.tsx");
const REDUCER_PATH = join(import.meta.dir, "reducer.ts");
const DETAIL_PATH = join(import.meta.dir, "phase-detail.tsx");

let panelSrc = "";
let stripSrc = "";
let reducerSrc = "";
let detailSrc = "";

beforeAll(() => {
  panelSrc = readFileSync(PANEL_PATH, "utf-8");
  stripSrc = readFileSync(STRIP_PATH, "utf-8");
  reducerSrc = readFileSync(REDUCER_PATH, "utf-8");
  detailSrc = readFileSync(DETAIL_PATH, "utf-8");
});

describe("Compliance Phase UX", () => {
  test("Add phase menu exposes UK and US compliance actions", () => {
    expect(panelSrc).toContain("showComplianceOptions");
    expect(stripSrc).toContain("UK SRA Check");
    expect(stripSrc).toContain("US Ethics Check");
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
