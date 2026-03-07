import { useState, useEffect, useCallback, useMemo, Component } from "react";
import type { ReactNode, ErrorInfo } from "react";
import { useLogs, useStatus } from "@/lib/api";
import { UIModeProvider, useUIMode } from "@/lib/ui-mode";
import type { UIMode } from "@/lib/ui-mode";
import { DomainProvider, useDomain } from "@/lib/domain";
import { AuthProvider, useAuth } from "@/lib/auth";
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
import { KnowledgePanel } from "@/components/knowledge-panel";
import { LoginPage } from "@/components/login-page";
import { BorgLogo } from "@/components/borg-logo";
import { ListTodo, Terminal, GitMerge, MessageSquare, Lightbulb, Settings, FolderOpen, Wrench, BookOpen } from "lucide-react";
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
        <div className="flex h-screen items-center justify-center bg-[#0f0e0c] text-[#9c9486]">
          <div className="max-w-md text-center space-y-3">
            <p className="text-lg font-semibold text-amber-400">Oh, Borg!</p>
            <pre className="text-xs text-[#6b6459] whitespace-pre-wrap">{(this.state.error as Error).message}</pre>
            <button onClick={() => this.setState({ error: null })} className="text-xs text-[#6b6459] hover:text-[#e8e0d4] underline">
              Try again
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

type View = "tasks" | "projects" | "creator" | "proposals" | "logs" | "queue" | "chat" | "knowledge" | "settings";
type MobileTab = "tasks" | "projects" | "queue" | "chat";

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
  { key: "knowledge" as const, label: "Knowledge", Icon: BookOpen, minimalVisible: false },
  { key: "settings" as const, label: "Settings", Icon: Settings, minimalVisible: true },
] as const;

const MOBILE_TABS = [
  { key: "tasks" as const, label: "Tasks", Icon: ListTodo },
  { key: "projects" as const, label: "Matters", Icon: FolderOpen },
  { key: "queue" as const, label: "Queue", Icon: GitMerge },
  { key: "chat" as const, label: "Chat", Icon: MessageSquare },
] as const;

function detectDefaultMode(domain: { defaultMode: "minimal" | "advanced" }, repos?: { mode: string; is_self: boolean }[]): UIMode {
  if (!repos || repos.length === 0) return domain.defaultMode;
  const primary = repos.find((r) => r.is_self) ?? repos[0];
  if (primary.mode === "lawborg" || primary.mode === "legal") return "minimal";
  return domain.defaultMode;
}

function detectDefaultView(domain: { defaultView: "tasks" | "projects" }, repos?: { mode: string; is_self: boolean }[]): View {
  if (!repos || repos.length === 0) return domain.defaultView;
  const primary = repos.find((r) => r.is_self) ?? repos[0];
  if (["lawborg", "legal"].includes(primary.mode)) {
    return "projects";
  }
  return domain.defaultView;
}

export default function App() {
  return (
    <ErrorBoundary>
      <AuthProvider>
        <AuthGate />
      </AuthProvider>
    </ErrorBoundary>
  );
}

function AuthGate() {
  const { ready, user, needsSetup } = useAuth();

  if (!ready) {
    return (
      <div className="flex h-screen items-center justify-center bg-[#0f0e0c]">
        <div className="flex flex-col items-center gap-4">
          <div className="h-10 w-10 animate-spin rounded-full border-2 border-[#2a2520] border-t-amber-400" />
        </div>
      </div>
    );
  }

  if (needsSetup || !user) {
    return <LoginPage />;
  }

  return (
    <DomainProvider>
      <AppWithDomain />
    </DomainProvider>
  );
}

function AppWithDomain() {
  const domain = useDomain();
  const { data: status } = useStatus();
  const defaultMode = useMemo(() => detectDefaultMode(domain, status?.watched_repos), [domain, status]);

  return (
    <UIModeProvider defaultMode={defaultMode}>
      <AppInner />
    </UIModeProvider>
  );
}

