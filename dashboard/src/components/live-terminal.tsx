import { useRef, useEffect, useMemo } from "react";
import type { StreamEvent } from "@/lib/api";
import { cn } from "@/lib/utils";

interface LiveTerminalProps {
  events: StreamEvent[];
  streaming: boolean;
}

interface TermLine {
  type: "system" | "text" | "tool" | "result" | "tool_result";
  tool?: string;
  label?: string;
  content: string;
}

function formatToolInput(tool: string, input: unknown): { label: string; content: string } {
  if (typeof input === "string") return { label: "", content: input };
  if (!input || typeof input !== "object") return { label: "", content: "" };

  const obj = input as Record<string, unknown>;

  // Bash: show description as label, command as content
  if (tool === "Bash") {
    const desc = (obj.description as string) || "";
    const cmd = (obj.command as string) || "";
    return { label: desc, content: cmd };
  }

  // Read: show file path
  if (tool === "Read") {
    const fp = (obj.file_path as string) || "";
    const parts: string[] = [fp];
    if (obj.offset) parts.push(`lines ${obj.offset}–${(obj.offset as number) + ((obj.limit as number) || 200)}`);
    return { label: "", content: parts.join("  ") };
  }

  // Write: show file path
  if (tool === "Write") {
    return { label: "", content: (obj.file_path as string) || "" };
  }

  // Edit: show file path + snippet of old_string
  if (tool === "Edit") {
    const fp = (obj.file_path as string) || "";
    const old = (obj.old_string as string) || "";
    const preview = old.length > 60 ? old.slice(0, 60) + "..." : old;
    return { label: fp, content: preview ? `replacing: ${preview}` : "" };
  }

  // Glob/Grep: show pattern + path
  if (tool === "Glob" || tool === "Grep") {
    const pat = (obj.pattern as string) || "";
    const path = (obj.path as string) || "";
    return { label: "", content: path ? `${pat}  in ${path}` : pat };
  }

  // WebSearch/WebFetch
  if (tool === "WebSearch") return { label: "", content: (obj.query as string) || "" };
  if (tool === "WebFetch") return { label: "", content: (obj.url as string) || "" };

  // Task: show description
  if (tool === "Task") return { label: (obj.description as string) || "", content: (obj.prompt as string)?.slice(0, 120) || "" };

  // Fallback: compact JSON
  const json = JSON.stringify(input);
  const preview = json.length > 200 ? json.slice(0, 200) + "..." : json;
  return { label: "", content: preview };
}

function parseEvents(events: StreamEvent[]): TermLine[] {
  const lines: TermLine[] = [];

  for (const ev of events) {
    if (!ev.type) continue;

    if (ev.type === "system") {
      if (ev.session_id) {
        lines.push({ type: "system", content: `session ${ev.session_id}` });
      }
    } else if (ev.type === "assistant") {
      const msg = ev.message;
      if (!msg?.content) continue;
      if (typeof msg.content === "string") {
        if (msg.content.trim()) lines.push({ type: "text", content: msg.content });
      } else if (Array.isArray(msg.content)) {
        for (const block of msg.content) {
          if (block.type === "text" && block.text?.trim()) {
            lines.push({ type: "text", content: block.text });
          } else if (block.type === "tool_use") {
            const name = block.name || "";
            const { label, content } = formatToolInput(name, block.input);
            lines.push({ type: "tool", tool: name, label, content });
          }
        }
      }
    } else if (ev.type === "tool_result" || ev.type === "tool") {
      const raw = ev.content ?? ev.output ?? "";
      const text = typeof raw === "string"
        ? raw
        : Array.isArray(raw)
          ? raw.map((c: { text?: string }) => c.text || "").join("\n")
          : JSON.stringify(raw);
      if (text.trim()) {
        const preview = text.length > 300 ? text.slice(0, 300) + "..." : text;
        lines.push({
          type: "tool_result",
          tool: ev.tool_name || ev.name || "",
          content: preview,
        });
      }
    } else if (ev.type === "result") {
      if (ev.result) {
        lines.push({ type: "result", content: ev.result });
      }
    }
  }

  return lines;
}

export function LiveTerminal({ events, streaming }: LiveTerminalProps) {
  const bottomRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const lines = useMemo(() => parseEvents(events), [events]);

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
      <div className="text-zinc-300 whitespace-pre-wrap break-words">
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

  return null;
}
