import { BorgClient } from "./client";
import { generateBatch, generateDocument } from "./generators";
import type { GeneratedDoc, IngestConfig, IngestMetrics } from "./types";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

async function createZipShard(docs: GeneratedDoc[], outPath: string): Promise<number> {
  const proc = Bun.spawn(["zip", "-j", "-q", outPath, "-"], {
    stdin: "pipe",
  });

  // zip from stdin doesn't work well — write files to a temp dir then zip
  const dir = await mkdtemp(join(tmpdir(), "borg-shard-"));
  try {
    await Promise.all(
      docs.map((doc) => Bun.write(join(dir, doc.file_name), doc.body)),
    );
    const zip = Bun.spawn(["zip", "-j", "-q", outPath, ...docs.map((d) => join(dir, d.file_name))]);
    const code = await zip.exited;
    if (code !== 0) throw new Error(`zip failed with code ${code}`);
    const stat = await Bun.file(outPath).stat();
    return stat?.size ?? 0;
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

async function uploadZipShard(
  client: BorgClient,
  projectId: number,
  zipPath: string,
  chunkSize: number,
): Promise<string> {
  const file = Bun.file(zipPath);
  const size = file.size;
  const totalChunks = Math.ceil(size / chunkSize);

  const session = await client.createUploadSession(projectId, {
    file_name: zipPath.split("/").pop()!,
    mime_type: "application/zip",
    file_size: size,
    chunk_size: chunkSize,
    total_chunks: totalChunks,
    is_zip: true,
  });

  const bytes = new Uint8Array(await file.arrayBuffer());
  for (let i = 0; i < totalChunks; i++) {
    const start = i * chunkSize;
    const end = Math.min(start + chunkSize, size);
    await client.uploadChunk(projectId, session.session_id, i, bytes.slice(start, end));
  }

  await client.completeUpload(projectId, session.session_id);
  return session.session_id;
}

async function waitForSessions(
  client: BorgClient,
  projectId: number,
  sessionIds: string[],
  timeoutMs: number,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  const pending = new Set(sessionIds);

  while (pending.size > 0 && Date.now() < deadline) {
    for (const sessionId of [...pending]) {
      const status = await client.getUploadSession(projectId, sessionId);
      if (status.session.status === "done") {
        pending.delete(sessionId);
      } else if (status.session.status === "failed") {
        throw new Error(`upload session ${sessionId} failed: ${status.session.error}`);
      }
    }
    if (pending.size > 0) {
      process.stdout.write(`\r  waiting for ${pending.size} upload sessions to process...`);
      await Bun.sleep(3000);
    }
  }
  if (pending.size > 0) {
    throw new Error(`timed out waiting for ${pending.size} upload sessions`);
  }
  console.log("\r  all upload sessions completed                          ");
}

async function waitForIndexing(
  client: BorgClient,
  projectId: number,
  expectedFiles: number,
  timeoutMs: number,
): Promise<{ total_files: number; text_files: number }> {
  const deadline = Date.now() + timeoutMs;
  let lastTotal = 0;

  while (Date.now() < deadline) {
    const payload = await client.getProjectFiles(projectId, 1);
    const summary = payload.summary;
    if (summary.total_files !== lastTotal) {
      lastTotal = summary.total_files;
      process.stdout.write(
        `\r  indexed: ${summary.text_files}/${expectedFiles} files (${summary.total_files} total)`,
      );
    }
    if (summary.total_files >= expectedFiles && summary.text_files >= expectedFiles) {
      console.log(
        `\r  indexing complete: ${summary.text_files} files indexed                    `,
      );
      return summary;
    }
    await Bun.sleep(5000);
  }
  throw new Error(`timed out waiting for indexing (got ${lastTotal}/${expectedFiles})`);
}

export async function ingestCorpus(config: IngestConfig): Promise<{
  metrics: IngestMetrics;
  projectId: number;
  groundTruthDocs: GeneratedDoc[];
}> {
  const totalStart = Date.now();

  const client = new BorgClient(config.baseUrl);
  await client.authenticate();
  console.log("authenticated");

  const project = await client.createProject({
    name: config.projectName ?? `Load Test ${new Date().toISOString()}`,
    mode: "legal",
    client_name: "Acme Holdings International",
    jurisdiction: "Delaware",
    matter_type: "discovery",
  });
  const projectId = project.id;
  console.log(`created project ${projectId}`);

  // generate and upload in batches
  const genStart = Date.now();
  const sessionIds: string[] = [];
  const groundTruthDocs: GeneratedDoc[] = [];
  let totalBytes = 0;
  const tmpDir = await mkdtemp(join(tmpdir(), "borg-loadtest-"));

  try {
    const totalShards = Math.ceil(config.totalFiles / config.filesPerZip);

    for (let shard = 0; shard < totalShards; shard++) {
      const startId = shard * config.filesPerZip;
      const count = Math.min(config.filesPerZip, config.totalFiles - startId);

      const docs = generateBatch(startId, count);

      // keep first 200 docs as ground truth for search tests
      if (startId < 200) {
        groundTruthDocs.push(...docs.slice(0, Math.min(count, 200 - startId)));
      }

      const zipPath = join(tmpDir, `shard-${shard}.zip`);
      const zipSize = await createZipShard(docs, zipPath);
      totalBytes += zipSize;

      const sessionId = await uploadZipShard(client, projectId, zipPath, config.chunkSize);
      sessionIds.push(sessionId);

      console.log(
        `  shard ${shard + 1}/${totalShards}: ${count} docs, ${(zipSize / 1024 / 1024).toFixed(1)}MB → session ${sessionId}`,
      );

      // upload up to N shards concurrently
      if (sessionIds.length % config.concurrency === 0 && sessionIds.length < totalShards) {
        // wait for oldest batch to finish before sending more
        const batch = sessionIds.slice(-config.concurrency);
        await waitForSessions(client, projectId, batch, config.timeoutMs);
      }
    }

    const genMs = Date.now() - genStart;
    console.log(`generation + upload complete in ${(genMs / 1000).toFixed(1)}s`);

    const uploadStart = Date.now();
    await waitForSessions(client, projectId, sessionIds, config.timeoutMs);
    const uploadMs = Date.now() - uploadStart;

    const indexStart = Date.now();
    await waitForIndexing(client, projectId, config.totalFiles, config.timeoutMs);
    const indexMs = Date.now() - indexStart;

    const totalMs = Date.now() - totalStart;

    const metrics: IngestMetrics = {
      projectId,
      totalFiles: config.totalFiles,
      totalBytes,
      generationMs: genMs,
      uploadMs,
      indexingMs: indexMs,
      totalMs,
      filesPerSecond: (config.totalFiles / totalMs) * 1000,
    };

    return { metrics, projectId, groundTruthDocs };
  } finally {
    await rm(tmpDir, { recursive: true, force: true });
  }
}

// save ground truth for later test runs
export async function saveGroundTruth(
  docs: GeneratedDoc[],
  projectId: number,
  outPath: string,
): Promise<void> {
  const data = { projectId, docs: docs.map((d) => ({
    file_name: d.file_name,
    doc_type: d.doc_type,
    jurisdiction: d.jurisdiction,
    privileged: d.privileged,
    ground_truth: d.ground_truth,
  }))};
  await Bun.write(outPath, JSON.stringify(data, null, 2));
  console.log(`ground truth saved to ${outPath} (${docs.length} docs)`);
}

export async function loadGroundTruth(
  path: string,
): Promise<{ projectId: number; docs: GeneratedDoc[] }> {
  const data = JSON.parse(await Bun.file(path).text());
  // regenerate full docs from IDs for body content
  const docs: GeneratedDoc[] = data.docs.map((d: any, i: number) => {
    const full = generateDocument(i);
    return { ...full, ...d, body: full.body };
  });
  return { projectId: data.projectId, docs };
}
