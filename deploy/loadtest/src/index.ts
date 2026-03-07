import { parseArgs } from "node:util";
import { ingestCorpus, loadGroundTruth, saveGroundTruth } from "./corpus";
import { generateBatch } from "./generators";
import { buildTestCases, runSearchTests, summarizeResults } from "./search-tests";
import { collectRealCorpus, buildRealTestCases } from "./collectors";
import type { IngestConfig, TestConfig } from "./types";

const GROUND_TRUTH_PATH = "ground-truth.json";

function usage(): never {
  console.log(`Usage: bun run src/index.ts <command> [options]

Commands:
  ingest    Generate and upload documents, then wait for indexing
  test      Run search quality tests against an existing project
  full      Ingest + test in one go
  collect   Collect real documents from CourtListener, EDGAR, Federal Register
  real      Collect + ingest + test with real documents

Options:
  --base-url <url>       Server URL (default: http://127.0.0.1:3131)
  --files <n>            Total documents to generate (default: 2000)
  --files-per-zip <n>    Documents per ZIP shard (default: 500)
  --chunk-size <n>       Upload chunk size in bytes (default: 8MB)
  --timeout <n>          Timeout in seconds (default: 3600)
  --project-name <name>  Project name (default: auto-generated)
  --project-id <id>      Project ID for test command
  --top-k <n>            Search result limit (default: 20)
  --concurrency <n>      Parallel upload shards (default: 4)

Examples:
  # Quick smoke test (2k docs)
  bun run src/index.ts full

  # Realistic load test (500k docs)
  bun run src/index.ts full --files 500000 --files-per-zip 5000 --timeout 7200

  # Ingest only, then test separately
  bun run src/index.ts ingest --files 50000
  bun run src/index.ts test --project-id 42
`);
  process.exit(1);
}

function parseOptions() {
  const { values, positionals } = parseArgs({
    args: Bun.argv.slice(2),
    options: {
      "base-url": { type: "string", default: "http://127.0.0.1:3131" },
      files: { type: "string", default: "2000" },
      "files-per-zip": { type: "string", default: "500" },
      "chunk-size": { type: "string", default: String(8 * 1024 * 1024) },
      timeout: { type: "string", default: "3600" },
      "project-name": { type: "string" },
      "project-id": { type: "string" },
      "top-k": { type: "string", default: "20" },
      concurrency: { type: "string", default: "4" },
      help: { type: "boolean", short: "h", default: false },
    },
    allowPositionals: true,
    strict: false,
  });

  if (values.help) usage();

  const command = positionals[0];
  if (!command || !["ingest", "test", "full", "collect", "real"].includes(command)) {
    usage();
  }

  return {
    command: command as "ingest" | "test" | "full" | "collect" | "real",
    baseUrl: values["base-url"]!,
    files: parseInt(values.files!, 10),
    filesPerZip: parseInt(values["files-per-zip"]!, 10),
    chunkSize: parseInt(values["chunk-size"]!, 10),
    timeoutS: parseInt(values.timeout!, 10),
    projectName: values["project-name"],
    projectId: values["project-id"] ? parseInt(values["project-id"]!, 10) : undefined,
    topK: parseInt(values["top-k"]!, 10),
    concurrency: parseInt(values.concurrency!, 10),
  };
}

async function cmdIngest(opts: ReturnType<typeof parseOptions>): Promise<number> {
  const config: IngestConfig = {
    baseUrl: opts.baseUrl,
    totalFiles: opts.files,
    filesPerZip: opts.filesPerZip,
    chunkSize: opts.chunkSize,
    timeoutMs: opts.timeoutS * 1000,
    projectName: opts.projectName,
    concurrency: opts.concurrency,
  };

  console.log(`\n=== INGEST: ${config.totalFiles.toLocaleString()} documents ===\n`);

  const { metrics, projectId, groundTruthDocs } = await ingestCorpus(config);

  await saveGroundTruth(groundTruthDocs, projectId, GROUND_TRUTH_PATH);

  console.log(`\n=== INGEST RESULTS ===`);
  console.log(`  Project ID:      ${metrics.projectId}`);
  console.log(`  Total files:     ${metrics.totalFiles.toLocaleString()}`);
  console.log(`  Total size:      ${(metrics.totalBytes / 1024 / 1024).toFixed(1)} MB (compressed)`);
  console.log(`  Generation:      ${(metrics.generationMs / 1000).toFixed(1)}s`);
  console.log(`  Upload wait:     ${(metrics.uploadMs / 1000).toFixed(1)}s`);
  console.log(`  Indexing:        ${(metrics.indexingMs / 1000).toFixed(1)}s`);
  console.log(`  Total time:      ${(metrics.totalMs / 1000).toFixed(1)}s`);
  console.log(`  Throughput:      ${metrics.filesPerSecond.toFixed(1)} files/sec`);
  console.log();

  return projectId;
}

