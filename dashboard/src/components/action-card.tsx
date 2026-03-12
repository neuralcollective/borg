import {
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  Copy,
  Download,
  FilePen,
  FileText,
  Globe,
  Pencil,
  Search,
  Sparkles,
  Terminal,
} from "lucide-react";
import { useMemo, useState } from "react";
import type { TermLine } from "@/lib/stream-utils";
import { cn } from "@/lib/utils";
import { pickWorkingLabel } from "./borging";
import { ChatMarkdown } from "./chat-markdown";

// Human-readable tool labels
const TOOL_LABELS: Record<string, string> = {
  Read: "Read file",
  Write: "Created file",
  Edit: "Edited file",
  Bash: "Ran command",
  Grep: "Searched for",
  Glob: "Found files",
  WebFetch: "Fetched page",
  WebSearch: "Searched web",
  ToolSearch: "Tool search",
  web_search: "Searched web",
  web_fetch: "Fetched page",
  Task: "Created task",
  Agent: "Sub-agent",
  LS: "Listed files",
  ls: "Listed files",
  NotebookEdit: "Edited notebook",
  TodoRead: "Read todos",
  TodoWrite: "Updated todos",
  AskFollowupQuestion: "Asked question",
  AttemptCompletion: "Completed task",
  ListCodeDefinitionNames: "Listed definitions",
  SearchReplace: "Search & replace",
  mcp__borg__search_documents: "BorgSearch",
  mcp__borg__list_documents: "BorgSearch · List",
  mcp__borg__read_document: "BorgSearch · Read",
  mcp__borg__check_coverage: "BorgSearch · Coverage",
  mcp__borg__get_document_categories: "BorgSearch · Categories",
  mcp__borg__create_task: "Borg · Create task",
  mcp__borg__get_task_status: "Borg · Task status",
  mcp__borg__list_project_tasks: "Borg · List tasks",
  mcp__borg__list_services: "Borg · Services",
};

const TOOL_ICONS: Record<string, React.ReactNode> = {
  Read: <FileText className="h-3.5 w-3.5" />,
  Write: <FilePen className="h-3.5 w-3.5" />,
  Edit: <Pencil className="h-3.5 w-3.5" />,
  Bash: <Terminal className="h-3.5 w-3.5" />,
  Grep: <Search className="h-3.5 w-3.5" />,
  Glob: <Search className="h-3.5 w-3.5" />,
  WebFetch: <Globe className="h-3.5 w-3.5" />,
  WebSearch: <Globe className="h-3.5 w-3.5" />,
  ToolSearch: <Search className="h-3.5 w-3.5" />,
  web_search: <Globe className="h-3.5 w-3.5" />,
  web_fetch: <Globe className="h-3.5 w-3.5" />,
  Task: <Sparkles className="h-3.5 w-3.5" />,
  Agent: <Sparkles className="h-3.5 w-3.5" />,
  mcp__borg__search_documents: <Search className="h-3.5 w-3.5" />,
  mcp__borg__list_documents: <FileText className="h-3.5 w-3.5" />,
  mcp__borg__read_document: <FileText className="h-3.5 w-3.5" />,
  mcp__borg__check_coverage: <Search className="h-3.5 w-3.5" />,
  mcp__borg__get_document_categories: <Search className="h-3.5 w-3.5" />,
  mcp__borg__create_task: <Sparkles className="h-3.5 w-3.5" />,
  mcp__borg__get_task_status: <Sparkles className="h-3.5 w-3.5" />,
  mcp__borg__list_project_tasks: <Sparkles className="h-3.5 w-3.5" />,
  mcp__borg__list_services: <Sparkles className="h-3.5 w-3.5" />,
};

// Pretty-print unknown MCP tool names: mcp__lawborg__courtlistener_search_opinions → "LawBorg · courtlistener search opinions"
const MCP_SERVER_LABELS: Record<string, string> = {
  borg: "BorgSearch",
  lawborg: "LawBorg",
};

function formatMcpToolName(tool: string): string {
  const parts = tool.match(/^mcp__(\w+)__(.+)$/);
  if (!parts) return tool;
  const [, server, action] = parts;
  const serverLabel = MCP_SERVER_LABELS[server] || server;
  const actionLabel = action.replace(/_/g, " ");
  return `${serverLabel} · ${actionLabel}`;
}

function getToolLabel(tool?: string, detail?: string): string {
  if (!tool) return "Action";
  // For Bash, prefer the description (label) over generic "Ran command"
  if (tool === "Bash" && detail) return detail;
  if (TOOL_LABELS[tool]) return TOOL_LABELS[tool];
  if (tool.startsWith("mcp__")) return formatMcpToolName(tool);
  return tool;
}

