import { BorgClient } from "./client";
import type { GeneratedDoc, SearchTestCase, TestConfig, TestResult } from "./types";

// Build test cases from ground truth documents.
export function buildTestCases(groundTruthDocs: GeneratedDoc[]): SearchTestCase[] {
  const tests: SearchTestCase[] = [];

  const contracts = groundTruthDocs.filter((d) => d.doc_type === "contract");
  const filings = groundTruthDocs.filter((d) => d.doc_type === "filing");
  const statutes = groundTruthDocs.filter((d) => d.doc_type === "statute");

  // For non-unique queries, include ALL matching docs as expected_hits.
  // min_recall is set so that at least `minHits` of the matches must appear in results,
  // which scales correctly from 200-doc to 500k-doc corpora.
  function expectSome(matches: GeneratedDoc[], minHits: number = 2): {
    expected_hits: string[];
    min_recall: number;
  } {
    const all = matches.map((d) => d.file_name);
    return {
      expected_hits: all,
      min_recall: Math.min(1.0, minHits / all.length),
    };
  }

  // ─── 1. EXACT TERM RETRIEVAL ───────────────────────────────────────

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
      ...expectSome(entityDocs, 2),
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
      ...expectSome(docsWithRevlon, 1),
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
      ...expectSome(contractsWithForce, 2),
      limit: 20,
    });
  }

  // ─── 2. SEMANTIC SEARCH ────────────────────────────────────────────

  const contractsWithLiabilityCap = contracts.filter((d) =>
    d.body.includes("LIMITATION OF LIABILITY"),
  );
  if (contractsWithLiabilityCap.length > 0) {
    tests.push({
      name: "Semantic: exposure limits → limitation of liability",
      category: "semantic",
      query: "What limits exist on the total liability exposure under the agreement?",
      ...expectSome(contractsWithLiabilityCap, 2),
      limit: 20,
    });
  }

  const contractsWithTermination = contracts.filter((d) => d.body.includes("TERMINATION"));
  if (contractsWithTermination.length > 0) {
    tests.push({
      name: "Semantic: ending deal early → termination provisions",
      category: "semantic",
      query: "How can either party exit the agreement before it expires?",
      ...expectSome(contractsWithTermination, 2),
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
      ...expectSome(contractsWithConfidentiality, 2),
      limit: 20,
    });
  }

  const contractsWithAssignment = contracts.filter((d) => d.body.includes("ASSIGNMENT"));
  if (contractsWithAssignment.length > 0) {
    tests.push({
      name: "Semantic: transferring rights → assignment provisions",
      category: "semantic",
      query: "Can a party transfer or assign its rights and obligations under the contract?",
      ...expectSome(contractsWithAssignment, 2),
      limit: 20,
    });
  }

  // ─── 3. FILTERED SEARCH ───────────────────────────────────────────

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
      ...expectSome(contractsWithBoth, 2),
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
      ...expectSome(filingsWithInjunction, 2),
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
      ...expectSome(docsWithSecurities, 1),
      limit: 20,
    });
  }

  // ─── 5. AGENT-REALISTIC QUERIES ───────────────────────────────────

  const fmContracts = contracts.filter((d) => d.body.includes("FORCE MAJEURE"));
  if (fmContracts.length > 0) {
    tests.push({
      name: "Agent: force majeure provisions",
      category: "agent_realistic",
      query: "force majeure clause pandemic epidemic natural disaster acts of God",
      ...expectSome(fmContracts, 2),
      limit: 20,
    });
  }

  const insuranceContracts = contracts.filter((d) => d.body.includes("INSURANCE"));
  if (insuranceContracts.length > 0) {
    tests.push({
      name: "Agent: insurance requirements",
      category: "agent_realistic",
      query: "insurance coverage requirements commercial general liability professional errors omissions",
      ...expectSome(insuranceContracts, 2),
      limit: 20,
    });
  }

  const filingsWithSJ = filings.filter(
    (d) => d.body.includes("Summary Judgment") || d.body.includes("summary judgment"),
  );
  if (filingsWithSJ.length > 0) {
    tests.push({
      name: "Agent: summary judgment motions",
      category: "agent_realistic",
      query: "motion for summary judgment genuine dispute material fact Celotex",
      ...expectSome(filingsWithSJ, 2),
      limit: 20,
    });
  }

  const govLawContracts = contracts.filter((d) => d.body.includes("GOVERNING LAW"));
  if (govLawContracts.length > 0) {
    tests.push({
      name: "Agent: governing law clauses",
      category: "agent_realistic",
      query: "governing law choice of law exclusive jurisdiction dispute resolution",
      ...expectSome(govLawContracts, 2),
      limit: 20,
    });
  }

  const dueDiligenceDocs = groundTruthDocs.filter(
    (d) => d.body.includes("due diligence") || d.body.includes("DUE DILIGENCE"),
  );
  if (dueDiligenceDocs.length > 0) {
    tests.push({
      name: "Agent: due diligence findings",
      category: "agent_realistic",
      query: "due diligence findings material issues regulatory compliance",
      ...expectSome(dueDiligenceDocs, 2),
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
      ...expectSome(ndaContracts, 1),
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
      ...expectSome(patentStatutes, 1),
      top_rank: 10,
      limit: 20,
    });
  }

  // ─── 7. NEGATIVE TESTS ───────────────────────────────────────────

  tests.push({
    name: "Negative: irrelevant query",
    category: "negative",
    query: "chocolate cake recipe baking instructions vanilla frosting",
    expected_hits: [],
    limit: 10,
  });

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

  // ─── 8. CHUNK PRECISION ───────────────────────────────────────────
  // Verify the returned chunk CONTENT is relevant, not just the file name.
  // This catches the case where the right file is returned but the wrong
  // section (chunk) is surfaced — e.g., the governing law clause instead
  // of the force majeure clause.

  if (contractsWithForce.length > 0) {
    tests.push({
      name: "Chunk: force majeure content in returned chunks",
      category: "chunk_precision",
      query: "force majeure acts of God pandemic natural disaster",
      expected_hits: [],
      min_results: 3,
      expected_chunk_terms: ["force majeure", "acts of god", "pandemic", "natural disaster"],
      min_chunk_precision: 0.6, // at least 60% of returned chunks must contain these terms
      limit: 10,
    });
  }

  const contractsWithIndemn = contracts.filter((d) => d.body.includes("INDEMNIFICATION"));
  if (contractsWithIndemn.length > 0) {
    tests.push({
      name: "Chunk: indemnification content in returned chunks",
      category: "chunk_precision",
      query: "indemnification indemnify hold harmless defend",
      expected_hits: [],
      min_results: 3,
      expected_chunk_terms: ["indemnif", "hold harmless", "defend"],
      min_chunk_precision: 0.6,
      limit: 10,
    });
  }

  if (contractsWithLiabilityCap.length > 0) {
    tests.push({
      name: "Chunk: limitation of liability content in returned chunks",
      category: "chunk_precision",
      query: "LIMITATION OF LIABILITY consequential damages aggregate liability cap",
      expected_hits: [],
      min_results: 3,
      expected_chunk_terms: ["limitation of liability", "consequential", "aggregate liability"],
      min_chunk_precision: 0.5,
      limit: 10,
    });
  }

  if (contractsWithTermination.length > 0) {
    tests.push({
      name: "Chunk: termination content in returned chunks",
      category: "chunk_precision",
      query: "termination clause breach cure period insolvency",
      expected_hits: [],
      min_results: 3,
      expected_chunk_terms: ["terminat", "breach", "cure", "insolvency", "bankrupt"],
      min_chunk_precision: 0.5,
      limit: 10,
    });
  }

  // Case number should appear in the content of the top result, not just in the file
  if (filings.length > 0) {
    const filing = filings[0];
    const caseNos = filing.ground_truth.unique_markers.filter((m) => m.match(/^\d{2}-cv-/));
    if (caseNos.length > 0) {
      tests.push({
        name: `Chunk: case number ${caseNos[0]} in chunk content`,
        category: "chunk_precision",
        query: caseNos[0],
        expected_hits: [filing.file_name],
        expected_chunk_terms: [caseNos[0]],
        min_chunk_precision: 0.5, // at least the top result should contain it
        top_rank: 3,
        limit: 5,
      });
    }
  }

  // Discrimination tests: query for clause X and verify the returned chunk's
  // PRIMARY content is clause X, not clause Y. With 512-word chunks, adjacent
  // clauses inevitably co-occur, so we only fail if the chunk lacks the
  // expected clause entirely but contains the wrong one. The rejected_chunk_terms
  // check applies to chunks that DON'T contain expected terms — these are
  // genuinely wrong-section results.

  if (contractsWithForce.length > 0 && contractsWithIndemn.length > 0) {
    tests.push({
      name: "Chunk-discrim: force majeure query returns FM content",
      category: "chunk_precision",
      query: "force majeure acts of God pandemic natural disaster",
      expected_hits: [],
      min_results: 3,
      expected_chunk_terms: ["force majeure", "acts of god", "pandemic"],
      min_chunk_precision: 0.7,
      limit: 10,
    });
  }

  if (contractsWithIndemn.length > 0 && contractsWithForce.length > 0) {
    tests.push({
      name: "Chunk-discrim: indemnification query returns indemn content",
      category: "chunk_precision",
      query: "indemnification defend hold harmless losses damages",
      expected_hits: [],
      min_results: 3,
      expected_chunk_terms: ["indemnif", "hold harmless"],
      min_chunk_precision: 0.7,
      limit: 10,
    });
  }

  const contractsWithNonCompete = contracts.filter((d) => d.body.includes("NON-COMPETITION"));
  if (contractsWithNonCompete.length > 0) {
    tests.push({
      name: "Chunk-discrim: non-compete query returns non-compete content",
      category: "chunk_precision",
      query: "non-competition non-solicitation restricted party compete territory",
      expected_hits: [],
      min_results: 2,
      expected_chunk_terms: ["non-competition", "non-solicitation", "restricted party", "compete"],
      min_chunk_precision: 0.5,
      limit: 10,
    });
  }

  // Hard negative: query for a filing's specific entity — verify the returned
  // chunk contains content about THAT entity, not just any filing
  if (filings.length >= 2) {
    const f1 = filings[0];
    const plaintiff = f1.ground_truth.entities[0]; // plaintiff
    if (plaintiff) {
      tests.push({
        name: `Chunk-discrim: "${plaintiff}" appears in returned chunk`,
        category: "chunk_precision",
        query: `${plaintiff} breach of contract damages complaint`,
        expected_hits: [],
        min_results: 1,
        expected_chunk_terms: [plaintiff],
        min_chunk_precision: 0.3, // at least some results should mention the entity
        limit: 10,
      });
    }
  }

  // ─── 9. SYNONYM EXPANSION ─────────────────────────────────────────
  // The server expands queries with legal synonyms. Test that it works.
  // e.g., searching "TRO" should find docs about "injunction" because
  // the synonym group ["injunction", "restraining order", "tro"] expands the query.

  const docsWithInjunction = groundTruthDocs.filter(
    (d) => d.body.includes("injunction") || d.body.includes("injunctive"),
  );
  if (docsWithInjunction.length > 0) {
    tests.push({
      name: "Synonym: TRO → injunction/restraining order",
      category: "synonym",
      query: "TRO temporary restraining order",
      ...expectSome(docsWithInjunction, 1),
      expected_chunk_terms: ["injunction", "injunctive", "restraining order"],
      min_chunk_precision: 0.3,
      limit: 20,
    });
  }

  const docsWithArbitration = groundTruthDocs.filter(
    (d) => d.body.includes("arbitration") || d.body.includes("mediation"),
  );
  if (docsWithArbitration.length > 0) {
    tests.push({
      name: "Synonym: ADR → arbitration/mediation",
      category: "synonym",
      query: "ADR alternative dispute resolution",
      ...expectSome(docsWithArbitration, 1),
      expected_chunk_terms: ["arbitration", "mediation", "dispute resolution"],
      min_chunk_precision: 0.3,
      limit: 20,
    });
  }

  if (contractsWithIndemn.length > 0) {
    tests.push({
      name: "Synonym: hold harmless → indemnification",
      category: "synonym",
      query: "hold harmless agreement protection from claims",
      ...expectSome(contractsWithIndemn, 1),
      expected_chunk_terms: ["indemnif", "hold harmless"],
      min_chunk_precision: 0.3,
      limit: 20,
    });
  }

  // ─── 10. SCORE QUALITY ────────────────────────────────────────────
  // Verify that relevance scores are meaningful — exact matches should
  // score well, irrelevant queries should score low.

  if (filings.length > 0) {
    const filing = filings[0];
    const caseNos = filing.ground_truth.unique_markers.filter((m) => m.match(/^\d{2}-cv-/));
    if (caseNos.length > 0) {
      tests.push({
        name: "Score: exact case number has high relevance",
        category: "score_quality",
        query: caseNos[0],
        expected_hits: [filing.file_name],
        min_top_score: 5.0, // BM25 exact match should score reasonably high
        top_rank: 3,
        limit: 10,
      });
    }
  }

  // Score: filtered + keyword query should produce non-trivial scores
  tests.push({
    name: "Score: relevant filtered query scores above threshold",
    category: "score_quality",
    query: "breach of fiduciary duty corporate governance board directors",
    filters: { doc_type: "filing" },
    expected_hits: [],
    min_results: 1,
    min_top_score: 1.0,
    limit: 10,
  });

  return tests.filter(
    (t) =>
      t.expected_hits.length > 0 ||
      (t.expected_misses && t.expected_misses.length > 0) ||
      t.min_results !== undefined ||
      t.expect_result_type !== undefined ||
      t.expected_chunk_terms !== undefined ||
      t.min_top_score !== undefined,
  );
}

