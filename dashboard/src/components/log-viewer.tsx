import { useState, useRef, useEffect } from "react";
import { cn } from "@/lib/utils";
import type { LogEvent } from "@/lib/types";

const FILTERS = ["all", "info", "warn", "err"] as const;

export function LogViewer({ logs }: { logs: LogEvent[] }) {
  const [filter, setFilter] = useState<string>("all");
  const bottomRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);

  const filtered = filter === "all" ? logs : logs.filter((l) => l.level === filter);

  useEffect(() => {
    if (autoScroll && bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: "instant" });
    }
  }, [filtered.length, autoScroll]);

  function handleScroll() {
    const el = containerRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 50;
    setAutoScroll(atBottom);
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-10 shrink-0 items-center justify-between border-b border-white/[0.06] px-4">
        <span className="text-[12px] md:text-[11px] font-medium text-zinc-400">Logs</span>
        <div className="flex gap-0.5">
          {FILTERS.map((f) => (
            <button
              key={f}
              className={cn(
                "rounded-md px-2.5 py-1.5 md:py-1 text-[11px] md:text-[10px] font-medium uppercase tracking-wide transition-colors",
                filter === f
                  ? "bg-white/[0.08] text-zinc-200"
                  : "text-zinc-600 hover:text-zinc-400"
              )}
              onClick={() => setFilter(f)}
            >
              {f}
            </button>
          ))}
        </div>
      </div>
      <div
        ref={containerRef}
        onScroll={handleScroll}
        className="flex-1 overflow-y-auto overscroll-contain"
      >
        <div className="p-3">
          {filtered.map((log, i) => (
            <LogLine key={i} log={log} />
          ))}
          <div ref={bottomRef} />
        </div>
      </div>
    </div>
  );
}

const levelColors: Record<string, string> = {
  info: "text-blue-400/80",
  warn: "text-amber-400/80",
  err: "text-red-400/80",
  debug: "text-zinc-600",
};

function LogLine({ log }: { log: LogEvent }) {
  const ts = new Date(log.ts * 1000).toLocaleTimeString("en-GB", { hour12: false });
  return (
    <div className="whitespace-pre-wrap break-all py-px font-mono text-[12px] md:text-[11px] leading-relaxed">
      <span className="text-zinc-600">{ts}</span>{" "}
      <span className={levelColors[log.level] ?? "text-zinc-500"}>{log.level.padEnd(4)}</span>{" "}
      <span className="text-zinc-300">{log.message}</span>
    </div>
  );
}
