import { useQuery, useQueryClient, useMutation } from "@tanstack/react-query";
import { useEffect, useRef, useCallback, useState } from "react";
import type {
  Task,
  TaskDetail,
  TaskOutput,
  QueueEntry,
  Status,
  LogEvent,
  Proposal,
  PipelineMode,
  TaskMessage,
  Project,
  ProjectFile,
  ProjectTask,
  ProjectDocument,
  PipelineModeFull,
  KnowledgeFile,
} from "./types";
import {
  MAX_LOG_BUFFER,
  MAX_STREAM_EVENTS,
  REFETCH_TASKS,
  REFETCH_TASK_DETAIL,
  REFETCH_QUEUE,
  REFETCH_STATUS,
  REFETCH_PROPOSALS,
  REFETCH_PROJECTS,
  REFETCH_TASK_MESSAGES,
} from "./constants";

// Runtime base URL: set window.__BORG_API_URL__ = "https://api.example.com" in a <script> before the app loads.
// Falls back to same-origin (empty string) which works in dev via the Vite proxy.
export function apiBase(): string {
  return (window as any).__BORG_API_URL__ || "";
}

// Fetched once at module load; null if server doesn't require auth or unreachable
let authToken: string | null = null;
export const tokenReady: Promise<void> = fetch(`${apiBase()}/api/auth/token`)
  .then((r) => (r.ok ? r.json() : null))
  .then((data: { token: string } | null) => {
    if (data?.token) authToken = data.token;
  })
  .catch(() => {});

export function authHeaders(): Record<string, string> {
  return authToken ? { Authorization: `Bearer ${authToken}` } : {};
}

export function sseUrl(path: string): string {
  const url = `${apiBase()}${path}`;
  return authToken ? `${url}${url.includes("?") ? "&" : "?"}token=${authToken}` : url;
}

