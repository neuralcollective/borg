import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useCallback, useState } from "react";
import type { Task, TaskDetail, QueueEntry, Status, LogEvent, Proposal } from "./types";

async function fetchJson<T>(url: string): Promise<T> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export function useTasks() {
  return useQuery<Task[]>({
    queryKey: ["tasks"],
    queryFn: () => fetchJson("/api/tasks"),
    refetchInterval: 2000,
  });
}

export function useTaskDetail(id: number | null) {
  return useQuery<TaskDetail>({
    queryKey: ["task", id],
    queryFn: () => fetchJson(`/api/tasks/${id}`),
    enabled: id !== null,
    refetchInterval: 2000,
  });
}

export function useQueue() {
  return useQuery<QueueEntry[]>({
    queryKey: ["queue"],
    queryFn: () => fetchJson("/api/queue"),
    refetchInterval: 3000,
  });
}

export function useStatus() {
  return useQuery<Status>({
    queryKey: ["status"],
    queryFn: () => fetchJson("/api/status"),
    refetchInterval: 3000,
  });
}

export function useProposals() {
  return useQuery<Proposal[]>({
    queryKey: ["proposals"],
    queryFn: () => fetchJson("/api/proposals"),
    refetchInterval: 5000,
  });
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

export function useLogs() {
  const [logs, setLogs] = useState<LogEvent[]>([]);
  const [connected, setConnected] = useState(false);
  const esRef = useRef<EventSource | null>(null);

  const connect = useCallback(() => {
    if (esRef.current) esRef.current.close();
    const es = new EventSource("/api/logs");
    esRef.current = es;

    es.onopen = () => setConnected(true);
    es.onerror = () => setConnected(false);
    es.onmessage = (e) => {
      try {
        const d: LogEvent = JSON.parse(e.data);
        setLogs((prev) => {
          const next = [...prev, d];
          return next.length > 500 ? next.slice(-500) : next;
        });
      } catch {
        // ignore parse errors
      }
    };
  }, []);

  useEffect(() => {
    connect();
    return () => esRef.current?.close();
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
}

export function useTaskStream(taskId: number | null, active: boolean) {
  const [events, setEvents] = useState<StreamEvent[]>([]);
  const [streaming, setStreaming] = useState(false);
  const [retryKey, setRetryKey] = useState(0);
  const esRef = useRef<EventSource | null>(null);
  const retryTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

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
          return next.length > 2000 ? next.slice(-2000) : next;
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