async function cmdTest(opts: ReturnType<typeof parseOptions>, projectId?: number) {
  let pid = projectId ?? opts.projectId;
  let groundTruthDocs;

  if (!pid) {
    // try loading ground truth
    try {
      const gt = await loadGroundTruth(GROUND_TRUTH_PATH);
      pid = gt.projectId;
      groundTruthDocs = gt.docs;
    } catch {
      console.error("No project ID provided and no ground-truth.json found.");
      console.error("Run 'ingest' first, or pass --project-id.");
      process.exit(1);
    }
  }

  if (!groundTruthDocs) {
    groundTruthDocs = generateBatch(0, opts.files);
  }

  const testCases = buildTestCases(groundTruthDocs);
  console.log(`\n=== SEARCH TESTS: ${testCases.length} cases against project ${pid} ===\n`);

  const config: TestConfig = {
    baseUrl: opts.baseUrl,
    projectId: pid!,
    topK: opts.topK,
    timeoutMs: opts.timeoutS * 1000,
  };

  const results = await runSearchTests(config, testCases);
  const summary = summarizeResults(results);

  console.log(`\n=== TEST RESULTS ===`);
  console.log(`  Total:     ${summary.total}`);
  console.log(`  Passed:    ${summary.passed}`);
  console.log(`  Failed:    ${summary.failed}`);
  console.log(`  Pass rate: ${(summary.passRate * 100).toFixed(1)}%`);
  console.log();
  console.log(`  By category:`);
  for (const [cat, stats] of Object.entries(summary.byCategory)) {
    console.log(
      `    ${cat.padEnd(20)} ${stats.passed}/${stats.total} (${(stats.passRate * 100).toFixed(0)}%)`,
    );
  }
  console.log();
  console.log(`  Quality metrics:`);
  console.log(`    MRR:              ${summary.mrr.toFixed(3)}`);
  console.log(`    Chunk precision:  ${(summary.avgChunkPrecision * 100).toFixed(0)}%`);
  console.log();
  console.log(`  Latency:`);
  console.log(`    Average: ${summary.avgLatencyMs.toFixed(0)}ms`);
  console.log(`    P95:     ${summary.p95LatencyMs.toFixed(0)}ms`);
  console.log(`    P95 (steady): ${summary.p95SteadyMs.toFixed(0)}ms`);
  if (summary.settlingQueries > 0) {
    console.log(`    Settling: ${summary.settlingQueries} queries >10s (Vespa HNSW)`);
  }
  console.log(`    SLA:     ${summary.latencySlaPassed ? "PASS" : "FAIL"} (p95 steady < 500ms)`);
  console.log();

  // write detailed results
  const reportPath = "test-results.json";
  await Bun.write(reportPath, JSON.stringify({ summary, results }, null, 2));
  console.log(`Detailed results written to ${reportPath}`);

  if (summary.failed > 0) {
    console.log(`\n  Failed tests:`);
    for (const r of results.filter((r) => !r.passed)) {
      console.log(`    - ${r.name}: ${r.details}`);
    }
  }
}

async function cmdCollect(): Promise<ReturnType<typeof collectRealCorpus>> {
  return collectRealCorpus({
    opinionsPerTopic: 15,
    docketsPerTopic: 8,
    edgarPerTopic: 8,
    fedregPerTopic: 8,
    ukPerTopic: 8,
    eurlexPerTopic: 8,
    usptoPerTopic: 8,
    cachePath: "real-corpus.json",
  });
}

