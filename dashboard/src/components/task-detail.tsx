import { useTaskDetail, useTaskStream, useTaskContainer, useFullModes, retryTask, setTaskBackend, approveTask, rejectTask, requestRevision, getRevisionHistory, getTaskDiagnostics } from "@/lib/api";
import type { RevisionHistory, TaskDiagnostics } from "@/lib/api";
import { PhaseTracker } from "./phase-tracker";
import { StatusBadge } from "./status-badge";
import { LiveTerminal } from "./live-terminal";
import { TaskChat } from "./task-chat";
import { repoName, isActiveStatus, type TaskOutput } from "@/lib/types";
import { useUIMode } from "@/lib/ui-mode";
import { cn } from "@/lib/utils";
import { parseRawStream, type ParsedStreamEvent } from "@/lib/stream-utils";
import { useState, useMemo, useRef, useEffect } from "react";
import { ArrowLeft, RotateCcw } from "lucide-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

interface TaskDetailProps {
  taskId: number;
  onBack: () => void;
}

interface ComplianceFinding {
  check_id: string;
  severity: string;
  issue: string;
  source_url?: string;
  as_of?: string;
}

interface ComplianceCheckData {
  phase?: string;
  profile?: string;
  enforcement?: string;
  checked_at?: string;
  passed?: boolean;
  findings?: ComplianceFinding[];
}

function complianceData(task: any): ComplianceCheckData | null {
  const raw = task?.structured_data?.compliance_check;
  if (!raw || typeof raw !== "object") return null;
  return raw as ComplianceCheckData;
}

