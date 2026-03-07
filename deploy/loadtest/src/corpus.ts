import { BorgClient } from "./client";
import { generateBatch, generateDocument } from "./generators";
import type { GeneratedDoc, IngestConfig, IngestMetrics } from "./types";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

async function createZipShard(docs: GeneratedDoc[], outPath: string): Promise<number> {
  const files: Record<string, Uint8Array> = {};
  const encoder = new TextEncoder();
  for (const doc of docs) {
    files[doc.file_name] = encoder.encode(doc.body);
  }

  // Build ZIP using Bun's built-in writer
  // Bun doesn't have a native zip writer yet, so we use the standard
  // approach: write files to a temp dir, then use tar+gzip or just
  // send them as a multipart. Actually, let's build a minimal ZIP manually.
  const zipBytes = buildZip(files);
  await Bun.write(outPath, zipBytes);
  return zipBytes.length;
}

// Minimal ZIP file builder (no compression — server handles text fine uncompressed)
function buildZip(files: Record<string, Uint8Array>): Uint8Array {
  const entries: { name: Uint8Array; data: Uint8Array; offset: number }[] = [];
  const encoder = new TextEncoder();
  const parts: Uint8Array[] = [];
  let offset = 0;

  for (const [name, data] of Object.entries(files)) {
    const nameBytes = encoder.encode(name);
    // Local file header (30 bytes + name + data)
    const header = new ArrayBuffer(30);
    const view = new DataView(header);
    view.setUint32(0, 0x04034b50, true); // signature
    view.setUint16(4, 20, true); // version needed
    view.setUint16(6, 0, true); // flags
    view.setUint16(8, 0, true); // compression: store
    view.setUint16(10, 0, true); // mod time
    view.setUint16(12, 0, true); // mod date
    view.setUint32(14, crc32(data), true); // crc32
    view.setUint32(18, data.length, true); // compressed size
    view.setUint32(22, data.length, true); // uncompressed size
    view.setUint16(26, nameBytes.length, true); // name length
    view.setUint16(28, 0, true); // extra length

    entries.push({ name: nameBytes, data, offset });
    const headerBytes = new Uint8Array(header);
    parts.push(headerBytes, nameBytes, data);
    offset += headerBytes.length + nameBytes.length + data.length;
  }

  // Central directory
  const cdStart = offset;
  for (const entry of entries) {
    const cd = new ArrayBuffer(46);
    const view = new DataView(cd);
    view.setUint32(0, 0x02014b50, true); // signature
    view.setUint16(4, 20, true); // version made by
    view.setUint16(6, 20, true); // version needed
    view.setUint16(8, 0, true); // flags
    view.setUint16(10, 0, true); // compression
    view.setUint16(12, 0, true); // mod time
    view.setUint16(14, 0, true); // mod date
    view.setUint32(16, crc32(entry.data), true); // crc32
    view.setUint32(20, entry.data.length, true); // compressed
    view.setUint32(24, entry.data.length, true); // uncompressed
    view.setUint16(28, entry.name.length, true); // name len
    view.setUint16(30, 0, true); // extra len
    view.setUint16(32, 0, true); // comment len
    view.setUint16(34, 0, true); // disk start
    view.setUint16(36, 0, true); // internal attrs
    view.setUint32(38, 0, true); // external attrs
    view.setUint32(42, entry.offset, true); // local header offset

    parts.push(new Uint8Array(cd), entry.name);
    offset += 46 + entry.name.length;
  }

  // End of central directory
  const eocd = new ArrayBuffer(22);
  const eocdView = new DataView(eocd);
  eocdView.setUint32(0, 0x06054b50, true); // signature
  eocdView.setUint16(4, 0, true); // disk number
  eocdView.setUint16(6, 0, true); // cd disk
  eocdView.setUint16(8, entries.length, true); // cd entries on disk
  eocdView.setUint16(10, entries.length, true); // total cd entries
  eocdView.setUint32(12, offset - cdStart, true); // cd size
  eocdView.setUint32(16, cdStart, true); // cd offset
  eocdView.setUint16(20, 0, true); // comment length
  parts.push(new Uint8Array(eocd));

  // Concatenate
  const totalLen = parts.reduce((s, p) => s + p.length, 0);
  const result = new Uint8Array(totalLen);
  let pos = 0;
  for (const part of parts) {
    result.set(part, pos);
    pos += part.length;
  }
  return result;
}

