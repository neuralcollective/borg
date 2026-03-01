import { useRef, useEffect, useMemo } from "react";
import type { StreamEvent } from "@/lib/api";
import { parseStreamEvents, type TermLine } from "@/lib/stream-utils";
import { cn } from "@/lib/utils";

export function LiveTerminal({ events, streaming }: LiveTerminalProps) {
  const bottomRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const lines = useMemo(() => parseStreamEvents(events), [events]);

  useEffect(() => {
    if (bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: "instant" });
    }
  }, [lines.length]);

  return (
    <div className="flex flex-col h-full rounded-lg border border-white/[0.08] bg-black overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-1.5 border-b border-white/[0.06] bg-white/[0.02]">
        <div className={cn(
          "h-2 w-2 rounded-full transition-colors",
          streaming
            ? "bg-emerald-400 shadow-[0_0_6px_rgba(52,211,153,0.5)] animate-pulse"
            : "bg-zinc-600"
        )} />
        <span className="text-[11px] font-medium text-zinc-500">
          {streaming ? "Live" : events.length > 0 ? "Stream ended" : "Waiting..."}
        </span>
        <span className="ml-auto text-[10px] tabular-nums text-zinc-700">
          {events.length} events
        </span>
      </div>

      <div
        ref={containerRef}
        className="flex-1 overflow-y-auto overscroll-contain font-mono text-[11px] leading-relaxed p-3 space-y-1"
      >
        {lines.length === 0 && (
          <div className="flex items-center gap-2 text-zinc-700 py-8 justify-center">
            {streaming && <span className="animate-pulse">Connecting to agent...</span>}
            {!streaming && <span>No live stream available</span>}
          </div>
        )}

        {lines.map((line, i) => (
          <TermLineView key={i} line={line} />
        ))}

        {streaming && (
          <div className="inline-block w-1.5 h-3.5 bg-zinc-400 animate-[blink_1s_steps(1)_infinite] ml-0.5" />
        )}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}

function TermLineView({ line }: { line: TermLine }) {
  if (line.type === "system") {
    return (
      <div className="text-blue-500/60 text-[10px]">
        <span className="text-blue-500/40">sys</span> {line.content}
      </div>
    );
  }

  if (line.type === "text") {
    return (
      <div className="border-y border-white/[0.06] py-2 my-1 text-[12px] text-zinc-200 whitespace-pre-wrap break-words">
        {line.content}
      </div>
    );
  }

  if (line.type === "tool") {
    return (
      <div className="rounded bg-white/[0.03] px-2 py-1.5">
        <div className="flex items-center gap-2">
          <span className="shrink-0 rounded bg-amber-500/20 px-1.5 py-0.5 text-[9px] font-bold text-amber-400">
            {line.tool}
          </span>
          {line.label && (
            <span className="text-zinc-400 text-[10px] truncate">{line.label}</span>
          )}
        </div>
        {line.content && (
          <div className="mt-1 text-zinc-500 text-[10px] break-all whitespace-pre-wrap">{line.content}</div>
        )}
      </div>
    );
  }

  if (line.type === "tool_result") {
    return (
      <div className="text-zinc-600 text-[10px] break-all whitespace-pre-wrap pl-2 border-l border-zinc-800">
        {line.content}
      </div>
    );
  }

  if (line.type === "result") {
    return (
      <div className="text-emerald-400/80 pt-1 border-t border-emerald-500/10 mt-1">
        <span className="text-emerald-500/50 text-[9px] uppercase tracking-wider">result</span>
        <div className="whitespace-pre-wrap break-words">{line.content}</div>
      </div>
    );
  }

  if (line.type === "phase_result") {
    return (
      <div className="rounded border border-emerald-500/30 bg-emerald-950/20 px-3 py-2 my-1">
        <div className="text-emerald-400/60 text-[9px] uppercase tracking-wider mb-1">
          Phase result{line.label ? `: ${line.label}` : ""}
        </div>
        <div className="text-emerald-300/80 text-[11px] whitespace-pre-wrap break-words">{line.content}</div>
      </div>
    );
  }

  return null;
}
