import { useState, useRef, useEffect } from "react";
import { cn } from "@/lib/utils";
import type { LogEvent, DbEvent } from "@/lib/types";

const LEVEL_FILTERS = ["all", "info", "warn", "err"] as const;
const CATEGORY_FILTERS = ["all", "system", "chat", "agent", "pipeline"] as const;

type ViewMode = "live" | "events";

export function LogViewer({ logs }: { logs: LogEvent[] }) {
  const [levelFilter, setLevelFilter] = useState<string>("all");
  const [categoryFilter, setCategoryFilter] = useState<string>("all");
  const [viewMode, setViewMode] = useState<ViewMode>("live");
  const [events, setEvents] = useState<DbEvent[]>([]);
  const [loadingEvents, setLoadingEvents] = useState(false);
  const bottomRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);

  // Fetch historical events when switching to events view or changing filters
  useEffect(() => {
    if (viewMode !== "events") return;
    setLoadingEvents(true);
    const params = new URLSearchParams();
    if (categoryFilter !== "all") params.set("category", categoryFilter);
    if (levelFilter !== "all") params.set("level", levelFilter === "warn" ? "warning" : levelFilter);
    params.set("limit", "500");
    fetch(`/api/events?${params}`)
      .then((r) => r.json())
      .then((data: DbEvent[]) => {
        setEvents(data.reverse()); // API returns newest-first, we want oldest-first
        setLoadingEvents(false);
      })
      .catch(() => setLoadingEvents(false));
  }, [viewMode, categoryFilter, levelFilter]);

  useEffect(() => {
    if (autoScroll && bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: "instant" });
    }
  }, [logs.length, events.length, autoScroll]);

  function handleScroll() {
    const el = containerRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 50;
    setAutoScroll(atBottom);
  }

  const filteredLogs =
    viewMode === "live"
      ? levelFilter === "all"
        ? logs
        : logs.filter((l) => l.level === levelFilter)
      : [];

  return (
    <div className="flex h-full flex-col">
      <div className="flex shrink-0 flex-col border-b border-white/[0.06]">
        {/* Mode toggle + level filter */}
        <div className="flex h-10 items-center justify-between px-4">
          <div className="flex gap-1">
            <button
              className={cn(
                "rounded-md px-2.5 py-1 text-[11px] md:text-[10px] font-medium transition-colors",
                viewMode === "live"
                  ? "bg-white/[0.08] text-zinc-200"
                  : "text-zinc-600 hover:text-zinc-400"
              )}
              onClick={() => setViewMode("live")}
            >
              Live
            </button>
            <button
              className={cn(
                "rounded-md px-2.5 py-1 text-[11px] md:text-[10px] font-medium transition-colors",
                viewMode === "events"
                  ? "bg-white/[0.08] text-zinc-200"
                  : "text-zinc-600 hover:text-zinc-400"
              )}
              onClick={() => setViewMode("events")}
            >
              Events
            </button>
          </div>
          <div className="flex gap-0.5">
            {LEVEL_FILTERS.map((f) => (
              <button
                key={f}
                className={cn(
                  "rounded-md px-2.5 py-1.5 md:py-1 text-[11px] md:text-[10px] font-medium uppercase tracking-wide transition-colors",
                  levelFilter === f
                    ? "bg-white/[0.08] text-zinc-200"
                    : "text-zinc-600 hover:text-zinc-400"
                )}
                onClick={() => setLevelFilter(f)}
              >
                {f}
              </button>
            ))}
          </div>
        </div>

        {/* Category filter (events mode only) */}
        {viewMode === "events" && (
          <div className="flex items-center gap-0.5 px-4 pb-2">
            {CATEGORY_FILTERS.map((f) => (
              <button
                key={f}
                className={cn(
                  "rounded-md px-2 py-1 text-[10px] font-medium transition-colors",
                  categoryFilter === f
                    ? "bg-blue-500/20 text-blue-300"
                    : "text-zinc-600 hover:text-zinc-400"
                )}
                onClick={() => setCategoryFilter(f)}
              >
                {f}
              </button>
            ))}
          </div>
        )}
      </div>

      <div
        ref={containerRef}
        onScroll={handleScroll}
        className="flex-1 overflow-y-auto overscroll-contain"
      >
        <div className="p-3">
          {viewMode === "live" ? (
            filteredLogs.map((log, i) => <LogLine key={i} log={log} />)
          ) : loadingEvents ? (
            <div className="text-[11px] text-zinc-600 py-4 text-center">Loading events...</div>
          ) : events.length === 0 ? (
            <div className="text-[11px] text-zinc-600 py-4 text-center">No events found</div>
          ) : (
            events.map((ev) => <EventLine key={ev.id} event={ev} />)
          )}
          <div ref={bottomRef} />
        </div>
      </div>
    </div>
  );
}

