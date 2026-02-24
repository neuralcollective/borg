import { useState, useRef, useEffect } from "react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Button } from "@/components/ui/button";
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
      <div className="flex items-center justify-between border-b border-border bg-card px-4 py-2">
        <span className="text-[10px] uppercase tracking-widest text-muted-foreground">Live Logs</span>
        <div className="flex gap-1">
          {FILTERS.map((f) => (
            <Button
              key={f}
              variant="outline"
              size="sm"
              className={cn(
                "h-5 px-2 text-[9px] uppercase",
                filter === f && "border-blue-400 text-blue-400"
              )}
              onClick={() => setFilter(f)}
            >
              {f}
            </Button>
          ))}
        </div>
      </div>
      <ScrollArea className="flex-1" onScrollCapture={handleScroll} ref={containerRef}>
        <div className="space-y-0 p-3">
          {filtered.map((log, i) => (
            <LogLine key={i} log={log} />
          ))}
          <div ref={bottomRef} />
        </div>
      </ScrollArea>
    </div>
  );
}

const levelColors: Record<string, string> = {
  info: "text-blue-400",
  warn: "text-amber-400",
  err: "text-red-400",
  debug: "text-muted-foreground/60",
};

function LogLine({ log }: { log: LogEvent }) {
  const ts = new Date(log.ts * 1000).toLocaleTimeString("en-GB", { hour12: false });
  return (
    <div className="whitespace-pre-wrap break-all py-px font-mono text-[11px] leading-relaxed">
      <span className="text-muted-foreground/40">{ts}</span>{" "}
      <span className={levelColors[log.level] ?? "text-muted-foreground"}>[{log.level}]</span>{" "}
      {log.message}
    </div>
  );
}