// CRC-32 (standard ZIP polynomial)
const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    let c = i;
    for (let j = 0; j < 8; j++) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[i] = c;
  }
  return table;
})();

function crc32(data: Uint8Array): number {
  let crc = 0xffffffff;
  for (let i = 0; i < data.length; i++) {
    crc = CRC_TABLE[(crc ^ data[i]) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
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
      try {
        const status = await client.getUploadSession(projectId, sessionId);
        if (status.session.status === "done") {
          pending.delete(sessionId);
        } else if (status.session.status === "failed") {
          throw new Error(`upload session ${sessionId} failed: ${status.session.error}`);
        }
      } catch (err) {
        if (err instanceof Error && err.message.includes("failed:")) throw err;
        // transient error, retry
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
  let lastLog = 0;

  while (Date.now() < deadline) {
    try {
      const payload = await client.getProjectFiles(projectId, 1);
      const summary = payload.summary;
      if (Date.now() - lastLog > 5000) {
        process.stdout.write(
          `\r  indexed: ${summary.text_files}/${expectedFiles} files (${summary.total_files} total)   `,
        );
        lastLog = Date.now();
      }
      if (summary.total_files >= expectedFiles && summary.text_files >= expectedFiles) {
        console.log(
          `\r  indexing complete: ${summary.text_files} files indexed                    `,
        );
        return summary;
      }
    } catch {
      // transient, retry
    }
    await Bun.sleep(3000);
  }
  throw new Error(`timed out waiting for indexing`);
}

export async function ingestCorpus(config: IngestConfig, pregenDocs?: GeneratedDoc[]): Promise<{
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

  const genStart = Date.now();
  const sessionIds: string[] = [];
  const groundTruthDocs: GeneratedDoc[] = [];
  let totalBytes = 0;
  const tmpDir = await mkdtemp(join(tmpdir(), "borg-loadtest-"));

  try {
    const allDocs = pregenDocs ?? generateBatch(0, config.totalFiles);
    const totalShards = Math.ceil(allDocs.length / config.filesPerZip);

    for (let shard = 0; shard < totalShards; shard++) {
      const startId = shard * config.filesPerZip;
      const count = Math.min(config.filesPerZip, allDocs.length - startId);

      const docs = allDocs.slice(startId, startId + count);

      groundTruthDocs.push(...docs);

      const zipPath = join(tmpDir, `shard-${shard}.zip`);
      const zipSize = await createZipShard(docs, zipPath);
      totalBytes += zipSize;

      const sessionId = await uploadZipShard(client, projectId, zipPath, config.chunkSize);
      sessionIds.push(sessionId);

      console.log(
        `  shard ${shard + 1}/${totalShards}: ${count} docs, ${(zipSize / 1024 / 1024).toFixed(1)}MB → session ${sessionId}`,
      );
    }

    const genMs = Date.now() - genStart;
    console.log(`generation + upload complete in ${(genMs / 1000).toFixed(1)}s`);

    const uploadStart = Date.now();
    await waitForSessions(client, projectId, sessionIds, config.timeoutMs);
    const uploadMs = Date.now() - uploadStart;

    const indexStart = Date.now();
    await waitForIndexing(client, projectId, allDocs.length, config.timeoutMs);
    const indexMs = Date.now() - indexStart;

    const totalMs = Date.now() - totalStart;

    const metrics: IngestMetrics = {
      projectId,
      totalFiles: allDocs.length,
      totalBytes,
      generationMs: genMs,
      uploadMs,
      indexingMs: indexMs,
      totalMs,
      filesPerSecond: (allDocs.length / totalMs) * 1000,
    };

    return { metrics, projectId, groundTruthDocs };
  } finally {
    await rm(tmpDir, { recursive: true, force: true });
  }
}

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
  const docs: GeneratedDoc[] = data.docs.map((d: any, i: number) => {
    const full = generateDocument(i);
    return { ...full, ...d, body: full.body };
  });
  return { projectId: data.projectId, docs };
}
