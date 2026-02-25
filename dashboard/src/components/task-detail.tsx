import { useTaskDetail, useTaskStream } from "@/lib/api";
import { PhaseTracker } from "./phase-tracker";
import { StatusBadge } from "./status-badge";
import { LiveTerminal } from "./live-terminal";
import { repoName, isActiveStatus, type TaskOutput } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useState, useMemo } from "react";

interface TaskDetailProps {
  taskId: number;
  onBack: () => void;
}

export function TaskDetail({ taskId, onBack }: TaskDetailProps) {
  const { data: task, isLoading } = useTaskDetail(taskId);
  const isActive = task ? isActiveStatus(task.status) : false;
  const { events, streaming } = useTaskStream(taskId, isActive);

  if (isLoading || !task) {
    return (
      <div className="flex h-full flex-col">
        <DetailHeader onBack={onBack} />
        <div className="flex flex-1 items-center justify-center text-xs text-zinc-600">Loading...</div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <DetailHeader onBack={onBack} />

      <div className="space-y-3 border-b border-white/[0.06] px-4 py-3">
        <div className="flex items-start justify-between gap-3">
          <h2 className="text-[14px] md:text-[13px] font-medium text-zinc-200">
            <span className="text-zinc-600">#{task.id}</span> {task.title}
          </h2>
          <StatusBadge status={task.status} />
        </div>

        <PhaseTracker status={task.status} />

        <div className="flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-zinc-500">
          {task.repo_path && (
            <span title={task.repo_path}>
              <span className="text-zinc-600">repo</span> {repoName(task.repo_path)}
            </span>
          )}
          {task.branch && (
            <span>
              <span className="text-zinc-600">branch</span> <span className="font-mono">{task.branch}</span>
            </span>
          )}
          {task.attempt > 0 && (
            <span>
              <span className="text-zinc-600">attempt</span> {task.attempt}/{task.max_attempts}
            </span>
          )}
          <span>
            <span className="text-zinc-600">by</span> {task.created_by || "pipeline"}
          </span>
          <span>
            <span className="text-zinc-600">at</span> {task.created_at}
          </span>
        </div>
      </div>

      {task.description && (
        <div className="max-h-20 md:max-h-16 overflow-y-auto border-b border-white/[0.06] px-4 py-2 text-[12px] md:text-[11px] leading-relaxed text-zinc-500">
          {task.description}
        </div>
      )}

      {task.last_error && (
        <div className="mx-3 mt-2 rounded-lg border border-red-500/20 bg-red-500/[0.05] p-3">
          <pre className="max-h-24 md:max-h-20 overflow-y-auto whitespace-pre-wrap font-mono text-[11px] text-red-400/90">
            {task.last_error}
          </pre>
        </div>
      )}

      {/* Live terminal for active tasks */}
      {(isActive || streaming) && (
        <div className="mx-3 mt-2 flex-1 min-h-0">
          <LiveTerminal events={events} streaming={streaming} />
        </div>
      )}

      {/* Completed phase outputs */}
      {!isActive && !streaming && task.outputs && task.outputs.length > 0 ? (
        <OutputSelector outputs={task.outputs} />
      ) : !isActive && !streaming ? (
        <div className="flex flex-1 items-center justify-center text-xs text-zinc-700">
          No agent outputs yet
        </div>
      ) : null}
    </div>
  );
}

interface StreamEvent {
  type: string;
  subtype?: string;
  tool?: string;
  label?: string;
  input?: string;
  output?: string;
  content?: string;
  timestamp?: string;
}

function formatToolInput(tool: string, input: unknown): { label: string; detail: string } {
  if (typeof input === "string") return { label: "", detail: input };
  if (!input || typeof input !== "object") return { label: "", detail: "" };
  const obj = input as Record<string, unknown>;
  if (tool === "Bash") {
    return { label: (obj.description as string) || "", detail: (obj.command as string) || "" };
  }
  if (tool === "Read") {
    const fp = (obj.file_path as string) || "";
    const suffix = obj.offset ? `  lines ${obj.offset}–${(obj.offset as number) + ((obj.limit as number) || 200)}` : "";
    return { label: "", detail: fp + suffix };
  }
  if (tool === "Write") return { label: "", detail: (obj.file_path as string) || "" };
  if (tool === "Edit") {
    const fp = (obj.file_path as string) || "";
    const old = (obj.old_string as string) || "";
    const preview = old.length > 80 ? old.slice(0, 80) + "..." : old;
    return { label: fp, detail: preview ? `replacing: ${preview}` : "" };
  }
  if (tool === "Glob" || tool === "Grep") {
    const pat = (obj.pattern as string) || "";
    const path = (obj.path as string) || "";
    return { label: "", detail: path ? `${pat}  in ${path}` : pat };
  }
  if (tool === "WebSearch") return { label: "", detail: (obj.query as string) || "" };
  if (tool === "WebFetch") return { label: "", detail: (obj.url as string) || "" };
  if (tool === "Task") return { label: (obj.description as string) || "", detail: ((obj.prompt as string) || "").slice(0, 120) };
  const json = JSON.stringify(input);
  return { label: "", detail: json.length > 200 ? json.slice(0, 200) + "..." : json };
}

function parseStream(raw: string): StreamEvent[] {
  if (!raw) return [];
  const events: StreamEvent[] = [];
  for (const line of raw.split("\n")) {
    if (!line.trim()) continue;
    try {
      const obj = JSON.parse(line);
      const type = obj.type;
      if (!type) continue;

      if (type === "assistant") {
        const msg = obj.message;
        if (msg?.content) {
          if (typeof msg.content === "string") {
            events.push({ type: "assistant", content: msg.content });
          } else if (Array.isArray(msg.content)) {
            for (const block of msg.content) {
              if (block.type === "text" && block.text) {
                events.push({ type: "assistant", content: block.text });
              } else if (block.type === "tool_use") {
                const { label, detail } = formatToolInput(block.name, block.input);
                events.push({
                  type: "tool_call",
                  tool: block.name,
                  label,
                  input: detail,
                });
              }
            }
          }
        }
      } else if (type === "tool_result" || type === "tool") {
        const content = obj.content ?? obj.result ?? obj.output ?? "";
        const text = typeof content === "string"
          ? content
          : Array.isArray(content)
            ? content.map((c: { text?: string }) => c.text || "").join("\n")
            : JSON.stringify(content);
        if (text) {
          events.push({
            type: "tool_result",
            tool: obj.tool_name || obj.name || "",
            output: text,
          });
        }
      } else if (type === "result") {
        events.push({ type: "result", content: obj.result || "" });
      } else if (type === "system") {
        if (obj.subtype === "init") {
          events.push({ type: "system", subtype: "init", content: `Session: ${obj.session_id || "?"}` });
        }
      }
    } catch {
      // skip unparseable lines
    }
  }
  return events;
}

function StreamView({ raw }: { raw: string }) {
  const events = useMemo(() => parseStream(raw), [raw]);

  if (events.length === 0) {
    return <div className="p-4 text-[11px] text-zinc-600">No stream data</div>;
  }

  return (
    <div className="space-y-1 p-3">
      {events.map((ev, i) => (
        <StreamEventBlock key={i} event={ev} />
      ))}
    </div>
  );
}

function StreamEventBlock({ event: ev }: { event: StreamEvent }) {
  const [expanded, setExpanded] = useState(false);

  if (ev.type === "system") {
    return (
      <div className="rounded bg-blue-500/[0.06] px-3 py-1.5 text-[10px] text-blue-400/70">
        {ev.content}
      </div>
    );
  }

  if (ev.type === "assistant") {
    return (
      <div className="rounded bg-white/[0.02] px-3 py-2">
        <pre className="whitespace-pre-wrap font-mono text-[12px] md:text-[11px] leading-relaxed text-zinc-300">
          {ev.content}
        </pre>
      </div>
    );
  }

  if (ev.type === "tool_call") {
    return (
      <div className="rounded border border-amber-500/10 bg-amber-500/[0.04]">
        <button
          onClick={() => setExpanded(!expanded)}
          className="flex w-full items-center gap-2 px-3 py-2 md:py-1.5 text-left active:bg-amber-500/[0.06]"
        >
          <span className="shrink-0 rounded bg-amber-500/20 px-1.5 py-0.5 font-mono text-[9px] font-bold text-amber-400">
            {ev.tool}
          </span>
          <span className="truncate text-[10px] text-zinc-400">
            {ev.label || (ev.input && ev.input.length > 80 ? ev.input.slice(0, 80) + "..." : ev.input)}
          </span>
          <span className="ml-auto shrink-0 text-[9px] text-zinc-600">{expanded ? "^" : "v"}</span>
        </button>
        {expanded && ev.input && (
          <pre className="max-h-60 overflow-y-auto border-t border-amber-500/10 px-3 py-2 font-mono text-[10px] leading-relaxed text-zinc-400">
            {ev.input}
          </pre>
        )}
      </div>
    );
  }

  if (ev.type === "tool_result") {
    const preview = ev.output && ev.output.length > 200 ? ev.output.slice(0, 200) + "..." : ev.output;
    return (
      <div className="rounded border border-white/[0.04] bg-white/[0.015]">
        <button
          onClick={() => setExpanded(!expanded)}
          className="flex w-full items-center gap-2 px-3 py-2 md:py-1.5 text-left active:bg-white/[0.03]"
        >
          <span className="shrink-0 rounded bg-zinc-500/20 px-1.5 py-0.5 font-mono text-[9px] font-bold text-zinc-400">
            result{ev.tool ? `: ${ev.tool}` : ""}
          </span>
          {!expanded && (
            <span className="truncate font-mono text-[10px] text-zinc-600">{preview}</span>
          )}
          <span className="ml-auto shrink-0 text-[9px] text-zinc-600">{expanded ? "^" : "v"}</span>
        </button>
        {expanded && ev.output && (
          <pre className="max-h-60 overflow-y-auto border-t border-white/[0.04] px-3 py-2 font-mono text-[10px] leading-relaxed text-zinc-500">
            {ev.output}
          </pre>
        )}
      </div>
    );
  }

  if (ev.type === "result") {
    return (
      <div className="rounded border border-emerald-500/10 bg-emerald-500/[0.04] px-3 py-2">
        <div className="mb-1 text-[9px] font-bold uppercase tracking-wider text-emerald-500/60">Final Result</div>
        <pre className="whitespace-pre-wrap font-mono text-[12px] md:text-[11px] leading-relaxed text-emerald-300/80">
          {ev.content}
        </pre>
      </div>
    );
  }

  return null;
}

function downloadText(text: string, filename: string) {
  const blob = new Blob([text], { type: "text/plain" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

function OutputSelector({ outputs }: { outputs: TaskOutput[] }) {
  const [viewMode, setViewMode] = useState<"summary" | "trace" | "diff">("summary");
  const [copied, setCopied] = useState(false);

  const labeled = useMemo(() => {
    const phaseCounts: Record<string, number> = {};
    const phaseIndices: Record<string, number> = {};
    for (const o of outputs) {
      phaseCounts[o.phase] = (phaseCounts[o.phase] || 0) + 1;
    }
    return outputs.map((o) => {
      phaseIndices[o.phase] = (phaseIndices[o.phase] || 0) + 1;
      const idx = phaseIndices[o.phase];
      const total = phaseCounts[o.phase];
      const label = total > 1
        ? `${o.phase} attempt #${idx}`
        : o.phase;
      return { ...o, label, isLatest: idx === total };
    });
  }, [outputs]);

  const [selectedKey, setSelectedKey] = useState(
    labeled[labeled.length - 1].phase + "-" + labeled[labeled.length - 1].id
  );

  const selected = labeled.find((o) => o.phase + "-" + o.id === selectedKey) ?? labeled[labeled.length - 1];
  const isDiff = selected.phase.endsWith("_diff");
  const hasStream = !!selected.raw_stream;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-white/[0.06] px-4 py-2">
        <select
          value={selectedKey}
          onChange={(e) => setSelectedKey(e.target.value)}
          className="rounded-md border border-white/[0.08] bg-white/[0.04] px-2.5 py-1.5 md:py-1 text-[12px] md:text-[11px] font-medium uppercase tracking-wide text-zinc-300 outline-none focus:border-blue-500/40"
        >
          {labeled.map((o) => {
            const key = o.phase + "-" + o.id;
            const status = o.exit_code === 0 ? " \u2713" : ` x${o.exit_code}`;
            return (
              <option key={key} value={key}>
                {o.label}{status}{o.isLatest ? " (latest)" : ""}
              </option>
            );
          })}
        </select>
        <span className={cn(
          "rounded-full px-2 py-0.5 text-[10px] font-medium ring-1 ring-inset",
          selected.exit_code === 0
            ? "bg-emerald-500/10 text-emerald-400 ring-emerald-500/20"
            : "bg-red-500/10 text-red-400 ring-red-500/20"
        )}>
          {selected.exit_code === 0 ? "passed" : `exit ${selected.exit_code}`}
        </span>
        {!isDiff && (
          <div className="ml-auto flex items-center gap-2">
            <div className="flex rounded-md border border-white/[0.08]">
              <button
                onClick={() => setViewMode("summary")}
                className={cn(
                  "px-2.5 md:px-2 py-1 md:py-0.5 text-[11px] md:text-[10px] font-medium transition-colors",
                  viewMode === "summary"
                    ? "bg-white/[0.08] text-zinc-200"
                    : "text-zinc-500 hover:text-zinc-300"
                )}
              >
                Summary
              </button>
              {hasStream && (
                <button
                  onClick={() => setViewMode("trace")}
                  className={cn(
                    "border-l border-white/[0.08] px-2.5 md:px-2 py-1 md:py-0.5 text-[11px] md:text-[10px] font-medium transition-colors",
                    viewMode === "trace"
                      ? "bg-white/[0.08] text-zinc-200"
                      : "text-zinc-500 hover:text-zinc-300"
                  )}
                >
                  Full Trace
                </button>
              )}
            </div>
            <button
              onClick={() => {
                const text = viewMode === "trace" && hasStream ? selected.raw_stream : selected.output;
                navigator.clipboard.writeText(text || "").then(() => {
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1500);
                });
              }}
              className="rounded-md px-2 py-1 md:py-0.5 text-[10px] font-medium text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.05] transition-colors"
            >
              {copied ? "Copied" : "Copy"}
            </button>
            <button
              onClick={() => {
                const text = viewMode === "trace" && hasStream ? selected.raw_stream : selected.output;
                const ext = viewMode === "trace" && hasStream ? "ndjson" : "txt";
                downloadText(text || "", `task-${selected.id}-${selected.phase}.${ext}`);
              }}
              className="rounded-md px-2 py-1 md:py-0.5 text-[10px] font-medium text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.05] transition-colors"
            >
              Download
            </button>
          </div>
        )}
      </div>
      <div className="flex-1 overflow-y-auto overscroll-contain">
        {isDiff ? (
          <DiffView diff={selected.output} />
        ) : viewMode === "trace" && hasStream ? (
          <StreamView raw={selected.raw_stream} />
        ) : (
          <pre className="p-4 font-mono text-[12px] md:text-[11px] leading-relaxed text-zinc-400 whitespace-pre-wrap break-words">
            {selected.output || "(empty)"}
          </pre>
        )}
      </div>
    </div>
  );
}

function DiffView({ diff }: { diff: string }) {
  if (!diff) return <div className="p-4 text-[11px] text-zinc-600">No diff data</div>;
  return (
    <pre className="p-4 font-mono text-[11px] leading-relaxed overflow-x-auto">
      {diff.split("\n").map((line, i) => {
        let color = "text-zinc-500";
        if (line.startsWith("+") && !line.startsWith("+++")) color = "text-emerald-400/80";
        else if (line.startsWith("-") && !line.startsWith("---")) color = "text-red-400/80";
        else if (line.startsWith("@@")) color = "text-blue-400/60";
        else if (line.startsWith("diff ") || line.startsWith("index ")) color = "text-zinc-600";
        return <div key={i} className={color}>{line}</div>;
      })}
    </pre>
  );
}

function DetailHeader({ onBack }: { onBack: () => void }) {
  return (
    <div className="flex h-11 md:h-10 shrink-0 items-center gap-3 border-b border-white/[0.06] px-4">
      <button
        onClick={onBack}
        className="rounded-md bg-white/[0.04] px-3 md:px-2.5 py-1.5 md:py-1 text-[12px] md:text-[11px] font-medium text-zinc-400 transition-colors active:bg-white/[0.1] hover:bg-white/[0.08] hover:text-zinc-200"
      >
        &larr; Back
      </button>
      <span className="text-[12px] md:text-[11px] font-medium text-zinc-500">Task Detail</span>
    </div>
  );
}
