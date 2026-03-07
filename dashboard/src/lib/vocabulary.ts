import { useMemo } from "react";
import { useSettings } from "./api";

export interface Vocabulary {
  mode: "swe" | "law" | "general";

  // Navigation
  projectsLabel: string;
  tasksLabel: string;

  // Projects
  projectSingular: string;
  projectPlural: string;
  newProjectPlaceholder: string;
  projectDocsLabel: string;
  projectDocsDescription: string;

  // Tasks
  taskSingular: string;
  taskPlural: string;

  // Statuses — human-readable overrides for pipeline statuses
  statusLabels: Record<string, string>;

  // Sections to hide
  hideGitColumns: boolean;
  hideAttemptCount: boolean;
  hidePipelineStats: boolean;
  hideRetryAll: boolean;
}

const SWE_VOCAB: Vocabulary = {
  mode: "swe",
  projectsLabel: "Projects",
  tasksLabel: "Tasks",
  projectSingular: "project",
  projectPlural: "projects",
  newProjectPlaceholder: "New project name",
  projectDocsLabel: "Project Documents",
  projectDocsDescription: "Documents scoped to this project. Chat with these docs via the bottom bar.",
  taskSingular: "task",
  taskPlural: "tasks",
  statusLabels: {},
  hideGitColumns: false,
  hideAttemptCount: false,
  hidePipelineStats: false,
  hideRetryAll: false,
};

const LAW_VOCAB: Vocabulary = {
  mode: "law",
  projectsLabel: "Matters",
  tasksLabel: "Tasks",
  projectSingular: "matter",
  projectPlural: "matters",
  newProjectPlaceholder: "New matter name",
  projectDocsLabel: "Matter Documents",
  projectDocsDescription: "Documents for this matter. Chat via the bottom bar.",
  taskSingular: "task",
  taskPlural: "tasks",
  statusLabels: {
    backlog: "Queued",
    implement: "Working",
    validate: "Reviewing",
    lint_fix: "Fixing",
    rebase: "Finalizing",
    done: "Complete",
    merged: "Complete",
    failed: "Failed",
  },
  hideGitColumns: true,
  hideAttemptCount: true,
  hidePipelineStats: true,
  hideRetryAll: true,
};

const GENERAL_VOCAB: Vocabulary = { ...SWE_VOCAB, mode: "general" };

export function getVocabulary(mode: string): Vocabulary {
  if (mode === "lawborg" || mode === "legal") return LAW_VOCAB;
  if (mode === "sweborg" || mode === "swe") return SWE_VOCAB;
  return GENERAL_VOCAB;
}

export function useVocabulary(): Vocabulary {
  const { data: settings } = useSettings();
  return useMemo(() => {
    const mode = settings?.dashboard_mode ?? "general";
    return getVocabulary(mode);
  }, [settings?.dashboard_mode]);
}
