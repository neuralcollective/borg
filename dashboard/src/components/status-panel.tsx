import { Activity, AlertTriangle, CheckCircle2, XCircle } from "lucide-react";
import { useMemo } from "react";
import { type McpStatusItem, useMcpStatus } from "@/lib/api";
import { useDashboardMode } from "@/lib/dashboard-mode";
import { cn } from "@/lib/utils";

const STATUS_STYLES: Record<
  McpStatusItem["status"],
  {
    dot: string;
    pill: string;
    label: string;
    summary: string;
  }
> = {
  verified: {
    dot: "bg-emerald-400 shadow-[0_0_10px_rgba(74,222,128,0.35)]",
    pill: "border-emerald-500/20 bg-emerald-500/10 text-emerald-300",
    label: "Verified",
    summary: "text-emerald-300",
  },
  configured: {
    dot: "bg-amber-300 shadow-[0_0_10px_rgba(252,211,77,0.28)]",
    pill: "border-amber-500/20 bg-amber-500/10 text-amber-200",
    label: "Configured",
    summary: "text-amber-200",
  },
  degraded: {
    dot: "bg-red-400 shadow-[0_0_10px_rgba(248,113,113,0.3)]",
    pill: "border-red-500/20 bg-red-500/10 text-red-200",
    label: "Issue",
    summary: "text-red-200",
  },
  missing: {
    dot: "bg-[#5b5348]",
    pill: "border-[#2f2a25] bg-[#171411] text-[#938a7d]",
    label: "Missing",
    summary: "text-[#938a7d]",
  },
};

function formatCheckedAt(value: string) {
  if (!value) return "";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleString();
}

function StatusRow({ item }: { item: McpStatusItem }) {
  const style = STATUS_STYLES[item.status];
  return (
    <div className="flex items-start gap-3 rounded-xl border border-[#2a2520] bg-[#151310]/70 p-4">
      <div className={cn("mt-1 h-2.5 w-2.5 shrink-0 rounded-full", style.dot)} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0">
            <div className="text-[13px] font-medium text-[#e8e0d4]">{item.label}</div>
            <div className="mt-1 text-[11px] leading-5 text-[#9c9486]">{item.detail}</div>
          </div>
          <span
            className={cn(
              "shrink-0 rounded-full border px-2.5 py-1 text-[10px] font-medium uppercase tracking-[0.14em]",
              style.pill,
            )}
          >
            {style.label}
          </span>
        </div>
        {(item.source || item.checked_at) && (
          <div className="mt-2 flex flex-wrap gap-3 text-[10px] uppercase tracking-[0.12em] text-[#6b6459]">
            {item.source && <span>Source: {item.source}</span>}
            {item.checked_at && <span>Checked: {formatCheckedAt(item.checked_at)}</span>}
          </div>
        )}
      </div>
    </div>
  );
}

function SummaryCard({
  label,
  value,
  tone,
  Icon,
}: {
  label: string;
  value: number;
  tone: keyof typeof STATUS_STYLES;
  Icon: typeof CheckCircle2;
}) {
  return (
    <div className="rounded-2xl border border-[#2a2520] bg-[#151310]/80 p-4">
      <div className="flex items-center justify-between">
        <span className="text-[11px] uppercase tracking-[0.14em] text-[#6b6459]">{label}</span>
        <Icon className={cn("h-4 w-4", STATUS_STYLES[tone].summary)} strokeWidth={1.8} />
      </div>
      <div className={cn("mt-3 text-3xl font-semibold tabular-nums", STATUS_STYLES[tone].summary)}>{value}</div>
    </div>
  );
}

function StatusSection({ title, desc, items }: { title: string; desc: string; items: McpStatusItem[] }) {
  return (
    <section className="space-y-3">
      <div>
        <h2 className="text-[15px] font-semibold text-[#e8e0d4]">{title}</h2>
        <p className="mt-1 text-[12px] text-[#7e7568]">{desc}</p>
      </div>
      <div className="grid gap-3 xl:grid-cols-2">
        {items.map((item) => (
          <StatusRow key={item.key} item={item} />
        ))}
      </div>
    </section>
  );
}

