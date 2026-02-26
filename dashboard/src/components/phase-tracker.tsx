import { getDisplayPhases, getPhaseLabel, effectivePhase } from "@/lib/types";
import { cn } from "@/lib/utils";

export function PhaseTracker({ status, mode }: { status: string; mode?: string }) {
  const phases = getDisplayPhases(mode);
  const effective = effectivePhase(status, mode);
  const currentIdx = phases.indexOf(effective);
  const isFailed = status === "failed";

  return (
    <div className="flex items-center gap-0 overflow-x-auto">
      {phases.map((phase, i) => {
        const isDone = i < currentIdx;
        const isCurrent = i === currentIdx;
        return (
          <div key={phase} className="flex shrink-0 items-center">
            {i > 0 && (
              <div className={cn("h-px w-4", isDone || isCurrent ? "bg-zinc-600" : "bg-white/[0.06]")} />
            )}
            <span
              className={cn(
                "rounded px-2 py-0.5 text-[11px] font-medium",
                isDone && "text-zinc-500",
                isCurrent && !isFailed && "bg-blue-500/10 text-blue-400 ring-1 ring-inset ring-blue-500/20",
                isCurrent && isFailed && "bg-red-500/10 text-red-400 ring-1 ring-inset ring-red-500/20",
                !isDone && !isCurrent && "text-zinc-700"
              )}
            >
              {getPhaseLabel(phase, mode)}
            </span>
          </div>
        );
      })}
    </div>
  );
}
