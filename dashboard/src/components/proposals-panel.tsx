import { useState } from "react";
import { useProposals, approveProposal, dismissProposal, triageProposals, reopenProposal } from "@/lib/api";
import { repoName, type Proposal } from "@/lib/types";
import { useQueryClient } from "@tanstack/react-query";

interface ProposalsPanelProps {
  repoFilter: string | null;
}

function scoreColor(score: number): string {
  if (score >= 8) return "text-emerald-400 bg-emerald-500/15 ring-emerald-500/30";
  if (score >= 5) return "text-amber-400 bg-amber-500/15 ring-amber-500/30";
  return "text-red-400 bg-red-500/15 ring-red-500/30";
}

function barWidth(value: number): string {
  return `${Math.max(value, 0) * 20}%`;
}

function barColor(key: string, value: number): string {
  if (key === "risk" || key === "effort") {
    // Inverted: low is good
    if (value <= 2) return "bg-emerald-400";
    if (value <= 3) return "bg-amber-400";
    return "bg-red-400";
  }
  if (value >= 4) return "bg-emerald-400";
  if (value >= 3) return "bg-amber-400";
  return "bg-red-400";
}

function TriageTooltip({ proposal }: { proposal: Proposal }) {
  const dims = [
    { key: "impact", label: "Impact", value: proposal.triage_impact },
    { key: "feasibility", label: "Feasibility", value: proposal.triage_feasibility },
    { key: "risk", label: "Risk", value: proposal.triage_risk },
    { key: "effort", label: "Effort", value: proposal.triage_effort },
  ];

  return (
    <div className="absolute left-0 top-full z-50 mt-1 w-64 rounded-lg border border-white/[0.08] bg-zinc-900 p-3 shadow-xl">
      <div className="space-y-2">
        {dims.map((d) => (
          <div key={d.key} className="flex items-center gap-2">
            <span className="w-20 text-[10px] text-zinc-500">{d.label}</span>
            <div className="flex-1 h-1.5 rounded-full bg-white/[0.06]">
              <div
                className={`h-full rounded-full ${barColor(d.key, d.value)}`}
                style={{ width: barWidth(d.value) }}
              />
            </div>
            <span className="w-4 text-right text-[10px] tabular-nums text-zinc-400">{d.value}</span>
          </div>
        ))}
      </div>
      {proposal.triage_reasoning && (
        <p className="mt-2 border-t border-white/[0.06] pt-2 text-[10px] leading-relaxed text-zinc-500">
          {proposal.triage_reasoning}
        </p>
      )}
    </div>
  );
}

