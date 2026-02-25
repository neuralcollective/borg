import { useStatus } from "@/lib/api";
import { cn } from "@/lib/utils";

function formatUptime(seconds: number) {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

export function Header({
  connected,
  onToggleChat,
  chatOpen,
  mobile,
}: {
  connected: boolean;
  onToggleChat?: () => void;
  chatOpen?: boolean;
  mobile?: boolean;
}) {
  const { data: status } = useStatus();

  if (mobile) {
    return (
      <header className="flex h-11 shrink-0 items-center gap-3 border-b border-white/[0.06] bg-[#0a0a0a] px-4">
        <div className="flex items-center gap-2">
          <div className="flex h-6 w-6 items-center justify-center rounded-md bg-white">
            <span className="text-[10px] font-black text-black">B</span>
          </div>
          <span className="text-[13px] font-semibold tracking-tight text-white">Borg</span>
        </div>

        <div className="ml-auto flex items-center gap-3">
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

  return (
    <header className="flex h-12 shrink-0 items-center gap-5 border-b border-white/[0.06] bg-[#0a0a0a] px-5">
      <div className="flex items-center gap-2.5">
        <div className="flex h-6 w-6 items-center justify-center rounded-md bg-white">
          <span className="text-[10px] font-black text-black">B</span>
        </div>
        <span className="text-[13px] font-semibold tracking-tight text-white">Borg</span>
        <span className="rounded-full bg-white/[0.06] px-2 py-0.5 font-mono text-[10px] text-zinc-500">{status?.version ?? ""}</span>
      </div>

      <div className="h-4 w-px bg-white/[0.08]" />

      <div className="flex items-center gap-4">
        {status?.continuous_mode ? (
          <span className="flex items-center gap-1.5 text-[11px] text-zinc-400">
            <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
            Continuous
          </span>
        ) : (
          <span className="text-[11px] text-zinc-500">
            Release every <span className="text-zinc-300">{status?.release_interval_mins ?? "?"}m</span>
          </span>
        )}

        <span className="text-[11px] text-zinc-500">
          Up <span className="text-zinc-300">{status ? formatUptime(status.uptime_s) : "--"}</span>
        </span>

        <span className="text-[11px] text-zinc-500">
          Model <span className="text-zinc-300">{status?.model ?? "--"}</span>
        </span>

        {(status?.watched_repos?.length ?? 0) > 1 && (
          <span className="text-[11px] text-zinc-500">
            Repos <span className="text-zinc-300">{status?.watched_repos.length}</span>
          </span>
        )}
      </div>

      <div className="ml-auto flex items-center gap-3">
        {onToggleChat && (
          <button
            onClick={onToggleChat}
            className={cn(
              "rounded-md px-2.5 py-1 text-[11px] font-medium transition-colors",
              chatOpen
                ? "bg-white/[0.1] text-zinc-200"
                : "text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.05]"
            )}
          >
            Chat
          </button>
        )}

        <span className={`h-2 w-2 rounded-full ${connected ? "bg-emerald-500 shadow-[0_0_6px_rgba(16,185,129,0.4)]" : "bg-red-500"}`} />
        <span className="text-[11px] text-zinc-500">{connected ? "Connected" : "Disconnected"}</span>
      </div>
    </header>
  );
}