const levelColors: Record<string, string> = {
  info: "text-blue-400/80",
  warn: "text-amber-400/80",
  warning: "text-amber-400/80",
  err: "text-red-400/80",
  error: "text-red-400/80",
  debug: "text-zinc-600",
};

const categoryColors: Record<string, string> = {
  system: "text-zinc-500",
  chat: "text-green-400/70",
  agent: "text-purple-400/70",
  pipeline: "text-cyan-400/70",
};

function safeText(v: unknown, fallback: string) {
  return typeof v === "string" && v.length > 0 ? v : fallback;
}

function safeTimestamp(ts: unknown): number | null {
  if (typeof ts === "number" && Number.isFinite(ts) && ts > 0) return ts;
  if (typeof ts === "string") {
    const parsed = Number(ts);
    if (Number.isFinite(parsed) && parsed > 0) return parsed;
  }
  return null;
}

function formatTime(ts: unknown): string {
  const seconds = safeTimestamp(ts);
  if (seconds === null) return "--:--:--";
  return new Date(seconds * 1000).toLocaleTimeString("en-GB", { hour12: false });
}

function formatDate(ts: unknown): string {
  const seconds = safeTimestamp(ts);
  if (seconds === null) return "-- ---";
  return new Date(seconds * 1000).toLocaleDateString("en-GB", {
    day: "2-digit",
    month: "short",
  });
}

function LogLine({ log }: { log: LogEvent }) {
  const ts = formatTime((log as unknown as { ts?: unknown }).ts);
  const level = safeText((log as unknown as { level?: unknown }).level, "info");
  const message = safeText((log as unknown as { message?: unknown }).message, "");
  return (
    <div className="whitespace-pre-wrap break-all py-px font-mono text-[12px] md:text-[11px] leading-relaxed">
      <span className="text-zinc-600">{ts}</span>{" "}
      <span className={levelColors[level] ?? "text-zinc-500"}>{`${level}`.padEnd(4)}</span>{" "}
      <span className="text-zinc-300">{message}</span>
    </div>
  );
}

function EventLine({ event }: { event: DbEvent }) {
  const ts = formatTime((event as unknown as { ts?: unknown }).ts);
  const date = formatDate((event as unknown as { ts?: unknown }).ts);
  const level = safeText((event as unknown as { level?: unknown }).level, "info");
  const category = safeText((event as unknown as { category?: unknown }).category, "system");
  const message = safeText((event as unknown as { message?: unknown }).message, "");
  return (
    <div className="whitespace-pre-wrap break-all py-px font-mono text-[12px] md:text-[11px] leading-relaxed">
      <span className="text-zinc-600">
        {date} {ts}
      </span>{" "}
      <span className={levelColors[level] ?? "text-zinc-500"}>
        {`${level}`.padEnd(5)}
      </span>{" "}
      <span className={categoryColors[category] ?? "text-zinc-500"}>
        [{category}]
      </span>{" "}
      <span className="text-zinc-300">{message}</span>
      {event.metadata && (
        <span className="text-zinc-600 ml-1">{event.metadata}</span>
      )}
    </div>
  );
}