export function ProposalsPanel({ repoFilter }: ProposalsPanelProps) {
  const { data: proposals } = useProposals();
  const queryClient = useQueryClient();
  const [expandedId, setExpandedId] = useState<number | null>(null);
  const [acting, setActing] = useState<number | null>(null);
  const [triaging, setTriaging] = useState(false);
  const [hoveredScore, setHoveredScore] = useState<number | null>(null);

  const filtered = (repoFilter
    ? proposals?.filter((p) => p.repo_path === repoFilter)
    : proposals) ?? [];

  const pending = filtered.filter((p) => p.status === "proposed");
  const handled = filtered.filter((p) => p.status !== "proposed");

  const hasTriage = pending.some((p) => p.triage_score > 0);
  const sorted = hasTriage
    ? [...pending].sort((a, b) => b.triage_score - a.triage_score)
    : pending;

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: ["proposals"] });
    queryClient.invalidateQueries({ queryKey: ["tasks"] });
  };

  const handleApprove = async (id: number) => {
    setActing(id);
    try {
      await approveProposal(id);
      invalidate();
    } finally {
      setActing(null);
    }
  };

  const handleDismiss = async (id: number) => {
    setActing(id);
    try {
      await dismissProposal(id);
      invalidate();
    } finally {
      setActing(null);
    }
  };

  const handleReopen = async (id: number) => {
    setActing(id);
    try {
      await reopenProposal(id);
      invalidate();
    } finally {
      setActing(null);
    }
  };

  const handleTriage = async () => {
    setTriaging(true);
    try {
      await triageProposals();
      invalidate();
    } finally {
      setTriaging(false);
    }
  };

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-10 shrink-0 items-center justify-between border-b border-white/[0.06] px-4">
        <span className="text-[12px] md:text-[11px] font-medium text-zinc-400">Proposals</span>
        <div className="flex items-center gap-2">
          {pending.length > 0 && (
            <button
              onClick={handleTriage}
              disabled={triaging}
              className="rounded-md bg-violet-500/10 px-2 py-0.5 text-[10px] font-medium text-violet-400 ring-1 ring-inset ring-violet-500/20 transition-colors hover:bg-violet-500/20 disabled:opacity-50"
            >
              {triaging ? "Scoring..." : "Triage"}
            </button>
          )}
          <span className="rounded-full bg-white/[0.06] px-2 py-0.5 text-[10px] tabular-nums text-zinc-500">
            {pending.length}
          </span>
        </div>
      </div>
      <div className="flex-1 overflow-y-auto overscroll-contain">
        <div className="p-2 space-y-1">
          {sorted.map((p) => (
            <div key={p.id} className="rounded-lg border border-white/[0.06] bg-white/[0.02]">
              <button
                onClick={() => setExpandedId(expandedId === p.id ? null : p.id)}
                className="flex w-full items-center gap-2.5 px-3 py-2.5 md:py-2 text-left active:bg-white/[0.03]"
              >
                <span className="font-mono text-[10px] text-zinc-600">#{p.id}</span>
                {p.triage_score > 0 ? (
                  <span
                    className={`relative inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-bold tabular-nums ring-1 ring-inset ${scoreColor(p.triage_score)}`}
                    onMouseEnter={() => setHoveredScore(p.id)}
                    onMouseLeave={() => setHoveredScore(null)}
                  >
                    {p.triage_score}
                    {hoveredScore === p.id && <TriageTooltip proposal={p} />}
                  </span>
                ) : (
                  <span className="inline-flex items-center rounded-full bg-amber-500/10 px-2 py-0.5 text-[10px] font-medium text-amber-400 ring-1 ring-inset ring-amber-500/20">
                    proposed
                  </span>
                )}
                {p.repo_path && (
                  <span className="shrink-0 rounded-md bg-white/[0.04] px-1.5 py-0.5 text-[9px] font-medium text-zinc-500">
                    {repoName(p.repo_path)}
                  </span>
                )}
                <span className="flex-1 truncate text-[13px] md:text-[12px] text-zinc-300">{p.title}</span>
              </button>

              {expandedId === p.id && (
                <div className="border-t border-white/[0.04] px-3 py-3 md:py-2.5 space-y-2.5 md:space-y-2">
                  <p className="text-[13px] md:text-[11px] text-zinc-400 leading-relaxed">{p.description}</p>
                  {p.rationale && (
                    <p className="text-[12px] md:text-[11px] text-zinc-500 italic leading-relaxed">{p.rationale}</p>
                  )}
                  {p.triage_score > 0 && (
                    <div className="grid grid-cols-2 gap-x-4 gap-y-1.5 pt-1">
                      {[
                        { label: "Impact", value: p.triage_impact, key: "impact" },
                        { label: "Feasibility", value: p.triage_feasibility, key: "feasibility" },
                        { label: "Risk", value: p.triage_risk, key: "risk" },
                        { label: "Effort", value: p.triage_effort, key: "effort" },
                      ].map((d) => (
                        <div key={d.key} className="flex items-center gap-2">
                          <span className="w-16 text-[10px] text-zinc-500">{d.label}</span>
                          <div className="flex-1 h-1 rounded-full bg-white/[0.06]">
                            <div
                              className={`h-full rounded-full ${barColor(d.key, d.value)}`}
                              style={{ width: barWidth(d.value) }}
                            />
                          </div>
                          <span className="w-3 text-right text-[10px] tabular-nums text-zinc-400">{d.value}</span>
                        </div>
                      ))}
                      {p.triage_reasoning && (
                        <p className="col-span-2 pt-1 text-[10px] text-zinc-500 italic">{p.triage_reasoning}</p>
                      )}
                    </div>
                  )}
                  <div className="flex gap-2.5 md:gap-2 pt-1">
                    <button
                      onClick={() => handleApprove(p.id)}
                      disabled={acting === p.id}
                      className="rounded-md bg-emerald-500/10 px-4 md:px-3 py-2 md:py-1 text-[13px] md:text-[11px] font-medium text-emerald-400 ring-1 ring-inset ring-emerald-500/20 transition-colors active:bg-emerald-500/25 hover:bg-emerald-500/20 disabled:opacity-50"
                    >
                      {acting === p.id ? "..." : "Approve"}
                    </button>
                    <button
                      onClick={() => handleDismiss(p.id)}
                      disabled={acting === p.id}
                      className="rounded-md bg-red-500/10 px-4 md:px-3 py-2 md:py-1 text-[13px] md:text-[11px] font-medium text-red-400 ring-1 ring-inset ring-red-500/20 transition-colors active:bg-red-500/25 hover:bg-red-500/20 disabled:opacity-50"
                    >
                      Dismiss
                    </button>
                  </div>
                </div>
              )}
            </div>
          ))}

          {handled.length > 0 && pending.length > 0 && (
            <div className="mx-3 my-1.5 h-px bg-white/[0.04]" />
          )}

          {handled.slice(0, 10).map((p) => (
            <div key={p.id} className="flex items-center gap-2.5 rounded-lg px-3 py-2 md:py-1.5 text-[13px] md:text-[12px] opacity-50">
              <span className="font-mono text-[10px] text-zinc-600">#{p.id}</span>
              <span className={`inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-medium ring-1 ring-inset ${
                p.status === "approved"
                  ? "bg-emerald-500/10 text-emerald-400 ring-emerald-500/20"
                  : p.status === "auto_dismissed"
                  ? "bg-orange-500/10 text-orange-400 ring-orange-500/20"
                  : "bg-zinc-500/10 text-zinc-400 ring-zinc-500/20"
              }`}>
                {p.status === "auto_dismissed" ? "auto-closed" : p.status}
              </span>
              <span className="flex-1 truncate text-zinc-500">{p.title}</span>
              {p.status === "auto_dismissed" && (
                <button
                  onClick={() => handleReopen(p.id)}
                  disabled={acting === p.id}
                  className="shrink-0 rounded-md bg-blue-500/10 px-2 py-0.5 text-[10px] font-medium text-blue-400 ring-1 ring-inset ring-blue-500/20 transition-colors hover:bg-blue-500/20 disabled:opacity-50"
                >
                  {acting === p.id ? "..." : "Reopen"}
                </button>
              )}
            </div>
          ))}

          {!pending.length && !handled.length && (
            <p className="py-8 text-center text-[13px] md:text-xs text-zinc-700">No proposals yet</p>
          )}
        </div>
      </div>
    </div>
  );
}