const LEGAL_RUNTIME_KEYS = new Set(["lawborg_mcp"]);

export function StatusPanel() {
  const { data, isLoading, error } = useMcpStatus();
  const { isLegal } = useDashboardMode();

  const filtered = useMemo(() => {
    if (!data) return null;
    const runtime = isLegal ? data.runtime : data.runtime.filter((i) => !LEGAL_RUNTIME_KEYS.has(i.key));
    const services = isLegal ? data.services : [];
    const all = [...data.agent_access, ...runtime, ...services];
    const summary = { verified: 0, configured: 0, degraded: 0, missing: 0 };
    for (const item of all) {
      const s = item.status as keyof typeof summary;
      if (s in summary) summary[s]++;
    }
    return { ...data, runtime, services, summary };
  }, [data, isLegal]);

  if (isLoading && !data) {
    return <div className="flex h-full items-center justify-center text-xs text-[#6b6459]">Loading MCP status...</div>;
  }

  if (error) {
    return (
      <div className="flex h-full items-center justify-center px-6 text-center">
        <div className="rounded-2xl border border-red-500/20 bg-red-500/8 px-5 py-4 text-sm text-red-200">
          {error instanceof Error ? error.message : "Failed to load MCP status."}
        </div>
      </div>
    );
  }

  if (!filtered) {
    return null;
  }

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-6xl space-y-8 px-6 py-8">
        <div className="flex flex-wrap items-end justify-between gap-4">
          <div>
            <div className="flex items-center gap-3">
              <div className="flex h-11 w-11 items-center justify-center rounded-2xl border border-[#2a2520] bg-[#161310] text-amber-300">
                <Activity className="h-5 w-5" strokeWidth={1.8} />
              </div>
              <div>
                <h1 className="text-[22px] font-semibold text-[#f1e7d8]">System Status</h1>
                <p className="mt-1 text-[12px] text-[#7e7568]">
                  Green means verified by Borg. Amber means configured but not actively probed. Red means missing or
                  failing.
                </p>
              </div>
            </div>
          </div>
          <div className="rounded-xl border border-[#2a2520] bg-[#151310]/80 px-4 py-3 text-right">
            <div className="text-[10px] uppercase tracking-[0.14em] text-[#6b6459]">Workspace</div>
            <div className="mt-1 text-[13px] font-medium text-[#e8e0d4]">{filtered.workspace.name}</div>
            <div className="mt-1 text-[11px] text-[#7e7568]">Updated {formatCheckedAt(filtered.generated_at)}</div>
          </div>
        </div>

        <div className="grid gap-3 md:grid-cols-4">
          <SummaryCard label="Verified" value={filtered.summary.verified} tone="verified" Icon={CheckCircle2} />
          <SummaryCard label="Configured" value={filtered.summary.configured} tone="configured" Icon={AlertTriangle} />
          <SummaryCard label="Issues" value={filtered.summary.degraded} tone="degraded" Icon={XCircle} />
          <SummaryCard label="Missing" value={filtered.summary.missing} tone="missing" Icon={XCircle} />
        </div>

        <StatusSection
          title="Agent Access"
          desc="Per-user Claude and OpenAI accounts that Borg can restore into the task sandbox."
          items={filtered.agent_access}
        />

        <StatusSection
          title="MCP Runtime"
          desc="Core MCP runtime pieces Borg can actually verify right now."
          items={filtered.runtime}
        />

        {isLegal && (
          <StatusSection
            title="External Services"
            desc="Workspace or global MCP service credentials available to legal and domain-specific tools."
            items={filtered.services}
          />
        )}
      </div>
    </div>
  );
}
