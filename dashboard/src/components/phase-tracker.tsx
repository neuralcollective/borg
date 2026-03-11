import { getDisplayPhases, getPhaseLabel } from "@/lib/types";
import { cn } from "@/lib/utils";

export function PhaseTracker({ status, mode }: { status: string; mode?: string }) {
  const phases = getDisplayPhases(mode);
  const currentIdx = phases.indexOf(status);
  const isFailed = status === "failed";

  return (
    <div className="flex items-center gap-0 overflow-x-auto py-1">
      {phases.map((phase, i) => {
        const isDone = i < currentIdx;
        const isCurrent = i === currentIdx;
        return (
          <div key={phase} className="flex shrink-0 items-center">
            {i > 0 && <div className={cn("h-px w-6", isDone ? "bg-[#6b6459]" : "bg-white/[0.06]")} />}
            <div className="flex flex-col items-center gap-1.5">
              <div
                className={cn(
                  "flex h-5 w-5 items-center justify-center rounded-full transition-all",
                  isDone && "bg-emerald-500 text-white",
                  isCurrent &&
                    !isFailed &&
                    "bg-amber-500 text-white shadow-[0_0_10px_rgba(200,160,60,0.4)] animate-pulse",
                  isCurrent && isFailed && "bg-red-500 text-white shadow-[0_0_10px_rgba(239,68,68,0.4)]",
                  !isDone && !isCurrent && "border border-[#2a2520] bg-transparent",
                )}
              >
                {isDone && (
                  <svg className="h-2.5 w-2.5" viewBox="0 0 12 12" fill="none">
                    <path
                      d="M2.5 6L5 8.5L9.5 3.5"
                      stroke="currentColor"
                      strokeWidth="2"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    />
                  </svg>
                )}
                {isCurrent && <div className="h-1.5 w-1.5 rounded-full bg-white" />}
              </div>
              <span
                className={cn(
                  "text-[11px] font-medium",
                  isDone && "text-zinc-400",
                  isCurrent && !isFailed && "text-amber-400",
                  isCurrent && isFailed && "text-red-400",
                  !isDone && !isCurrent && "text-zinc-600",
                )}
              >
                {getPhaseLabel(phase, mode)}
              </span>
            </div>
          </div>
        );
      })}
    </div>
  );
}
