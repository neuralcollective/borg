export interface Task {
  id: number;
  title: string;
  description: string;
  status: string;
  branch: string;
  repo_path: string;
  attempt: number;
  max_attempts: number;
  created_by: string;
  created_at: string;
  last_error?: string;
}

export interface TaskDetail extends Task {
  last_error: string;
  outputs: TaskOutput[];
}

export interface TaskOutput {
  id: number;
  phase: string;
  output: string;
  raw_stream: string;
  exit_code: number;
  created_at: string;
}

export interface QueueEntry {
  id: number;
  task_id: number;
  branch: string;
  repo_path: string;
  status: string;
  queued_at: string;
}

export interface WatchedRepo {
  path: string;
  test_cmd: string;
  is_self: boolean;
  auto_merge: boolean;
}

export interface Status {
  version: string;
  uptime_s: number;
  model: string;
  watched_repos: WatchedRepo[];
  release_interval_mins: number;
  continuous_mode: boolean;
  assistant_name: string;
  active_tasks: number;
  merged_tasks: number;
  failed_tasks: number;
  total_tasks: number;
}

export function repoName(path: string): string {
  if (!path) return "";
  const parts = path.replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || path;
}

export interface Proposal {
  id: number;
  repo_path: string;
  title: string;
  description: string;
  rationale: string;
  status: string;
  created_at: string;
}

export interface LogEvent {
  level: string;
  message: string;
  ts: number;
  category?: string;
  metadata?: string;
}

export interface DbEvent {
  id: number;
  ts: number;
  level: string;
  category: string;
  message: string;
  metadata: string;
}

export const PHASES = ["backlog", "spec", "qa", "impl", "done", "merged"] as const;
export const PHASE_LABELS: Record<string, string> = {
  backlog: "Backlog",
  spec: "Spec",
  qa: "QA",
  impl: "Implement",
  done: "Testing",
  merged: "Merged",
};

export const ACTIVE_STATUSES = ["backlog", "spec", "qa", "qa_fix", "impl", "retry", "rebase"];

export function isActiveStatus(status: string) {
  return ACTIVE_STATUSES.includes(status);
}

export function effectivePhase(status: string): string {
  if (status === "retry" || status === "rebase") return "impl";
  if (status === "failed") return "impl";
  if (status === "qa_fix") return "qa";
  return status;
}
