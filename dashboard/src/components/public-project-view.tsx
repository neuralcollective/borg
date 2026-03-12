import { AlertTriangle, ExternalLink, FileText, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { fetchPublicProject, fetchPublicProjectTasks } from "@/lib/api";
import type { Project, ProjectTask } from "@/lib/types";
import { BorgLogo } from "./borg-logo";
import { StatusBadge } from "./status-badge";

function TaskRow({ task }: { task: ProjectTask }) {
  return (
    <div className="flex items-center justify-between rounded-lg px-4 py-3 hover:bg-white/[0.03]">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-[11px] tabular-nums text-zinc-600">#{task.id}</span>
          <span className="truncate text-[13px] text-zinc-200">{task.title}</span>
        </div>
        {task.description && <p className="mt-0.5 truncate text-[12px] text-zinc-500">{task.description}</p>}
      </div>
      <StatusBadge status={task.status} />
    </div>
  );
}

export function PublicProjectView({ token }: { token: string }) {
  const [project, setProject] = useState<Project | null>(null);
  const [tasks, setTasks] = useState<ProjectTask[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      try {
        const [p, t] = await Promise.all([fetchPublicProject(token), fetchPublicProjectTasks(token)]);
        if (cancelled) return;
        setProject(p);
        setTasks(t);
      } catch (e: any) {
        if (cancelled) return;
        if (e.message === "expired") {
          setError("This share link has expired.");
        } else if (e.message === "404") {
          setError("Share link not found or has been revoked.");
        } else {
          setError("Failed to load shared project.");
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
          <a href="/" className="mt-4 inline-block text-[12px] text-blue-400 hover:text-blue-300">
            Go to dashboard
          </a>
        </div>
      </div>
    );
  }

  if (!project) return null;

  return (
    <div className="min-h-screen bg-[#0a0a0b]">
      {/* Banner */}
      <div className="border-b border-white/[0.07] bg-blue-500/5">
        <div className="mx-auto flex max-w-4xl items-center justify-between px-6 py-2.5">
          <div className="flex items-center gap-2 text-[12px] text-blue-400">
            <ExternalLink size={13} />
            Read-only shared view
          </div>
          <BorgLogo />
        </div>
      </div>

      {/* Project header */}
      <div className="mx-auto max-w-4xl px-6 pt-8 pb-6">
        <div className="flex items-center gap-3">
          <h1 className="text-[20px] font-semibold text-zinc-100">{project.name}</h1>
          {project.jurisdiction && (
            <span className="rounded-lg bg-blue-500/10 px-2 py-0.5 text-[11px] font-medium text-blue-400">
              {project.jurisdiction}
            </span>
          )}
        </div>
        {project.task_counts && (
          <div className="mt-2 flex gap-4 text-[12px] text-zinc-500">
            {project.task_counts.total > 0 && <span>{project.task_counts.total} tasks</span>}
            {project.task_counts.done > 0 && <span className="text-green-500">{project.task_counts.done} done</span>}
            {project.task_counts.active > 0 && (
              <span className="text-amber-500">{project.task_counts.active} active</span>
            )}
          </div>
        )}
      </div>

      {/* Tasks */}
      <div className="mx-auto max-w-4xl px-6 pb-12">
        <h2 className="mb-3 flex items-center gap-2 text-[13px] font-medium text-zinc-400">
          <FileText size={14} />
          Tasks
        </h2>
        {tasks.length === 0 ? (
          <p className="text-[13px] text-zinc-600">No tasks yet.</p>
        ) : (
          <div className="divide-y divide-white/[0.05] rounded-xl border border-white/[0.07] bg-white/[0.02]">
            {tasks.map((t) => (
              <TaskRow key={t.id} task={t} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
