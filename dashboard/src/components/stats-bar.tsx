import { useStatus } from "@/lib/api";
import { repoName } from "@/lib/types";

function Stat({ value, label, color }: { value: number | string; label: string; color: string }) {
  return (
    <div className="flex items-baseline gap-1.5">
      <span className={`text-sm font-semibold tabular-nums ${color}`}>{value}</span>
      <span className="text-[10px] text-zinc-600">{label}</span>
    </div>
  );
}

interface StatsBarProps {
  repoFilter: string | null;
  onRepoFilterChange: (repo: string | null) => void;
}

export function StatsBar({ repoFilter, onRepoFilterChange }: StatsBarProps) {
  const { data: status } = useStatus();
  const repos = status?.watched_repos ?? [];
  const multiRepo = repos.length > 1;

  return (
    <div className="flex h-10 shrink-0 items-center gap-4 md:gap-6 border-b border-white/[0.06] px-4 md:px-5 overflow-x-auto">
      {multiRepo && (
        <select
          value={repoFilter ?? ""}
          onChange={(e) => onRepoFilterChange(e.target.value || null)}
          className="h-7 md:h-6 shrink-0 rounded border border-white/[0.08] bg-transparent px-1.5 text-[12px] md:text-[11px] text-zinc-300 outline-none"
        >
          <option value="">All repos</option>
          {repos.map((r) => (
            <option key={r.path} value={r.path}>
              {repoName(r.path)}{!r.auto_merge ? " (manual)" : ""}
            </option>
          ))}
        </select>
      )}
      <Stat value={status?.active_tasks ?? 0} label="Active" color="text-blue-400" />
      <Stat value={status?.merged_tasks ?? 0} label="Merged" color="text-emerald-400" />
      <Stat value={status?.failed_tasks ?? 0} label="Failed" color="text-red-400" />
      <Stat value={status?.total_tasks ?? 0} label="Total" color="text-zinc-400" />
    </div>
  );
}
