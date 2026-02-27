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
  mode?: string;
  backend?: string;
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
  mode: string;
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
  dispatched_agents: number;
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
  triage_score: number;
  triage_impact: number;
  triage_feasibility: number;
  triage_risk: number;
  triage_effort: number;
  triage_reasoning: string;
}

export interface TaskMessage {
  id: number;
  task_id: number;
  role: "user" | "director" | "system";
  content: string;
  created_at: string;
  delivered_phase: string | null;
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

export interface PhaseInfo {
  name: string;
  label: string;
  priority: number;
}

export interface PipelineMode {
  name: string;
  label: string;
  phases: PhaseInfo[];
}

export interface Project {
  id: number;
  name: string;
  mode: string;
  created_at: string;
}

export interface ProjectFile {
  id: number;
  project_id: number;
  file_name: string;
  mime_type: string;
  size_bytes: number;
  created_at: string;
}

// sweborg phases (default fallback)
const SWE_DISPLAY_PHASES = ["backlog", "spec", "qa", "impl", "done", "merged"] as const;
const SWE_PHASE_LABELS: Record<string, string> = {
  backlog: "Backlog", spec: "Spec", qa: "QA", impl: "Implement",
  done: "Testing", merged: "Merged",
};

// lawborg phases
const LEGAL_DISPLAY_PHASES = ["backlog", "research", "draft", "review", "done"] as const;
const LEGAL_PHASE_LABELS: Record<string, string> = {
  backlog: "Backlog", research: "Research", draft: "Drafting",
  review: "Review", done: "Complete",
};

// webborg phases
const WEB_DISPLAY_PHASES = ["backlog", "audit", "improve", "done", "merged"] as const;
const WEB_PHASE_LABELS: Record<string, string> = {
  backlog: "Backlog", audit: "Audit", improve: "Improve",
  done: "Done", merged: "Merged",
};

export function getDisplayPhases(mode?: string): readonly string[] {
  if (mode === "lawborg" || mode === "legal") return LEGAL_DISPLAY_PHASES;
  if (mode === "webborg") return WEB_DISPLAY_PHASES;
  return SWE_DISPLAY_PHASES;
}

export function getPhaseLabel(phase: string, mode?: string): string {
  if (mode === "lawborg" || mode === "legal") return LEGAL_PHASE_LABELS[phase] ?? phase;
  if (mode === "webborg") return WEB_PHASE_LABELS[phase] ?? phase;
  return SWE_PHASE_LABELS[phase] ?? phase;
}

// Keep legacy exports for backward compat
export const PHASES = SWE_DISPLAY_PHASES;
export const PHASE_LABELS = SWE_PHASE_LABELS;

export function isActiveStatus(status: string) {
  const all = ["backlog", "spec", "qa", "qa_fix", "impl", "retry", "rebase",
               "research", "draft", "review", "audit", "improve"];
  return all.includes(status);
}

export function effectivePhase(status: string, mode?: string): string {
  if (mode === "lawborg" || mode === "legal") return status;
  if (mode === "webborg") {
    if (status === "rebase") return "improve";
    if (status === "failed") return "improve";
    return status;
  }
  if (status === "retry" || status === "rebase") return "impl";
  if (status === "failed") return "impl";
  if (status === "qa_fix") return "qa";
  return status;
}
