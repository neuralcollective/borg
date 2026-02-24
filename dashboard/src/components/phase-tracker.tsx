import { PHASES, PHASE_LABELS, effectivePhase } from "@/lib/types";
import { cn } from "@/lib/utils";

export function PhaseTracker({ status }: { status: string }) {
  const effective = effectivePhase(status);
  const currentIdx = PHASES.indexOf(effective as (typeof PHASES)[number]);
  const isFailed = status === "failed";

  return (
    <div className="flex items-start gap-0">
      {PHASES.map((phase, i) => {
        const isDone = i < currentIdx;
        const isCurrent = i === currentIdx;
        return (
          <div key={phase} className="flex items-center">
            {i > 0 && (
              <div className={cn("h-0.5 w-5", isDone || isCurrent ? "bg-muted-foreground/40" : "bg-border")} />
            )}
            <div className="flex flex-col items-center gap-1">
              <div
                className={cn(
                  "h-2.5 w-2.5 rounded-full border-2",
                  isDone && "border-muted-foreground bg-muted-foreground",
                  isCurrent && !isFailed && "border-blue-400 bg-blue-400 shadow-[0_0_8px_rgba(96,165,250,0.5)]",
                  isCurrent && isFailed && "border-red-400 bg-red-400",
                  !isDone && !isCurrent && "border-border bg-background"
                )}
              />
              <span
                className={cn(
                  "text-[8px] uppercase",
                  isDone && "text-muted-foreground",
                  isCurrent && !isFailed && "text-blue-400",
                  isCurrent && isFailed && "text-red-400",
                  !isDone && !isCurrent && "text-muted-foreground/30"
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
