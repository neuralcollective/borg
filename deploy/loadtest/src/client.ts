import { existsSync } from "node:fs";
import { resolve } from "node:path";
import type { SearchResult } from "./types";

// Token file locations to try, in order
const TOKEN_PATHS = [
  resolve(import.meta.dir, "../../../store/.api-token"),
  resolve(import.meta.dir, "../../../.local-borg-data/.api-token"),
];

export class BorgClient {
  private baseUrl: string;
  private token: string = "";

  constructor(baseUrl: string) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
  }

  async authenticate(): Promise<void> {
    // try reading token from file first (auth endpoint itself requires auth)
    for (const path of TOKEN_PATHS) {
      if (existsSync(path)) {
        this.token = (await Bun.file(path).text()).trim();
        if (this.token) return;
      }
    }
    // fallback: try the API endpoint (works if no auth middleware on it)
    try {
      const resp = await fetch(`${this.baseUrl}/api/auth/token`, {
        headers: this.token ? { Authorization: `Bearer ${this.token}` } : {},
      });
      if (resp.ok) {
        const data = (await resp.json()) as { token: string };
        this.token = data.token;
        return;
      }
    } catch {}
    throw new Error("Could not authenticate — no .api-token file found and /api/auth/token failed");
  }

  private headers(extra?: Record<string, string>): Record<string, string> {
    return { Authorization: `Bearer ${this.token}`, ...extra };
  }

  private async request<T>(
    method: string,
    path: string,
    body?: BodyInit | object,
    extraHeaders?: Record<string, string>,
  ): Promise<T> {
    const headers = this.headers(extraHeaders);
    let init: RequestInit = { method, headers };
    if (body !== undefined) {
      if (body instanceof Uint8Array || body instanceof ArrayBuffer || typeof body === "string") {
        init.body = body as BodyInit;
      } else {
        init.body = JSON.stringify(body);
        headers["Content-Type"] = "application/json";
      }
    }
    const resp = await fetch(`${this.baseUrl}${path}`, init);
    if (!resp.ok) {
      const text = await resp.text();
      throw new Error(`${method} ${path} → ${resp.status}: ${text}`);
    }
    const text = await resp.text();
    if (!text) return null as T;
    return JSON.parse(text) as T;
  }

  async createProject(opts: {
    name: string;
    mode: string;
    client_name: string;
    jurisdiction: string;
    matter_type: string;
  }): Promise<{ id: number }> {
    return this.request("POST", "/api/projects", opts);
  }

  async createUploadSession(
    projectId: number,
    opts: {
      file_name: string;
      mime_type: string;
      file_size: number;
      chunk_size: number;
      total_chunks: number;
      is_zip: boolean;
      privileged?: boolean;
    },
  ): Promise<{ session_id: string }> {
    return this.request("POST", `/api/projects/${projectId}/uploads/sessions`, opts);
  }

  async uploadChunk(
    projectId: number,
    sessionId: string,
    chunkIndex: number,
    data: Uint8Array,
  ): Promise<void> {
    await this.request(
      "PUT",
      `/api/projects/${projectId}/uploads/sessions/${sessionId}/chunks/${chunkIndex}`,
      data,
      { "Content-Type": "application/octet-stream" },
    );
  }

  async completeUpload(projectId: number, sessionId: string): Promise<void> {
    await this.request("POST", `/api/projects/${projectId}/uploads/sessions/${sessionId}/complete`);
  }

  async getUploadSession(
    projectId: number,
    sessionId: string,
  ): Promise<{ session: { status: string; error?: string } }> {
    return this.request("GET", `/api/projects/${projectId}/uploads/sessions/${sessionId}`);
  }

  async getProjectFiles(
    projectId: number,
    limit = 1,
  ): Promise<{ summary: { total_files: number; text_files: number } }> {
    return this.request("GET", `/api/projects/${projectId}/files?limit=${limit}`);
  }

  async search(
    query: string,
    projectId: number,
    opts?: {
      limit?: number;
      doc_type?: string;
      jurisdiction?: string;
      privileged_only?: boolean;
    },
  ): Promise<SearchResult[]> {
    const params = new URLSearchParams({
      q: query,
      project_id: String(projectId),
      limit: String(opts?.limit ?? 20),
    });
    if (opts?.doc_type) params.set("doc_type", opts.doc_type);
    if (opts?.jurisdiction) params.set("jurisdiction", opts.jurisdiction);
    if (opts?.privileged_only) params.set("privileged_only", "true");

    // agent search returns plain text, web search returns JSON
    const resp = await fetch(`${this.baseUrl}/api/search?${params}`, {
      headers: this.headers(),
    });
    if (!resp.ok) {
      const text = await resp.text();
      throw new Error(`search failed: ${resp.status}: ${text}`);
    }
    return (await resp.json()) as SearchResult[];
  }

  async agentSearch(
    query: string,
    projectId: number,
    opts?: {
      limit?: number;
      doc_type?: string;
      jurisdiction?: string;
      privileged_only?: boolean;
    },
  ): Promise<string> {
    const params = new URLSearchParams({
      q: query,
      project_id: String(projectId),
      limit: String(opts?.limit ?? 20),
    });
    if (opts?.doc_type) params.set("doc_type", opts.doc_type);
    if (opts?.jurisdiction) params.set("jurisdiction", opts.jurisdiction);
    if (opts?.privileged_only) params.set("privileged_only", "true");

    const resp = await fetch(`${this.baseUrl}/api/borgsearch/query?${params}`, {
      headers: this.headers(),
    });
    if (!resp.ok) {
      const text = await resp.text();
      throw new Error(`agent search failed: ${resp.status}: ${text}`);
    }
    return resp.text();
  }
}
