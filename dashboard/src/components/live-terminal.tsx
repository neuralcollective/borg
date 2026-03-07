import { useRef, useEffect, useMemo, useState } from "react";
import type { StreamEvent } from "@/lib/api";
import { parseStreamEvents, type TermLine } from "@/lib/stream-utils";
import { cn } from "@/lib/utils";
import { TimelineItem } from "./borging";
import {
  FileText,
  FilePen,
  Pencil,
  Terminal,
  Search,
  Globe,
  Sparkles,
  Circle,
} from "lucide-react";

interface LiveTerminalProps {
  events: StreamEvent[];
  streaming: boolean;
}

interface KeyedTermLine extends TermLine {
  _key: number;
}

const toolIconMap: Record<string, React.ReactNode> = {
  Read: <FileText className="w-3.5 h-3.5 text-amber-400/70" />,
  Write: <FilePen className="w-3.5 h-3.5 text-amber-400/70" />,
  Edit: <Pencil className="w-3.5 h-3.5 text-amber-400/70" />,
  Bash: <Terminal className="w-3.5 h-3.5 text-amber-400/70" />,
  Grep: <Search className="w-3.5 h-3.5 text-amber-400/70" />,
  Glob: <Search className="w-3.5 h-3.5 text-amber-400/70" />,
  WebFetch: <Globe className="w-3.5 h-3.5 text-amber-400/70" />,
  WebSearch: <Globe className="w-3.5 h-3.5 text-amber-400/70" />,
  Task: <Sparkles className="w-3.5 h-3.5 text-amber-400/70" />,
  Agent: <Sparkles className="w-3.5 h-3.5 text-amber-400/70" />,
};

function getToolIcon(tool?: string): React.ReactNode {
  if (!tool) return <Circle className="w-3 h-3 text-[#6b6459]" />;
  return toolIconMap[tool] || <Circle className="w-3 h-3 text-[#6b6459]" />;
}

export function LiveTerminal({ events, streaming }: LiveTerminalProps) {
  const bottomRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const keyCounterRef = useRef(0);
  const prevLinesRef = useRef<KeyedTermLine[]>([]);
  const autoScrollRef = useRef(true);

  const lines = useMemo(() => {
    const parsed = parseStreamEvents(events);
    const prev = prevLinesRef.current;
    const reused = parsed.length >= prev.length ? prev : [];
    const result: KeyedTermLine[] = parsed.map((line, i) =>
      i < reused.length
        ? { ...line, _key: reused[i]._key }
        : { ...line, _key: keyCounterRef.current++ }
    );
    prevLinesRef.current = result;
    return result;
  }, [events]);

  useEffect(() => {
    if (autoScrollRef.current && bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: "instant" });
    }
  }, [lines.length]);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const onScroll = () => {
      const atBottom =
        el.scrollHeight - el.scrollTop - el.clientHeight < 40;
      autoScrollRef.current = atBottom;
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, []);

  const statusText = streaming
    ? "Working..."
    : events.length > 0
      ? "Completed"
      : "Waiting...";

  return (
    <div className="flex flex-col h-full rounded-2xl border border-[#2a2520] bg-[#151412] overflow-hidden">
      {/* Header */}
      <div className="flex items-center gap-2.5 px-4 py-2.5 border-b border-[#2a2520] bg-[#1c1a17]/50">
        {streaming ? (
          <div className="flex items-center gap-2">
            <span className="relative flex h-2 w-2">
              <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-amber-400 opacity-75" />
              <span className="relative inline-flex rounded-full h-2 w-2 bg-amber-400" />
            </span>
            <span className="shimmer-text text-[13px] font-medium text-amber-400">
              {statusText}
            </span>
          </div>
        ) : (
          <span className="text-[13px] font-medium text-[#9c9486]">
            {statusText}
          </span>
        )}
        <span className="ml-auto text-[11px] tabular-nums text-[#6b6459]">
          {events.length} events
        </span>
      </div>

      {/* Content */}
      <div
        ref={containerRef}
        className="flex-1 overflow-y-auto overscroll-contain text-[13px] leading-relaxed p-3 space-y-0.5"
      >
        {lines.length === 0 && (
          <div className="flex items-center gap-2 text-[#6b6459] py-8 justify-center">
            {streaming && (
              <span className="animate-pulse">Connecting to agent...</span>
            )}
            {!streaming && <span>No live stream available</span>}
          </div>
        )}

        {lines.map((line, i) => (
          <TimelineLineView
            key={line._key}
            line={line}
            isFirst={i === 0}
            isLast={i === lines.length - 1 && !streaming}
          />
        ))}

        {streaming && (
          <div className="flex items-center gap-2 pl-9 pt-1">
            <span className="relative flex h-1.5 w-1.5">
              <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-amber-400 opacity-75" />
              <span className="relative inline-flex rounded-full h-1.5 w-1.5 bg-amber-400" />
            </span>
          </div>
        )}
        <div ref={bottomRef} />
      </div>
    </div>
  );
}

