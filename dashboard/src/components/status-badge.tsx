import { cn } from "@/lib/utils";

const statusStyles: Record<string, string> = {
  backlog: "bg-amber-500/10 text-amber-400 ring-amber-500/20",
  implement: "bg-amber-500/10 text-amber-400 ring-amber-500/20",
  validate: "bg-cyan-500/10 text-cyan-400 ring-cyan-500/20",
  lint_fix: "bg-violet-500/10 text-violet-400 ring-violet-500/20",
  rebase: "bg-violet-500/10 text-violet-400 ring-violet-500/20",
  done: "bg-emerald-500/10 text-emerald-400 ring-emerald-500/20",
  merged: "bg-emerald-500/10 text-emerald-300 ring-emerald-500/20",
  failed: "bg-red-500/10 text-red-400 ring-red-500/20",
  blocked: "bg-orange-500/10 text-orange-400 ring-orange-500/20",
  // Integration queue
  queued: "bg-amber-500/10 text-amber-400 ring-amber-500/20",
  merging: "bg-amber-500/10 text-amber-400 ring-amber-500/20",
  excluded: "bg-red-500/10 text-red-400 ring-red-500/20",
  pending_review: "bg-orange-500/10 text-orange-400 ring-orange-500/20",
  // Legal/domain phases
  research: "bg-amber-500/10 text-amber-400 ring-amber-500/20",
  draft: "bg-amber-500/10 text-amber-400 ring-amber-500/20",
  review: "bg-cyan-500/10 text-cyan-400 ring-cyan-500/20",
};

export function StatusBadge({ status }: { status: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full px-2.5 py-0.5 text-[11px] font-medium ring-1 ring-inset",
        statusStyles[status] ?? "bg-[#232019] text-[#9c9486] ring-[#2a2520]"
      )}
    >
      {status}
    </span>
  );
}
