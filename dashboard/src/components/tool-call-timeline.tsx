import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { getToolCalls } from "@/lib/api";
import type { ToolCallEvent } from "@/lib/types";
import { isActiveStatus } from "@/lib/types";
import { cn } from "@/lib/utils";

interface ToolCallTimelineProps {
  taskId?: number;
  chatKey?: string;
  runId?: string;
  taskStatus?: string;
}

function durationColor(ms: number): string {
  if (ms < 500) return "bg-emerald-500/15 text-emerald-400";
  if (ms <= 2000) return "bg-amber-500/15 text-amber-400";
  return "bg-red-500/15 text-red-400";
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${(ms / 60_000).toFixed(1)}m`;
}

function ToolCallCard({ event }: { event: ToolCallEvent }) {
  const [expanded, setExpanded] = useState(false);
  const hasDetails = !!(event.input_summary || event.output_summary || event.error);

  return (
    <div className="rounded-lg border border-[#2a2520] bg-[#1c1a17]/40">
      <button
        onClick={() => hasDetails && setExpanded(!expanded)}
        className={cn(
          "flex w-full items-center gap-2.5 px-3 py-2 text-left",
          hasDetails && "cursor-pointer hover:bg-white/[0.02]",
          !hasDetails && "cursor-default",
        )}
      >
        {/* success/error indicator */}
        <span className="shrink-0">
          {event.success === false ? (
            <span className="inline-flex h-4 w-4 items-center justify-center rounded-full bg-red-500/15 text-[10px] text-red-400">
              ✕
            </span>
          ) : event.success === true ? (
            <span className="inline-flex h-4 w-4 items-center justify-center rounded-full bg-emerald-500/15 text-[10px] text-emerald-400">
              ✓
            </span>
          ) : (
            <span className="inline-flex h-4 w-4 items-center justify-center rounded-full bg-[#2a2520] text-[10px] text-[#6b6459]">
              ?
            </span>
          )}
        </span>

        {/* tool name */}
        <span className="shrink-0 rounded bg-amber-500/15 px-1.5 py-0.5 font-mono text-[10px] font-bold text-amber-400">
          {event.tool_name}
        </span>

        {/* input summary preview */}
        {event.input_summary && !expanded && (
          <span className="min-w-0 truncate text-[11px] text-[#6b6459]">
            {event.input_summary.length > 80 ? `${event.input_summary.slice(0, 80)}...` : event.input_summary}
          </span>
        )}

        <span className="ml-auto flex shrink-0 items-center gap-2">
          {/* duration badge */}
          {event.duration_ms != null && (
            <span className={cn("rounded px-1.5 py-0.5 text-[10px] font-medium", durationColor(event.duration_ms))}>
              {formatDuration(event.duration_ms)}
            </span>
          )}
          {/* expand indicator */}
          {hasDetails && (
            <span className="text-[9px] text-[#6b6459]">{expanded ? "▲" : "▼"}</span>
          )}
        </span>
      </button>

      {expanded && hasDetails && (
        <div className="border-t border-[#2a2520] px-3 py-2 space-y-2">
          {event.input_summary && (
            <div>
              <div className="mb-0.5 text-[9px] font-medium uppercase tracking-wider text-[#6b6459]">Input</div>
              <pre className="max-h-40 overflow-y-auto whitespace-pre-wrap font-mono text-[10px] leading-relaxed text-[#9c9486]">
                {event.input_summary}
              </pre>
            </div>
          )}
          {event.output_summary && (
            <div>
              <div className="mb-0.5 text-[9px] font-medium uppercase tracking-wider text-[#6b6459]">Output</div>
              <pre className="max-h-40 overflow-y-auto whitespace-pre-wrap font-mono text-[10px] leading-relaxed text-[#9c9486]">
                {event.output_summary}
              </pre>
            </div>
          )}
          {event.error && (
            <div>
              <div className="mb-0.5 text-[9px] font-medium uppercase tracking-wider text-red-400/60">Error</div>
              <pre className="max-h-40 overflow-y-auto whitespace-pre-wrap font-mono text-[10px] leading-relaxed text-red-400/80">
                {event.error}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function ToolCallTimeline({ taskId, chatKey, runId, taskStatus }: ToolCallTimelineProps) {
  const isActive = taskStatus ? isActiveStatus(taskStatus) : false;

  const { data: toolCalls = [], isLoading, isError } = useQuery<ToolCallEvent[]>({
    queryKey: ["tool_calls", taskId, chatKey, runId],
    queryFn: () => getToolCalls({ taskId, chatKey, runId, limit: 500 }),
    refetchInterval: isActive ? 5_000 : false,
    staleTime: isActive ? 3_000 : 30_000,
  });

  const totalMs = toolCalls.reduce((sum, tc) => sum + (tc.duration_ms ?? 0), 0);

  if (isLoading) {
    return <div className="flex h-full items-center justify-center text-xs text-[#6b6459]">Loading tool calls...</div>;
  }

  if (isError) {
    return <div className="flex h-full items-center justify-center text-xs text-red-400/70">Failed to load tool calls</div>;
  }

  if (toolCalls.length === 0) {
    return <div className="flex h-full items-center justify-center text-xs text-[#6b6459]">No tool calls recorded</div>;
  }

  return (
    <div className="flex h-full flex-col">
      {/* Stats header */}
      <div className="shrink-0 border-b border-[#2a2520] px-4 py-2.5 flex items-center gap-3 text-[11px] text-[#9c9486]">
        <span>
          <span className="font-medium text-[#e8e0d4]">{toolCalls.length}</span> tool call{toolCalls.length !== 1 ? "s" : ""}
        </span>
        <span className="text-[#6b6459]">/</span>
        <span>
          <span className="font-medium text-[#e8e0d4]">{formatDuration(totalMs)}</span> total
        </span>
        {isActive && (
          <span className="ml-auto flex items-center gap-1.5 text-[10px] text-emerald-400/60">
            <span className="inline-block h-1.5 w-1.5 rounded-full bg-emerald-400 animate-pulse" />
            live
          </span>
        )}
      </div>

      {/* Timeline list */}
      <div className="flex-1 overflow-y-auto overscroll-contain p-4 space-y-1.5">
        {toolCalls.map((tc) => (
          <ToolCallCard key={tc.id} event={tc} />
        ))}
      </div>
    </div>
  );
}
