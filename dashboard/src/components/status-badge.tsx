import { cn } from "@/lib/utils";

const statusStyles: Record<string, string> = {
  backlog: "bg-zinc-500/10 text-zinc-400 ring-zinc-500/20",
  spec: "bg-blue-500/10 text-blue-400 ring-blue-500/20",
  qa: "bg-cyan-500/10 text-cyan-400 ring-cyan-500/20",
  impl: "bg-amber-500/10 text-amber-400 ring-amber-500/20",
  retry: "bg-red-500/10 text-red-400 ring-red-500/20",
  qa_fix: "bg-cyan-500/10 text-cyan-400 ring-cyan-500/20",
  done: "bg-emerald-500/10 text-emerald-400 ring-emerald-500/20",
  merged: "bg-emerald-500/10 text-emerald-300 ring-emerald-500/20",
  rebase: "bg-violet-500/10 text-violet-400 ring-violet-500/20",
  failed: "bg-red-500/10 text-red-400 ring-red-500/20",
  queued: "bg-blue-500/10 text-blue-400 ring-blue-500/20",
  merging: "bg-amber-500/10 text-amber-400 ring-amber-500/20",
  excluded: "bg-red-500/10 text-red-400 ring-red-500/20",
};

export function StatusBadge({ status }: { status: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-medium ring-1 ring-inset",
        statusStyles[status] ?? "bg-zinc-500/10 text-zinc-400 ring-zinc-500/20"
      )}
    >
      {status}
    </span>
  );
}
