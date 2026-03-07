export interface GeneratedDoc {
  file_name: string;
  body: string;
  doc_type: "contract" | "filing" | "statute" | "memo" | "data" | "document";
  jurisdiction: string;
  privileged: boolean;
  // ground truth for search verification
  ground_truth: GroundTruth;
}

export interface GroundTruth {
  // unique phrases that should match this doc and only this doc
  unique_markers: string[];
  // broader topics this doc covers (may match multiple docs)
  topics: string[];
  // specific legal citations in this doc
  citations: string[];
  // party names / entities mentioned
  entities: string[];
  // category tags for filtered search verification
  tags: string[];
}

export interface SearchTestCase {
  name: string;
  category:
    | "exact_term"
    | "semantic"
    | "filtered"
    | "multi_concept"
    | "agent_realistic"
    | "ranking"
    | "negative"
    | "chunk_precision"
    | "synonym"
    | "score_quality";
  query: string;
  filters?: {
    doc_type?: string;
    jurisdiction?: string;
    privileged_only?: boolean;
  };
  // doc file_names that MUST appear in results
  expected_hits: string[];
  // doc file_names that MUST NOT appear in results
  expected_misses?: string[];
  // if set, expected_hits[0] must be ranked at or above this position
  top_rank?: number;
  limit?: number;
  // minimum recall threshold (0-1) to pass; default 1.0 (all expected_hits required)
  min_recall?: number;
  // if set, check that returned results' doc_type matches this value
  expect_result_type?: string;
  // minimum number of results expected (for filter verification)
  min_results?: number;
  // terms that must appear in returned chunk content (not just file name)
  expected_chunk_terms?: string[];
  // minimum fraction of top results whose content must contain expected_chunk_terms
  min_chunk_precision?: number;
  // terms that should NOT appear in majority of returned chunks (wrong-section detection)
  rejected_chunk_terms?: string[];
  // max fraction of results allowed to contain rejected terms
  max_rejected_fraction?: number;
  // minimum score threshold for top result
  min_top_score?: number;
}

export interface SearchResult {
  file_path: string;
  title_snippet: string;
  content_snippet: string;
  score: number;
}

export interface IngestConfig {
  baseUrl: string;
  totalFiles: number;
  filesPerZip: number;
  chunkSize: number;
  timeoutMs: number;
  projectName?: string;
  concurrency: number;
}

export interface TestConfig {
  baseUrl: string;
  projectId: number;
  topK: number;
  timeoutMs: number;
  latencySlaMsP95?: number;
}

export interface TestResult {
  name: string;
  category: string;
  passed: boolean;
  query: string;
  expected_hits: string[];
  actual_hits: string[];
  expected_misses_found: string[];
  rank_of_primary?: number;
  latency_ms: number;
  details?: string;
  chunk_precision?: number;
  reciprocal_rank?: number;
  top_score?: number;
}

export interface IngestMetrics {
  projectId: number;
  totalFiles: number;
  totalBytes: number;
  generationMs: number;
  uploadMs: number;
  indexingMs: number;
  totalMs: number;
  filesPerSecond: number;
}
