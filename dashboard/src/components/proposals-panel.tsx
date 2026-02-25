import { useState } from "react";
import { useProposals, approveProposal, dismissProposal } from "@/lib/api";
import { repoName } from "@/lib/types";
import { useQueryClient } from "@tanstack/react-query";

interface ProposalsPanelProps {
  repoFilter: string | null;
}

export function ProposalsPanel({ repoFilter }: ProposalsPanelProps) {
  const { data: proposals } = useProposals();
  const queryClient = useQueryClient();
  const [expandedId, setExpandedId] = useState<number | null>(null);
  const [acting, setActing] = useState<number | null>(null);

  const pending = (repoFilter
    ? proposals?.filter((p) => p.repo_path === repoFilter)
    : proposals
  )?.filter((p) => p.status === "pending") ?? [];

  const handled = (repoFilter
    ? proposals?.filter((p) => p.repo_path === repoFilter)
    : proposals
  )?.filter((p) => p.status !== "pending") ?? [];

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

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-10 shrink-0 items-center justify-between border-b border-white/[0.06] px-4">
        <span className="text-[11px] font-medium text-zinc-400">Proposals</span>
        <span className="rounded-full bg-white/[0.06] px-2 py-0.5 text-[10px] tabular-nums text-zinc-500">
          {pending.length}
        </span>
      </div>
      <div className="flex-1 overflow-y-auto overscroll-contain">
        <div className="p-2 space-y-1">
          {pending.map((p) => (
            <div key={p.id} className="rounded-lg border border-white/[0.06] bg-white/[0.02]">
              <button
                onClick={() => setExpandedId(expandedId === p.id ? null : p.id)}
                className="flex w-full items-center gap-2.5 px-3 py-2 text-left"
              >
                <span className="font-mono text-[10px] text-zinc-600">#{p.id}</span>
                <span className="inline-flex items-center rounded-full bg-amber-500/10 px-2 py-0.5 text-[10px] font-medium text-amber-400 ring-1 ring-inset ring-amber-500/20">
                  pending
                </span>
                {p.repo_path && (
                  <span className="shrink-0 rounded-md bg-white/[0.04] px-1.5 py-0.5 text-[9px] font-medium text-zinc-500">
                    {repoName(p.repo_path)}
                  </span>
                )}
                <span className="flex-1 truncate text-[12px] text-zinc-300">{p.title}</span>
              </button>

              {expandedId === p.id && (
                <div className="border-t border-white/[0.04] px-3 py-2.5 space-y-2">
                  <p className="text-[11px] text-zinc-400 leading-relaxed">{p.description}</p>
                  {p.rationale && (
                    <p className="text-[11px] text-zinc-500 italic leading-relaxed">{p.rationale}</p>
                  )}
                  <div className="flex gap-2 pt-1">
                    <button
                      onClick={() => handleApprove(p.id)}
                      disabled={acting === p.id}
                      className="rounded-md bg-emerald-500/10 px-3 py-1 text-[11px] font-medium text-emerald-400 ring-1 ring-inset ring-emerald-500/20 transition-colors hover:bg-emerald-500/20 disabled:opacity-50"
                    >
                      {acting === p.id ? "..." : "Approve"}
                    </button>
                    <button
                      onClick={() => handleDismiss(p.id)}
                      disabled={acting === p.id}
                      className="rounded-md bg-red-500/10 px-3 py-1 text-[11px] font-medium text-red-400 ring-1 ring-inset ring-red-500/20 transition-colors hover:bg-red-500/20 disabled:opacity-50"
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
            <div key={p.id} className="flex items-center gap-2.5 rounded-lg px-3 py-1.5 text-[12px] opacity-50">
              <span className="font-mono text-[10px] text-zinc-600">#{p.id}</span>
              <span className={`inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-medium ring-1 ring-inset ${
                p.status === "approved"
                  ? "bg-emerald-500/10 text-emerald-400 ring-emerald-500/20"
                  : "bg-zinc-500/10 text-zinc-400 ring-zinc-500/20"
              }`}>
                {p.status}
              </span>
              <span className="flex-1 truncate text-zinc-500">{p.title}</span>
            </div>
          ))}

          {!pending.length && !handled.length && (
            <p className="py-8 text-center text-xs text-zinc-700">No proposals yet</p>
          )}
        </div>
      </div>
    </div>
  );
}
