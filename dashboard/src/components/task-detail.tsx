import { useTaskDetail } from "@/lib/api";
import { PhaseTracker } from "./phase-tracker";
import { StatusBadge } from "./status-badge";
import { repoName } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useState } from "react";

interface TaskDetailProps {
  taskId: number;
  onBack: () => void;
}

export function TaskDetail({ taskId, onBack }: TaskDetailProps) {
  const { data: task, isLoading } = useTaskDetail(taskId);

  if (isLoading || !task) {
    return (
      <div className="flex h-full flex-col">
        <DetailHeader onBack={onBack} />
        <div className="flex flex-1 items-center justify-center text-xs text-zinc-600">Loading...</div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <DetailHeader onBack={onBack} />

      {/* Task info */}
      <div className="space-y-3 border-b border-white/[0.06] px-4 py-3">
        <div className="flex items-start justify-between gap-4">
          <h2 className="text-[13px] font-medium text-zinc-200">
            <span className="text-zinc-600">#{task.id}</span> {task.title}
          </h2>
          <StatusBadge status={task.status} />
        </div>

        <PhaseTracker status={task.status} />

        <div className="flex flex-wrap gap-3 text-[11px] text-zinc-500">
          {task.repo_path && (
            <span title={task.repo_path}>
              <span className="text-zinc-600">repo</span> {repoName(task.repo_path)}
            </span>
          )}
          {task.branch && (
            <span>
              <span className="text-zinc-600">branch</span> <span className="font-mono">{task.branch}</span>
            </span>
          )}
          {task.attempt > 0 && (
            <span>
              <span className="text-zinc-600">attempt</span> {task.attempt}/{task.max_attempts}
            </span>
          )}
          <span>
            <span className="text-zinc-600">by</span> {task.created_by || "pipeline"}
          </span>
          <span>
            <span className="text-zinc-600">at</span> {task.created_at}
          </span>
        </div>
      </div>

      {/* Description */}
      {task.description && (
        <div className="max-h-16 overflow-y-auto border-b border-white/[0.06] px-4 py-2 text-[11px] leading-relaxed text-zinc-500">
          {task.description}
        </div>
      )}

      {/* Error */}
      {task.last_error && (
        <div className="mx-3 mt-2 rounded-lg border border-red-500/20 bg-red-500/[0.05] p-3">
          <pre className="max-h-20 overflow-y-auto whitespace-pre-wrap font-mono text-[11px] text-red-400/90">
            {task.last_error}
          </pre>
        </div>
      )}

      {/* Agent outputs */}
      {task.outputs && task.outputs.length > 0 ? (
        <OutputTabs outputs={task.outputs} />
      ) : (
        <div className="flex flex-1 items-center justify-center text-xs text-zinc-700">
          No agent outputs yet
        </div>
      )}
    </div>
  );
}

function OutputTabs({ outputs }: { outputs: { id: number; phase: string; output: string; exit_code: number }[] }) {
  const [activeTab, setActiveTab] = useState(outputs[0].phase + "-" + outputs[0].id);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 border-b border-white/[0.06]">
        {outputs.map((o) => {
          const key = o.phase + "-" + o.id;
          return (
            <button
              key={key}
              onClick={() => setActiveTab(key)}
              className={cn(
                "border-b-2 px-4 py-2 text-[11px] font-medium uppercase tracking-wide transition-colors",
                activeTab === key
                  ? "border-blue-400 text-blue-400"
                  : "border-transparent text-zinc-600 hover:text-zinc-400"
              )}
            >
              {o.phase}
              {o.exit_code === 0 ? (
                <span className="ml-1.5 text-emerald-500">ok</span>
              ) : (
                <span className="ml-1.5 text-red-400">x{o.exit_code}</span>
              )}
            </button>
          );
        })}
      </div>
      <div className="flex-1 overflow-y-auto overscroll-contain">
        {outputs.map((o) => {
          const key = o.phase + "-" + o.id;
          if (activeTab !== key) return null;
          return (
            <pre key={key} className="p-4 font-mono text-[11px] leading-relaxed text-zinc-400">
              {o.output}
            </pre>
          );
        })}
      </div>
    </div>
  );
}

function DetailHeader({ onBack }: { onBack: () => void }) {
  return (
    <div className="flex h-10 shrink-0 items-center gap-3 border-b border-white/[0.06] px-4">
      <button
        onClick={onBack}
        className="rounded-md bg-white/[0.04] px-2.5 py-1 text-[11px] font-medium text-zinc-400 transition-colors hover:bg-white/[0.08] hover:text-zinc-200"
      >
        &larr; Back
      </button>
      <span className="text-[11px] font-medium text-zinc-500">Task Detail</span>
    </div>
  );
}
