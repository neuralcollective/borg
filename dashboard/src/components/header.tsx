import { useStatus } from "@/lib/api";
import { useUIMode } from "@/lib/ui-mode";
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

type View = "tasks" | "projects" | "creator" | "proposals" | "logs" | "queue" | "chat" | "settings";

const VIEW_TITLES: Record<View, string> = {
  tasks: "Pipeline Tasks",
  projects: "Projects",
  creator: "Borg Creator",
  proposals: "Proposals",
  logs: "System Logs",
  queue: "Integration Queue",
  chat: "Chat",
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
  const isMinimal = uiMode === "minimal";

  if (mobile) {
    return (
      <header className="flex h-11 shrink-0 items-center gap-3 border-b border-white/[0.06] bg-[#09090b] px-4">
        <div className="flex items-center gap-2">
          <div className="borg-logo h-6 w-6 bg-orange-500">
            <BorgLogo size="mobile" />
            <div className="borg-logo-ghost grid grid-cols-2 grid-rows-2" aria-hidden>
              {"BORG".split("").map((c, i) => (
                <span key={i} className="flex items-center justify-center text-[16px]">{c}</span>
              ))}
            </div>
          </div>
          <span className="text-[13px] font-semibold tracking-tight text-white">Borg</span>
        </div>

        <div className="ml-auto flex items-center gap-3">
          <TaskCreator />
          {status?.continuous_mode && (
            <span className="flex items-center gap-1 text-[11px] text-zinc-400">
              <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
              Cont
            </span>
          )}
          <span className="text-[11px] tabular-nums text-zinc-500">
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
    <header className="flex h-11 shrink-0 items-center gap-4 border-b border-white/[0.06] px-5">
      <h1 className="text-[13px] font-semibold text-zinc-200">
        {VIEW_TITLES[view ?? "tasks"]}
      </h1>

      {!isMinimal && (
        <>
          <div className="h-4 w-px bg-white/[0.06]" />
          <div className="flex items-center gap-3 text-[11px] text-zinc-500">
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
            {status?.version && (
              <span className="rounded-full bg-white/[0.04] px-1.5 py-0.5 font-mono text-[10px] text-zinc-600">
                {status.version}
              </span>
            )}
          </div>
        </>
      )}

      <div className="ml-auto flex items-center gap-3">
        <FocusPicker />
        {multiRepo && onRepoFilterChange && (
          <select
            value={repoFilter ?? ""}
            onChange={(e) => onRepoFilterChange(e.target.value || null)}
            className="h-6 shrink-0 rounded border border-white/[0.08] bg-transparent px-1.5 text-[11px] text-zinc-300 outline-none"
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
