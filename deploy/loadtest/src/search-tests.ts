import { BorgClient } from "./client";
import type { GeneratedDoc, SearchTestCase, TestConfig, TestResult } from "./types";

// Build test cases from ground truth documents.
// Tests are designed to validate search infrastructure, not achieve 100% recall
// on a synthetic corpus. They use min_recall thresholds appropriate to each category.
export function buildTestCases(groundTruthDocs: GeneratedDoc[]): SearchTestCase[] {
  const tests: SearchTestCase[] = [];

  const contracts = groundTruthDocs.filter((d) => d.doc_type === "contract");
  const filings = groundTruthDocs.filter((d) => d.doc_type === "filing");
  const statutes = groundTruthDocs.filter((d) => d.doc_type === "statute");

  // ─── 1. EXACT TERM RETRIEVAL ───────────────────────────────────────
  // These use unique identifiers or very specific terms.

  // Case number lookups — unique identifiers, should always work
  for (const filing of filings.slice(0, 3)) {
    const caseNos = filing.ground_truth.unique_markers.filter((m) => m.match(/^\d{2}-cv-/));
    if (caseNos.length > 0) {
      tests.push({
        name: `Exact: case number ${caseNos[0]}`,
        category: "exact_term",
        query: caseNos[0],
        expected_hits: [filing.file_name],
        top_rank: 3,
        limit: 10,
      });
    }
  }

  // Specific entity name — should find docs mentioning it
  const entityName = "Meridian Capital Partners";
  const entityDocs = groundTruthDocs.filter((d) => d.body.includes(entityName));
  if (entityDocs.length > 0) {
    tests.push({
      name: `Exact: entity name "${entityName}"`,
      category: "exact_term",
      query: entityName,
      expected_hits: entityDocs.slice(0, 5).map((d) => d.file_name),
      min_recall: 0.4,
      limit: 20,
    });
  }

  // Specific case law citation
  const docsWithRevlon = groundTruthDocs.filter((d) => d.body.includes("Revlon"));
  if (docsWithRevlon.length > 0) {
    tests.push({
      name: "Exact: Revlon case citation",
      category: "exact_term",
      query: "Revlon MacAndrews Forbes Holdings 506 A.2d 173",
      expected_hits: docsWithRevlon.slice(0, 3).map((d) => d.file_name),
      min_recall: 0.33,
      limit: 20,
    });
  }

  // Specific clause type — exact term in document
  const contractsWithForce = contracts.filter((d) => d.body.includes("FORCE MAJEURE"));
  if (contractsWithForce.length > 0) {
    tests.push({
      name: "Exact: force majeure clauses",
      category: "exact_term",
      query: "FORCE MAJEURE acts of God pandemic epidemic",
      expected_hits: contractsWithForce.slice(0, 3).map((d) => d.file_name),
      min_recall: 0.33,
      limit: 20,
    });
  }

  // ─── 2. SEMANTIC SEARCH ────────────────────────────────────────────
  // Queries use different words than the document. We test that at least
  // some relevant docs surface — semantic search is probabilistic.

  const contractsWithLiabilityCap = contracts.filter((d) =>
    d.body.includes("LIMITATION OF LIABILITY"),
  );
  if (contractsWithLiabilityCap.length > 0) {
    tests.push({
      name: "Semantic: liability cap → limitation of liability",
      category: "semantic",
      query: "What is the liability cap and are there carveouts for fraud?",
      expected_hits: contractsWithLiabilityCap.slice(0, 3).map((d) => d.file_name),
      min_recall: 0.33,
      limit: 20,
    });
  }

  const contractsWithTermination = contracts.filter((d) => d.body.includes("TERMINATION"));
  if (contractsWithTermination.length > 0) {
    tests.push({
      name: "Semantic: ending deal early → termination provisions",
      category: "semantic",
      query: "How can either party exit the agreement before it expires?",
      expected_hits: contractsWithTermination.slice(0, 3).map((d) => d.file_name),
      min_recall: 0.33,
      limit: 20,
    });
  }

  const contractsWithConfidentiality = contracts.filter(
    (d) => d.body.includes("CONFIDENTIALITY") && d.body.includes("Confidential Information"),
  );
  if (contractsWithConfidentiality.length > 0) {
    tests.push({
      name: "Semantic: keeping secrets → confidentiality",
      category: "semantic",
      query: "obligations around protecting proprietary business information from disclosure",
      expected_hits: contractsWithConfidentiality.slice(0, 3).map((d) => d.file_name),
      min_recall: 0.33,
      limit: 20,
    });
  }

  const filingsWithDamages = filings.filter(
    (d) => d.body.includes("PRAYER FOR RELIEF") && d.body.includes("compensatory damages"),
  );
  if (filingsWithDamages.length > 0) {
    tests.push({
      name: "Semantic: suing for damages → prayer for relief",
      category: "semantic",
      query: "What monetary relief is being sought in the lawsuit?",
      expected_hits: filingsWithDamages.slice(0, 3).map((d) => d.file_name),
      min_recall: 0.33,
      limit: 20,
    });
  }

  // ─── 3. FILTERED SEARCH ───────────────────────────────────────────
  // Verify the filter mechanism works by checking returned result metadata,
  // not specific files. The server detects doc_type from content which may
  // differ from the generator's labels.

  tests.push({
    name: "Filter: doc_type=contract returns contracts",
    category: "filtered",
    query: "agreement between parties obligations",
    filters: { doc_type: "contract" },
    expected_hits: [],
    expect_result_type: "contract",
    min_results: 3,
    limit: 20,
  });

  tests.push({
    name: "Filter: doc_type=filing returns filings",
    category: "filtered",
    query: "motion court proceedings ruling",
    filters: { doc_type: "filing" },
    expected_hits: [],
    expect_result_type: "filing",
    min_results: 1,
    limit: 20,
  });

  // Jurisdiction filter: verify results are from the right jurisdiction
  tests.push({
    name: "Filter: jurisdiction=Delaware",
    category: "filtered",
    query: "corporate governance fiduciary duty",
    filters: { jurisdiction: "Delaware" },
    expected_hits: [],
    min_results: 1,
    limit: 20,
  });

  tests.push({
    name: "Filter: jurisdiction=Texas",
    category: "filtered",
    query: "breach of contract damages",
    filters: { jurisdiction: "Texas" },
    expected_hits: [],
    min_results: 1,
    limit: 20,
  });

  // Combined filter
  tests.push({
    name: "Filter: contract + Delaware",
    category: "filtered",
    query: "governing law dispute resolution",
    filters: { doc_type: "contract", jurisdiction: "Delaware" },
    expected_hits: [],
    expect_result_type: "contract",
    min_results: 1,
    limit: 20,
  });

  // ─── 4. MULTI-CONCEPT QUERIES ─────────────────────────────────────

  const contractsWithBoth = contracts.filter(
    (d) => d.body.includes("INDEMNIFICATION") && d.body.includes("LIMITATION OF LIABILITY"),
  );
  if (contractsWithBoth.length > 0) {
    tests.push({
      name: "Multi: indemnification + liability cap",
      category: "multi_concept",
      query: "indemnification obligations and limitation of liability cap carveouts",
      expected_hits: contractsWithBoth.slice(0, 3).map((d) => d.file_name),
      min_recall: 0.33,
      limit: 20,
    });
  }

  const filingsWithInjunction = filings.filter(
    (d) => d.body.includes("injunction") || d.body.includes("injunctive"),
  );
  if (filingsWithInjunction.length > 0) {
    tests.push({
      name: "Multi: breach + damages + injunctive relief",
      category: "multi_concept",
      query: "breach of contract damages injunctive relief preliminary injunction",
      expected_hits: filingsWithInjunction.slice(0, 3).map((d) => d.file_name),
      min_recall: 0.33,
      limit: 20,
    });
  }

  const docsWithSecurities = groundTruthDocs.filter(
    (d) => d.body.includes("10b-5") || d.body.includes("securities fraud") || d.body.includes("Section 10(b)"),
  );
  if (docsWithSecurities.length > 0) {
    tests.push({
      name: "Multi: securities fraud + scienter + 10b-5",
      category: "multi_concept",
      query: "securities fraud scienter material misrepresentation Rule 10b-5",
      expected_hits: docsWithSecurities.slice(0, 3).map((d) => d.file_name),
      min_recall: 0.33,
      limit: 20,
    });
  }

  // ─── 5. AGENT-REALISTIC QUERIES ───────────────────────────────────

  tests.push({
    name: "Agent: force majeure provisions",
    category: "agent_realistic",
    query: "force majeure clause pandemic epidemic natural disaster acts of God",
    expected_hits: contracts
      .filter((d) => d.body.includes("FORCE MAJEURE"))
      .slice(0, 5)
      .map((d) => d.file_name),
    min_recall: 0.4,
    limit: 20,
  });

  tests.push({
    name: "Agent: insurance requirements",
    category: "agent_realistic",
    query: "insurance coverage requirements commercial general liability professional errors omissions",
    expected_hits: contracts
      .filter((d) => d.body.includes("INSURANCE"))
      .slice(0, 5)
      .map((d) => d.file_name),
    min_recall: 0.4,
    limit: 20,
  });

  const filingsWithSJ = filings.filter(
    (d) => d.body.includes("Summary Judgment") || d.body.includes("summary judgment"),
  );
  if (filingsWithSJ.length > 0) {
    tests.push({
      name: "Agent: summary judgment motions",
      category: "agent_realistic",
      query: "motion for summary judgment genuine dispute material fact Celotex",
      expected_hits: filingsWithSJ.slice(0, 3).map((d) => d.file_name),
      min_recall: 0.33,
      limit: 20,
    });
  }

  tests.push({
    name: "Agent: governing law clauses",
    category: "agent_realistic",
    query: "governing law choice of law exclusive jurisdiction dispute resolution",
    expected_hits: contracts
      .filter((d) => d.body.includes("GOVERNING LAW"))
      .slice(0, 5)
      .map((d) => d.file_name),
    min_recall: 0.4,
    limit: 20,
  });

  const dueDiligenceDocs = groundTruthDocs.filter(
    (d) => d.body.includes("due diligence") || d.body.includes("DUE DILIGENCE"),
  );
  if (dueDiligenceDocs.length > 0) {
    tests.push({
      name: "Agent: due diligence findings",
      category: "agent_realistic",
      query: "due diligence findings material issues regulatory compliance",
      expected_hits: dueDiligenceDocs.slice(0, 5).map((d) => d.file_name),
      min_recall: 0.4,
      limit: 20,
    });
  }

  // ─── 6. RANKING QUALITY ───────────────────────────────────────────

  const ndaContracts = contracts.filter(
    (d) => d.file_name.includes("non_disclosure") || d.body.includes("Non-Disclosure Agreement"),
  );
  if (ndaContracts.length > 0) {
    tests.push({
      name: "Ranking: NDA contract in top results",
      category: "ranking",
      query: "non-disclosure agreement confidential information mutual NDA",
      expected_hits: ndaContracts.slice(0, 1).map((d) => d.file_name),
      top_rank: 10,
      limit: 20,
    });
  }

  const patentStatutes = statutes.filter(
    (d) => d.body.includes("Patent Infringement") || d.body.includes("35 U.S.C."),
  );
  if (patentStatutes.length > 0) {
    tests.push({
      name: "Ranking: patent statute for patent query",
      category: "ranking",
      query: "patent infringement 35 U.S.C. 271",
      expected_hits: patentStatutes.slice(0, 1).map((d) => d.file_name),
      top_rank: 10,
      limit: 20,
    });
  }

  // ─── 7. NEGATIVE TESTS ───────────────────────────────────────────

  // Irrelevant query — should return no meaningful legal results
  tests.push({
    name: "Negative: irrelevant query",
    category: "negative",
    query: "chocolate cake recipe baking instructions vanilla frosting",
    expected_hits: [],
    limit: 10,
  });

  // Jurisdiction filter excludes other jurisdictions
  const texasDocs = groundTruthDocs.filter((d) => d.jurisdiction === "Texas");
  const delawareDocs = groundTruthDocs.filter((d) => d.jurisdiction === "Delaware");
  if (texasDocs.length > 0 && delawareDocs.length > 0) {
    tests.push({
      name: "Negative: Texas filter excludes Delaware docs",
      category: "negative",
      query: "breach of contract damages",
      filters: { jurisdiction: "Texas" },
      expected_hits: [],
      expected_misses: delawareDocs.slice(0, 3).map((d) => d.file_name),
      limit: 20,
    });
  }

  return tests.filter(
    (t) =>
      t.expected_hits.length > 0 ||
      (t.expected_misses && t.expected_misses.length > 0) ||
      t.min_results !== undefined ||
      t.expect_result_type !== undefined,
  );
}

