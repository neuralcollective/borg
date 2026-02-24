import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

const statusColors: Record<string, string> = {
  backlog: "border-zinc-700 bg-zinc-900 text-zinc-400",
  spec: "border-blue-800 bg-blue-950 text-blue-400",
  qa: "border-cyan-800 bg-cyan-950 text-cyan-400",
  impl: "border-amber-800 bg-amber-950 text-amber-400",
  retry: "border-red-800 bg-red-950 text-red-400",
  done: "border-green-800 bg-green-950 text-green-400",
  merged: "border-emerald-800 bg-emerald-950 text-emerald-400",
  rebase: "border-purple-800 bg-purple-950 text-purple-400",
  failed: "border-red-800 bg-red-950 text-red-400",
  queued: "border-blue-800 bg-blue-950 text-blue-400",
  merging: "border-amber-800 bg-amber-950 text-amber-400",
  excluded: "border-red-800 bg-red-950 text-red-400",
};

export function StatusBadge({ status }: { status: string }) {
  return (
    <Badge
      variant="outline"
      className={cn("text-[10px] font-semibold uppercase tracking-wider", statusColors[status] ?? "")}
    >
      {status}
    </Badge>
  );
}
