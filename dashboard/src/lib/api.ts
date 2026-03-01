import { useQuery, useQueryClient, useMutation } from "@tanstack/react-query";
import { useEffect, useRef, useCallback, useState } from "react";
import type {
  Task,
  TaskDetail,
  QueueEntry,
  Status,
  LogEvent,
  Proposal,
  PipelineMode,
  TaskMessage,
  Project,
  ProjectFile,
  PipelineModeFull,
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

async function fetchJson<T>(url: string): Promise<T> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
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
  const res = await fetch("/api/modes/custom", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(mode),
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function removeCustomMode(name: string): Promise<{ ok: boolean }> {
  const res = await fetch(`/api/modes/custom/${encodeURIComponent(name)}`, {
    method: "DELETE",
  });
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
}

export function useSettings() {
  return useQuery<Settings>({
    queryKey: ["settings"],
    queryFn: () => fetchJson("/api/settings"),
    staleTime: 60_000,
  });
}

export async function updateSettings(settings: Partial<Settings>): Promise<{ updated: number }> {
  const res = await fetch("/api/settings", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(settings),
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export function useFocus() {
  return useQuery<{ text: string; active: boolean }>({
    queryKey: ["focus"],
    queryFn: () => fetchJson("/api/focus"),
    staleTime: 10_000,
  });
}

export async function setFocus(text: string): Promise<void> {
  const res = await fetch("/api/focus", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ text }),
  });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function clearFocus(): Promise<void> {
  const res = await fetch("/api/focus", { method: "DELETE" });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function approveProposal(id: number): Promise<{ task_id: number }> {
  const res = await fetch(`/api/proposals/${id}/approve`, { method: "POST" });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function dismissProposal(id: number): Promise<void> {
  const res = await fetch(`/api/proposals/${id}/dismiss`, { method: "POST" });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function triageProposals(): Promise<{ scored: number }> {
  const res = await fetch("/api/proposals/triage", { method: "POST" });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export async function reopenProposal(id: number): Promise<void> {
  const res = await fetch(`/api/proposals/${id}/reopen`, { method: "POST" });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function retryTask(id: number): Promise<void> {
  const res = await fetch(`/api/tasks/${id}/retry`, { method: "POST" });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function retryAllFailed(): Promise<void> {
  const res = await fetch("/api/tasks/retry-all-failed", { method: "POST" });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function setTaskBackend(id: number, backend: string): Promise<void> {
  const res = await fetch(`/api/tasks/${id}/backend`, {
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
  const res = await fetch(`/api/repos/${id}/backend`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ backend }),
  });
  if (!res.ok) throw new Error(`${res.status}`);
}

export async function createTask(title: string, description: string, mode: string, repo_path?: string): Promise<{ id: number }> {
  const res = await fetch("/api/tasks/create", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ title, description, mode, repo: repo_path }),
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export function useLogs() {
  const [logs, setLogs] = useState<LogEvent[]>([]);
  const [connected, setConnected] = useState(false);
  const esRef = useRef<EventSource | null>(null);
  const invalidateTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const queryClient = useQueryClient();

  const connect = useCallback(() => {
    if (esRef.current) esRef.current.close();
    const es = new EventSource("/api/logs");
    esRef.current = es;

    es.onopen = () => setConnected(true);
    es.onerror = () => setConnected(false);
    es.onmessage = (e) => {
      try {
        const d = normalizeLogEvent(JSON.parse(e.data));
        if (!d) return;
        setLogs((prev) => {
          const next = [...prev, d];
          return next.length > MAX_LOG_BUFFER ? next.slice(-MAX_LOG_BUFFER) : next;
        });
        // Debounced cache invalidation — at most once per second
        if (!invalidateTimer.current) {
          invalidateTimer.current = setTimeout(() => {
            queryClient.invalidateQueries();
            invalidateTimer.current = null;
          }, 1000);
        }
      } catch {
        // ignore parse errors
      }
    };
  }, [queryClient]);

  useEffect(() => {
    connect();
    return () => {
      esRef.current?.close();
      if (invalidateTimer.current) clearTimeout(invalidateTimer.current);
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
}

export function useProjects() {
  return useQuery<Project[]>({
    queryKey: ["projects"],
    queryFn: () => fetchJson("/api/projects"),
    refetchInterval: REFETCH_PROJECTS,
  });
}

export async function createProject(
  name: string,
  mode = "general"
): Promise<{ id: number }> {
  const res = await fetch("/api/projects", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name, mode }),
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export function useProjectFiles(projectId: number | null) {
  return useQuery<ProjectFile[]>({
    queryKey: ["project_files", projectId],
    queryFn: () => fetchJson(`/api/projects/${projectId}/files`),
    enabled: projectId !== null,
    refetchInterval: REFETCH_PROJECTS,
  });
}

export async function uploadProjectFiles(
  projectId: number,
  files: FileList | File[]
): Promise<{ uploaded: ProjectFile[] }> {
  const form = new FormData();
  Array.from(files).forEach((file) => form.append("files", file));
  const res = await fetch(`/api/projects/${projectId}/files/upload`, {
    method: "POST",
    body: form,
  });
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
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
  const res = await fetch(`/api/projects/${projectId}/chat`, {
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
        const res = await fetch(`/api/tasks/${taskId}/messages`);
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
      const res = await fetch(`/api/tasks/${taskId}/messages`, {
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

    const es = new EventSource(`/api/tasks/${taskId}/stream`);
    esRef.current = es;
    setStreaming(true);

    es.onmessage = (e) => {
      try {
        const obj: StreamEvent = JSON.parse(e.data);
        if (obj.type === "stream_end") {
          setStreaming(false);
          es.close();
          esRef.current = null;
          // Agent finished — retry in case it restarts
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
      // Reconnect after 3s
      retryTimer.current = setTimeout(() => setRetryKey((k) => k + 1), 3000);
    };

    return () => {
      es.close();
      esRef.current = null;
      if (retryTimer.current) clearTimeout(retryTimer.current);
    };
  }, [taskId, active, retryKey]);

  return { events, streaming };
}
