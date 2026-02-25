import { useTasks, useStatus } from "@/lib/api";
import { isActiveStatus, repoName } from "@/lib/types";
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
      <div className="flex h-10 shrink-0 items-center justify-between border-b border-white/[0.06] px-4">
        <span className="text-[12px] md:text-[11px] font-medium text-zinc-400">Pipeline Tasks</span>
        <span className="rounded-full bg-white/[0.06] px-2 py-0.5 text-[10px] tabular-nums text-zinc-500">{filtered?.length ?? 0}</span>
      </div>
      <div className="flex-1 overflow-y-auto overscroll-contain">
        <div className="p-1">
          {active.map((t) => (
            <TaskRow key={t.id} task={t} isActive showRepo={multiRepo && !repoFilter} selected={selectedId === t.id} onClick={() => onSelect(t.id)} />
          ))}
          {done.length > 0 && active.length > 0 && (
            <div className="mx-3 my-1.5 h-px bg-white/[0.04]" />
          )}
          {done.slice(0, 20).map((t) => (
            <TaskRow key={t.id} task={t} showRepo={multiRepo && !repoFilter} selected={selectedId === t.id} onClick={() => onSelect(t.id)} />
          ))}
          {!filtered?.length && <p className="py-12 text-center text-[13px] md:text-xs text-zinc-600">No tasks yet</p>}
        </div>
      </div>
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
  task: { id: number; title: string; status: string; repo_path?: string; attempt: number; max_attempts: number; last_error?: string };
  isActive?: boolean;
  showRepo?: boolean;
  selected?: boolean;
  onClick: () => void;
}) {
  const isStuck = task.attempt >= 3 && isActive;
  const hasError = !!task.last_error;

  return (
    <button
      onClick={onClick}
      className={cn(
        "flex w-full flex-col gap-0.5 rounded-lg px-3 py-2.5 md:py-2 text-left transition-colors active:bg-white/[0.06]",
        "hover:bg-white/[0.03]",
        isActive && !isStuck && "bg-blue-500/[0.04]",
        isStuck && "bg-red-500/[0.04]",
        selected && "bg-white/[0.06]"
      )}
    >
      <div className="flex w-full items-center gap-2.5">
        <span className="min-w-[24px] font-mono text-[10px] text-zinc-600">#{task.id}</span>
        <StatusBadge status={task.status} />
        {showRepo && task.repo_path && (
          <span className="shrink-0 rounded-md bg-white/[0.04] px-1.5 py-0.5 text-[9px] font-medium text-zinc-500">
            {repoName(task.repo_path)}
          </span>
        )}
        <span className="flex-1 truncate text-[13px] md:text-[12px] text-zinc-300">{task.title}</span>
        {task.attempt > 0 && (
          <span className={cn("font-mono text-[10px]", isStuck ? "text-red-400/80" : "text-zinc-600")}>
            {task.attempt}/{task.max_attempts}
          </span>
        )}
      </div>
      {hasError && isActive && (
        <div className="ml-[34px] truncate text-[10px] text-red-400/60">
          {task.last_error}
        </div>
      )}
    </button>
  );
}
