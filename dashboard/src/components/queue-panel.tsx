import { useQueue, useStatus } from "@/lib/api";
import { repoName } from "@/lib/types";
import { StatusBadge } from "./status-badge";
import { GitMerge } from "lucide-react";

interface QueuePanelProps {
  repoFilter: string | null;
}

export function QueuePanel({ repoFilter }: QueuePanelProps) {
  const { data: queue } = useQueue();
  const { data: status } = useStatus();
  const multiRepo = (status?.watched_repos?.length ?? 0) > 1;

  const filtered = repoFilter
    ? queue?.filter((e) => e.repo_path === repoFilter)
    : queue;

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-14 shrink-0 items-center justify-between border-b border-white/[0.07] px-5">
        <span className="text-[14px] font-semibold text-zinc-100">Integration Queue</span>
        <span className="rounded-full bg-white/[0.05] px-2.5 py-0.5 text-[11px] tabular-nums text-zinc-500">{filtered?.length ?? 0}</span>
      </div>
      <div className="flex-1 overflow-y-auto overscroll-contain">
        <div className="p-2">
          {filtered?.map((e) => (
            <div key={e.id} className="flex items-center gap-3 rounded-xl px-4 py-3 text-[13px] transition-colors hover:bg-white/[0.03]">
              <StatusBadge status={e.status} />
              {multiRepo && !repoFilter && e.repo_path && (
                <span className="shrink-0 rounded-lg bg-white/[0.04] px-2 py-0.5 text-[11px] font-medium text-zinc-500">
                  {repoName(e.repo_path)}
                </span>
              )}
              <span className="flex-1 truncate font-mono text-[13px] text-zinc-300">{e.branch}</span>
              <span className="font-mono text-[11px] text-zinc-600">#{e.task_id}</span>
            </div>
          ))}
          {!filtered?.length && (
            <div className="flex flex-col items-center justify-center py-20 text-center">
              <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-white/[0.04] ring-1 ring-white/[0.06]">
                <GitMerge className="h-6 w-6 text-zinc-600" strokeWidth={1.5} />
              </div>
              <p className="text-[14px] text-zinc-400">Queue is empty</p>
              <p className="mt-1 text-[12px] text-zinc-600">Completed tasks will appear here for integration</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
