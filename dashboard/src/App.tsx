import { useState, useEffect, useCallback, useMemo, Component } from "react";
import type { ReactNode, ErrorInfo } from "react";
import { useLogs, useStatus } from "@/lib/api";
import { UIModeProvider, useUIMode } from "@/lib/ui-mode";
import type { UIMode } from "@/lib/ui-mode";
import { Header } from "@/components/header";
import { TaskList } from "@/components/task-list";
import { TaskDetail } from "@/components/task-detail";
import { LogViewer } from "@/components/log-viewer";
import { QueuePanel } from "@/components/queue-panel";
import { ProposalsPanel } from "@/components/proposals-panel";
import { ChatPanel } from "@/components/chat-panel";
import { ProjectsPanel } from "@/components/projects-panel";
import { ModeCreatorPanel } from "@/components/mode-creator-panel";
import { SettingsPanel } from "@/components/settings-panel";
import { BorgLogo } from "@/components/borg-logo";
import { ListTodo, Terminal, GitMerge, MessageSquare, Lightbulb, Settings, FolderOpen, Wrench } from "lucide-react";
import { cn } from "@/lib/utils";

class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null };
  static getDerivedStateFromError(error: Error) { return { error }; }
  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("Dashboard error:", error, info.componentStack);
  }
  render() {
    if (this.state.error) {
      return (
        <div className="flex h-screen items-center justify-center bg-[#09090b] text-zinc-400">
          <div className="max-w-md text-center space-y-3">
            <p className="text-sm font-medium text-red-400">Something went wrong</p>
            <pre className="text-xs text-zinc-600 whitespace-pre-wrap">{(this.state.error as Error).message}</pre>
            <button onClick={() => this.setState({ error: null })} className="text-xs text-zinc-500 hover:text-zinc-300 underline">
              Try again
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

type View = "tasks" | "projects" | "creator" | "proposals" | "logs" | "queue" | "chat" | "settings";
type MobileTab = "tasks" | "logs" | "queue" | "chat";

function useIsMobile() {
  const [mobile, setMobile] = useState(() =>
    typeof window !== "undefined" ? window.innerWidth < 768 : false
  );
  useEffect(() => {
    const mq = window.matchMedia("(max-width: 767px)");
    const handler = () => setMobile(mq.matches);
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, []);
  return mobile;
}

const ALL_NAV_ITEMS = [
  { key: "tasks" as const, label: "Tasks", Icon: ListTodo, minimalVisible: true },
  { key: "projects" as const, label: "Projects", Icon: FolderOpen, minimalVisible: true },
  { key: "creator" as const, label: "Creator", Icon: Wrench, minimalVisible: true },
  { key: "proposals" as const, label: "Proposals", Icon: Lightbulb, minimalVisible: true },
  { key: "logs" as const, label: "Logs", Icon: Terminal, minimalVisible: false },
  { key: "queue" as const, label: "Queue", Icon: GitMerge, minimalVisible: false },
  { key: "chat" as const, label: "Chat", Icon: MessageSquare, minimalVisible: true },
  { key: "settings" as const, label: "Settings", Icon: Settings, minimalVisible: true },
] as const;

const MOBILE_TABS = [
  { key: "tasks" as const, label: "Tasks", Icon: ListTodo },
  { key: "logs" as const, label: "Logs", Icon: Terminal },
  { key: "queue" as const, label: "Queue", Icon: GitMerge },
  { key: "chat" as const, label: "Chat", Icon: MessageSquare },
] as const;

function detectDefaultMode(repos?: { mode: string; is_self: boolean }[]): UIMode {
  if (!repos || repos.length === 0) return "advanced";
  const primary = repos.find((r) => r.is_self) ?? repos[0];
  return primary.mode === "lawborg" || primary.mode === "legal" ? "minimal" : "advanced";
}

function detectDefaultView(repos?: { mode: string; is_self: boolean }[]): View {
  if (!repos || repos.length === 0) return "tasks";
  const primary = repos.find((r) => r.is_self) ?? repos[0];
  if (["lawborg", "legal", "databorg", "salesborg"].includes(primary.mode)) {
    return "projects";
  }
  return "tasks";
}

export default function App() {
  const { data: status } = useStatus();
  const defaultMode = useMemo(() => detectDefaultMode(status?.watched_repos), [status]);

  return (
    <ErrorBoundary>
      <UIModeProvider defaultMode={defaultMode}>
        <AppInner />
      </UIModeProvider>
    </ErrorBoundary>
  );
}

function AppInner() {
  const [selectedTaskId, setSelectedTaskId] = useState<number | null>(null);
  const [view, setView] = useState<View>("tasks");
  const [repoFilter, setRepoFilter] = useState<string | null>(null);
  const [mobileTab, setMobileTab] = useState<MobileTab>("tasks");
  const [mobileBottomTab, setMobileBottomTab] = useState<"queue" | "proposals">("proposals");
  const { logs, connected } = useLogs();
  const { data: status } = useStatus();
  const { mode: uiMode } = useUIMode();
  const isMobile = useIsMobile();
  const defaultView = useMemo(() => detectDefaultView(status?.watched_repos), [status]);

  useEffect(() => {
    setView((curr) => (curr === "tasks" ? defaultView : curr));
  }, [defaultView]);

  const navItems = useMemo(
    () => ALL_NAV_ITEMS.filter((item) => uiMode === "advanced" || item.minimalVisible),
    [uiMode]
  );

  const handleSelectTask = useCallback((id: number) => setSelectedTaskId(id), []);
  const handleBackFromTask = useCallback(() => setSelectedTaskId(null), []);

  if (isMobile) {
    return (
      <div className="flex flex-col bg-[#09090b] text-foreground antialiased" style={{ height: "100dvh" }}>
        <Header connected={connected} mobile />

        <div className="min-h-0 flex-1 flex flex-col overflow-hidden">
          {mobileTab === "tasks" && (
            <div className="min-h-0 flex-1 overflow-hidden">
              {selectedTaskId !== null ? (
                <TaskDetail taskId={selectedTaskId} onBack={handleBackFromTask} />
              ) : (
                <TaskList selectedId={selectedTaskId} onSelect={handleSelectTask} repoFilter={repoFilter} />
              )}
            </div>
          )}

          {mobileTab === "logs" && <LogViewer logs={logs} />}

          {mobileTab === "queue" && (
            <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
              <div className="flex shrink-0 border-b border-white/[0.06]">
                <button
                  onClick={() => setMobileBottomTab("proposals")}
                  className={`flex-1 py-2.5 text-[13px] font-medium transition-colors ${
                    mobileBottomTab === "proposals"
                      ? "text-zinc-200 border-b-2 border-blue-400"
                      : "text-zinc-500"
                  }`}
                >
                  Proposals
                </button>
                <button
                  onClick={() => setMobileBottomTab("queue")}
                  className={`flex-1 py-2.5 text-[13px] font-medium transition-colors ${
                    mobileBottomTab === "queue"
                      ? "text-zinc-200 border-b-2 border-blue-400"
                      : "text-zinc-500"
                  }`}
                >
                  Queue
                </button>
              </div>
              <div className="min-h-0 flex-1 overflow-hidden">
                {mobileBottomTab === "queue" ? (
                  <QueuePanel repoFilter={repoFilter} />
                ) : (
                  <ProposalsPanel repoFilter={repoFilter} />
                )}
              </div>
            </div>
          )}

          {mobileTab === "chat" && <ChatPanel />}
        </div>

        <nav
          className="flex shrink-0 border-t border-white/[0.06] bg-[#09090b]"
          style={{ paddingBottom: "env(safe-area-inset-bottom)" }}
        >
          {MOBILE_TABS.map(({ key, label, Icon }) => (
            <button
              key={key}
              onClick={() => setMobileTab(key)}
              className={cn(
                "flex flex-1 flex-col items-center gap-0.5 pt-2 pb-1.5 active:opacity-70 transition-colors",
                mobileTab === key ? "text-blue-400" : "text-zinc-600"
              )}
            >
              <Icon className="h-5 w-5" strokeWidth={mobileTab === key ? 2 : 1.5} />
              <span className="text-[10px] font-medium">{label}</span>
            </button>
          ))}
        </nav>
      </div>
    );
  }

  // Desktop layout
  return (
    <div className="flex h-screen bg-[#09090b] text-foreground antialiased">
      {/* Sidebar nav */}
      <nav className="flex w-[52px] shrink-0 flex-col items-center border-r border-white/[0.06] bg-[#09090b] py-3">
        <div className="borg-logo mb-3 w-full bg-orange-500 aspect-square">
          <BorgLogo />
          <div className="borg-logo-ghost grid grid-cols-2 grid-rows-2" aria-hidden>
            {"BORG".split("").map((c, i) => (
              <span key={i} className="flex items-center justify-center text-[15px]">{c}</span>
            ))}
          </div>
        </div>

        <div className="flex flex-1 flex-col items-center gap-1">
          {navItems.map(({ key, label, Icon }) => (
            <button
              key={key}
              onClick={() => setView(key)}
              title={label}
              className={cn(
                "group relative flex h-9 w-9 items-center justify-center rounded-lg transition-all",
                view === key
                  ? "bg-white/[0.1] text-zinc-100"
                  : "text-zinc-600 hover:bg-white/[0.05] hover:text-zinc-400"
              )}
            >
              <Icon className="h-[18px] w-[18px]" strokeWidth={view === key ? 2 : 1.5} />
              {view === key && (
                <div className="absolute left-0 top-1/2 -translate-y-1/2 h-4 w-0.5 rounded-r bg-blue-400" />
              )}
            </button>
          ))}
        </div>

        {/* Status indicator at bottom */}
        <div className="mt-auto flex flex-col items-center gap-2">
          {(status?.dispatched_agents ?? 0) > 0 && (
            <div className="flex h-5 w-5 items-center justify-center" title={`${status?.dispatched_agents} active agent(s)`}>
              <span className="text-[10px] font-bold tabular-nums text-blue-400">{status?.dispatched_agents}</span>
            </div>
          )}
          <div
            className={cn(
              "h-2 w-2 rounded-full",
              connected
                ? "bg-emerald-500 shadow-[0_0_6px_rgba(16,185,129,0.4)]"
                : "bg-red-500"
            )}
            title={connected ? "Connected" : "Disconnected"}
          />
        </div>
      </nav>

      {/* Main content */}
      <div className="flex min-w-0 flex-1 flex-col">
        <Header connected={connected} view={view} repoFilter={repoFilter} onRepoFilterChange={setRepoFilter} />

        <div className="min-h-0 flex-1 overflow-hidden">
          {view === "tasks" && (
            <div className="flex h-full">
              <div className="w-[300px] shrink-0 overflow-hidden border-r border-white/[0.06]">
                <TaskList
                  selectedId={selectedTaskId}
                  onSelect={handleSelectTask}
                  repoFilter={repoFilter}
                />
              </div>
              <div className="min-w-0 flex-1 overflow-hidden">
                {selectedTaskId !== null ? (
                  <TaskDetail taskId={selectedTaskId} onBack={handleBackFromTask} />
                ) : (
                  <EmptyState status={status} />
                )}
              </div>
            </div>
          )}

          {view === "projects" && <ProjectsPanel />}
          {view === "creator" && <ModeCreatorPanel />}
          {view === "proposals" && <ProposalsPanel repoFilter={repoFilter} />}
          {view === "logs" && <LogViewer logs={logs} />}
          {view === "queue" && <QueuePanel repoFilter={repoFilter} />}
          {view === "chat" && <ChatPanel />}
          {view === "settings" && <SettingsPanel />}
        </div>
      </div>
    </div>
  );
}

function EmptyState({ status }: { status?: { active_tasks: number; merged_tasks: number; failed_tasks: number; total_tasks: number } | null }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-6 text-center">
      <div className="flex h-16 w-16 items-center justify-center rounded-2xl bg-white/[0.03] ring-1 ring-white/[0.06]">
        <ListTodo className="h-7 w-7 text-zinc-700" strokeWidth={1.5} />
      </div>
      <div>
        <p className="text-[13px] font-medium text-zinc-500">Select a task to view details</p>
        <p className="mt-1 text-[11px] text-zinc-700">or create a new one from the header</p>
      </div>
      {status && (
        <div className="flex gap-6 mt-2">
          <StatPill value={status.active_tasks} label="Active" color="text-blue-400" />
          <StatPill value={status.merged_tasks} label="Merged" color="text-emerald-400" />
          <StatPill value={status.failed_tasks} label="Failed" color="text-red-400" />
          <StatPill value={status.total_tasks} label="Total" color="text-zinc-400" />
        </div>
      )}
    </div>
  );
}

function StatPill({ value, label, color }: { value: number; label: string; color: string }) {
  return (
    <div className="flex flex-col items-center gap-0.5">
      <span className={cn("text-lg font-semibold tabular-nums", color)}>{value}</span>
      <span className="text-[10px] text-zinc-600">{label}</span>
    </div>
  );
}
