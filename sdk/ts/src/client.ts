import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import type {
  ChatMessage,
  ChatPostBody,
  CreateProjectBody,
  CreateTaskBody,
  CreateUploadSessionBody,
  JsonObject,
  PatchTaskBody,
  Project,
  ProjectDocument,
  ProjectFile,
  ProjectFilesResponse,
  ProjectFilesSummary,
  SearchResult,
  SystemStatus,
  Task,
  TaskMessage,
  TaskOutput,
  UpdateProjectBody,
  UploadSession,
} from "./types.js";

export const DEFAULT_BASE_URL = "http://127.0.0.1:3131";
export const DEFAULT_POLL_INTERVAL_MS = 2_000;
export const DEFAULT_TIMEOUT_MS = 20 * 60 * 1000;
export const UPLOAD_CHUNK_SIZE = 256 * 1024;

export class BorgError extends Error {
  constructor(
    public readonly method: string,
    public readonly path: string,
    public readonly status: number,
    public readonly body: string,
  ) {
    super(`${method} ${path} -> ${status}: ${body}`);
    this.name = "BorgError";
  }
}

export type BorgClientConfig = {
  baseUrl?: string;
  token?: string;
  tokenFile?: string;
  tokenSearchPaths?: string[];
};

export class BorgClient {
  readonly baseUrl: string;
  private token: string;

