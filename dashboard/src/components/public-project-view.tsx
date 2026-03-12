import { AlertTriangle, ExternalLink, FileText, Loader2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import {
  fetchPublicProject,
  fetchPublicProjectDocuments,
  fetchPublicProjectTasks,
} from "@/lib/api";
import type { Project, ProjectDocument, ProjectTask } from "@/lib/types";
import { cn } from "@/lib/utils";
import { BorgLogo } from "./borg-logo";
import { PhaseTracker } from "./phase-tracker";
import { StatusBadge } from "./status-badge";

type TabKey = "documents" | "tasks" | "activity";

const TABS: { key: TabKey; label: string }[] = [
  { key: "documents", label: "Documents" },
  { key: "tasks", label: "Tasks" },
  { key: "activity", label: "Activity" },
];

function fmtDateTime(ts: string): string {
  if (!ts) return "";
  try {
    return new Date(ts).toLocaleString("en-US", {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
    });
  } catch {
    return ts;
  }
}

function formatDuration(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  const rm = m % 60;
  return rm > 0 ? `${h}h ${rm}m` : `${h}h`;
}

function PublicProjectHeader({ project }: { project: Project }) {
  return (
    <div className="border-b border-white/[0.07] px-5 py-4">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-[15px] font-semibold text-zinc-100 flex items-center gap-1">
              <span className="text-[12px] text-[#6b6459] tabular-nums mr-1.5">
                #{project.id}
              </span>
              <span>{project.name}</span>
            </h2>
            {project.jurisdiction && (
              <span className="rounded-lg bg-blue-500/10 px-2 py-0.5 text-[10px] font-medium text-blue-400">
                {project.jurisdiction}
              </span>
            )}
            {project.mode && (
              <span className="rounded-lg bg-violet-500/10 px-2 py-0.5 text-[10px] font-medium text-violet-400">
                {project.mode}
              </span>
            )}
          </div>
          {project.task_counts && (
            <div className="mt-1.5 flex gap-4 text-[12px] text-zinc-500">
              {project.task_counts.total > 0 && (
                <span>{project.task_counts.total} tasks</span>
              )}
              {project.task_counts.done > 0 && (
                <span className="text-emerald-500">
                  {project.task_counts.done} done
                </span>
              )}
              {project.task_counts.active > 0 && (
                <span className="text-amber-500">
                  {project.task_counts.active} active
                </span>
              )}
              {project.task_counts.failed > 0 && (
                <span className="text-red-500">
                  {project.task_counts.failed} failed
                </span>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function PublicDocumentsTab({ docs }: { docs: ProjectDocument[] }) {
  if (docs.length === 0) {
    return (
      <div className="flex h-32 flex-col items-center justify-center text-center">
        <FileText className="h-6 w-6 text-zinc-600 mb-2" />
        <div className="text-[13px] text-zinc-400">No documents yet</div>
      </div>
    );
  }

  return (
    <div className="space-y-3 p-5">
      <div className="mb-2 text-[13px] font-medium text-[#e8e0d4]">
        Agent Work
        <span className="ml-1.5 text-[12px] text-[#6b6459]">
          ({docs.length})
        </span>
      </div>
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        {docs.map((doc) => (
          <div
            key={`${doc.task_id}-${doc.file_name}`}
            className="flex flex-col gap-2 rounded-xl border border-[#2a2520] bg-[#151412] p-4"
          >
            <div className="flex items-center gap-2">
              <FileText className="h-4 w-4 shrink-0 text-blue-400/60" />
              <span className="text-[13px] font-medium text-[#e8e0d4] truncate">
                {doc.file_name}
              </span>
              <StatusBadge status={doc.task_status} />
            </div>
            <div className="text-[12px] text-[#6b6459] truncate">
              #{doc.task_id} · {doc.task_title}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function PublicTasksTab({ tasks }: { tasks: ProjectTask[] }) {
  if (tasks.length === 0) {
    return (
      <div className="flex h-32 flex-col items-center justify-center text-center">
        <FileText className="h-6 w-6 text-zinc-600 mb-2" />
        <div className="text-[13px] text-zinc-400">No tasks yet</div>
      </div>
    );
  }

  const totalSecs = tasks.reduce((sum, t) => sum + (t.duration_secs ?? 0), 0);

  return (
    <div className="space-y-2.5 p-4">
      {totalSecs > 0 && (
        <div className="text-[12px] text-zinc-400 pb-1">
          Total time:{" "}
          <span className="text-zinc-300">{formatDuration(totalSecs)}</span>
          {" · "}
          {tasks.filter((t) => t.duration_secs != null).length} tracked
        </div>
      )}
      {tasks.map((task) => {
        const isActive = [
          "implement",
          "review",
          "validate",
          "lint_fix",
          "rebase",
          "spec",
          "qa",
          "qa_fix",
          "retry",
        ].includes(task.status);

        return (
          <div
            key={task.id}
            className="rounded-xl border border-white/[0.07] bg-white/[0.03] p-4"
          >
            <div className="flex items-start gap-2">
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-mono text-[11px] text-zinc-500">
                    #{task.id}
                  </span>
                  <StatusBadge status={task.status} />
                  {isActive && (
                    <span
                      className="h-1.5 w-1.5 animate-pulse rounded-full bg-blue-400"
                      title="Running"
                    />
                  )}
                  {task.revision_count != null && task.revision_count > 0 && (
                    <span className="text-[9px] text-amber-500/80">
                      rev {task.revision_count}
                    </span>
                  )}
                  {task.mode && (
                    <span className="rounded bg-violet-500/10 px-1.5 py-0.5 text-[9px] font-medium text-violet-400">
                      {task.mode}
                    </span>
                  )}
                </div>
                <div className="mt-1 text-[13px] font-medium text-zinc-200">
                  {task.title}
                </div>
                {task.description && (
                  <div className="mt-0.5 line-clamp-2 text-[12px] text-zinc-400">
                    {task.description}
                  </div>
                )}
              </div>
            </div>
            <div className="mt-2">
              <PhaseTracker status={task.status} mode={task.mode} />
            </div>
            <div className="mt-2 text-[11px] text-zinc-500">
              created {fmtDateTime(task.created_at)}
              {task.attempt > 0 &&
                ` · attempt ${task.attempt}/${task.max_attempts}`}
              {task.duration_secs != null &&
                ` · ${formatDuration(task.duration_secs)}`}
            </div>
          </div>
        );
      })}
    </div>
  );
}

function PublicActivityTab({
  tasks,
  docs,
}: {
  tasks: ProjectTask[];
  docs: ProjectDocument[];
}) {
  type ActivityItem = {
    id: string;
    ts: string;
    label: string;
    sub?: string;
    kind: "task" | "document";
    dotColor: string;
  };

  const items = useMemo(() => {
    const list: ActivityItem[] = [];

    for (const t of tasks) {
      list.push({
        id: `task-${t.id}`,
        ts: t.created_at,
        label: t.title,
        sub: `Task #${t.id} created`,
        kind: "task",
        dotColor: "bg-emerald-400/60",
      });
    }

    for (const d of docs) {
      list.push({
        id: `doc-${d.task_id}-${d.file_name}`,
        ts: d.created_at,
        label: d.file_name,
        sub: `from task #${d.task_id} · ${d.task_title}`,
        kind: "document",
        dotColor: "bg-blue-400/60",
      });
    }

    list.sort((a, b) => (a.ts < b.ts ? 1 : a.ts > b.ts ? -1 : 0));
    return list;
  }, [tasks, docs]);

  if (items.length === 0) {
    return (
      <div className="flex h-32 flex-col items-center justify-center text-center">
        <FileText className="h-6 w-6 text-zinc-600 mb-2" />
        <div className="text-[13px] text-zinc-400">No activity yet</div>
      </div>
    );
  }

  return (
    <div className="space-y-0 overflow-y-auto p-5">
      {items.map((item, idx) => (
        <div key={item.id} className="flex gap-3">
          <div className="flex flex-col items-center">
            <div
              className={cn(
                "mt-1.5 h-2.5 w-2.5 shrink-0 rounded-full",
                item.dotColor,
              )}
            />
            {idx < items.length - 1 && (
              <div
                className="mt-1 w-px flex-1 bg-white/[0.07]"
                style={{ minHeight: "28px" }}
              />
            )}
          </div>
          <div className="pb-4 min-w-0">
            <div className="text-[13px] font-medium text-zinc-300 truncate">
              {item.label}
            </div>
            {item.sub && (
              <div className="mt-0.5 text-[12px] text-zinc-400">
                {item.sub} · {fmtDateTime(item.ts)}
              </div>
            )}
            {!item.sub && (
              <div className="mt-0.5 text-[12px] text-zinc-400">
                {fmtDateTime(item.ts)}
              </div>
            )}
          </div>
        </div>
      ))}
    </div>
  );
}

export function PublicProjectView({ token }: { token: string }) {
  const [project, setProject] = useState<Project | null>(null);
  const [tasks, setTasks] = useState<ProjectTask[]>([]);
  const [docs, setDocs] = useState<ProjectDocument[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<TabKey>("documents");

  useEffect(() => {
    let cancelled = false;
    async function load() {
      try {
        const [p, t, d] = await Promise.all([
          fetchPublicProject(token),
          fetchPublicProjectTasks(token),
          fetchPublicProjectDocuments(token).catch(() => [] as ProjectDocument[]),
        ]);
        if (cancelled) return;
        setProject(p);
        setTasks(t);
        setDocs(d);
      } catch (e: any) {
        if (cancelled) return;
        if (e.message === "expired") {
          setError("This share link has expired.");
        } else if (e.message === "404") {
          setError("Share link not found or has been revoked.");
        } else {
          setError("Failed to load shared workspace.");
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    }
    load();
    return () => {
      cancelled = true;
    };
  }, [token]);

  if (loading) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-[#0a0a0b]">
        <Loader2 className="h-6 w-6 animate-spin text-zinc-500" />
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-[#0a0a0b]">
        <div className="text-center">
          <AlertTriangle className="mx-auto mb-3 h-8 w-8 text-amber-500" />
          <p className="text-[14px] text-zinc-300">{error}</p>
          <a
            href="/"
            className="mt-4 inline-block text-[12px] text-blue-400 hover:text-blue-300"
          >
            Go to dashboard
          </a>
        </div>
      </div>
    );
  }

  if (!project) return null;

  return (
    <div className="min-h-screen bg-[#0a0a0b]">
      {/* Read-only banner */}
      <div className="border-b border-white/[0.07] bg-blue-500/5">
        <div className="mx-auto flex max-w-5xl items-center justify-between px-6 py-2.5">
          <div className="flex items-center gap-2 text-[12px] text-blue-400">
            <ExternalLink size={13} />
            Read-only shared view
          </div>
          <div className="h-8 w-8">
            <BorgLogo size="mobile" />
          </div>
        </div>
      </div>

      {/* Project detail container */}
      <div className="mx-auto max-w-5xl">
        <PublicProjectHeader project={project} />

        {/* Tab bar */}
        <div className="flex gap-0 border-b border-white/[0.07] px-5">
          {TABS.map((tab) => (
            <button
              key={tab.key}
              onClick={() => setActiveTab(tab.key)}
              className={cn(
                "border-b-2 px-4 py-3 text-[13px] font-medium transition-colors",
                activeTab === tab.key
                  ? "border-blue-500 text-zinc-200"
                  : "border-transparent text-zinc-400 hover:text-zinc-200",
              )}
            >
              {tab.label}
            </button>
          ))}
        </div>

        {/* Tab content */}
        <div>
          {activeTab === "documents" && <PublicDocumentsTab docs={docs} />}
          {activeTab === "tasks" && <PublicTasksTab tasks={tasks} />}
          {activeTab === "activity" && (
            <PublicActivityTab tasks={tasks} docs={docs} />
          )}
        </div>
      </div>
    </div>
  );
}