function getToolIcon(tool?: string): React.ReactNode {
  if (!tool) return <Sparkles className="h-3.5 w-3.5" />;
  if (TOOL_ICONS[tool]) return TOOL_ICONS[tool];
  if (tool.startsWith("mcp__borg__")) return <Search className="h-3.5 w-3.5" />;
  if (tool.startsWith("mcp__")) return <Globe className="h-3.5 w-3.5" />;
  return <Sparkles className="h-3.5 w-3.5" />;
}

function summarizeResult(tool: string, resultText: string): string | null {
  if (!resultText.trim()) return null;
  const lines = resultText.trim().split("\n").filter(Boolean);
  if (tool === "Grep" || tool === "Glob") {
    const fileCount = lines.length;
    if (resultText.includes("No matches found")) return "no matches";
    return fileCount === 1 ? "1 file" : `${fileCount} files`;
  }
  if (tool === "Bash") {
    if (lines.length === 0) return "done";
    if (lines.length === 1 && lines[0].length < 60) return lines[0];
    return `${lines.length} lines`;
  }
  if (tool === "WebSearch") {
    return lines.length === 1 ? "1 result" : `${lines.length} results`;
  }
  if (tool === "ToolSearch" || tool === "web_search") {
    return lines.length === 1 ? "1 result" : `${lines.length} results`;
  }
  if (tool === "WebFetch") {
    const chars = resultText.length;
    return chars > 1000 ? `${(chars / 1000).toFixed(1)}k chars` : `${chars} chars`;
  }
  if (tool === "web_fetch") {
    const chars = resultText.length;
    return chars > 1000 ? `${(chars / 1000).toFixed(1)}k chars` : `${chars} chars`;
  }
  if (tool === "Agent") {
    if (lines.length === 0) return "done";
    const last = lines[lines.length - 1];
    return last.length > 60 ? `${last.slice(0, 57)}...` : last;
  }
  if (tool.startsWith("mcp__borg__")) {
    if (resultText.includes("No matches") || resultText.includes("No documents")) return "no results";
    if (lines.length <= 1) return lines.length === 1 ? "1 result" : "done";
    return `${lines.length} results`;
  }
  return null;
}

// Group consecutive same-tool lines
export interface ActionGroup {
  type: "tool" | "text" | "result" | "thinking" | "phase" | "error" | "final";
  tool?: string;
  lines: TermLine[];
  label: string;
  detail?: string;
  autoExpand?: boolean;
}

export function groupActions(lines: TermLine[], streaming: boolean): ActionGroup[] {
  const groups: ActionGroup[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    if (line.type === "tool") {
      // Group consecutive same-tool calls
      const tool = line.tool || "";
      const toolLines: TermLine[] = [line];
      let j = i + 1;

      // Look ahead for tool_results and more of the same tool
      while (j < lines.length) {
        if (lines[j].type === "tool_result" && lines[j].tool === tool) {
          toolLines.push(lines[j]);
          j++;
        } else if (lines[j].type === "tool" && lines[j].tool === tool) {
          toolLines.push(lines[j]);
          j++;
        } else {
          break;
        }
      }

      const callCount = toolLines.filter((l) => l.type === "tool").length;
      // For single Bash calls with a description, use the description as the label
      const singleLabel = callCount === 1 ? line.label : undefined;
      const label =
        callCount > 1 ? `${getToolLabel(tool, singleLabel)} (${callCount})` : getToolLabel(tool, singleLabel);
      // For Bash with description, show command as detail; otherwise show the label/content
      const detail =
        callCount === 1
          ? tool === "Bash" && line.label
            ? line.content
            : line.label || line.content || undefined
          : undefined;
      const autoExpand = true;

      groups.push({ type: "tool", tool, lines: toolLines, label, detail, autoExpand });
      i = j;
      continue;
    }

    if (line.type === "tool_result") {
      // Orphaned tool result
      groups.push({
        type: "tool",
        tool: line.tool,
        lines: [line],
        label: `Result: ${line.tool || ""}`,
      });
      i++;
      continue;
    }

    if (line.type === "text") {
      groups.push({
        type: "text",
        lines: [line],
        label: "",
      });
      i++;
      continue;
    }

    if (line.type === "result") {
      groups.push({
        type: "final",
        lines: [line],
        label: "Complete",
      });
      i++;
      continue;
    }

    if (line.type === "phase_result") {
      groups.push({
        type: "phase",
        lines: [line],
        label: line.label || "Phase complete",
      });
      i++;
      continue;
    }

    if (line.type === "container" && line.variant === "error") {
      groups.push({
        type: "error",
        lines: [line],
        label: "Error",
      });
      i++;
      continue;
    }

    // System, container, or other
    groups.push({
      type: "text",
      lines: [line],
      label: "",
    });
    i++;
  }

  // Add thinking indicator at end if streaming and last item isn't text
  if (streaming && groups.length > 0) {
    const last = groups[groups.length - 1];
    if (last.type !== "text") {
      groups.push({ type: "thinking", lines: [], label: "Working..." });
    }
  } else if (streaming && groups.length === 0) {
    groups.push({ type: "thinking", lines: [], label: "Working..." });
  }

  return groups;
}