async function cmdRealTest(opts: ReturnType<typeof parseOptions>, projectId?: number) {
  let pid = projectId ?? opts.projectId;

  const realDocs = await cmdCollect();
  if (realDocs.length === 0) {
    console.error("No real documents collected.");
    process.exit(1);
  }

  if (!pid) {
    // Ingest real docs
    const config: IngestConfig = {
      baseUrl: opts.baseUrl,
      totalFiles: realDocs.length,
      filesPerZip: Math.min(100, realDocs.length),
      chunkSize: 2 * 1024 * 1024,
      timeoutMs: opts.timeoutS * 1000,
      projectName: opts.projectName || `Real Corpus ${new Date().toISOString().slice(0, 10)}`,
      concurrency: opts.concurrency,
    };

    console.log(`\n=== INGEST: ${realDocs.length} real documents ===\n`);
    const { metrics, projectId: newPid } = await ingestCorpus(config, realDocs);
    pid = newPid;

    await saveGroundTruth(realDocs, pid, "real-ground-truth.json");

    console.log(`\n=== INGEST RESULTS ===`);
    console.log(`  Project ID:      ${metrics.projectId}`);
    console.log(`  Total files:     ${metrics.totalFiles}`);
    console.log(`  Throughput:      ${metrics.filesPerSecond.toFixed(1)} files/sec\n`);
  }

  // Build and run real-doc test cases
  const testCases = buildRealTestCases(realDocs);
  console.log(`\n=== REAL DOC TESTS: ${testCases.length} cases against project ${pid} ===\n`);

  const config: TestConfig = {
    baseUrl: opts.baseUrl,
    projectId: pid!,
    topK: opts.topK,
    timeoutMs: opts.timeoutS * 1000,
  };

  const results = await runSearchTests(config, testCases);
  const summary = summarizeResults(results);

  console.log(`\n=== REAL DOC TEST RESULTS ===`);
  console.log(`  Total:     ${summary.total}`);
  console.log(`  Passed:    ${summary.passed}`);
  console.log(`  Failed:    ${summary.failed}`);
  console.log(`  Pass rate: ${(summary.passRate * 100).toFixed(1)}%`);
  console.log();
  console.log(`  By category:`);
  for (const [cat, stats] of Object.entries(summary.byCategory)) {
    console.log(
      `    ${cat.padEnd(20)} ${stats.passed}/${stats.total} (${(stats.passRate * 100).toFixed(0)}%)`,
    );
  }
  console.log();
  console.log(`  Quality metrics:`);
  console.log(`    MRR:              ${summary.mrr.toFixed(3)}`);
  console.log(`    Chunk precision:  ${(summary.avgChunkPrecision * 100).toFixed(0)}%`);
  console.log();
  console.log(`  Latency:`);
  console.log(`    Average: ${summary.avgLatencyMs.toFixed(0)}ms`);
  console.log(`    P95:     ${summary.p95LatencyMs.toFixed(0)}ms`);
  console.log(`    P95 (steady): ${summary.p95SteadyMs.toFixed(0)}ms`);
  console.log(`    SLA:     ${summary.latencySlaPassed ? "PASS" : "FAIL"} (p95 steady < 500ms)`);
  console.log();

  const reportPath = "real-test-results.json";
  await Bun.write(reportPath, JSON.stringify({ summary, results }, null, 2));
  console.log(`Detailed results written to ${reportPath}`);

  if (summary.failed > 0) {
    console.log(`\n  Failed tests:`);
    for (const r of results.filter((r) => !r.passed)) {
      console.log(`    - ${r.name}: ${r.details}`);
    }
  }
}

async function main() {
  const opts = parseOptions();

  switch (opts.command) {
    case "ingest": {
      await cmdIngest(opts);
      break;
    }
    case "test": {
      await cmdTest(opts);
      break;
    }
    case "full": {
      const projectId = await cmdIngest(opts);
      await cmdTest(opts, projectId);
      break;
    }
    case "collect": {
      const docs = await cmdCollect();
      console.log(`\nCollected ${docs.length} documents.`);
      break;
    }
    case "real": {
      await cmdRealTest(opts);
      break;
    }
  }
}

main().catch((err) => {
  console.error("Fatal:", err);
  process.exit(1);
});
