import { useState } from "react";
import { useTasks, useStatus, retryAllFailed } from "@/lib/api";
import { useQueryClient } from "@tanstack/react-query";
import { isActiveStatus, repoName } from "@/lib/types";
import { useUIMode } from "@/lib/ui-mode";
import { useVocabulary } from "@/lib/vocabulary";
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
  const queryClient = useQueryClient();
  const vocab = useVocabulary();
  const multiRepo = (status?.watched_repos?.length ?? 0) > 1;
  const [retryingAll, setRetryingAll] = useState(false);

  const filtered = repoFilter
    ? tasks?.filter((t) => t.repo_path === repoFilter)
    : tasks;
  const terminalStatuses = new Set(["merged", "failed"]);
  const active = filtered?.filter((t) => isActiveStatus(t.status)) ?? [];
  const pending = filtered?.filter((t) => !isActiveStatus(t.status) && !terminalStatuses.has(t.status)) ?? [];
  const terminal = filtered?.filter((t) => terminalStatuses.has(t.status)) ?? [];

  return (
    <div className="flex h-full flex-col">
      {/* Stats row */}
      <div className="flex h-14 shrink-0 items-center gap-3 border-b border-[#2a2520] px-4">
        {!vocab.hideRetryAll && (status?.failed_tasks ?? 0) > 0 && (
          <button
            onClick={async () => {
              setRetryingAll(true);
              try {
                await retryAllFailed();
                await queryClient.invalidateQueries({ queryKey: ["tasks"] });
                await queryClient.invalidateQueries({ queryKey: ["status"] });
              } finally {
                setRetryingAll(false);
              }
            }}
            disabled={retryingAll}
            className="rounded-lg border border-[#2a2520] px-2.5 py-1 text-[12px] text-[#9c9486] hover:border-amber-500/40 hover:text-amber-400 disabled:opacity-50 transition-colors"
          >
            {retryingAll ? "..." : "Retry all failed"}
          </button>
        )}
        <span className="ml-auto rounded-full bg-amber-500/[0.06] px-2.5 py-0.5 text-[11px] tabular-nums text-[#9c9486]">
          {filtered?.length ?? 0} {vocab.taskPlural}
        </span>
      </div>

      <div className="flex-1 overflow-y-auto overscroll-contain">
        <div className="p-2">
          {active.map((t) => (
            <TaskRow key={t.id} task={t} isActive showRepo={multiRepo && !repoFilter} selected={selectedId === t.id} onClick={() => onSelect(t.id)} />
          ))}
          {pending.length > 0 && active.length > 0 && (
            <div className="mx-3.5 my-3 h-px bg-[#2a2520]/50" />
          )}
          {pending.length > 0 && (
            <div className="px-3.5 pt-1.5 pb-1 text-[10px] font-medium uppercase tracking-widest text-[#6b6459]">Pending</div>
          )}
          {pending.map((t) => (
            <TaskRow key={t.id} task={t} showRepo={multiRepo && !repoFilter} selected={selectedId === t.id} onClick={() => onSelect(t.id)} />
          ))}
          {terminal.length > 0 && (active.length > 0 || pending.length > 0) && (
            <div className="mx-3.5 my-3 h-px bg-[#2a2520]/50" />
          )}
          {terminal.slice(0, 30).map((t) => (
            <TaskRow key={t.id} task={t} showRepo={multiRepo && !repoFilter} selected={selectedId === t.id} onClick={() => onSelect(t.id)} />
          ))}
          {!filtered?.length && (
            <p className="py-12 text-center text-xs text-[#6b6459]">No tasks yet</p>
          )}
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
  task: { id: number; title: string; status: string; repo_path?: string; attempt: number; max_attempts: number; last_error?: string; mode?: string };
  isActive?: boolean;
  showRepo?: boolean;
  selected?: boolean;
  onClick: () => void;
}) {
  const { mode: uiMode } = useUIMode();
  const vocab = useVocabulary();
  const isMinimal = uiMode === "minimal";
  const isStuck = task.attempt >= 3 && isActive;

  return (
    <button
      onClick={onClick}
      className={cn(
        "flex w-full flex-col gap-1 rounded-xl px-3.5 py-2.5 text-left transition-all",
        "hover:bg-[#1c1a17]",
        isActive && !isStuck && "bg-amber-500/[0.03]",
        isStuck && "bg-red-500/[0.04]",
        selected && "bg-amber-500/[0.06] ring-1 ring-amber-500/20"
      )}
    >
      <div className="flex w-full items-center gap-2">
        <span className="min-w-[24px] font-mono text-[11px] text-[#6b6459]">#{task.id}</span>
        <StatusBadge status={task.status} />
        {showRepo && task.repo_path && (
          <span className="shrink-0 rounded bg-amber-500/[0.06] px-1.5 py-0.5 text-[10px] font-medium text-[#9c9486]">
            {repoName(task.repo_path)}
          </span>
        )}
        {task.mode && task.mode !== "sweborg" && task.mode !== "swe" && (
          <span className="shrink-0 rounded bg-violet-500/10 px-1.5 py-0.5 text-[10px] font-medium text-violet-400">
            {task.mode}
          </span>
        )}
      </div>
      <div className="flex items-center gap-2 pl-[32px]">
        <span className="flex-1 truncate text-[13px] text-[#e8e0d4]">{task.title}</span>
        {!isMinimal && !vocab.hideAttemptCount && task.attempt > 0 && (
          <span className={cn("shrink-0 font-mono text-[11px]", isStuck ? "text-red-400/80" : "text-[#6b6459]")}>
            {task.attempt}/{task.max_attempts}
          </span>
        )}
      </div>
      {task.last_error && isActive && (
        <div className="pl-[32px] truncate text-[11px] text-red-400/60">
          {task.last_error}
        </div>
      )}
    </button>
  );
}
