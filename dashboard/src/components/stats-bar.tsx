import { useStatus } from "@/lib/api";

function Stat({ value, label, color }: { value: number | string; label: string; color: string }) {
  return (
    <div className="flex items-baseline gap-1.5">
      <span className={`text-sm font-semibold tabular-nums ${color}`}>{value}</span>
      <span className="text-[10px] text-zinc-600">{label}</span>
    </div>
  );
}

export function StatsBar() {
  const { data: status } = useStatus();

  return (
    <div className="flex h-10 shrink-0 items-center gap-6 border-b border-white/[0.06] px-5">
      <Stat value={status?.active_tasks ?? 0} label="Active" color="text-blue-400" />
      <Stat value={status?.merged_tasks ?? 0} label="Merged" color="text-emerald-400" />
      <Stat value={status?.failed_tasks ?? 0} label="Failed" color="text-red-400" />
      <Stat value={status?.total_tasks ?? 0} label="Total" color="text-zinc-400" />
    </div>
  );
}