// ─── Test runner ─────────────────────────────────────────────────────

export async function runSearchTests(
  config: TestConfig,
  testCases: SearchTestCase[],
): Promise<TestResult[]> {
  const client = new BorgClient(config.baseUrl);
  await client.authenticate();

  const results: TestResult[] = [];

  for (const tc of testCases) {
    const start = Date.now();
    let passed = true;
    let details = "";
    let actualHits: string[] = [];
    let expectedMissesFound: string[] = [];
    let rankOfPrimary: number | undefined;

    try {
      const rawResults = await client.agentSearch(tc.query, config.projectId, {
        limit: tc.limit ?? config.topK,
        doc_type: tc.filters?.doc_type,
        jurisdiction: tc.filters?.jurisdiction,
        privileged_only: tc.filters?.privileged_only,
      });

      const parsed = parseAgentSearchResults(rawResults);
      actualHits = parsed.filePaths;

      // Check min_results (for filter verification)
      if (tc.min_results !== undefined) {
        if (actualHits.length < tc.min_results) {
          passed = false;
          details += `Only ${actualHits.length} results, expected ≥${tc.min_results}. `;
        } else {
          details += `${actualHits.length} results returned. `;
        }
      }

      // Check expect_result_type (verify filter works by checking doc_type in output)
      if (tc.expect_result_type && parsed.docTypes.length > 0) {
        const wrongType = parsed.docTypes.filter(
          (t) => t !== tc.expect_result_type && t !== "unknown" && t !== "",
        );
        if (wrongType.length > 0) {
          passed = false;
          details += `Wrong doc_types in results: ${[...new Set(wrongType)].join(", ")}. `;
        } else {
          details += `All results have type=${tc.expect_result_type}. `;
        }
      }

      // Check expected hits with min_recall threshold
      if (tc.expected_hits.length > 0) {
        const found = tc.expected_hits.filter((expected) =>
          actualHits.some((hit) => hit.includes(expected) || expected.includes(hit)),
        );
        const recall = found.length / tc.expected_hits.length;
        const minRecall = tc.min_recall ?? 1.0;

        if (recall < minRecall) {
          passed = false;
          const missed = tc.expected_hits.filter(
            (expected) => !actualHits.some((hit) => hit.includes(expected) || expected.includes(hit)),
          );
          details += `Recall ${(recall * 100).toFixed(0)}% < ${(minRecall * 100).toFixed(0)}% threshold (${found.length}/${tc.expected_hits.length}). Missing: ${missed.join(", ")}. `;
        } else {
          details += `Recall: ${(recall * 100).toFixed(0)}% (${found.length}/${tc.expected_hits.length}). `;
        }
      }

      // Check expected misses
      if (tc.expected_misses && tc.expected_misses.length > 0) {
        expectedMissesFound = tc.expected_misses.filter((miss) =>
          actualHits.some((hit) => hit.includes(miss) || miss.includes(hit)),
        );
        if (expectedMissesFound.length > 0) {
          passed = false;
          details += `Unexpectedly found: ${expectedMissesFound.join(", ")}. `;
        }
      }

      // Check ranking
      if (tc.top_rank !== undefined && tc.expected_hits.length > 0) {
        const primaryExpected = tc.expected_hits[0];
        const idx = actualHits.findIndex(
          (hit) => hit.includes(primaryExpected) || primaryExpected.includes(hit),
        );
        rankOfPrimary = idx === -1 ? undefined : idx + 1;
        if (rankOfPrimary === undefined || rankOfPrimary > tc.top_rank) {
          passed = false;
          details += `Primary ranked ${rankOfPrimary ?? "not found"}, expected top ${tc.top_rank}. `;
        }
      }
    } catch (err) {
      passed = false;
      details = `Error: ${err instanceof Error ? err.message : String(err)}`;
    }

    const latencyMs = Date.now() - start;

    const icon = passed ? "PASS" : "FAIL";
    console.log(
      `  [${icon}] ${tc.name} (${latencyMs}ms)${details ? " — " + details.trim() : ""}`,
    );

    results.push({
      name: tc.name,
      category: tc.category,
      passed,
      query: tc.query,
      expected_hits: tc.expected_hits,
      actual_hits: actualHits.slice(0, 10),
      expected_misses_found: expectedMissesFound,
      rank_of_primary: rankOfPrimary,
      latency_ms: latencyMs,
      details: details || undefined,
    });
  }

  return results;
}