  private constructor(baseUrl: string, token: string) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this.token = token;
  }

  static async create(config: BorgClientConfig = {}): Promise<BorgClient> {
    const baseUrl = config.baseUrl ?? process.env.BORG_BASE_URL ?? DEFAULT_BASE_URL;
    const token = await BorgClient.resolveToken(baseUrl, config);
    return new BorgClient(baseUrl, token);
  }

  private static async resolveToken(baseUrl: string, config: BorgClientConfig): Promise<string> {
    const candidates: Array<{ source: string; token: string | undefined }> = [
      { source: "config.token", token: config.token?.trim() },
      { source: "BORG_API_TOKEN", token: process.env.BORG_API_TOKEN?.trim() },
      { source: "API_TOKEN", token: process.env.API_TOKEN?.trim() },
    ];
    for (const { source, token } of candidates) {
      if (!token) continue;
      if (await BorgClient.tokenWorks(baseUrl, token)) return token;
      throw new Error(`Borg token from ${source} was rejected by ${baseUrl}/api/projects`);
    }

    const filePaths = config.tokenSearchPaths ?? [];
    if (config.tokenFile) filePaths.unshift(resolve(config.tokenFile));
    for (const path of filePaths) {
      if (!existsSync(path)) continue;
      const token = (await readFile(path, "utf8")).trim();
      if (!token) continue;
      if (await BorgClient.tokenWorks(baseUrl, token)) return token;
    }

    const url = `${baseUrl.replace(/\/+$/, "")}/api/auth/token`;
    const response = await fetch(url);
    if (response.ok) {
      const data = (await response.json()) as { token?: string };
      const token = data.token?.trim();
      if (token && (await BorgClient.tokenWorks(baseUrl, token))) return token;
    }

    throw new Error(`Could not authenticate with Borg at ${baseUrl}: no valid token found`);
  }

  private static async tokenWorks(baseUrl: string, token: string): Promise<boolean> {
    const response = await fetch(`${baseUrl.replace(/\/+$/, "")}/api/projects`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    return response.ok;
  }

  // -- low-level request helpers --

  private async request<T>(method: string, path: string, options?: {
    jsonBody?: JsonObject;
    binaryBody?: Uint8Array;
    contentType?: string;
  }): Promise<T> {
    const headers: Record<string, string> = {
      Authorization: `Bearer ${this.token}`,
    };

    let body: BodyInit | undefined;
    if (options?.jsonBody) {
      headers["Content-Type"] = "application/json";
      body = JSON.stringify(options.jsonBody);
    } else if (options?.binaryBody) {
      headers["Content-Type"] = options.contentType ?? "application/octet-stream";
      const buffer = new ArrayBuffer(options.binaryBody.byteLength);
      new Uint8Array(buffer).set(options.binaryBody);
      body = buffer;
    }

    const response = await fetch(`${this.baseUrl}${path}`, { method, headers, body });
    if (!response.ok) {
      const text = await response.text();
      throw new BorgError(method, path, response.status, text);
    }

    const text = await response.text();
    if (!text.trim()) return null as T;
    return JSON.parse(text) as T;
  }

  private async requestText(method: string, path: string): Promise<string> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      method,
      headers: { Authorization: `Bearer ${this.token}` },
    });
    if (!response.ok) {
      const text = await response.text();
      throw new BorgError(method, path, response.status, text);
    }
    return response.text();
  }

  // -- Projects --

  async listProjects(): Promise<Project[]> {
    return this.request("GET", "/api/projects");
  }

  async createProject(body: CreateProjectBody): Promise<Project> {
    return this.request("POST", "/api/projects", { jsonBody: body as unknown as JsonObject });
  }

  async getProject(projectId: number): Promise<Project> {
    return this.request("GET", `/api/projects/${projectId}`);
  }

  async updateProject(projectId: number, body: UpdateProjectBody): Promise<Project> {
    return this.request("PUT", `/api/projects/${projectId}`, {
      jsonBody: body as unknown as JsonObject,
    });
  }

  async deleteProject(projectId: number): Promise<void> {
    await this.request("DELETE", `/api/projects/${projectId}`);
  }

  async searchProjects(query: string): Promise<Project[]> {
    return this.request("GET", `/api/projects/search?q=${encodeURIComponent(query)}`);
  }

  // -- Tasks --

  async listTasks(repo?: string): Promise<Task[]> {
    const qs = repo ? `?repo=${encodeURIComponent(repo)}` : "";
    return this.request("GET", `/api/tasks${qs}`);
  }

  async createTask(body: CreateTaskBody): Promise<Task> {
    return this.request("POST", "/api/tasks/create", { jsonBody: body as unknown as JsonObject });
  }

  async getTask(taskId: number): Promise<Task> {
    return this.request("GET", `/api/tasks/${taskId}`);
  }

  async patchTask(taskId: number, body: PatchTaskBody): Promise<Task> {
    return this.request("PATCH", `/api/tasks/${taskId}`, {
      jsonBody: body as unknown as JsonObject,
    });
  }

  async getTaskOutputs(taskId: number): Promise<{ outputs: TaskOutput[] }> {
    return this.request("GET", `/api/tasks/${taskId}/outputs`);
  }

  async getTaskMessages(taskId: number): Promise<{ messages: TaskMessage[] }> {
    return this.request("GET", `/api/tasks/${taskId}/messages`);
  }

  async postTaskMessage(taskId: number, role: string, content: string): Promise<TaskMessage> {
    return this.request("POST", `/api/tasks/${taskId}/messages`, {
      jsonBody: { role, content },
    });
  }

  async approveTask(taskId: number): Promise<void> {
    await this.request("POST", `/api/tasks/${taskId}/approve`);
  }

  async rejectTask(taskId: number): Promise<void> {
    await this.request("POST", `/api/tasks/${taskId}/reject`);
  }

  async retryTask(taskId: number): Promise<void> {
    await this.request("POST", `/api/tasks/${taskId}/retry`);
  }

  async unblockTask(taskId: number, response: string): Promise<void> {
    await this.request("POST", `/api/tasks/${taskId}/unblock`, {
      jsonBody: { response },
    });
  }

  async requestTaskRevision(
    taskId: number,
    feedback: string,
  ): Promise<{ ok: boolean; target_phase: string }> {
    return this.request("POST", `/api/tasks/${taskId}/request-revision`, {
      jsonBody: { feedback },
    });
  }

  async getTaskRevisions(taskId: number): Promise<{ revisions: unknown[] }> {
    return this.request("GET", `/api/tasks/${taskId}/revisions`);
  }

  async setTaskBackend(taskId: number, backend: string): Promise<{ ok?: boolean; backend?: string }> {
    return this.request("PUT", `/api/tasks/${taskId}/backend`, {
      jsonBody: { backend },
    });
  }

  async getProjectTasks(projectId: number): Promise<Task[]> {
    return this.request("GET", `/api/projects/${projectId}/tasks`);
  }

  // -- Files & Uploads --

  async listProjectFiles(projectId: number, limit = 50): Promise<ProjectFilesResponse> {
    return this.request("GET", `/api/projects/${projectId}/files?limit=${limit}`);
  }

  async getProjectFile(projectId: number, fileId: number): Promise<ProjectFile> {
    return this.request("GET", `/api/projects/${projectId}/files/${fileId}`);
  }

  async deleteProjectFile(projectId: number, fileId: number): Promise<void> {
    await this.request("DELETE", `/api/projects/${projectId}/files/${fileId}`);
  }

  async deleteAllProjectFiles(projectId: number): Promise<void> {
    await this.request("DELETE", `/api/projects/${projectId}/files`);
  }

  async createUploadSession(
    projectId: number,
    body: CreateUploadSessionBody,
  ): Promise<UploadSession> {
    return this.request("POST", `/api/projects/${projectId}/uploads/sessions`, {
      jsonBody: body as unknown as JsonObject,
    });
  }

  async uploadChunk(
    projectId: number,
    sessionId: number,
    chunkIndex: number,
    bytes: Uint8Array,
  ): Promise<void> {
    await this.request(
      "PUT",
      `/api/projects/${projectId}/uploads/sessions/${sessionId}/chunks/${chunkIndex}`,
      { binaryBody: bytes },
    );
  }

  async completeUpload(projectId: number, sessionId: number): Promise<void> {
    await this.request(
      "POST",
      `/api/projects/${projectId}/uploads/sessions/${sessionId}/complete`,
    );
  }

  // -- Documents --

  async listProjectDocuments(projectId: number): Promise<ProjectDocument[]> {
    return this.request("GET", `/api/projects/${projectId}/documents`);
  }

  async getProjectDocumentContent(
    projectId: number,
    taskId: number,
    path: string,
    refName?: string,
  ): Promise<string> {
    const params = new URLSearchParams({ path });
    if (refName) params.set("ref_name", refName);
    return this.requestText(
      "GET",
      `/api/projects/${projectId}/documents/${taskId}/content?${params.toString()}`,
    );
  }

  async deleteProjectDocument(projectId: number, taskId: number): Promise<void> {
    await this.request("DELETE", `/api/projects/${projectId}/documents/${taskId}`);
  }

  // -- Chat --

  async postProjectChat(projectId: number, text: string, sender?: string): Promise<void> {
    const body: ChatPostBody = { text, sender };
    await this.request("POST", `/api/projects/${projectId}/chat`, {
      jsonBody: body as unknown as JsonObject,
    });
  }

  async getProjectChatMessages(projectId: number, limit = 200): Promise<ChatMessage[]> {
    return this.request("GET", `/api/projects/${projectId}/chat/messages?limit=${limit}`);
  }

  async postChat(text: string, options?: { sender?: string; thread?: string; model?: string }): Promise<void> {
    const body: ChatPostBody = { text, ...options };
    await this.request("POST", "/api/chat", { jsonBody: body as unknown as JsonObject });
  }

  async getChatMessages(thread?: string, limit = 200): Promise<ChatMessage[]> {
    const params = new URLSearchParams();
    if (thread) params.set("thread", thread);
    params.set("limit", String(limit));
    return this.request("GET", `/api/chat/messages?${params.toString()}`);
  }

  // -- Search --

  async search(
    query: string,
    options?: { project_id?: number; limit?: number; semantic?: boolean },
  ): Promise<SearchResult[]> {
    const params = new URLSearchParams({ q: query });
    if (options?.project_id != null) params.set("project_id", String(options.project_id));
    if (options?.limit != null) params.set("limit", String(options.limit));
    if (options?.semantic != null) params.set("semantic", String(options.semantic));
    return this.request("GET", `/api/search?${params.toString()}`);
  }

  // -- System --

  async getStatus(): Promise<SystemStatus> {
    return this.request("GET", "/api/status");
  }

  async getHealth(): Promise<boolean> {
    try {
      const response = await fetch(`${this.baseUrl}/api/health`);
      return response.ok;
    } catch {
      return false;
    }
  }

  async getQueue(): Promise<unknown[]> {
    return this.request("GET", "/api/queue");
  }
}
