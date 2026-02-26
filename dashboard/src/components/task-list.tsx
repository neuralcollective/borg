import { useTasks, useStatus } from "@/lib/api";
import { isActiveStatus, repoName } from "@/lib/types";
import { useUIMode } from "@/lib/ui-mode";
import { StatusBadge } from "./status-badge";
import { cn } from "@/lib/utils";

interface TaskListProps {
  selectedId: number | null;
  onSelect: (id: number) => void;
  repoFilter: string | null;
}

export function TaskList({ selectedId, onSelect, repoFilter }: TaskListProps) {
  const { data: tasks } = useTasks();
  const { data: status } = useStatus();
  const multiRepo = (status?.watched_repos?.length ?? 0) > 1;

  const filtered = repoFilter
    ? tasks?.filter((t) => t.repo_path === repoFilter)
    : tasks;
  const active = filtered?.filter((t) => isActiveStatus(t.status)) ?? [];
  const done = filtered?.filter((t) => !isActiveStatus(t.status)) ?? [];

  return (
    <div className="flex h-full flex-col">
      {/* Stats row */}
      <div className="flex shrink-0 items-center gap-3 border-b border-white/[0.06] px-4 py-2.5">
        <Stat value={status?.active_tasks ?? 0} label="Active" color="text-blue-400" />
        <Stat value={status?.merged_tasks ?? 0} label="Merged" color="text-emerald-400" />
        <Stat value={status?.failed_tasks ?? 0} label="Failed" color="text-red-400" />
        <span className="ml-auto rounded-full bg-white/[0.04] px-2 py-0.5 text-[10px] tabular-nums text-zinc-600">
          {filtered?.length ?? 0}
        </span>
      </div>

      <div className="flex-1 overflow-y-auto overscroll-contain">
        <div className="p-1.5">
          {active.map((t) => (
            <TaskRow key={t.id} task={t} isActive showRepo={multiRepo && !repoFilter} selected={selectedId === t.id} onClick={() => onSelect(t.id)} />
          ))}
          {done.length > 0 && active.length > 0 && (
            <div className="mx-3 my-2 h-px bg-white/[0.04]" />
          )}
          {done.slice(0, 30).map((t) => (
            <TaskRow key={t.id} task={t} showRepo={multiRepo && !repoFilter} selected={selectedId === t.id} onClick={() => onSelect(t.id)} />
          ))}
          {!filtered?.length && (
            <p className="py-12 text-center text-xs text-zinc-700">No tasks yet</p>
          )}
        </div>
      </div>
    </div>
  );
}

function Stat({ value, label, color }: { value: number; label: string; color: string }) {
  return (
    <div className="flex items-baseline gap-1">
      <span className={cn("text-[13px] font-semibold tabular-nums", color)}>{value}</span>
      <span className="text-[10px] text-zinc-600">{label}</span>
    </div>
  );
}

function TaskRow({
  task,
  isActive,
  showRepo,
  selected,
  onClick,
}: {
  task: { id: number; title: string; status: string; repo_path?: string; attempt: number; max_attempts: number; last_error?: string; mode?: string };
  isActive?: boolean;
  showRepo?: boolean;
  selected?: boolean;
  onClick: () => void;
}) {
  const { mode: uiMode } = useUIMode();
  const isMinimal = uiMode === "minimal";
  const isStuck = task.attempt >= 3 && isActive;

  return (
    <button
      onClick={onClick}
      className={cn(
        "flex w-full flex-col gap-0.5 rounded-lg px-3 py-2 text-left transition-all",
        "hover:bg-white/[0.03]",
        isActive && !isStuck && "bg-blue-500/[0.03]",
        isStuck && "bg-red-500/[0.04]",
        selected && "bg-white/[0.07] ring-1 ring-inset ring-white/[0.08]"
      )}
    >
      <div className="flex w-full items-center gap-2">
        <span className="min-w-[22px] font-mono text-[10px] text-zinc-600">#{task.id}</span>
        <StatusBadge status={task.status} />
        {showRepo && task.repo_path && (
          <span className="shrink-0 rounded bg-white/[0.04] px-1.5 py-0.5 text-[9px] font-medium text-zinc-500">
            {repoName(task.repo_path)}
          </span>
        )}
        {task.mode && task.mode !== "swe" && (
          <span className="shrink-0 rounded bg-violet-500/10 px-1.5 py-0.5 text-[9px] font-medium text-violet-400">
            {task.mode}
          </span>
        )}
      </div>
      <div className="flex items-center gap-2 pl-[30px]">
        <span className="flex-1 truncate text-[12px] text-zinc-300">{task.title}</span>
        {!isMinimal && task.attempt > 0 && (
          <span className={cn("shrink-0 font-mono text-[10px]", isStuck ? "text-red-400/80" : "text-zinc-600")}>
            {task.attempt}/{task.max_attempts}
          </span>
        )}
      </div>
      {task.last_error && isActive && (
        <div className="pl-[30px] truncate text-[10px] text-red-400/60">
          {task.last_error}
        </div>
      )}
    </button>
  );
}