// ActionCard: the main card component
interface ActionCardProps {
  group: ActionGroup;
  isLatest?: boolean;
  compact?: boolean;
  defaultExpanded?: boolean;
}

export function ActionCard({ group, isLatest, compact, defaultExpanded }: ActionCardProps) {
  if (group.type === "thinking") {
    return <ThinkingCard compact={compact} />;
  }

  if (group.type === "text") {
    return <TextCard lines={group.lines} compact={compact} />;
  }

  if (group.type === "phase") {
    return <PhaseCard label={group.label} />;
  }

  if (group.type === "error") {
    return <ErrorCard lines={group.lines} />;
  }

  if (group.type === "final") {
    return <FinalOutputCard lines={group.lines} />;
  }

  // Tool action card
  return <ToolActionCard group={group} isLatest={isLatest} compact={compact} defaultExpanded={defaultExpanded} />;
}

function ToolActionCard({
  group,
  isLatest,
  compact,
}: {
  group: ActionGroup;
  isLatest?: boolean;
  compact?: boolean;
  defaultExpanded?: boolean;
}) {
  const [manualToggle, setManualToggle] = useState<boolean | null>(null);
  const autoExpanded = group.autoExpand ?? true;
  const expanded = manualToggle ?? autoExpanded;
  const icon = getToolIcon(group.tool);

  const toolCalls = group.lines.filter((l) => l.type === "tool");
  const results = group.lines.filter((l) => l.type === "tool_result");
  const hasResults = results.length > 0;
  const resultText = results.map((r) => r.content).join("\n");
  const lineCount = resultText.split("\n").length;
  const resultHint = hasResults && toolCalls.length <= 1 ? summarizeResult(group.tool || "", resultText) : null;
  const canExpand = hasResults || toolCalls.length > 1;

  return (
    <div
      className={cn(
        "rounded-xl border bg-[#1c1a17] transition-all duration-200 animate-[action-fade-in_0.2s_ease-out]",
        expanded ? "border-amber-500/20" : "border-[#2a2520]",
        isLatest && !expanded && "border-amber-500/10",
        compact && "rounded-lg",
      )}
    >
      {/* Summary header */}
      <button
        onClick={() => canExpand && setManualToggle(!expanded)}
        className={cn(
          "flex w-full items-center gap-2.5 text-left transition-colors",
          compact ? "px-2.5 py-1.5" : "px-3.5 py-2.5",
          canExpand && "cursor-pointer hover:bg-amber-500/[0.03]",
          !canExpand && "cursor-default",
        )}
      >
        <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-lg bg-amber-500/10 text-amber-400/80">
          {icon}
        </span>
        <div className="min-w-0 flex-1">
          <span className={cn("font-medium text-[#e8e0d4]", compact ? "text-[12px]" : "text-[13px]")}>
            {group.label}
          </span>
          {group.detail && (
            <span className={cn("ml-2 truncate text-[#6b6459]", compact ? "text-[11px]" : "text-[12px]")}>
              {group.detail}
            </span>
          )}
          {resultHint && (
            <span className={cn("ml-2 text-[#6b6459]", compact ? "text-[10px]" : "text-[11px]")}>→ {resultHint}</span>
          )}
        </div>
        {canExpand && (
          <ChevronDown
            className={cn(
              "h-3.5 w-3.5 shrink-0 text-[#6b6459] transition-transform duration-200",
              expanded && "rotate-180",
            )}
          />
        )}
      </button>

      {/* Expanded content */}
      <div
        className={cn(
          "grid transition-[grid-template-rows] duration-200 ease-out",
          expanded ? "grid-rows-[1fr]" : "grid-rows-[0fr]",
        )}
      >
        <div className="overflow-hidden">
          {/* Show individual tool calls if grouped */}
          {expanded && toolCalls.length > 1 && (
            <div className="border-t border-[#2a2520] px-3.5 py-2 space-y-1">
              {toolCalls.map((call, i) => (
                <div key={i} className="flex items-center gap-2 text-[12px]">
                  <span className="h-1 w-1 rounded-full bg-amber-400/40 shrink-0" />
                  <span className="truncate text-[#9c9486]">{call.label || call.content}</span>
                </div>
              ))}
            </div>
          )}

          {/* Result output */}
          {expanded && hasResults && (
            <div className="border-t border-[#2a2520]">
              <pre className="max-h-[300px] overflow-y-auto px-3.5 py-2.5 font-mono text-[11px] leading-relaxed text-[#9c9486] whitespace-pre-wrap break-words">
                {resultText}
              </pre>
              {lineCount > 5 && (
                <div className="border-t border-[#2a2520] px-3.5 py-1.5 text-right text-[10px] text-[#6b6459]">
                  {lineCount} lines
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function ThinkingCard({ compact }: { compact?: boolean }) {
  const [label] = useState(pickWorkingLabel);
  return (
    <div
      className={cn(
        "relative overflow-hidden rounded-xl border border-[#2a2520] bg-[#1c1a17] animate-[action-fade-in_0.2s_ease-out]",
        compact ? "rounded-lg px-2.5 py-2" : "px-3.5 py-3",
      )}
    >
      <div className="flex items-center gap-2.5">
        <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-lg bg-amber-500/10">
          <span className="relative flex h-2 w-2">
            <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-amber-400 opacity-75" />
            <span className="relative inline-flex h-2 w-2 rounded-full bg-amber-400" />
          </span>
        </span>
        <span className="shimmer-text text-[13px] font-medium text-amber-400">{label}</span>
      </div>
      <div className="absolute inset-0 -translate-x-full animate-[shimmer_2s_infinite] bg-gradient-to-r from-transparent via-amber-400/[0.03] to-transparent" />
    </div>
  );
}

function TextCard({ lines, compact }: { lines: TermLine[]; compact?: boolean }) {
  const text = lines.map((l) => l.content).join("\n");
  if (!text.trim()) return null;

  return (
    <div className={cn("animate-[action-fade-in_0.2s_ease-out]", compact ? "py-1" : "py-1.5")}>
      <div className={cn("text-[#e8e0d4] leading-relaxed", compact ? "text-[12px]" : "text-[13px]")}>
        <ChatMarkdown text={text} variant="panel" />
      </div>
    </div>
  );
}

function PhaseCard({ label }: { label: string }) {
  return (
    <div className="flex items-center gap-3 py-2 animate-[action-fade-in_0.2s_ease-out]">
      <div className="h-px flex-1 bg-gradient-to-r from-transparent to-amber-500/20" />
      <div className="flex items-center gap-1.5 text-[12px]">
        <CheckCircle2 className="h-3.5 w-3.5 text-amber-400/60" />
        <span className="font-medium text-amber-400/80">{label}</span>
      </div>
      <div className="h-px flex-1 bg-gradient-to-l from-transparent to-amber-500/20" />
    </div>
  );
}

function ErrorCard({ lines }: { lines: TermLine[] }) {
  const text = lines.map((l) => l.content).join("\n");

  return (
    <div className="rounded-xl border border-red-500/20 bg-red-500/[0.04] animate-[action-fade-in_0.2s_ease-out]">
      <div className="flex items-center gap-2.5 px-3.5 py-2.5">
        <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-lg bg-red-500/10 text-red-400/80">
          <AlertTriangle className="h-3.5 w-3.5" />
        </span>
        <span className="text-[13px] font-medium text-red-400/90">Oh, Borg!</span>
      </div>
      <div className="border-t border-red-500/10 px-3.5 py-2.5">
        <pre className="max-h-[200px] overflow-y-auto font-mono text-[11px] leading-relaxed text-red-400/70 whitespace-pre-wrap break-words">
          {text}
        </pre>
      </div>
    </div>
  );
}

function FinalOutputCard({ lines }: { lines: TermLine[] }) {
  const text = lines.map((l) => l.content).join("\n");
  const [copied, setCopied] = useState(false);

  return (
    <div className="rounded-xl border-2 border-emerald-500/20 bg-emerald-500/[0.02] animate-[action-fade-in_0.2s_ease-out]">
      <div className="flex items-center gap-2.5 px-4 py-3">
        <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-emerald-500/10 text-emerald-400">
          <CheckCircle2 className="h-4 w-4" />
        </span>
        <span className="text-[14px] font-medium text-emerald-300">Complete</span>
        <div className="ml-auto flex items-center gap-1.5">
          <button
            onClick={() => {
              navigator.clipboard.writeText(text);
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            }}
            className="flex items-center gap-1 rounded-lg px-2.5 py-1.5 text-[11px] text-[#9c9486] hover:bg-[#1c1a17] hover:text-[#e8e0d4] transition-colors"
          >
            <Copy className="h-3 w-3" />
            {copied ? "Copied" : "Copy"}
          </button>
          <button
            onClick={() => {
              const blob = new Blob([text], { type: "text/plain" });
              const url = URL.createObjectURL(blob);
              const a = document.createElement("a");
              a.href = url;
              a.download = "output.txt";
              a.click();
              URL.revokeObjectURL(url);
            }}
            className="flex items-center gap-1 rounded-lg px-2.5 py-1.5 text-[11px] text-[#9c9486] hover:bg-[#1c1a17] hover:text-[#e8e0d4] transition-colors"
          >
            <Download className="h-3 w-3" />
            Download
          </button>
        </div>
      </div>
      <div className="border-t border-emerald-500/10 px-4 py-3">
        <div className="text-[13px] leading-relaxed text-[#e8e0d4]">
          <ChatMarkdown text={text} variant="panel" />
        </div>
      </div>
    </div>
  );
}

// Progress header for task/work item activity view
interface ProgressHeaderProps {
  title?: string;
  phase?: string;
  phaseIndex?: number;
  totalPhases?: number;
  actionCount: number;
  elapsed?: number;
  streaming: boolean;
  status?: "active" | "done" | "error";
}

export function ProgressHeader({
  title,
  phase,
  phaseIndex,
  totalPhases,
  actionCount,
  elapsed,
  streaming,
  status = streaming ? "active" : "done",
}: ProgressHeaderProps) {
  const elapsedStr = elapsed
    ? elapsed >= 60
      ? `${Math.floor(elapsed / 60)}m ${elapsed % 60}s`
      : `${elapsed}s`
    : undefined;

  const dotColor = status === "active" ? "bg-amber-400" : status === "done" ? "bg-emerald-500" : "bg-red-500";

  return (
    <div className="sticky top-0 z-10 flex items-center gap-3 rounded-xl border border-[#2a2520] bg-[#151412]/95 px-4 py-2.5 backdrop-blur">
      <span className="relative flex h-2.5 w-2.5">
        {streaming && (
          <span className={cn("absolute inline-flex h-full w-full animate-ping rounded-full opacity-75", dotColor)} />
        )}
        <span className={cn("relative inline-flex h-2.5 w-2.5 rounded-full", dotColor)} />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-[13px] font-medium text-[#e8e0d4]">
            {streaming ? "Working" : status === "error" ? "Failed" : "Complete"}
          </span>
          {title && (
            <>
              <span className="text-[#6b6459]">&middot;</span>
              <span className="truncate text-[13px] text-[#9c9486]">{title}</span>
            </>
          )}
        </div>
        <div className="flex items-center gap-2 text-[11px] text-[#6b6459]">
          {phase && (
            <span>
              Phase: {phase}
              {phaseIndex != null && totalPhases ? ` (${phaseIndex + 1}/${totalPhases})` : ""}
            </span>
          )}
          {actionCount > 0 && (
            <>
              <span>&middot;</span>
              <span>{actionCount} actions</span>
            </>
          )}
          {elapsedStr && (
            <>
              <span>&middot;</span>
              <span>{elapsedStr}</span>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

// Full activity view using ActionCards (replaces LiveTerminal)
interface ActionActivityProps {
  lines: TermLine[];
  streaming: boolean;
  title?: string;
  phase?: string;
  compact?: boolean;
  hideFinalOutput?: boolean;
}

export function ActionActivity({ lines, streaming, title, phase, compact, hideFinalOutput }: ActionActivityProps) {
  const groups = useMemo(() => groupActions(lines, streaming), [lines, streaming]);
  const filtered = hideFinalOutput ? groups.filter((g) => g.type !== "final" && g.type !== "text") : groups;
  const actionCount = filtered.filter((g) => g.type === "tool").length;

  return (
    <div className={cn("space-y-2", compact && "space-y-1.5")}>
      {!compact && <ProgressHeader title={title} phase={phase} actionCount={actionCount} streaming={streaming} />}
      {filtered.map((group, i) => (
        <ActionCard
          key={i}
          group={group}
          isLatest={i === filtered.length - 1}
          compact={compact}
          defaultExpanded={false}
        />
      ))}
    </div>
  );
}
