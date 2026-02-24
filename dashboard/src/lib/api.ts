import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useCallback, useState } from "react";
import type { Task, TaskDetail, QueueEntry, Status, LogEvent } from "./types";

async function fetchJson<T>(url: string): Promise<T> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${res.status}`);
  return res.json();
}

export function useTasks() {
  return useQuery<Task[]>({
    queryKey: ["tasks"],
    queryFn: () => fetchJson("/api/tasks"),
    refetchInterval: 3000,
  });
}

export function useTaskDetail(id: number | null) {
  return useQuery<TaskDetail>({
    queryKey: ["task", id],
    queryFn: () => fetchJson(`/api/tasks/${id}`),
    enabled: id !== null,
    refetchInterval: 3000,
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
