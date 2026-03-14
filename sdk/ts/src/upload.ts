import { readFile, readdir } from "node:fs/promises";
import { extname, join, relative } from "node:path";
import { BorgClient, UPLOAD_CHUNK_SIZE, DEFAULT_POLL_INTERVAL_MS } from "./client.js";
import type { ProjectFilesSummary, UploadedFile } from "./types.js";

export function guessMimeType(path: string): string {
  const ext = extname(path).toLowerCase();
  const map: Record<string, string> = {
    ".md": "text/markdown",
    ".json": "application/json",
    ".txt": "text/plain",
    ".csv": "text/csv",
    ".xml": "application/xml",
    ".pdf": "application/pdf",
    ".doc": "application/msword",
    ".docx": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ".xlsx": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ".pptx": "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  };
  return map[ext] ?? "application/octet-stream";
}

export function isExpectedTextFile(path: string): boolean {
  const ext = extname(path).toLowerCase();
  return [".md", ".json", ".txt", ".csv", ".xml", ".pdf", ".doc", ".docx", ".xlsx", ".pptx"].includes(ext);
}

async function listFilesRecursive(dir: string): Promise<string[]> {
  const entries = await readdir(dir, { withFileTypes: true });
  const files: string[] = [];
  for (const entry of entries) {
    const fullPath = join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await listFilesRecursive(fullPath)));
    } else if (entry.isFile()) {
      files.push(fullPath);
    }
  }
  return files.sort();
}

export async function uploadDirectory(
  client: BorgClient,
  projectId: number,
  dir: string,
  options?: { chunkSize?: number; privileged?: boolean },
): Promise<UploadedFile[]> {
  const chunkSize = options?.chunkSize ?? UPLOAD_CHUNK_SIZE;
  const privileged = options?.privileged ?? false;
  const uploaded: UploadedFile[] = [];

  for (const file of await listFilesRecursive(dir)) {
    const bytes = await readFile(file);
    const relativePath = relative(dir, file).replaceAll("\\", "/");
    const mimeType = guessMimeType(file);
    const totalChunks = Math.max(1, Math.ceil(bytes.length / chunkSize));

    const session = await client.createUploadSession(projectId, {
      file_name: relativePath,
      mime_type: mimeType,
      file_size: bytes.length,
      chunk_size: chunkSize,
      total_chunks: totalChunks,
      is_zip: false,
      privileged,
    });

    for (let i = 0; i < totalChunks; i++) {
      const start = i * chunkSize;
      const end = Math.min(start + chunkSize, bytes.length);
      await client.uploadChunk(projectId, session.session_id, i, bytes.subarray(start, end));
    }
    await client.completeUpload(projectId, session.session_id);

    uploaded.push({
      relative_path: relativePath,
      mime_type: mimeType,
      size_bytes: bytes.length,
      expected_text: isExpectedTextFile(file),
    });
  }
  return uploaded;
}

export async function waitForIngestion(
  client: BorgClient,
  projectId: number,
  uploadedFiles: UploadedFile[],
  timeoutMs: number,
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
): Promise<ProjectFilesSummary> {
  const expectedTotal = uploadedFiles.length;
  const expectedText = uploadedFiles.filter((f) => f.expected_text).length;
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    const listing = await client.listProjectFiles(projectId, 5);
    const summary = listing.summary;
    if (summary && summary.total_files >= expectedTotal && summary.text_files >= expectedText) {
      return summary;
    }
    await new Promise((r) => setTimeout(r, pollIntervalMs));
  }

  throw new Error(`Timed out waiting for project ${projectId} ingestion after ${timeoutMs}ms`);
}
