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
  updated_at?: string;
  last_error?: string;
  mode?: string;
  backend?: string;
  started_at?: string;
  completed_at?: string;
  duration_secs?: number;
  review_status?: string;
  revision_count?: number;
}

export interface TaskDetail extends Task {
  last_error: string;
  outputs: TaskOutput[];
  structured_data?: Record<string, unknown>;
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
  ai_requests: number;
  failed_tasks: number;
  total_tasks: number;
  dispatched_agents: number;
  guardrail_alert?: boolean;
  guardrail_rebase_count?: number;
  guardrail_queued_count?: number;
  guardrail_no_merge_mins?: number;
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
  category?: string;
  phases: PhaseInfo[];
  experimental?: boolean;
}

export type PhaseType =
  | "setup"
  | "agent"
  | "rebase"
  | "lint_fix"
  | "human_review"
  | "validate"
  | "compliance_check";
export type IntegrationType = "git_pr" | "git_branch" | "none";
export type SeedOutputType = "task" | "proposal";

export interface PhaseConfigFull {
  name: string;
  label: string;
  phase_type: PhaseType;
  system_prompt: string;
  instruction: string;
  error_instruction: string;
  allowed_tools: string;
  use_docker: boolean;
  include_task_context: boolean;
  include_file_listing: boolean;
  runs_tests: boolean;
  commits: boolean;
  commit_message: string;
  check_artifact: string | null;
  allow_no_changes: boolean;
  next: string;
  fresh_session: boolean;
  revision_target: string;
  fix_instruction: string;
  retry_phase: string;
  compliance_profile: string;
  compliance_enforcement: "warn" | "block" | string;
}

export interface SeedConfigFull {
  name: string;
  label: string;
  prompt: string;
  output_type: SeedOutputType;
  allowed_tools: string;
  target_primary_repo: boolean;
}

export interface PipelineModeFull {
  name: string;
  label: string;
  category?: string;
  phases: PhaseConfigFull[];
  seed_modes: SeedConfigFull[];
  initial_status: string;
  uses_docker: boolean;
  uses_test_cmd: boolean;
  integration: IntegrationType;
  default_max_attempts: number;
}

export interface Project {
  id: number;
  name: string;
  mode: string;
  created_at: string;
  jurisdiction?: string;
}

export interface ProjectTask {
  id: number;
  title: string;
  description: string;
  status: string;
  branch: string;
  mode?: string;
  task_type?: string;
  created_at: string;
  updated_at?: string;
  attempt: number;
  max_attempts: number;
  started_at?: string;
  completed_at?: string;
  duration_secs?: number;
  review_status?: string;
  revision_count?: number;
}

export interface ProjectDocument {
  task_id: number;
  task_title: string;
  task_status: string;
  file_name: string;
  content: string;
  created_at: string;
  branch: string;
}

export interface ProjectFile {
  id: number;
  project_id: number;
  file_name: string;
  source_path: string;
  mime_type: string;
  size_bytes: number;
  privileged?: boolean;
  has_text: boolean;
  text_chars: number;
  created_at: string;
}

export interface ProjectFileSummary {
  project_id: number;
  total_files: number;
  total_bytes: number;
  privileged_files: number;
  text_files: number;
  text_chars: number;
  updated_at: string;
}

export interface ProjectFilePage {
  items: ProjectFile[];
  total: number;
  offset: number;
  limit: number;
  has_more: boolean;
  next_cursor?: string | null;
  summary: ProjectFileSummary;
}

export interface KnowledgeFile {
  id: number;
  file_name: string;
  description: string;
  size_bytes: number;
  inline: boolean;
  tags: string;
  category: string;
  jurisdiction: string;
  project_id: number | null;
  created_at: string;
}

export interface KnowledgeFilePage {
  files: KnowledgeFile[];
  total: number;
  offset: number;
  limit: number;
  has_more: boolean;
  total_bytes: number;
}

// sweborg phases (default fallback)
const SWE_DISPLAY_PHASES = ["backlog", "implement", "validate", "lint_fix", "rebase", "done", "merged"] as const;
const SWE_PHASE_LABELS: Record<string, string> = {
  backlog: "Backlog", implement: "Implement", validate: "Validate",
  lint_fix: "Lint Fix", rebase: "Rebase", done: "Done", merged: "Merged",
};

// knowledge work phases (legal, knowledge modes)
const LEGAL_DISPLAY_PHASES = ["backlog", "implement", "review", "done"] as const;
const LEGAL_PHASE_LABELS: Record<string, string> = {
  backlog: "Backlog", implement: "Research & Draft", review: "Review", done: "Complete",
};

const LEGAL_TASK_TYPE_LABELS: Record<string, Record<string, string>> = {
  contract_analysis: { backlog: "Backlog", implement: "Extract & Analyze", review: "Review", done: "Complete" },
  contract_review: { backlog: "Backlog", implement: "Review & Redline", review: "Review", done: "Complete" },
  nda_triage: { backlog: "Backlog", implement: "Screen & Classify", review: "Review", done: "Complete" },
  regulatory_analysis: { backlog: "Backlog", implement: "Monitor & Analyze", review: "Review", done: "Complete" },
  compliance: { backlog: "Backlog", implement: "Audit & Assess", review: "Review", done: "Complete" },
  risk_assessment: { backlog: "Backlog", implement: "Assess & Score", review: "Review", done: "Complete" },
  vendor_check: { backlog: "Backlog", implement: "Search & Compile", review: "Review", done: "Complete" },
  meeting_briefing: { backlog: "Backlog", implement: "Gather & Brief", review: "Review", done: "Complete" },
  demand_letter: { backlog: "Backlog", implement: "Research & Draft", review: "Review", done: "Complete" },
  motion_brief: { backlog: "Backlog", implement: "Research & Draft", review: "Review", done: "Complete" },
};

// webborg phases
const WEB_DISPLAY_PHASES = ["backlog", "audit", "improve", "done", "merged"] as const;
const WEB_PHASE_LABELS: Record<string, string> = {
  backlog: "Backlog", audit: "Audit", improve: "Improve",
  done: "Done", merged: "Merged",
};

const SWE_MODES = new Set(["sweborg", "swe", "webborg"]);

export function getDisplayPhases(mode?: string, _taskType?: string): readonly string[] {
  if (mode === "webborg") return WEB_DISPLAY_PHASES;
  if (mode && !SWE_MODES.has(mode)) return LEGAL_DISPLAY_PHASES;
  return SWE_DISPLAY_PHASES;
}

export function getPhaseLabel(phase: string, mode?: string, taskType?: string): string {
  if (mode === "webborg") return WEB_PHASE_LABELS[phase] ?? phase;
  if (mode && !SWE_MODES.has(mode)) {
    const typeLabels = taskType ? LEGAL_TASK_TYPE_LABELS[taskType] : undefined;
    if (typeLabels?.[phase]) return typeLabels[phase];
    return LEGAL_PHASE_LABELS[phase] ?? phase;
  }
  return SWE_PHASE_LABELS[phase] ?? phase;
}

// Keep legacy exports for backward compat
export const PHASES = SWE_DISPLAY_PHASES;
export const PHASE_LABELS = SWE_PHASE_LABELS;

export function isActiveStatus(status: string) {
  return !["done", "merged", "failed", "blocked"].includes(status);
}

export function effectivePhase(status: string, _mode?: string): string {
  return status;
}
