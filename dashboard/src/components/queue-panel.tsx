import { useQueue, useStatus } from "@/lib/api";
import { repoName } from "@/lib/types";
import { StatusBadge } from "./status-badge";

export function QueuePanel() {
  const { data: queue } = useQueue();
  const { data: status } = useStatus();
  const multiRepo = (status?.watched_repos?.length ?? 0) > 1;

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-10 shrink-0 items-center justify-between border-b border-white/[0.06] px-4">
        <span className="text-[11px] font-medium text-zinc-400">Integration Queue</span>
        <span className="rounded-full bg-white/[0.06] px-2 py-0.5 text-[10px] tabular-nums text-zinc-500">{queue?.length ?? 0}</span>
      </div>
      <div className="flex-1 overflow-y-auto overscroll-contain">
        <div className="p-2">
          {queue?.map((e) => (
            <div key={e.id} className="flex items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-[12px] transition-colors hover:bg-white/[0.02]">
              <StatusBadge status={e.status} />
              {multiRepo && e.repo_path && (
                <span className="shrink-0 rounded-md bg-white/[0.04] px-1.5 py-0.5 text-[9px] font-medium text-zinc-500">
                  {repoName(e.repo_path)}
                </span>
              )}
              <span className="flex-1 truncate font-mono text-zinc-300">{e.branch}</span>
              <span className="font-mono text-[10px] text-zinc-600">#{e.task_id}</span>
            </div>
          ))}
          {!queue?.length && <p className="py-8 text-center text-xs text-zinc-700">Queue empty</p>}
        </div>
      </div>
    </div>
  );
}
