export interface Task {
  id: number;
  title: string;
  description: string;
  status: string;
  branch: string;
  attempt: number;
  max_attempts: number;
  created_by: string;
  created_at: string;
}

export interface TaskDetail extends Task {
  last_error: string;
  outputs: TaskOutput[];
}

export interface TaskOutput {
  id: number;
  phase: string;
  output: string;
  exit_code: number;
  created_at: string;
}

export interface QueueEntry {
  id: number;
  task_id: number;
  branch: string;
  status: string;
  queued_at: string;
}

export interface Status {
  uptime_s: number;
  model: string;
  pipeline_repo: string;
  release_interval_mins: number;
  test_cmd: string;
  continuous_mode: boolean;
  assistant_name: string;
  active_tasks: number;
  merged_tasks: number;
  failed_tasks: number;
  total_tasks: number;
}

export interface LogEvent {
  level: string;
  message: string;
  ts: number;
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

export const ACTIVE_STATUSES = ["backlog", "spec", "qa", "impl", "retry", "rebase"];

export function isActiveStatus(status: string) {
  return ACTIVE_STATUSES.includes(status);
}

export function effectivePhase(status: string): string {
  if (status === "retry" || status === "rebase") return "impl";
  if (status === "failed") return "impl";
  return status;
}