export function TaskDetail({ taskId, onBack }: TaskDetailProps) {
  const { data: task, isLoading } = useTaskDetail(taskId);
  const isActive = task ? isActiveStatus(task.status) : false;
  const { events, streaming } = useTaskStream(taskId, isActive);
  const { data: container } = useTaskContainer(taskId, isActive);
  const { mode: uiMode } = useUIMode();
  const isMinimal = uiMode === "minimal";
  const { data: fullModes = [] } = useFullModes();
  const queryClient = useQueryClient();
  const [retrying, setRetrying] = useState(false);
  const [showRevision, setShowRevision] = useState(false);
  const [revisionFeedback, setRevisionFeedback] = useState("");
  const [revHistory, setRevHistory] = useState<RevisionHistory | null>(null);
  const [showRevHistory, setShowRevHistory] = useState(false);
  const [showDiagnostics, setShowDiagnostics] = useState(false);
  const { data: diagnostics, isFetching: diagnosticsLoading } = useQuery<TaskDiagnostics>({
    queryKey: ["task_diagnostics", taskId],
    queryFn: () => getTaskDiagnostics(taskId),
    enabled: showDiagnostics,
    staleTime: 10_000,
  });

  if (isLoading || !task) {
    return (
      <div className="flex h-full items-center justify-center text-xs text-zinc-600">Loading...</div>
    );
  }

  const compliance = complianceData(task);

  return (
    <div className="flex h-full flex-col">
      {/* Task header */}
      <div className="space-y-3 border-b border-white/[0.06] px-5 py-4">
        <div className="flex items-start gap-3">
          <button
            onClick={onBack}
            aria-label="Back"
            className="mt-0.5 rounded-md p-1 text-zinc-600 transition-colors hover:bg-white/[0.06] hover:text-zinc-300 md:hidden"
          >
            <ArrowLeft className="h-4 w-4" />
          </button>
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2.5">
              <span className="font-mono text-[11px] text-zinc-600">#{task.id}</span>
              <StatusBadge status={task.status} />
              {task.mode && task.mode !== "sweborg" && task.mode !== "swe" && (
                <span className="rounded bg-violet-500/10 px-1.5 py-0.5 text-[9px] font-medium text-violet-400">
                  {task.mode}
                </span>
              )}
              {task.status === "failed" && (
                <button
                  onClick={async () => {
                    setRetrying(true);
                    try {
                      await retryTask(task.id);
                      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
                      await queryClient.invalidateQueries({ queryKey: ["task", task.id] });
                    } finally {
                      setRetrying(false);
                    }
                  }}
                  disabled={retrying}
                  className="ml-auto flex items-center gap-1.5 rounded-md border border-white/[0.08] px-2.5 py-1 text-[11px] font-medium text-zinc-400 hover:border-blue-500/40 hover:text-blue-400 disabled:opacity-50 transition-colors"
                >
                  <RotateCcw className="h-3 w-3" />
                  {retrying ? "Retrying…" : "Retry"}
                </button>
              )}
            </div>
            <h2 className="mt-1 text-[14px] font-medium leading-snug text-zinc-200">
              {task.title}
            </h2>
          </div>
        </div>

        <PhaseTracker status={task.status} mode={task.mode} />

        {/* Human review gate */}
        {fullModes.some((m) => m.name === task.mode && m.phases.some((p) => p.name === task.status && p.phase_type === "human_review")) && (() => {
          const phaseInstruction = fullModes.find((m) => m.name === task.mode)
            ?.phases.find((p) => p.name === task.status)?.instruction;
          const invalidate = () => {
            queryClient.invalidateQueries({ queryKey: ["tasks"] });
            queryClient.invalidateQueries({ queryKey: ["task", task.id] });
          };
          return (
            <div className="rounded-lg border border-emerald-500/20 bg-emerald-500/[0.04] p-3 space-y-2">
              {phaseInstruction && (
                <div className="text-[11px] text-emerald-400/70 leading-relaxed">{phaseInstruction}</div>
              )}
              <div className="flex items-center gap-2">
                <button
                  onClick={async () => { await approveTask(task.id); invalidate(); }}
                  className="rounded-md bg-emerald-500/15 px-3 py-1.5 text-[11px] font-medium text-emerald-400 hover:bg-emerald-500/25 transition-colors"
                >Approve</button>
                <button
                  onClick={() => setShowRevision(!showRevision)}
                  className="rounded-md bg-amber-500/10 px-3 py-1.5 text-[11px] font-medium text-amber-400 hover:bg-amber-500/20 transition-colors"
                >Request Revision</button>
                <button
                  onClick={async () => {
                    if (confirm("Reject this task? It will be marked as failed.")) {
                      await rejectTask(task.id, "Rejected by reviewer");
                      invalidate();
                    }
                  }}
                  className="rounded-md bg-red-500/10 px-3 py-1.5 text-[11px] font-medium text-red-400 hover:bg-red-500/20 transition-colors"
                >Reject</button>
              </div>
              {showRevision && (
                <div className="space-y-1.5">
                  <textarea
                    value={revisionFeedback}
                    onChange={(e) => setRevisionFeedback(e.target.value)}
                    rows={3}
                    className="w-full rounded-md border border-amber-500/20 bg-black/30 px-2.5 py-1.5 text-[11px] text-zinc-200 outline-none focus:border-amber-500/40 resize-y placeholder:text-zinc-600"
                    placeholder="Describe what needs to change..."
                  />
                  <div className="flex items-center gap-2">
                    <button
                      onClick={async () => {
                        if (!revisionFeedback.trim()) return;
                        await requestRevision(task.id, revisionFeedback.trim());
                        setRevisionFeedback("");
                        setShowRevision(false);
                        invalidate();
                      }}
                      disabled={!revisionFeedback.trim()}
                      className="rounded-md bg-amber-500/15 px-3 py-1 text-[11px] font-medium text-amber-400 hover:bg-amber-500/25 disabled:opacity-40 transition-colors"
                    >Send Revision Request</button>
                    <button
                      onClick={() => { setShowRevision(false); setRevisionFeedback(""); }}
                      className="text-[11px] text-zinc-600 hover:text-zinc-400"
                    >Cancel</button>
                  </div>
                </div>
              )}
            </div>
          );
        })()}

        {task.status === "pending_review" && compliance && (compliance.findings?.length ?? 0) > 0 && (
          <div className="rounded-lg border border-fuchsia-500/20 bg-fuchsia-500/[0.04] p-3 space-y-2">
            <div className="text-[11px] text-fuchsia-300/80">
              Compliance check blocked this task ({compliance.profile ?? "unknown profile"}).
            </div>
            <div className="space-y-1">
              {(compliance.findings ?? []).map((f, idx) => (
                <div key={`${f.check_id}-${idx}`} className="rounded border border-fuchsia-500/10 bg-black/20 px-2 py-1.5">
                  <div className="text-[11px] text-zinc-200">{f.issue}</div>
                  <div className="mt-0.5 flex items-center gap-2 text-[10px] text-zinc-500">
                    <span className="uppercase">{f.severity}</span>
                    {f.as_of && <span>as of {f.as_of}</span>}
                    {f.source_url && (
                      <a className="text-blue-400 hover:text-blue-300" href={f.source_url} target="_blank" rel="noreferrer">
                        source
                      </a>
                    )}
                  </div>
                </div>
              ))}
            </div>
            <div className="flex items-center gap-2">
              <button
                onClick={() => {
                  const prefill = `Compliance remediation required (${compliance.profile ?? "profile"}):\n` +
                    (compliance.findings ?? [])
                      .map((f) => `- [${f.severity}] ${f.issue}${f.source_url ? ` (source: ${f.source_url})` : ""}`)
                      .join("\n");
                  setRevisionFeedback(prefill);
                  setShowRevision(true);
                }}
                className="rounded-md bg-fuchsia-500/15 px-3 py-1.5 text-[11px] font-medium text-fuchsia-300 hover:bg-fuchsia-500/25 transition-colors"
              >
                Prefill Revision Request
              </button>
              <button
                onClick={async () => {
                  const prefill = `Compliance remediation required (${compliance.profile ?? "profile"}):\n` +
                    (compliance.findings ?? [])
                      .map((f) => `- [${f.severity}] ${f.issue}${f.source_url ? ` (source: ${f.source_url})` : ""}`)
                      .join("\n");
                  await requestRevision(task.id, prefill);
                  queryClient.invalidateQueries({ queryKey: ["tasks"] });
                  queryClient.invalidateQueries({ queryKey: ["task", task.id] });
                }}
                className="rounded-md bg-amber-500/15 px-3 py-1.5 text-[11px] font-medium text-amber-300 hover:bg-amber-500/25 transition-colors"
              >
                Request Revision Now
              </button>
            </div>
          </div>
        )}

        <div className="flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-zinc-500">
          {task.repo_path && (
            <span title={task.repo_path}>
              <span className="text-zinc-600">repo</span> {repoName(task.repo_path)}
            </span>
          )}
          {!isMinimal && task.branch && (
            <span>
              <span className="text-zinc-600">branch</span> <span className="font-mono">{task.branch}</span>
            </span>
          )}
          {!isMinimal && task.attempt > 0 && (
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
          <button
            onClick={() => setShowDiagnostics((v) => !v)}
            className="rounded border border-white/[0.08] px-1.5 py-0.5 text-[10px] text-zinc-500 hover:border-white/[0.16] hover:text-zinc-300 transition-colors"
          >
            {showDiagnostics ? "Hide diagnostics" : "Show diagnostics"}
          </button>
          <span className="flex items-center gap-1">
            <span className="text-zinc-600">backend</span>
            <select
              value={task.backend || ""}
              onChange={async (e) => {
                await setTaskBackend(task.id, e.target.value);
                queryClient.invalidateQueries({ queryKey: ["task", task.id] });
              }}
              className="rounded border border-white/[0.06] bg-transparent py-0 text-[11px] text-zinc-400 outline-none hover:border-white/[0.12] focus:border-blue-500/40"
            >
              <option value="">default</option>
              <option value="claude">claude</option>
              <option value="codex">codex</option>
              <option value="local">local</option>
            </select>
          </span>
          {container && (
            <span
              className={cn(
                "font-mono rounded px-1.5 py-0.5 text-[9px]",
                container.status === "running"
                  ? "bg-emerald-500/10 text-emerald-400"
                  : "bg-zinc-500/10 text-zinc-500"
              )}
              title={`Container: ${container.container_id}`}
            >
              container {container.status}
            </span>
          )}
        </div>
      </div>

      {task.description && (
        <div className="max-h-16 overflow-y-auto border-b border-white/[0.06] px-5 py-2.5 text-[11px] leading-relaxed text-zinc-500">
          {task.description}
        </div>
      )}

      {(task.revision_count ?? 0) > 0 && (
        <div className="mx-4 mt-3">
          <button
            onClick={async () => {
              if (showRevHistory) { setShowRevHistory(false); return; }
              const h = await getRevisionHistory(task.id);
              setRevHistory(h);
              setShowRevHistory(true);
            }}
            className="flex items-center gap-2 text-[11px] text-amber-500/70 hover:text-amber-400 transition-colors"
          >
            <span>{showRevHistory ? "Hide" : "Show"} Revision History ({task.revision_count} revision{task.revision_count !== 1 ? "s" : ""})</span>
          </button>
          {showRevHistory && revHistory && (
            <div className="mt-2 space-y-2 border-l-2 border-amber-500/20 pl-3">
              {revHistory.rounds.map((round) => (
                <div key={round.round} className="space-y-1">
                  <div className="text-[10px] font-medium text-zinc-300">
                    {round.round === 0 ? "Initial Draft" : `Draft ${round.round + 1}`}
                  </div>
                  {round.feedback && (
                    <div className="rounded border border-amber-500/10 bg-amber-500/[0.03] px-2 py-1.5 text-[11px]">
                      <div className="text-[9px] text-amber-500/60 mb-0.5">Reviewer feedback</div>
                      <div className="text-zinc-300 whitespace-pre-wrap">{round.feedback}</div>
                    </div>
                  )}
                  {round.phases.length > 0 && (
                    <div className="flex flex-wrap gap-1">
                      {round.phases.map((p, j) => (
                        <span key={j} className={cn(
                          "rounded px-1.5 py-0.5 text-[9px] font-medium",
                          p.exit_code === 0 ? "bg-emerald-500/10 text-emerald-400" : "bg-red-500/10 text-red-400"
                        )}>{p.phase}</span>
                      ))}
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {task.last_error && task.status === "failed" && (
        <div className="mx-4 mt-3 rounded-lg border border-red-500/20 bg-red-500/[0.05] p-3">
          <pre className="max-h-20 overflow-y-auto whitespace-pre-wrap font-mono text-[11px] text-red-400/90">
            {task.last_error}
          </pre>
        </div>
      )}

      {showDiagnostics && (
        <div className="mx-4 mt-3 rounded-lg border border-white/[0.08] bg-white/[0.02] p-3 text-[11px]">
          {diagnosticsLoading && !diagnostics ? (
            <div className="text-zinc-500">Loading diagnostics…</div>
          ) : diagnostics ? (
            <div className="space-y-2">
              <div className="flex flex-wrap gap-x-3 gap-y-1 text-zinc-400">
                <span>stuck_suspected: <span className={diagnostics.summary.stuck_suspected ? "text-amber-400" : "text-zinc-500"}>{String(diagnostics.summary.stuck_suspected)}</span></span>
                <span>same_failure_streak: {diagnostics.summary.same_failure_streak}</span>
                <span>queue_entries: {diagnostics.queue_entries.length}</span>
                <span>attempt: {diagnostics.summary.attempt}/{diagnostics.summary.max_attempts}</span>
              </div>
              <div>
                <div className="mb-1 text-zinc-500">Recent events</div>
                <div className="max-h-24 overflow-y-auto space-y-1">
                  {diagnostics.recent_events.slice(0, 8).map((e) => (
                    <div key={e.id} className="font-mono text-[10px] text-zinc-500">
                      [{e.created_at}] {e.kind}
                    </div>
                  ))}
                </div>
              </div>
            </div>
          ) : (
            <div className="text-zinc-500">Diagnostics unavailable</div>
          )}
        </div>
      )}

      {/* Main content area: terminal / outputs + chat */}
      <div className="flex flex-1 min-h-0 flex-col overflow-hidden">
        {/* Live terminal for active tasks */}
        {(isActive || streaming) && (
          <div className="mx-4 mt-3 flex-1 min-h-0">
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

        <TaskChat taskId={task.id} />
      </div>
    </div>
  );
}

// StreamEvent and formatToolInput/parseRawStream imported from @/lib/stream-utils

function StreamView({ raw }: { raw: string }) {
  const events = useMemo(() => parseRawStream(raw), [raw]);

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

function StreamEventBlock({ event: ev }: { event: ParsedStreamEvent }) {
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
        <pre className="whitespace-pre-wrap font-mono text-[11px] leading-relaxed text-zinc-300">
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
          className="flex w-full items-center gap-2 px-3 py-1.5 text-left"
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
          className="flex w-full items-center gap-2 px-3 py-1.5 text-left"
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
        <pre className="whitespace-pre-wrap font-mono text-[11px] leading-relaxed text-emerald-300/80">
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
  setTimeout(() => URL.revokeObjectURL(url), 100);
}

function OutputSelector({ outputs }: { outputs: TaskOutput[] }) {
  const { mode: uiMode } = useUIMode();
  const [viewMode, setViewMode] = useState<"summary" | "trace" | "diff">("summary");
  const [copied, setCopied] = useState(false);
  const copiedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => { if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current); };
  }, []);

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

  const lastLabeled = labeled[labeled.length - 1];
  const [selectedKey, setSelectedKey] = useState(
    lastLabeled ? lastLabeled.phase + "-" + lastLabeled.id : ""
  );

  const selected = labeled.find((o) => o.phase + "-" + o.id === selectedKey) ?? lastLabeled;

  if (!selected) return null;
  const isDiff = selected.phase.endsWith("_diff");
  const hasStream = !!selected.raw_stream;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-white/[0.06] px-5 py-2">
        <select
          value={selectedKey}
          onChange={(e) => setSelectedKey(e.target.value)}
          className="rounded-md border border-white/[0.08] bg-white/[0.04] px-2.5 py-1 text-[11px] font-medium uppercase tracking-wide text-zinc-300 outline-none focus:border-blue-500/40"
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
                  "px-2 py-0.5 text-[10px] font-medium transition-colors",
                  viewMode === "summary"
                    ? "bg-white/[0.08] text-zinc-200"
                    : "text-zinc-500 hover:text-zinc-300"
                )}
              >
                Summary
              </button>
              {hasStream && uiMode === "advanced" && (
                <button
                  onClick={() => setViewMode("trace")}
                  className={cn(
                    "border-l border-white/[0.08] px-2 py-0.5 text-[10px] font-medium transition-colors",
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
                  if (copiedTimerRef.current) clearTimeout(copiedTimerRef.current);
                  copiedTimerRef.current = setTimeout(() => setCopied(false), 1500);
                });
              }}
              className="rounded-md px-2 py-0.5 text-[10px] font-medium text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.05] transition-colors"
            >
              {copied ? "Copied" : "Copy"}
            </button>
            <button
              onClick={() => {
                const text = viewMode === "trace" && hasStream ? selected.raw_stream : selected.output;
                const ext = viewMode === "trace" && hasStream ? "ndjson" : "txt";
                downloadText(text || "", `task-${selected.id}-${selected.phase}.${ext}`);
              }}
              className="rounded-md px-2 py-0.5 text-[10px] font-medium text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.05] transition-colors"
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
          <pre className="p-4 font-mono text-[11px] leading-relaxed text-zinc-400 whitespace-pre-wrap break-words">
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
