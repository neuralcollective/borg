import { useTaskDetail } from "@/lib/api";
import { PhaseTracker } from "./phase-tracker";
import { StatusBadge } from "./status-badge";
import { repoName } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useState, useMemo } from "react";

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
        <OutputSelector outputs={task.outputs} />
      ) : (
        <div className="flex flex-1 items-center justify-center text-xs text-zinc-700">
          No agent outputs yet
        </div>
      )}
    </div>
  );
}

interface OutputEntry {
  id: number;
  phase: string;
  output: string;
  exit_code: number;
}

function OutputSelector({ outputs }: { outputs: OutputEntry[] }) {
  // Label each output: unique phases get plain name, repeated phases get "Phase Attempt #N"
  const labeled = useMemo(() => {
    const phaseCounts: Record<string, number> = {};
    const phaseIndices: Record<string, number> = {};
    for (const o of outputs) {
      phaseCounts[o.phase] = (phaseCounts[o.phase] || 0) + 1;
    }
    return outputs.map((o) => {
      phaseIndices[o.phase] = (phaseIndices[o.phase] || 0) + 1;
      const idx = phaseIndices[o.phase];
      const total = phaseCounts[o.phase];
      const label = total > 1
        ? `${o.phase} attempt #${idx}`
        : o.phase;
      return { ...o, label, isLatest: idx === total };
    });
  }, [outputs]);

  // Default to last output (most recent)
  const [selectedKey, setSelectedKey] = useState(
    labeled[labeled.length - 1].phase + "-" + labeled[labeled.length - 1].id
  );

  const selected = labeled.find((o) => o.phase + "-" + o.id === selectedKey) ?? labeled[labeled.length - 1];

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-white/[0.06] px-4 py-2">
        <select
          value={selectedKey}
          onChange={(e) => setSelectedKey(e.target.value)}
          className="rounded-md border border-white/[0.08] bg-white/[0.04] px-2.5 py-1 text-[11px] font-medium uppercase tracking-wide text-zinc-300 outline-none focus:border-blue-500/40"
        >
          {labeled.map((o) => {
            const key = o.phase + "-" + o.id;
            const status = o.exit_code === 0 ? " \u2713" : ` x${o.exit_code}`;
            return (
              <option key={key} value={key}>
                {o.label}{status}{o.isLatest ? " (latest)" : ""}
              </option>
            );
          })}
        </select>
        <span className={cn(
          "rounded-full px-2 py-0.5 text-[10px] font-medium ring-1 ring-inset",
          selected.exit_code === 0
            ? "bg-emerald-500/10 text-emerald-400 ring-emerald-500/20"
            : "bg-red-500/10 text-red-400 ring-red-500/20"
        )}>
          {selected.exit_code === 0 ? "passed" : `exit ${selected.exit_code}`}
        </span>
      </div>
      <div className="flex-1 overflow-y-auto overscroll-contain">
        <pre className="p-4 font-mono text-[11px] leading-relaxed text-zinc-400">
          {selected.output}
        </pre>
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