interface ParsedResults {
  filePaths: string[];
  docTypes: string[];
}

function parseAgentSearchResults(text: string): ParsedResults {
  const filePaths: string[] = [];
  const docTypes: string[] = [];

  // try JSON first
  try {
    const parsed = JSON.parse(text);
    if (Array.isArray(parsed)) {
      return {
        filePaths: parsed.map((r: any) => r.file_path || r.file_name || "").filter(Boolean),
        docTypes: parsed.map((r: any) => r.doc_type || "").filter(Boolean),
      };
    }
  } catch {
    // not JSON, parse plain text
  }

  // parse plain text format:
  // --- Result 1 (score: 0.85, type: contract) ---
  // File: path/to/file.md [id=123, chunk=0]
  for (const line of text.split("\n")) {
    const fileMatch = line.match(/^File:\s*(\S+)/);
    if (fileMatch) {
      filePaths.push(fileMatch[1].trim());
      continue;
    }
    const resultMatch = line.match(/^--- Result \d+ \(.*?type:\s*(\w+)/);
    if (resultMatch) {
      docTypes.push(resultMatch[1]);
      continue;
    }
    // fallback: catch file paths in other formats
    const pathMatch = line.match(/[a-z_]+_\d+\.md/);
    if (pathMatch && !filePaths.includes(pathMatch[0])) {
      filePaths.push(pathMatch[0]);
    }
  }

  return { filePaths, docTypes };
}

export function summarizeResults(results: TestResult[]): {
  total: number;
  passed: number;
  failed: number;
  passRate: number;
  byCategory: Record<string, { total: number; passed: number; passRate: number }>;
  avgLatencyMs: number;
  p95LatencyMs: number;
} {
  const total = results.length;
  const passed = results.filter((r) => r.passed).length;
  const failed = total - passed;
  const passRate = total > 0 ? passed / total : 0;

  const byCategory: Record<string, { total: number; passed: number; passRate: number }> = {};
  for (const r of results) {
    if (!byCategory[r.category]) {
      byCategory[r.category] = { total: 0, passed: 0, passRate: 0 };
    }
    byCategory[r.category].total++;
    if (r.passed) byCategory[r.category].passed++;
  }
  for (const cat of Object.values(byCategory)) {
    cat.passRate = cat.total > 0 ? cat.passed / cat.total : 0;
  }

  const latencies = results.map((r) => r.latency_ms).sort((a, b) => a - b);
  const avgLatencyMs =
    latencies.length > 0
      ? latencies.reduce((a, b) => a + b, 0) / latencies.length
      : 0;
  const p95LatencyMs =
    latencies.length > 0
      ? latencies[Math.floor(latencies.length * 0.95)]
      : 0;

  return { total, passed, failed, passRate, byCategory, avgLatencyMs, p95LatencyMs };
}
