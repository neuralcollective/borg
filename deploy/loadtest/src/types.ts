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
    | "negative";
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
