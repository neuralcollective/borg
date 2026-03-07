import { BorgClient } from "./client";
import { generateDocument } from "./generators";
import type { GeneratedDoc, SearchTestCase, TestConfig, TestResult } from "./types";

// Build test cases from ground truth documents.
// We examine the first 200 generated docs (deterministic) and craft queries
// that an agent would realistically pose during legal research.
export function buildTestCases(groundTruthDocs: GeneratedDoc[]): SearchTestCase[] {
  const tests: SearchTestCase[] = [];

  // partition docs by type for targeted tests
  const contracts = groundTruthDocs.filter((d) => d.doc_type === "contract");
  const filings = groundTruthDocs.filter((d) => d.doc_type === "filing");
  const memos = groundTruthDocs.filter((d) => d.doc_type === "memo");
  const statutes = groundTruthDocs.filter((d) => d.doc_type === "statute");
  const privileged = groundTruthDocs.filter((d) => d.privileged);
  const nonPrivileged = groundTruthDocs.filter((d) => !d.privileged);

  // ─── 1. EXACT TERM RETRIEVAL ───────────────────────────────────────

  // Find contracts by specific clause types
  const contractsWithIndemnification = contracts.filter((d) =>
    d.body.includes("INDEMNIFICATION"),
  );
  if (contractsWithIndemnification.length > 0) {
    tests.push({
      name: "Find contracts with indemnification clauses",
      category: "exact_term",
      query: "indemnification hold harmless losses damages",
      expected_hits: contractsWithIndemnification.slice(0, 5).map((d) => d.file_name),
      limit: 20,
    });
  }

  const contractsWithNonCompete = contracts.filter((d) =>
    d.body.includes("NON-COMPETITION"),
  );
  if (contractsWithNonCompete.length > 0) {
    tests.push({
      name: "Find contracts with non-compete provisions",
      category: "exact_term",
      query: "non-competition non-solicitation restricted party territory",
      expected_hits: contractsWithNonCompete.slice(0, 3).map((d) => d.file_name),
      limit: 20,
    });
  }

  const contractsWithIPAssignment = contracts.filter((d) =>
    d.body.includes("INTELLECTUAL PROPERTY ASSIGNMENT"),
  );
  if (contractsWithIPAssignment.length > 0) {
    tests.push({
      name: "Find IP assignment clauses in contracts",
      category: "exact_term",
      query: "intellectual property assignment work product work made for hire",
      expected_hits: contractsWithIPAssignment.slice(0, 3).map((d) => d.file_name),
      limit: 20,
    });
  }

  // Find filings by case number
  for (const filing of filings.slice(0, 3)) {
    const caseNos = filing.ground_truth.unique_markers.filter((m) => m.match(/^\d{2}-cv-/));
    if (caseNos.length > 0) {
      tests.push({
        name: `Find filing by case number ${caseNos[0]}`,
        category: "exact_term",
        query: caseNos[0],
        expected_hits: [filing.file_name],
        top_rank: 3,
        limit: 10,
      });
    }
  }

  // Find documents mentioning specific case law
  const docsWithRevlon = groundTruthDocs.filter((d) =>
    d.body.includes("Revlon"),
  );
  if (docsWithRevlon.length > 0) {
    tests.push({
      name: "Find documents citing Revlon duties",
      category: "exact_term",
      query: "Revlon MacAndrews Forbes Holdings 506 A.2d 173",
      expected_hits: docsWithRevlon.slice(0, 5).map((d) => d.file_name),
      limit: 20,
    });
  }

  // ─── 2. SEMANTIC SEARCH ────────────────────────────────────────────
  // Queries that use different words than the document but same meaning

  // "liability cap" → should find "LIMITATION OF LIABILITY"
  const contractsWithLiabilityCap = contracts.filter((d) =>
    d.body.includes("LIMITATION OF LIABILITY"),
  );
  if (contractsWithLiabilityCap.length > 0) {
    tests.push({
      name: "Semantic: liability cap → limitation of liability",
      category: "semantic",
      query: "What is the liability cap and are there carveouts for fraud?",
      expected_hits: contractsWithLiabilityCap.slice(0, 3).map((d) => d.file_name),
      limit: 20,
    });
  }

  // "can we end the deal early" → should find termination clauses
  const contractsWithTermination = contracts.filter((d) =>
    d.body.includes("TERMINATION"),
  );
  if (contractsWithTermination.length > 0) {
    tests.push({
      name: "Semantic: ending deal early → termination provisions",
      category: "semantic",
      query: "How can either party exit the agreement before it expires?",
      expected_hits: contractsWithTermination.slice(0, 3).map((d) => d.file_name),
      limit: 20,
    });
  }

  // "keeping information secret" → should find confidentiality
  const contractsWithConfidentiality = contracts.filter((d) =>
    d.body.includes("CONFIDENTIALITY") && d.body.includes("Confidential Information"),
  );
  if (contractsWithConfidentiality.length > 0) {
    tests.push({
      name: "Semantic: keeping information secret → confidentiality",
      category: "semantic",
      query: "obligations around protecting proprietary business information from disclosure",
      expected_hits: contractsWithConfidentiality.slice(0, 3).map((d) => d.file_name),
      limit: 20,
    });
  }

  // "suing for damages" → should find filing prayer for relief
  const filingsWithDamages = filings.filter((d) =>
    d.body.includes("PRAYER FOR RELIEF") && d.body.includes("compensatory damages"),
  );
  if (filingsWithDamages.length > 0) {
    tests.push({
      name: "Semantic: suing for damages → prayer for relief",
      category: "semantic",
      query: "What monetary relief is being sought in the lawsuit?",
      expected_hits: filingsWithDamages.slice(0, 3).map((d) => d.file_name),
      limit: 20,
    });
  }

  // "board oversight failures" → should find Caremark claims
  const docsWithCaremark = groundTruthDocs.filter((d) =>
    d.body.includes("Caremark") || d.body.includes("oversight"),
  );
  if (docsWithCaremark.length > 0) {
    tests.push({
      name: "Semantic: board oversight failures → Caremark duties",
      category: "semantic",
      query: "director liability for failing to monitor corporate compliance",
      expected_hits: docsWithCaremark.slice(0, 3).map((d) => d.file_name),
      limit: 20,
    });
  }

  // ─── 3. FILTERED SEARCH ───────────────────────────────────────────

  // doc_type filter
  if (contracts.length > 0) {
    tests.push({
      name: "Filter: only contracts",
      category: "filtered",
      query: "agreement between parties",
      filters: { doc_type: "contract" },
      expected_hits: contracts.slice(0, 3).map((d) => d.file_name),
      expected_misses: filings.slice(0, 3).map((d) => d.file_name),
      limit: 20,
    });
  }

  if (filings.length > 0) {
    tests.push({
      name: "Filter: only filings",
      category: "filtered",
      query: "motion court plaintiff defendant",
      filters: { doc_type: "filing" },
      expected_hits: filings.slice(0, 3).map((d) => d.file_name),
      expected_misses: contracts.slice(0, 3).map((d) => d.file_name),
      limit: 20,
    });
  }

  // jurisdiction filter
  const delawareDocs = groundTruthDocs.filter((d) => d.jurisdiction === "Delaware");
  const nonDelawareDocs = groundTruthDocs.filter((d) => d.jurisdiction !== "Delaware");
  if (delawareDocs.length > 0 && nonDelawareDocs.length > 0) {
    tests.push({
      name: "Filter: Delaware jurisdiction only",
      category: "filtered",
      query: "corporate governance fiduciary duty",
      filters: { jurisdiction: "Delaware" },
      expected_hits: delawareDocs
        .filter((d) => d.body.toLowerCase().includes("fiduciary") || d.body.toLowerCase().includes("corporate"))
        .slice(0, 3)
        .map((d) => d.file_name),
      limit: 20,
    });
  }

  // privileged filter
  if (privileged.length > 0 && nonPrivileged.length > 0) {
    tests.push({
      name: "Filter: privileged documents only",
      category: "filtered",
      query: "attorney work product analysis recommendation",
      filters: { privileged_only: true },
      expected_hits: privileged
        .filter((d) => d.body.includes("PRIVILEGED") || d.body.includes("ATTORNEY"))
        .slice(0, 3)
        .map((d) => d.file_name),
      limit: 20,
    });
  }

  // combined filters
  const delawareContracts = contracts.filter((d) => d.jurisdiction === "Delaware");
  if (delawareContracts.length > 0) {
    tests.push({
      name: "Filter: Delaware contracts only",
      category: "filtered",
      query: "governing law dispute resolution",
      filters: { doc_type: "contract", jurisdiction: "Delaware" },
      expected_hits: delawareContracts.slice(0, 3).map((d) => d.file_name),
      limit: 20,
    });
  }

  // ─── 4. MULTI-CONCEPT QUERIES ─────────────────────────────────────

  // combining indemnification + limitation of liability in same contract
  const contractsWithBoth = contracts.filter(
    (d) => d.body.includes("INDEMNIFICATION") && d.body.includes("LIMITATION OF LIABILITY"),
  );
  if (contractsWithBoth.length > 0) {
    tests.push({
      name: "Multi-concept: indemnification AND liability cap in same contract",
      category: "multi_concept",
      query: "indemnification obligations and limitation of liability cap carveouts",
      expected_hits: contractsWithBoth.slice(0, 3).map((d) => d.file_name),
      limit: 20,
    });
  }

  // breach + damages + injunction
  const filingsWithInjunction = filings.filter(
    (d) => d.body.includes("injunction") || d.body.includes("injunctive"),
  );
  if (filingsWithInjunction.length > 0) {
    tests.push({
      name: "Multi-concept: breach + damages + injunctive relief",
      category: "multi_concept",
      query: "breach of contract damages injunctive relief preliminary injunction",
      expected_hits: filingsWithInjunction.slice(0, 3).map((d) => d.file_name),
      limit: 20,
    });
  }

  // securities fraud + scienter + 10b-5
  const docsWithSecurities = groundTruthDocs.filter(
    (d) => d.body.includes("10b-5") || d.body.includes("securities fraud") || d.body.includes("Section 10(b)"),
  );
  if (docsWithSecurities.length > 0) {
    tests.push({
      name: "Multi-concept: securities fraud elements",
      category: "multi_concept",
      query: "securities fraud scienter material misrepresentation Rule 10b-5",
      expected_hits: docsWithSecurities.slice(0, 3).map((d) => d.file_name),
      limit: 20,
    });
  }

  // ─── 5. AGENT-REALISTIC QUERIES ───────────────────────────────────
  // These simulate what a legal agent would actually ask during research

  tests.push({
    name: "Agent: review force majeure provisions across all contracts",
    category: "agent_realistic",
    query: "force majeure clause pandemic epidemic natural disaster acts of God",
    expected_hits: contracts
      .filter((d) => d.body.includes("FORCE MAJEURE"))
      .slice(0, 5)
      .map((d) => d.file_name),
    limit: 20,
  });

  tests.push({
    name: "Agent: find all insurance requirements in agreements",
    category: "agent_realistic",
    query: "insurance coverage requirements commercial general liability professional liability errors omissions",
    expected_hits: contracts
      .filter((d) => d.body.includes("INSURANCE"))
      .slice(0, 5)
      .map((d) => d.file_name),
    limit: 20,
  });

  const filingsWithSummaryJudgment = filings.filter(
    (d) => d.body.includes("Summary Judgment") || d.body.includes("summary judgment"),
  );
  if (filingsWithSummaryJudgment.length > 0) {
    tests.push({
      name: "Agent: find summary judgment motions and standards",
      category: "agent_realistic",
      query: "motion for summary judgment genuine dispute material fact Celotex",
      expected_hits: filingsWithSummaryJudgment.slice(0, 3).map((d) => d.file_name),
      limit: 20,
    });
  }

  const docsWithGDPR = groundTruthDocs.filter(
    (d) => d.body.includes("GDPR") || d.body.includes("data protection") || d.body.includes("data processing"),
  );
  if (docsWithGDPR.length > 0) {
    tests.push({
      name: "Agent: GDPR compliance assessment documents",
      category: "agent_realistic",
      query: "GDPR data protection processing agreement sub-processor cross-border transfer",
      expected_hits: docsWithGDPR.slice(0, 5).map((d) => d.file_name),
      limit: 20,
    });
  }

  tests.push({
    name: "Agent: find documents about change of control provisions",
    category: "agent_realistic",
    query: "change of control provision consent required assignment merger acquisition",
    expected_hits: groundTruthDocs
      .filter((d) => d.body.includes("change-of-control") || d.body.includes("change of control"))
      .slice(0, 5)
      .map((d) => d.file_name),
    limit: 20,
  });

  const docsWithSettlement = groundTruthDocs.filter(
    (d) => d.body.toLowerCase().includes("settlement"),
  );
  if (docsWithSettlement.length > 0) {
    tests.push({
      name: "Agent: settlement discussions and proposals",
      category: "agent_realistic",
      query: "settlement proposal terms payment release mutual general release",
      expected_hits: docsWithSettlement.slice(0, 5).map((d) => d.file_name),
      limit: 20,
    });
  }

  // agent doing contract review
  tests.push({
    name: "Agent: identify all governing law clauses",
    category: "agent_realistic",
    query: "governing law choice of law exclusive jurisdiction dispute resolution",
    expected_hits: contracts
      .filter((d) => d.body.includes("GOVERNING LAW"))
      .slice(0, 5)
      .map((d) => d.file_name),
    limit: 20,
  });

  // agent doing due diligence
  const dueDiligenceDocs = groundTruthDocs.filter(
    (d) => d.body.includes("due diligence") || d.body.includes("DUE DILIGENCE"),
  );
  if (dueDiligenceDocs.length > 0) {
    tests.push({
      name: "Agent: due diligence findings and material issues",
      category: "agent_realistic",
      query: "due diligence findings material issues regulatory compliance IP ownership",
      expected_hits: dueDiligenceDocs.slice(0, 5).map((d) => d.file_name),
      limit: 20,
    });
  }

  // agent searching for specific entity across all document types
  const entityName = "Meridian Capital Partners";
  const entityDocs = groundTruthDocs.filter((d) => d.body.includes(entityName));
  if (entityDocs.length > 0) {
    tests.push({
      name: `Agent: find all documents mentioning ${entityName}`,
      category: "agent_realistic",
      query: entityName,
      expected_hits: entityDocs.slice(0, 10).map((d) => d.file_name),
      limit: 30,
    });
  }

  // ─── 6. RANKING QUALITY ───────────────────────────────────────────

  // a contract about NDAs should rank higher than a filing that mentions NDAs in passing
  const ndaContracts = contracts.filter((d) =>
    d.file_name.includes("non_disclosure") || d.body.includes("Non-Disclosure Agreement"),
  );
  if (ndaContracts.length > 0) {
    tests.push({
      name: "Ranking: NDA contract should rank above filing mentioning NDA",
      category: "ranking",
      query: "non-disclosure agreement confidential information mutual NDA",
      expected_hits: ndaContracts.slice(0, 1).map((d) => d.file_name),
      top_rank: 5,
      limit: 20,
    });
  }

  // a statute about patent infringement should rank higher than a memo that mentions it
  const patentStatutes = statutes.filter((d) =>
    d.body.includes("Patent Infringement") || d.body.includes("35 U.S.C."),
  );
  if (patentStatutes.length > 0) {
    tests.push({
      name: "Ranking: patent statute should rank high for patent queries",
      category: "ranking",
      query: "patent infringement 35 U.S.C. 271",
      expected_hits: patentStatutes.slice(0, 1).map((d) => d.file_name),
      top_rank: 5,
      limit: 20,
    });
  }

  // ─── 7. NEGATIVE TESTS ───────────────────────────────────────────

  // search without privileged_only should NOT filter out privileged docs
  // but search WITH privileged_only should only return privileged docs
  if (privileged.length > 0 && nonPrivileged.length > 0) {
    tests.push({
      name: "Negative: privileged_only excludes non-privileged documents",
      category: "negative",
      query: "agreement contract obligations",
      filters: { privileged_only: true },
      expected_hits: [],
      expected_misses: nonPrivileged
        .filter((d) => d.doc_type === "contract")
        .slice(0, 5)
        .map((d) => d.file_name),
      limit: 50,
    });
  }

  // completely unrelated query should not return legal docs
  tests.push({
    name: "Negative: irrelevant query returns low-relevance results",
    category: "negative",
    query: "chocolate cake recipe baking instructions vanilla frosting",
    expected_hits: [],
    limit: 10,
  });

  // filter by wrong jurisdiction should exclude docs from other jurisdictions
  const texasDocs = groundTruthDocs.filter((d) => d.jurisdiction === "Texas");
  const nonTexasDocs = groundTruthDocs.filter(
    (d) => d.jurisdiction !== "Texas" && d.jurisdiction !== "Federal",
  );
  if (texasDocs.length > 0 && nonTexasDocs.length > 0) {
    tests.push({
      name: "Negative: Texas filter excludes non-Texas documents",
      category: "negative",
      query: "breach of contract damages",
      filters: { jurisdiction: "Texas" },
      expected_hits: [],
      expected_misses: nonTexasDocs
        .filter((d) => d.jurisdiction === "Delaware" || d.jurisdiction === "New York")
        .slice(0, 5)
        .map((d) => d.file_name),
      limit: 20,
    });
  }

  return tests.filter((t) => t.expected_hits.length > 0 || (t.expected_misses && t.expected_misses.length > 0));
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

      // parse plain text results to extract file paths
      actualHits = parseAgentSearchResults(rawResults);

      // check expected hits
      if (tc.expected_hits.length > 0) {
        const found = tc.expected_hits.filter((expected) =>
          actualHits.some((hit) => hit.includes(expected) || expected.includes(hit)),
        );
        const missed = tc.expected_hits.filter(
          (expected) => !actualHits.some((hit) => hit.includes(expected) || expected.includes(hit)),
        );

        if (missed.length > 0) {
          passed = false;
          details += `Missing expected: ${missed.join(", ")}. `;
        }

        // check recall
        const recall = found.length / tc.expected_hits.length;
        details += `Recall: ${(recall * 100).toFixed(0)}% (${found.length}/${tc.expected_hits.length}). `;
      }

      // check expected misses
      if (tc.expected_misses && tc.expected_misses.length > 0) {
        expectedMissesFound = tc.expected_misses.filter((miss) =>
          actualHits.some((hit) => hit.includes(miss) || miss.includes(hit)),
        );
        if (expectedMissesFound.length > 0) {
          passed = false;
          details += `Unexpectedly found: ${expectedMissesFound.join(", ")}. `;
        }
      }

      // check ranking
      if (tc.top_rank !== undefined && tc.expected_hits.length > 0) {
        const primaryExpected = tc.expected_hits[0];
        const idx = actualHits.findIndex(
          (hit) => hit.includes(primaryExpected) || primaryExpected.includes(hit),
        );
        rankOfPrimary = idx === -1 ? undefined : idx + 1;
        if (rankOfPrimary === undefined || rankOfPrimary > tc.top_rank) {
          passed = false;
          details += `Primary result ranked ${rankOfPrimary ?? "not found"}, expected top ${tc.top_rank}. `;
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

function parseAgentSearchResults(text: string): string[] {
  // agent search returns plain text like:
  // --- result 1 (score: 0.85) ---
  // File: path/to/file.md
  // ...content...
  // or it may return JSON array
  const paths: string[] = [];

  // try JSON first
  try {
    const parsed = JSON.parse(text);
    if (Array.isArray(parsed)) {
      return parsed.map((r: any) => r.file_path || r.file_name || "").filter(Boolean);
    }
  } catch {
    // not JSON, parse plain text
  }

  // parse plain text format
  for (const line of text.split("\n")) {
    const fileMatch = line.match(/(?:File|Source|Path):\s*(.+)/i);
    if (fileMatch) {
      paths.push(fileMatch[1].trim());
    }
    // also try to catch file paths in other formats
    const pathMatch = line.match(/[a-z_]+_\d+\.md/);
    if (pathMatch && !paths.includes(pathMatch[0])) {
      paths.push(pathMatch[0]);
    }
  }

  return paths;
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
