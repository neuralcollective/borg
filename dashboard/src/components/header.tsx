import { useStatus } from "@/lib/api";
import { useUIMode } from "@/lib/ui-mode";
import { useDomain } from "@/lib/domain";
import { TaskCreator } from "./task-creator";
import { FocusPicker } from "./focus-picker";
import { BorgLogo } from "./borg-logo";
import { repoName } from "@/lib/types";

function formatUptime(seconds: number) {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

type View = "tasks" | "projects" | "creator" | "proposals" | "logs" | "queue" | "chat" | "knowledge" | "settings";

const VIEW_TITLES: Record<View, string> = {
  tasks: "Pipeline Tasks",
  projects: "Projects",
  creator: "Borg Creator",
  proposals: "Proposals",
  logs: "System Logs",
  queue: "Integration Queue",
  chat: "Chat",
  knowledge: "Knowledge Base",
  settings: "Settings",
};

export function Header({
  connected,
  mobile,
  view,
  repoFilter,
  onRepoFilterChange,
}: {
  connected: boolean;
  mobile?: boolean;
  view?: View;
  repoFilter?: string | null;
  onRepoFilterChange?: (repo: string | null) => void;
}) {
  const { data: status } = useStatus();
  const { mode: uiMode } = useUIMode();
  const domain = useDomain();
  const isMinimal = uiMode === "minimal";

  if (mobile) {
    return (
      <header className="flex h-12 shrink-0 items-center gap-3 border-b border-white/[0.07] bg-[#09090b] px-5">
        <div className="flex items-center gap-2.5">
          <div className={`borg-logo h-6 w-6 ${domain.accentBg}`}>
            <BorgLogo size="mobile" />
            <div className="borg-logo-ghost grid grid-cols-2 grid-rows-2" aria-hidden>
              {"BORG".split("").map((c, i) => (
                <span key={i} className="flex items-center justify-center text-[16px]">{c}</span>
              ))}
            </div>
          </div>
          <span className="text-[14px] font-semibold tracking-tight text-white">Borg</span>
        </div>

        <div className="ml-auto flex items-center gap-3">
          <TaskCreator />
          {status?.continuous_mode && (
            <span className="flex items-center gap-1.5 text-[12px] text-zinc-400">
              <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
              Cont
            </span>
          )}
          <span className="text-[12px] tabular-nums text-zinc-400">
            {status ? formatUptime(status.uptime_s) : "--"}
          </span>
          <span className={`h-2 w-2 rounded-full ${connected ? "bg-emerald-500 shadow-[0_0_6px_rgba(16,185,129,0.4)]" : "bg-red-500"}`} />
        </div>
      </header>
    );
  }

  const repos = status?.watched_repos ?? [];
  const multiRepo = repos.length > 1;

  return (
    <header className="flex h-14 shrink-0 items-center gap-4 border-b border-white/[0.07] px-6">
      <h1 className="text-[15px] font-semibold text-zinc-100">
        {VIEW_TITLES[view ?? "tasks"]}
      </h1>

      {!isMinimal && (
        <>
          <div className="h-4 w-px bg-white/[0.07]" />
          <div className="flex items-center gap-4 text-[12px] text-zinc-500">
            {status?.continuous_mode && (
              <span className="flex items-center gap-1.5 text-zinc-400">
                <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
                Continuous
              </span>
            )}
            <span>
              Up <span className="text-zinc-300">{status ? formatUptime(status.uptime_s) : "--"}</span>
            </span>
            <span>
              Model <span className="text-zinc-300">{status?.model ?? "--"}</span>
            </span>
            <span className="h-3 w-px bg-white/[0.07]" />
            <span>
              Active <span className="text-blue-400 tabular-nums">{status?.active_tasks ?? 0}</span>
            </span>
            <span>
              Merged <span className="text-emerald-400 tabular-nums">{status?.merged_tasks ?? 0}</span>
            </span>
            <span>
              AI Calls <span className="text-cyan-400 tabular-nums">{status?.ai_requests ?? 0}</span>
            </span>
            <span>
              Failed <span className="text-red-400 tabular-nums">{status?.failed_tasks ?? 0}</span>
            </span>
            {status?.version && (
              <span className="rounded-full bg-white/[0.04] px-1.5 py-0.5 font-mono text-[10px] text-zinc-600">
                {status.version}
              </span>
            )}
          </div>
        </>
      )}

      <div className="ml-auto flex items-center gap-4">
        <FocusPicker />
        {multiRepo && onRepoFilterChange && (
          <select
            value={repoFilter ?? ""}
            onChange={(e) => onRepoFilterChange(e.target.value || null)}
            className="h-7 shrink-0 rounded-lg border border-white/[0.07] bg-white/[0.03] px-2 text-[12px] text-zinc-300 outline-none"
          >
            <option value="">All repos</option>
            {repos.map((r) => (
              <option key={r.path} value={r.path}>
                {repoName(r.path)}{!r.auto_merge ? " (manual)" : ""}
              </option>
            ))}
          </select>
        )}
        <TaskCreator />
      </div>
    </header>
  );
}