async function fetchJson<T>(path: string): Promise<T> {
  await tokenReady;
  const res = await fetch(`${apiBase()}${path}`, { headers: authHeaders() });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

async function apiFetch(path: string, init?: RequestInit): Promise<Response> {
  await tokenReady;
  const { headers: extraHeaders, ...rest } = init ?? {};
  return fetch(`${apiBase()}${path}`, {
    headers: { ...authHeaders(), ...(extraHeaders as Record<string, string> | undefined) },
    ...rest,
  });
}

function normalizeLogEvent(raw: unknown): LogEvent | null {
  if (!raw || typeof raw !== "object") return null;
  const data = raw as Record<string, unknown>;
  const level = typeof data.level === "string" && data.level.length > 0 ? data.level : "info";
  const message = typeof data.message === "string" ? data.message : "";

  let ts: number | null = null;
  if (typeof data.ts === "number" && Number.isFinite(data.ts)) ts = data.ts;
  if (typeof data.ts === "string") {
    const parsed = Number(data.ts);
    if (Number.isFinite(parsed)) ts = parsed;
  }
  if (ts === null) ts = Math.floor(Date.now() / 1000);

  return {
    level,
    message,
    ts,
    category: typeof data.category === "string" ? data.category : undefined,
    metadata: typeof data.metadata === "string" ? data.metadata : undefined,
  };
}

export function useTasks() {
  return useQuery<Task[]>({
    queryKey: ["tasks"],
    queryFn: () => fetchJson("/api/tasks"),
    refetchInterval: REFETCH_TASKS,
  });
}

export function useTaskDetail(id: number | null) {
  return useQuery<TaskDetail>({
    queryKey: ["task", id],
    queryFn: () => fetchJson(`/api/tasks/${id}`),
    enabled: id !== null,
    refetchInterval: REFETCH_TASK_DETAIL,
  });
}

export async function getTaskStructuredData(id: number): Promise<Record<string, unknown> | null> {
  const detail: TaskDetail = await fetchJson(`/api/tasks/${id}`);
  return detail.structured_data ?? null;
}

export interface TaskDiagnosticsSummary {
  attempt: number;
  max_attempts: number;
  status: string;
  review_status?: string | null;
  started_at?: string | null;
  completed_at?: string | null;
  duration_secs?: number | null;
  stuck_suspected: boolean;
  same_failure_streak: number;
  has_queue_entry: boolean;
}

export interface TaskDiagnosticsEvent {
  id: number;
  task_id: number | null;
  project_id: number | null;
  actor: string;
  kind: string;
  payload: string;
  created_at: string;
}

export interface TaskDiagnostics {
  task: Task;
  summary: TaskDiagnosticsSummary;
  queue_entries: QueueEntry[];
  recent_outputs: Array<Pick<TaskOutput, "id" | "phase" | "output" | "exit_code" | "created_at">>;
  recent_events: TaskDiagnosticsEvent[];
}

export async function getTaskDiagnostics(id: number): Promise<TaskDiagnostics> {
  return fetchJson(`/api/tasks/${id}/diagnostics`);
}

export function useQueue() {
  return useQuery<QueueEntry[]>({
    queryKey: ["queue"],
    queryFn: () => fetchJson("/api/queue"),
    refetchInterval: REFETCH_QUEUE,
  });
}

export function useStatus() {
  return useQuery<Status>({
    queryKey: ["status"],
    queryFn: () => fetchJson("/api/status"),
    refetchInterval: REFETCH_STATUS,
  });
}

export function useProposals() {
  return useQuery<Proposal[]>({
    queryKey: ["proposals"],
    queryFn: () => fetchJson("/api/proposals"),
    refetchInterval: REFETCH_PROPOSALS,
  });
}

export function useModes() {
  return useQuery<PipelineMode[]>({
    queryKey: ["modes"],
    queryFn: () => fetchJson("/api/modes"),
    staleTime: 300_000,
  });
}

export function useFullModes() {
  return useQuery<PipelineModeFull[]>({
    queryKey: ["modes_full"],
    queryFn: () => fetchJson("/api/modes/full"),
    staleTime: 30_000,
  });
}

export function useCustomModes() {
  return useQuery<PipelineModeFull[]>({
    queryKey: ["modes_custom"],
    queryFn: () => fetchJson("/api/modes/custom"),
    staleTime: 30_000,
  });
}

export async function saveCustomMode(mode: PipelineModeFull): Promise<{ ok: boolean }> {
  const res = await apiFetch("/api/modes/custom", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(mode),
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function removeCustomMode(name: string): Promise<{ ok: boolean }> {
  const res = await apiFetch(`/api/modes/custom/${encodeURIComponent(name)}`, { method: "DELETE" });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export interface Settings {
  continuous_mode: boolean;
  release_interval_mins: number;
  pipeline_max_backlog: number;
  agent_timeout_s: number;
  pipeline_seed_cooldown_s: number;
  pipeline_tick_s: number;
  model: string;
  backend: string;
  container_memory_mb: number;
  assistant_name: string;
  pipeline_max_agents: number;
  proposal_promote_threshold: number;
  git_claude_coauthor: boolean;
  git_user_coauthor: string;
  chat_disallowed_tools: string;
  pipeline_disallowed_tools: string;
  public_url: string;
  dropbox_client_id: string;
  dropbox_client_secret: string;
  google_client_id: string;
  google_client_secret: string;
  ms_client_id: string;
  ms_client_secret: string;
  storage_backend: string;
  s3_bucket: string;
  s3_region: string;
  s3_endpoint: string;
  s3_prefix: string;
  project_max_bytes: number;
  knowledge_max_bytes: number;
  cloud_import_max_batch_files: number;
  ingestion_queue_backend: string;
  sqs_queue_url: string;
  sqs_region: string;
  search_backend: string;
  opensearch_url: string;
  opensearch_index: string;
  opensearch_username: string;
}

export interface PlanTodo {
  id: number;
  title: string;
  details: string;
  status: "todo" | "doing" | "blocked" | "done";
  priority: number;
  created_at: string;
  updated_at: string;
}

export interface CreatePlanTodoInput {
  title: string;
  details?: string;
  status?: "todo" | "doing" | "blocked" | "done";
  priority?: number;
}

export function useSettings() {
  return useQuery<Settings>({
    queryKey: ["settings"],
    queryFn: () => fetchJson("/api/settings"),
    staleTime: 60_000,
  });
}

export async function updateSettings(settings: Partial<Settings>): Promise<{ updated: number }> {
  const res = await apiFetch("/api/settings", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(settings),
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function getPlanTodos(status?: PlanTodo["status"]): Promise<PlanTodo[]> {
  const qs = status ? `?status=${encodeURIComponent(status)}` : "";
  return fetchJson(`/api/plan/todos${qs}`);
}

export async function createPlanTodo(input: CreatePlanTodoInput): Promise<{ id: number }> {
  const res = await apiFetch("/api/plan/todos", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(input),
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function bulkUpsertPlanTodos(
  items: CreatePlanTodoInput[],
): Promise<{ upserted: number; ids: number[] }> {
  const res = await apiFetch("/api/plan/todos/bulk_upsert", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ items }),
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function updatePlanTodo(id: number, input: Partial<CreatePlanTodoInput>): Promise<void> {
  const res = await apiFetch(`/api/plan/todos/${id}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(input),
  });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function deletePlanTodo(id: number): Promise<void> {
  const res = await apiFetch(`/api/plan/todos/${id}`, { method: "DELETE" });
  if (!res.ok) throw new Error(`${res.status}`);
}

export function useFocus() {
  return useQuery<{ text: string; active: boolean }>({
    queryKey: ["focus"],
    queryFn: () => fetchJson("/api/focus"),
    staleTime: 10_000,
  });
}

export async function setFocus(text: string): Promise<void> {
  const res = await apiFetch("/api/focus", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ text }),
  });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function clearFocus(): Promise<void> {
  const res = await apiFetch("/api/focus", { method: "DELETE" });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function approveProposal(id: number): Promise<{ task_id: number }> {
  const res = await apiFetch(`/api/proposals/${id}/approve`, { method: "POST" });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function dismissProposal(id: number): Promise<void> {
  const res = await apiFetch(`/api/proposals/${id}/dismiss`, { method: "POST" });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function triageProposals(): Promise<{ scored: number }> {
  const res = await apiFetch("/api/proposals/triage", { method: "POST" });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function reopenProposal(id: number): Promise<void> {
  const res = await apiFetch(`/api/proposals/${id}/reopen`, { method: "POST" });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function patchTask(id: number, patch: { title?: string; description?: string }): Promise<void> {
  const res = await apiFetch(`/api/tasks/${id}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(patch),
  });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function retryTask(id: number): Promise<void> {
  const res = await apiFetch(`/api/tasks/${id}/retry`, { method: "POST" });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function approveTask(id: number): Promise<{ next_phase: string }> {
  const res = await apiFetch(`/api/tasks/${id}/approve`, { method: "POST" });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function rejectTask(id: number, feedback?: string): Promise<void> {
  const res = await apiFetch(`/api/tasks/${id}/reject`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ feedback }),
  });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function requestRevision(id: number, feedback: string): Promise<{ target_phase: string }> {
  const res = await apiFetch(`/api/tasks/${id}/request-revision`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ feedback }),
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export interface RevisionRound {
  round: number;
  feedback: string | null;
  feedback_at: string | null;
  phases: Array<{
    phase: string;
    exit_code: number;
    output_preview: string;
    created_at: string;
  }>;
}

export interface RevisionHistory {
  task_id: number;
  revision_count: number;
  review_status: string | null;
  rounds: RevisionRound[];
}

export async function getRevisionHistory(taskId: number): Promise<RevisionHistory> {
  return fetchJson(`/api/tasks/${taskId}/revisions`);
}

export interface CitationVerification {
  id: number;
  task_id: number;
  citation_text: string;
  citation_type: string;
  status: string;
  source: string;
  treatment: string;
  checked_at: string;
  created_at: string;
}

export async function getTaskCitations(id: number): Promise<CitationVerification[]> {
  return fetchJson(`/api/tasks/${id}/citations`);
}

export async function verifyTaskCitations(id: number): Promise<{ verified: number; total: number; citations: CitationVerification[] }> {
  const res = await apiFetch(`/api/tasks/${id}/verify-citations`, { method: "POST" });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function retryAllFailed(): Promise<void> {
  const res = await apiFetch("/api/tasks/retry-all-failed", { method: "POST" });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function setTaskBackend(id: number, backend: string): Promise<void> {
  const res = await apiFetch(`/api/tasks/${id}/backend`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ backend }),
  });
  if (!res.ok) throw new Error(`${res.status}`);
}

export interface RepoInfo {
  id: number;
  path: string;
  name: string;
  mode: string;
  backend: string | null;
  test_cmd: string;
  auto_merge: boolean;
}

export function useRepos() {
  return useQuery<RepoInfo[]>({
    queryKey: ["repos"],
    queryFn: () => fetchJson("/api/repos"),
    staleTime: 30_000,
  });
}

export async function setRepoBackend(id: number, backend: string): Promise<void> {
  const res = await apiFetch(`/api/repos/${id}/backend`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ backend }),
  });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function createTask(
  title: string,
  description: string,
  mode: string,
  repo_path?: string,
  project_id?: number,
  task_type?: string
): Promise<{ id: number }> {
  const res = await apiFetch("/api/tasks/create", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ title, description, mode, repo: repo_path, project_id, task_type }),
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export function useLogs() {
  const [logs, setLogs] = useState<LogEvent[]>([]);
  const [connected, setConnected] = useState(false);
  const esRef = useRef<EventSource | null>(null);
  const invalidateTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const retryTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const retriesRef = useRef(0);
  const queryClient = useQueryClient();

  const connect = useCallback(() => {
    if (esRef.current) esRef.current.close();
    // Wait for auth token before opening SSE (EventSource can't set headers)
    tokenReady.then(() => {
      const es = new EventSource(sseUrl("/api/logs"));
      esRef.current = es;

      es.onopen = () => {
        setConnected(true);
        retriesRef.current = 0;
      };
      es.onerror = () => {
        setConnected(false);
        es.close();
        esRef.current = null;
        if (retriesRef.current < 10) {
          const delay = Math.min(1000 * Math.pow(2, retriesRef.current), 30000);
          retriesRef.current++;
          retryTimer.current = setTimeout(connect, delay);
        }
      };
      es.onmessage = (e) => {
        try {
          const d = normalizeLogEvent(JSON.parse(e.data));
          if (!d) return;
          setLogs((prev) => {
            const next = [...prev, d];
            return next.length > MAX_LOG_BUFFER ? next.slice(-MAX_LOG_BUFFER) : next;
          });
          if (!invalidateTimer.current) {
            invalidateTimer.current = setTimeout(() => {
              queryClient.invalidateQueries({ queryKey: ["tasks"] });
              queryClient.invalidateQueries({ queryKey: ["status"] });
              invalidateTimer.current = null;
            }, 1000);
          }
        } catch {
          // ignore parse errors
        }
      };
    });
  }, [queryClient]);

  useEffect(() => {
    connect();
    return () => {
      esRef.current?.close();
      if (invalidateTimer.current) clearTimeout(invalidateTimer.current);
      if (retryTimer.current) clearTimeout(retryTimer.current);
    };
  }, [connect]);

  return { logs, connected };
}

export interface StreamEvent {
  type: string;
  subtype?: string;
  message?: { content: string | Array<{ type: string; text?: string; name?: string; input?: unknown }> };
  result?: string;
  session_id?: string;
  tool_name?: string;
  name?: string;
  content?: unknown;
  output?: unknown;
  phase?: string;
  // container_event fields
  event?: string;
  image?: string;
  repo?: string;
  branch?: string;
  duration_ms?: number;
  exit_code?: number;
  stderr_tail?: string;
  id?: string;
}

export function useProjects() {
  return useQuery<Project[]>({
    queryKey: ["projects"],
    queryFn: () => fetchJson("/api/projects"),
    refetchInterval: REFETCH_PROJECTS,
  });
}

export interface CreateProjectOptions {
  mode?: string;
  client_name?: string;
  opposing_counsel?: string;
  jurisdiction?: string;
  matter_type?: string;
  privilege_level?: string;
}

export interface ConflictHit {
  project_id: number;
  project_name: string;
  party_name: string;
  party_role: string;
  matched_field: string;
}

export async function createProject(
  name: string,
  mode = "general",
  opts: CreateProjectOptions = {}
): Promise<{ id: number; conflicts?: ConflictHit[] }> {
  const res = await apiFetch("/api/projects", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name, mode, ...opts }),
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function checkConflicts(
  clientName: string,
  opposingCounsel: string,
  excludeProjectId?: number
): Promise<ConflictHit[]> {
  const params = new URLSearchParams();
  if (clientName) params.set("client_name", clientName);
  if (opposingCounsel) params.set("opposing_counsel", opposingCounsel);
  if (excludeProjectId) params.set("exclude_project_id", String(excludeProjectId));
  const res = await apiFetch(`/api/projects/conflicts?${params}`);
  if (!res.ok) return [];
  const data = await res.json();
  return data.conflicts || [];
}

export function useProjectFiles(projectId: number | null) {
  return useQuery<ProjectFile[]>({
    queryKey: ["project_files", projectId],
    queryFn: () => fetchJson(`/api/projects/${projectId}/files`),
    enabled: projectId !== null,
    refetchInterval: REFETCH_PROJECTS,
  });
}

export async function fetchProjectFileContent(
  projectId: number,
  fileId: number
): Promise<ArrayBuffer> {
  const res = await apiFetch(`/api/projects/${projectId}/files/${fileId}/content`);
  if (!res.ok) throw new Error(`Failed to load file (${res.status})`);
  return res.arrayBuffer();
}

export async function uploadProjectFiles(
  projectId: number,
  files: FileList | File[]
): Promise<{ uploaded: ProjectFile[] }> {
  await tokenReady;
  const form = new FormData();
  Array.from(files).forEach((file) => form.append("files", file));
  const res = await fetch(`${apiBase()}/api/projects/${projectId}/files/upload`, {
    method: "POST",
    headers: authHeaders(),
    body: form,
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export interface CloudConnection {
  id: number;
  provider: "dropbox" | "google_drive" | "onedrive" | string;
  account_email: string;
  connected_at: string;
}

export interface CloudBrowseItem {
  id: string;
  name: string;
  type: "file" | "folder";
  size?: number;
  modified?: string;
  mime_type?: string;
}

export interface CloudBrowseResponse {
  items: CloudBrowseItem[];
  cursor?: string | null;
  next_page_token?: string | null;
  has_more?: boolean;
  folder_id?: string;
}

export function useProjectCloudConnections(projectId: number | null) {
  return useQuery<CloudConnection[]>({
    queryKey: ["project_cloud_connections", projectId],
    queryFn: () => fetchJson(`/api/projects/${projectId}/cloud`),
    enabled: projectId !== null,
    refetchInterval: REFETCH_PROJECTS,
  });
}

export async function deleteProjectCloudConnection(projectId: number, connectionId: number): Promise<void> {
  const res = await apiFetch(`/api/projects/${projectId}/cloud/${connectionId}`, {
    method: "DELETE",
  });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function browseProjectCloudFiles(
  projectId: number,
  connectionId: number,
  opts: { folder_id?: string; cursor?: string } = {}
): Promise<CloudBrowseResponse> {
  const params = new URLSearchParams();
  if (opts.folder_id) params.set("folder_id", opts.folder_id);
  if (opts.cursor) params.set("cursor", opts.cursor);
  const query = params.toString();
  return fetchJson(`/api/projects/${projectId}/cloud/${connectionId}/browse${query ? `?${query}` : ""}`);
}

export async function importProjectCloudFiles(
  projectId: number,
  connectionId: number,
  files: Array<{ id: string; name: string; size?: number }>
): Promise<{ imported: ProjectFile[] }> {
  const res = await apiFetch(`/api/projects/${projectId}/cloud/${connectionId}/import`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ files }),
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function fetchProjectFileText(
  projectId: number,
  fileId: number
): Promise<{ id: number; file_name: string; extracted_text: string; has_text: boolean }> {
  return fetchJson(`/api/projects/${projectId}/files/${fileId}/text`);
}

export async function reextractProjectFile(
  projectId: number,
  fileId: number
): Promise<{ id: number; extracted_text_chars: number; has_text: boolean }> {
  const res = await apiFetch(`/api/projects/${projectId}/files/${fileId}/reextract`, {
    method: "POST",
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

// ── Deadlines ──────────────────────────────────────────────────────────

export interface Deadline {
  id: number;
  project_id: number;
  label: string;
  due_date: string;
  rule_basis: string;
  status: string;
  created_at?: string;
  project_name?: string;
}

export function useProjectDeadlines(projectId: number | null) {
  return useQuery<Deadline[]>({
    queryKey: ["project_deadlines", projectId],
    queryFn: () => fetchJson(`/api/projects/${projectId}/deadlines`),
    enabled: projectId !== null,
    refetchInterval: REFETCH_PROJECTS,
  });
}

export async function createDeadline(projectId: number, label: string, dueDate: string, ruleBasis?: string): Promise<{ id: number }> {
  const res = await apiFetch(`/api/projects/${projectId}/deadlines`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ label, due_date: dueDate, rule_basis: ruleBasis || "" }),
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function updateDeadline(projectId: number, id: number, updates: Partial<{ label: string; due_date: string; rule_basis: string; status: string }>): Promise<void> {
  const res = await apiFetch(`/api/projects/${projectId}/deadlines/${id}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(updates),
  });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function deleteDeadline(projectId: number, id: number): Promise<void> {
  const res = await apiFetch(`/api/projects/${projectId}/deadlines/${id}`, {
    method: "DELETE",
  });
  if (!res.ok) throw new Error(`${res.status}`);
}

export function useTemplates(category?: string) {
  return useQuery<KnowledgeFile[]>({
    queryKey: ["templates", category],
    queryFn: () => {
      const params = new URLSearchParams();
      if (category) params.set("category", category);
      return fetchJson(`/api/knowledge/templates?${params}`);
    },
    refetchInterval: REFETCH_PROJECTS,
  });
}

export function useUpcomingDeadlines(limit = 50) {
  return useQuery<Deadline[]>({
    queryKey: ["upcoming_deadlines", limit],
    queryFn: () => fetchJson(`/api/deadlines?limit=${limit}`),
    refetchInterval: REFETCH_PROJECTS,
  });
}

// ── Full-text search ──────────────────────────────────────────────────

export interface FtsSearchResult {
  project_id: number;
  project_name?: string;
  task_id: number;
  file_path: string;
  title_snippet?: string;
  content_snippet: string;
  rank?: number;
  score?: number;
  source?: "keyword" | "semantic";
}

// ── Audit ─────────────────────────────────────────────────────────────

export interface AuditEvent {
  id: number;
  task_id: number | null;
  project_id: number | null;
  actor: string;
  kind: string;
  payload: string;
  created_at: string;
}

export function useProjectAudit(projectId: number | null) {
  return useQuery<AuditEvent[]>({
    queryKey: ["project_audit", projectId],
    queryFn: () => fetchJson(`/api/projects/${projectId}/audit`),
    enabled: projectId !== null,
    refetchInterval: REFETCH_PROJECTS,
  });
}

export async function searchDocuments(query: string, projectId?: number, semantic = true): Promise<FtsSearchResult[]> {
  const params = new URLSearchParams({ q: query });
  if (projectId) params.set("project_id", String(projectId));
  if (semantic) params.set("semantic", "true");
  return fetchJson(`/api/search?${params}`);
}

export function useProjectTasks(projectId: number | null) {
  return useQuery<ProjectTask[]>({
    queryKey: ["project_tasks", projectId],
    queryFn: () => fetchJson(`/api/projects/${projectId}/tasks`),
    enabled: projectId !== null,
    refetchInterval: REFETCH_PROJECTS,
  });
}

export function useProjectDocuments(projectId: number | null) {
  return useQuery<ProjectDocument[]>({
    queryKey: ["project_documents", projectId],
    queryFn: () => fetchJson(`/api/projects/${projectId}/documents`),
    enabled: projectId !== null,
    refetchInterval: REFETCH_PROJECTS,
  });
}

export function useProjectDocumentVersions(projectId: number | null, taskId: number | null, path: string | null) {
  return useQuery<{ sha: string; message: string; date: string; author: string }[]>({
    queryKey: ["project_doc_versions", projectId, taskId, path],
    queryFn: () => fetchJson(`/api/projects/${projectId}/documents/${taskId}/versions?path=${encodeURIComponent(path!)}`),
    enabled: projectId !== null && taskId !== null && !!path,
  });
}

export function useProjectDetail(projectId: number | null) {
  return useQuery<Project>({
    queryKey: ["project", projectId],
    queryFn: () => fetchJson(`/api/projects/${projectId}`),
    enabled: projectId !== null,
    refetchInterval: REFETCH_PROJECTS,
  });
}

export function useUpdateProject(projectId: number) {
  const queryClient = useQueryClient();
  return useMutation<Project, Error, Partial<Project>>({
    mutationFn: async (patch) => {
      const res = await apiFetch(`/api/projects/${projectId}`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(patch),
      });
      if (!res.ok) throw new Error(`${res.status}`);
      return res.json();
    },
    onSuccess: (data) => {
      queryClient.setQueryData(["project", projectId], data);
      queryClient.invalidateQueries({ queryKey: ["projects"] });
    },
  });
}

export function useDeleteProject() {
  const queryClient = useQueryClient();
  return useMutation<void, Error, number>({
    mutationFn: async (projectId) => {
      const res = await apiFetch(`/api/projects/${projectId}`, { method: "DELETE" });
      if (!res.ok) throw new Error(`${res.status}`);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["projects"] });
    },
  });
}

export async function getProjectChatMessages(
  projectId: number
): Promise<Array<{ role: "user" | "assistant"; sender?: string; text: string; ts: string | number; thread?: string }>> {
  return fetchJson(`/api/projects/${projectId}/chat/messages`);
}

export async function sendProjectChat(
  projectId: number,
  text: string,
  sender = "web-user"
): Promise<{ ok: boolean }> {
  const res = await apiFetch(`/api/projects/${projectId}/chat`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ text, sender }),
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export function useTaskMessages(taskId: number | null) {
  return useQuery<TaskMessage[]>({
    queryKey: ["task_messages", taskId],
    queryFn: async () => {
      if (!taskId) return [];
      try {
        const res = await apiFetch(`/api/tasks/${taskId}/messages`);
        if (!res.ok) return [];
        const data = await res.json();
        return data.messages ?? [];
      } catch {
        return [];
      }
    },
    enabled: taskId !== null,
    refetchInterval: REFETCH_TASK_MESSAGES,
  });
}

export function useSendTaskMessage(taskId: number) {
  const queryClient = useQueryClient();
  return useMutation<void, Error, string>({
    mutationFn: async (content: string) => {
      const res = await apiFetch(`/api/tasks/${taskId}/messages`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ role: "user", content }),
      });
      if (!res.ok) throw new Error(`${res.status}`);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["task_messages", taskId] });
    },
  });
}

export function useTaskStream(taskId: number | null, active: boolean) {
  const [events, setEvents] = useState<StreamEvent[]>([]);
  const [streaming, setStreaming] = useState(false);
  const [retryKey, setRetryKey] = useState(0);
  const esRef = useRef<EventSource | null>(null);
  const retryTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Clear events immediately when switching tasks
  useEffect(() => {
    setEvents([]);
    setStreaming(false);
    setRetryKey(0);
  }, [taskId]);

  useEffect(() => {
    if (!taskId || !active) {
      setEvents([]);
      setStreaming(false);
      return;
    }

    setEvents([]);
    let cancelled = false;

    tokenReady.then(() => {
      if (cancelled) return;
      const es = new EventSource(sseUrl(`/api/tasks/${taskId}/stream`));
      esRef.current = es;
      setStreaming(true);

      es.onmessage = (e) => {
        try {
          const obj: StreamEvent = JSON.parse(e.data);
          if (obj.type === "stream_end") {
            setStreaming(false);
            es.close();
            esRef.current = null;
            retryTimer.current = setTimeout(() => setRetryKey((k) => k + 1), 5000);
            return;
          }
          setEvents((prev) => {
            const next = [...prev, obj];
            return next.length > MAX_STREAM_EVENTS ? next.slice(-MAX_STREAM_EVENTS) : next;
          });
        } catch {
          // ignore
        }
      };

      es.onerror = () => {
        setStreaming(false);
        es.close();
        esRef.current = null;
        retryTimer.current = setTimeout(() => setRetryKey((k) => k + 1), 3000);
      };
    });

    return () => {
      cancelled = true;
      esRef.current?.close();
      esRef.current = null;
      if (retryTimer.current) clearTimeout(retryTimer.current);
    };
  }, [taskId, active, retryKey]);

  return { events, streaming };
}

// ── Knowledge base ─────────────────────────────────────────────────────────

export function useKnowledgeFiles() {
  return useQuery<KnowledgeFile[]>({
    queryKey: ["knowledge"],
    queryFn: () => fetchJson<{ files: KnowledgeFile[] }>("/api/knowledge").then((r) => r.files),
    staleTime: 30_000,
  });
}

export async function uploadKnowledgeFile(
  file: File,
  description: string,
  inline: boolean,
  category?: string,
): Promise<{ id: number; file_name: string }> {
  await tokenReady;
  const form = new FormData();
  form.append("file", file);
  form.append("description", description);
  form.append("inline", inline ? "true" : "false");
  if (category) form.append("category", category);
  const res = await fetch(`${apiBase()}/api/knowledge/upload`, { method: "POST", headers: authHeaders(), body: form });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function fetchKnowledgeContent(id: number): Promise<ArrayBuffer> {
  const res = await apiFetch(`/api/knowledge/${id}/content`);
  if (!res.ok) throw new Error(`Failed to load knowledge file (${res.status})`);
  return res.arrayBuffer();
}

export async function updateKnowledgeFile(
  id: number,
  patch: { description?: string; inline?: boolean; tags?: string; category?: string; jurisdiction?: string },
): Promise<{ ok: boolean }> {
  const res = await apiFetch(`/api/knowledge/${id}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(patch),
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function deleteKnowledgeFile(id: number): Promise<{ ok: boolean }> {
  const res = await apiFetch(`/api/knowledge/${id}`, { method: "DELETE" });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

// ── Container & cache ───────────────────────────────────────────────────────

export interface ContainerInfo {
  task_id: number;
  container_id: string;
  status: string;
}

export function useTaskContainer(taskId: number | null, enabled: boolean) {
  return useQuery<ContainerInfo>({
    queryKey: ["task_container", taskId],
    queryFn: () => fetchJson(`/api/tasks/${taskId}/container`),
    enabled: taskId !== null && enabled,
    refetchInterval: 5000,
    retry: false,
  });
}

export interface CacheVolume {
  name: string;
  size: number;
}

export function useCacheVolumes() {
  return useQuery<{ volumes: CacheVolume[] }>({
    queryKey: ["cache_volumes"],
    queryFn: () => fetchJson("/api/cache"),
    staleTime: 15_000,
  });
}

export async function deleteCacheVolume(name: string): Promise<{ ok: boolean }> {
  const res = await apiFetch(`/api/cache/${encodeURIComponent(name)}`, { method: "DELETE" });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}