function AppInner() {
  const domain = useDomain();
  const [selectedTaskId, setSelectedTaskId] = useState<number | null>(null);
  const [view, setView] = useState<View>("tasks");
  const [repoFilter, setRepoFilter] = useState<string | null>(null);
  const [mobileTab, setMobileTab] = useState<MobileTab>("tasks");
  const [mobileBottomTab, setMobileBottomTab] = useState<"queue" | "proposals">("proposals");
  const { logs, connected } = useLogs();
  const { data: status } = useStatus();
  const { mode: uiMode } = useUIMode();
  const isMobile = useIsMobile();
  const defaultView = useMemo(() => detectDefaultView(domain, status?.watched_repos), [domain, status]);
  const sidebarAlert = !!status?.guardrail_alert;

  useEffect(() => {
    setView((curr) => (curr === "tasks" ? defaultView : curr));
  }, [defaultView]);

  const isGlobalLawMode = useMemo(() => {
    const repos = status?.watched_repos;
    if (!repos?.length) return false;
    const primary = repos.find((r) => r.is_self) ?? repos[0];
    return primary.mode === "lawborg" || primary.mode === "legal";
  }, [status]);

  const LAW_HIDDEN_KEYS = ["queue", "proposals", "logs"];

  const navItems = useMemo(
    () => ALL_NAV_ITEMS.filter((item) => {
      if (domain.hiddenNavKeys.includes(item.key)) return false;
      if (isGlobalLawMode && LAW_HIDDEN_KEYS.includes(item.key)) return false;
      return uiMode === "advanced" || item.minimalVisible;
    }),
    [uiMode, domain, isGlobalLawMode]
  );

  const handleSelectTask = useCallback((id: number) => setSelectedTaskId(id), []);
  const handleBackFromTask = useCallback(() => setSelectedTaskId(null), []);

  if (isMobile) {
    return (
      <div className="flex flex-col bg-[#0f0e0c] text-foreground antialiased" style={{ height: "100dvh" }}>
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

          {mobileTab === "projects" && <ProjectsPanel />}

          {mobileTab === "queue" && (
            <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
              <div className="flex shrink-0 border-b border-[#2a2520]">
                <button
                  onClick={() => setMobileBottomTab("proposals")}
                  className={`flex-1 py-2.5 text-[13px] font-medium transition-colors ${
                    mobileBottomTab === "proposals"
                      ? "text-[#e8e0d4] border-b-2 border-amber-400"
                      : "text-[#6b6459]"
                  }`}
                >
                  Proposals
                </button>
                <button
                  onClick={() => setMobileBottomTab("queue")}
                  className={`flex-1 py-2.5 text-[13px] font-medium transition-colors ${
                    mobileBottomTab === "queue"
                      ? "text-[#e8e0d4] border-b-2 border-amber-400"
                      : "text-[#6b6459]"
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
          className="flex shrink-0 border-t border-[#2a2520] bg-[#0f0e0c]"
          style={{ paddingBottom: "env(safe-area-inset-bottom)" }}
        >
          {MOBILE_TABS.filter((t) => !(isGlobalLawMode && t.key === "queue")).map(({ key, label, Icon }) => (
            <button
              key={key}
              onClick={() => setMobileTab(key)}
              className={cn(
                "flex flex-1 flex-col items-center gap-0.5 pt-2 pb-1.5 active:opacity-70 transition-colors",
                mobileTab === key ? "text-amber-400" : "text-[#6b6459]"
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
    <div className="flex h-screen bg-[#0f0e0c] text-foreground antialiased">
      {/* Sidebar nav — slim icon bar with overlay expansion */}
      <div className="w-14 shrink-0" />
      <nav
        className={cn(
          "group/nav fixed left-0 top-0 z-30 flex h-full w-14 hover:w-[180px] flex-col items-start border-r pb-4 transition-[width] duration-200 ease-out overflow-hidden",
          sidebarAlert
            ? "border-red-500/30 bg-red-950/35"
            : "border-[#2a2520] bg-gradient-to-b from-[#1c1a17] to-[#151412]",
          "hover:shadow-[4px_0_24px_rgba(20,15,10,0.6)]"
        )}
      >
        <div className={cn("borg-logo mb-2 w-full shrink-0 h-14", domain.accentBg)}>
          <BorgLogo expanded />
          <div className="borg-logo-ghost grid grid-cols-2 grid-rows-2 group-hover/nav:grid-cols-4 group-hover/nav:grid-rows-1" aria-hidden>
            {"BORG".split("").map((c, i) => (
              <span key={i} className="flex items-center justify-center text-[22px]">{c}</span>
            ))}
          </div>
        </div>

        <div className="flex flex-1 flex-col items-start gap-0.5 w-full px-2">
          {navItems.map(({ key, label, Icon }) => (
            <button
              key={key}
              onClick={() => setView(key)}
              title={label}
              aria-label={label}
              className={cn(
                "group relative flex h-10 w-full items-center gap-3 rounded-xl px-[10px] transition-all duration-150",
                view === key
                  ? sidebarAlert
                    ? "bg-red-400/20 text-red-50"
                    : "bg-amber-500/[0.08] text-[#e8e0d4]"
                  : sidebarAlert
                    ? "text-red-200/80 hover:bg-red-400/15 hover:text-red-50"
                    : "text-[#6b6459] hover:bg-amber-500/[0.05] hover:text-[#9c9486]"
              )}
            >
              <Icon className="h-[18px] w-[18px] shrink-0" strokeWidth={view === key ? 2 : 1.5} />
              <span className="truncate text-[13px] font-medium opacity-0 group-hover/nav:opacity-100 transition-opacity duration-200">{label}</span>
              {view === key && (
                <div
                  className={cn(
                    "absolute left-0 top-1/2 -translate-y-1/2 h-5 w-[3px] rounded-r-full",
                    sidebarAlert ? "bg-red-300" : "bg-amber-400"
                  )}
                />
              )}
            </button>
          ))}
        </div>

        {/* Status indicator at bottom */}
        <div className="mt-auto flex flex-col items-center gap-3 w-14 shrink-0">
          {(status?.dispatched_agents ?? 0) > 0 && (
            <div className="flex h-6 w-6 items-center justify-center rounded-full bg-amber-500/15 ring-1 ring-amber-500/20" title={`${status?.dispatched_agents} active agent(s)`}>
              <span className="text-[11px] font-bold tabular-nums text-amber-400">{status?.dispatched_agents}</span>
            </div>
          )}
          <div
            className={cn(
              "h-2.5 w-2.5 rounded-full transition-colors",
              connected
                ? "bg-emerald-500 shadow-[0_0_8px_rgba(200,160,80,0.3)]"
                : "bg-red-500 shadow-[0_0_8px_rgba(239,68,68,0.3)]"
            )}
            title={connected ? "Connected" : "Disconnected"}
          />
        </div>
      </nav>

      {/* Main content */}
      <div className="flex min-w-0 flex-1 flex-col">
        <Header connected={connected} view={view} repoFilter={repoFilter} onRepoFilterChange={setRepoFilter} />

        <div className="min-h-0 flex-1 flex flex-col overflow-hidden">
          <div className="min-h-0 flex-1 overflow-hidden">
            {view === "tasks" && (
              <div className="flex h-full">
                <div className="w-[420px] shrink-0 overflow-hidden border-r border-[#2a2520]">
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
            {view === "knowledge" && <KnowledgePanel />}
            {view === "settings" && <SettingsPanel />}
          </div>

        </div>
      </div>
    </div>
  );
}

function EmptyState({ status }: { status?: { active_tasks: number; merged_tasks: number; ai_requests: number; failed_tasks: number; total_tasks: number } | null }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-8 text-center">
      <div className="flex h-20 w-20 items-center justify-center rounded-3xl bg-gradient-to-br from-amber-500/[0.04] to-amber-500/[0.02] ring-1 ring-amber-900/15">
        <ListTodo className="h-8 w-8 text-[#6b6459]" strokeWidth={1.5} />
      </div>
      <div>
        <p className="text-[15px] font-medium text-[#9c9486]">Select a task to view details</p>
        <p className="mt-2 text-[13px] text-[#6b6459]">or create a new one from the header</p>
      </div>
      {status && (
        <div className="flex gap-8 mt-2">
          <StatPill value={status.active_tasks} label="Active" color="text-blue-400" />
          <StatPill value={status.merged_tasks} label="Merged" color="text-emerald-400" />
          <StatPill value={status.ai_requests} label="AI Calls" color="text-cyan-400" />
          <StatPill value={status.failed_tasks} label="Failed" color="text-red-400" />
          <StatPill value={status.total_tasks} label="Total" color="text-zinc-400" />
        </div>
      )}
    </div>
  );
}

function StatPill({ value, label, color }: { value: number; label: string; color: string }) {
  return (
    <div className="flex flex-col items-center gap-1">
      <span className={cn("text-2xl font-semibold tabular-nums", color)}>{value}</span>
      <span className="text-[11px] font-medium text-[#6b6459]">{label}</span>
    </div>
  );
}