function TimelineLineView({
  line,
  isFirst,
  isLast,
}: {
  line: TermLine;
  isFirst: boolean;
  isLast: boolean;
}) {
  if (line.type === "system") {
    return (
      <div className="text-[#6b6459] text-[11px] pl-9 py-0.5">
        {line.content}
      </div>
    );
  }

  if (line.type === "text") {
    return (
      <div className="text-[#e8e0d4] text-[13px] whitespace-pre-wrap break-words pl-9 py-1.5 leading-relaxed">
        {line.content}
      </div>
    );
  }

  if (line.type === "tool") {
    return (
      <TimelineItem
        icon={getToolIcon(line.tool)}
        label={line.tool || "Tool"}
        detail={line.label || line.content || undefined}
        isActive
        isFirst={isFirst}
        isLast={isLast}
      />
    );
  }

  if (line.type === "tool_result") {
    return <CollapsibleToolResult content={line.content} tool={line.tool} />;
  }

  if (line.type === "result") {
    return (
      <div className="text-[#e8e0d4] pl-9 pt-1">
        <div className="whitespace-pre-wrap break-words text-[13px]">
          {line.content}
        </div>
      </div>
    );
  }

  if (line.type === "phase_result") {
    return (
      <div className="rounded-xl border border-amber-500/10 bg-amber-500/[0.04] px-4 py-2.5 my-1.5 ml-9">
        <div className="text-amber-400/60 text-[10px] uppercase tracking-wider mb-1 font-medium">
          Phase result{line.label ? `: ${line.label}` : ""}
        </div>
        <div className="text-[#e8e0d4] text-[13px] whitespace-pre-wrap break-words">
          {line.content}
        </div>
      </div>
    );
  }

  if (line.type === "container") {
    const colors: Record<string, string> = {
      success: "text-emerald-400/80 border-emerald-500/20",
      error: "text-red-400/80 border-red-500/20",
      warn: "text-amber-400/80 border-amber-500/20",
      info: "text-[#9c9486] border-[#2a2520]",
    };
    const cls = colors[line.variant ?? "info"] ?? colors.info;
    return (
      <div className={cn("border-l pl-2 py-0.5 text-[11px] ml-9", cls)}>
        {line.content}
      </div>
    );
  }

  return null;
}

function CollapsibleToolResult({
  content,
}: {
  content: string;
  tool?: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const preview =
    content.length > 120 ? content.slice(0, 120) + "..." : content;

  return (
    <div
      className="pl-9 py-0.5 cursor-pointer group"
      onClick={() => setExpanded(!expanded)}
    >
      <div className="text-[11px] text-[#6b6459] break-all whitespace-pre-wrap group-hover:text-[#9c9486] transition-colors">
        {expanded ? content : preview}
      </div>
    </div>
  );
}
