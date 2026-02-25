import { PHASES, PHASE_LABELS, effectivePhase } from "@/lib/types";
import { cn } from "@/lib/utils";

export function PhaseTracker({ status }: { status: string }) {
  const effective = effectivePhase(status);
  const currentIdx = PHASES.indexOf(effective as (typeof PHASES)[number]);
  const isFailed = status === "failed";

  return (
    <div className="flex items-center gap-0 overflow-x-auto">
      {PHASES.map((phase, i) => {
        const isDone = i < currentIdx;
        const isCurrent = i === currentIdx;
        return (
          <div key={phase} className="flex shrink-0 items-center">
            {i > 0 && (
              <div className={cn("h-px w-4 md:w-6", isDone || isCurrent ? "bg-zinc-600" : "bg-white/[0.06]")} />
            )}
            <div className="flex flex-col items-center gap-1">
              <div
                className={cn(
                  "h-2 w-2 rounded-full transition-all",
                  isDone && "bg-zinc-500",
                  isCurrent && !isFailed && "bg-blue-400 shadow-[0_0_8px_rgba(96,165,250,0.5)]",
                  isCurrent && isFailed && "bg-red-400 shadow-[0_0_8px_rgba(248,113,113,0.4)]",
                  !isDone && !isCurrent && "bg-white/[0.06]"
                )}
              />
              <span
                className={cn(
                  "text-[8px] font-medium uppercase tracking-wider",
                  isDone && "text-zinc-600",
                  isCurrent && !isFailed && "text-blue-400",
                  isCurrent && isFailed && "text-red-400",
                  !isDone && !isCurrent && "text-zinc-700"
                )}
              >
                {PHASE_LABELS[phase]}
              </span>
            </div>
          </div>
        );
      })}
    </div>
  );
}