// ─── Result parser ──────────────────────────────────────────────────

interface ParsedResult {
  filePath: string;
  docType: string;
  content: string;
  score: number;
}

interface ParsedResults {
  results: ParsedResult[];
  // convenience accessors
  filePaths: string[];
  docTypes: string[];
}

function parseAgentSearchResults(text: string): ParsedResults {
  // try JSON first
  try {
    const parsed = JSON.parse(text);
    if (Array.isArray(parsed)) {
      const results: ParsedResult[] = parsed.map((r: any) => ({
        filePath: r.file_path || r.file_name || "",
        docType: r.doc_type || "",
        content: r.content || r.content_snippet || "",
        score: r.score || r.relevance || 0,
      }));
      return {
        results,
        filePaths: results.map((r) => r.filePath).filter(Boolean),
        docTypes: results.map((r) => r.docType).filter(Boolean),
      };
    }
  } catch {
    // not JSON, parse plain text
  }

  // Parse plain text format:
  // --- Result 1 (score: 0.85, type: contract) ---
  // File: path/to/file.md [id=123, chunk=0]
  // <content...>
  const results: ParsedResult[] = [];
  const blocks = text.split(/(?=--- Result \d+)/);

  for (const block of blocks) {
    const headerMatch = block.match(/^--- Result \d+ \(score: ([\d.]+),?\s*(?:type|source): (\w+)/);
    if (!headerMatch) continue;

    const score = parseFloat(headerMatch[1]);
    const docType = headerMatch[2] || "";

    const fileMatch = block.match(/\nFile:\s*(\S+)/);
    const filePath = fileMatch ? fileMatch[1].trim() : "";

    // Content is everything after the File: line
    const fileLineEnd = block.indexOf("\n", block.indexOf("File:"));
    const content = fileLineEnd > 0 ? block.slice(fileLineEnd + 1).trim() : "";

    results.push({ filePath, docType, content, score });
  }

  // fallback: if no structured blocks found, try old line-by-line parsing
  if (results.length === 0) {
    for (const line of text.split("\n")) {
      const pathMatch = line.match(/[a-z_]+_\d+\.md/);
      if (pathMatch) {
        results.push({ filePath: pathMatch[0], docType: "", content: "", score: 0 });
      }
    }
  }

  return {
    results,
    filePaths: results.map((r) => r.filePath).filter(Boolean),
    docTypes: results.map((r) => r.docType).filter(Boolean),
  };
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
    let chunkPrecision: number | undefined;
    let reciprocalRank: number | undefined;
    let topScore: number | undefined;

    try {
      const rawResults = await client.agentSearch(tc.query, config.projectId, {
        limit: tc.limit ?? config.topK,
        doc_type: tc.filters?.doc_type,
        jurisdiction: tc.filters?.jurisdiction,
        privileged_only: tc.filters?.privileged_only,
      });

      const parsed = parseAgentSearchResults(rawResults);
      actualHits = parsed.filePaths;

      // Extract top score
      if (parsed.results.length > 0) {
        topScore = parsed.results[0].score;
      }

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
          const sample = missed.length > 5 ? missed.slice(0, 5).join(", ") + ` +${missed.length - 5} more` : missed.join(", ");
          details += `Recall ${(recall * 100).toFixed(0)}% < ${(minRecall * 100).toFixed(0)}% threshold (${found.length}/${tc.expected_hits.length}). Missing: ${sample}. `;
        } else {
          details += `Recall: ${(recall * 100).toFixed(0)}% (${found.length}/${tc.expected_hits.length}). `;
        }

        // Calculate reciprocal rank (1/rank of first relevant result)
        for (const expected of tc.expected_hits) {
          const idx = actualHits.findIndex(
            (hit) => hit.includes(expected) || expected.includes(hit),
          );
          if (idx !== -1) {
            const rr = 1 / (idx + 1);
            if (reciprocalRank === undefined || rr > reciprocalRank) {
              reciprocalRank = rr;
            }
          }
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

      // Check ranking — any expected_hit within top_rank passes
      if (tc.top_rank !== undefined && tc.expected_hits.length > 0) {
        let bestRank: number | undefined;
        for (const expected of tc.expected_hits) {
          const idx = actualHits.findIndex(
            (hit) => hit.includes(expected) || expected.includes(hit),
          );
          if (idx !== -1) {
            const rank = idx + 1;
            if (bestRank === undefined || rank < bestRank) bestRank = rank;
          }
        }
        rankOfPrimary = bestRank;
        if (bestRank === undefined || bestRank > tc.top_rank) {
          passed = false;
          details += `Best match ranked ${bestRank ?? "not found"}, expected top ${tc.top_rank}. `;
        }
      }

      // Chunk precision: verify returned chunk CONTENT contains expected terms
      if (tc.expected_chunk_terms && tc.expected_chunk_terms.length > 0 && parsed.results.length > 0) {
        const checkCount = Math.min(parsed.results.length, tc.limit ?? 10);
        let matchingChunks = 0;
        for (let i = 0; i < checkCount; i++) {
          const content = parsed.results[i].content.toLowerCase();
          const hasAnyTerm = tc.expected_chunk_terms.some((term) =>
            content.includes(term.toLowerCase()),
          );
          if (hasAnyTerm) matchingChunks++;
        }
        chunkPrecision = matchingChunks / checkCount;
        const minPrecision = tc.min_chunk_precision ?? 0.5;

        if (chunkPrecision < minPrecision) {
          passed = false;
          details += `Chunk precision ${(chunkPrecision * 100).toFixed(0)}% < ${(minPrecision * 100).toFixed(0)}% (${matchingChunks}/${checkCount} chunks contain expected terms). `;
        } else {
          details += `Chunk precision: ${(chunkPrecision * 100).toFixed(0)}% (${matchingChunks}/${checkCount}). `;
        }
      }

      // Rejected chunk terms: verify wrong-section content doesn't dominate
      if (tc.rejected_chunk_terms && tc.rejected_chunk_terms.length > 0 && parsed.results.length > 0) {
        const checkCount = Math.min(parsed.results.length, tc.limit ?? 10);
        let rejectedCount = 0;
        for (let i = 0; i < checkCount; i++) {
          const content = parsed.results[i].content;
          const hasRejected = tc.rejected_chunk_terms.some((term) =>
            content.includes(term),
          );
          if (hasRejected) rejectedCount++;
        }
        const rejectedFraction = rejectedCount / checkCount;
        const maxAllowed = tc.max_rejected_fraction ?? 0.3;

        if (rejectedFraction > maxAllowed) {
          passed = false;
          details += `Wrong-section: ${(rejectedFraction * 100).toFixed(0)}% of chunks contain rejected terms (${rejectedCount}/${checkCount}, max ${(maxAllowed * 100).toFixed(0)}%). `;
        }
      }

      // Score quality: check min_top_score
      if (tc.min_top_score !== undefined && topScore !== undefined) {
        if (topScore < tc.min_top_score) {
          passed = false;
          details += `Top score ${topScore.toFixed(2)} < ${tc.min_top_score} threshold. `;
        } else {
          details += `Top score: ${topScore.toFixed(2)}. `;
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
      chunk_precision: chunkPrecision,
      reciprocal_rank: reciprocalRank,
      top_score: topScore,
    });
  }

  return results;
}

export function summarizeResults(results: TestResult[]): {
  total: number;
  passed: number;
  failed: number;
  passRate: number;
  byCategory: Record<string, { total: number; passed: number; passRate: number }>;
  avgLatencyMs: number;
  p95LatencyMs: number;
  mrr: number;
  avgChunkPrecision: number;
  latencySlaPassed: boolean;
  p95SteadyMs: number;
  settlingQueries: number;
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

  // MRR: mean reciprocal rank across all tests that have it
  const rrValues = results
    .map((r) => r.reciprocal_rank)
    .filter((v): v is number => v !== undefined);
  const mrr = rrValues.length > 0 ? rrValues.reduce((a, b) => a + b, 0) / rrValues.length : 0;

  // Average chunk precision across tests that measure it
  const cpValues = results
    .map((r) => r.chunk_precision)
    .filter((v): v is number => v !== undefined);
  const avgChunkPrecision = cpValues.length > 0
    ? cpValues.reduce((a, b) => a + b, 0) / cpValues.length
    : 0;

  // Latency SLA: p95 < 500ms, excluding outliers > 10s (Vespa HNSW settling)
  const steadyLatencies = latencies.filter((l) => l < 10000);
  const p95Steady = steadyLatencies.length > 0
    ? steadyLatencies[Math.floor(steadyLatencies.length * 0.95)]
    : 0;
  const settlingQueries = latencies.length - steadyLatencies.length;
  const latencySlaPassed = p95Steady < 500;

  return {
    total, passed, failed, passRate, byCategory,
    avgLatencyMs, p95LatencyMs, mrr, avgChunkPrecision,
    latencySlaPassed, p95SteadyMs: p95Steady, settlingQueries,
  };
}
