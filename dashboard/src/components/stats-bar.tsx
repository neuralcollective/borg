import { useStatus } from "@/lib/api";

function Stat({ value, label, color }: { value: number | string; label: string; color: string }) {
  return (
    <div className="flex items-center gap-1.5">
      <span className={`text-lg font-bold ${color}`}>{value}</span>
      <span className="text-[10px] uppercase tracking-wide text-muted-foreground">{label}</span>
    </div>
  );
}

export function StatsBar() {
  const { data: status } = useStatus();

  return (
    <div className="flex gap-6 border-b border-border bg-card px-5 py-2">
      <Stat value={status?.active_tasks ?? "-"} label="active" color="text-blue-400" />
      <Stat value={status?.merged_tasks ?? "-"} label="merged" color="text-green-400" />
      <Stat value={status?.failed_tasks ?? "-"} label="failed" color="text-red-400" />
      <Stat value={status?.total_tasks ?? "-"} label="total" color="text-muted-foreground" />
    </div>
  );
}
