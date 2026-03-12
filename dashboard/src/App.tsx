import { useQueryClient } from "@tanstack/react-query";
import {
  Activity,
  FolderOpen,
  GitMerge,
  Lightbulb,
  ListTodo,
  MessageSquare,
  Plug,
  Settings,
  Terminal,
  Wrench,
  Zap,
} from "lucide-react";
import type { ErrorInfo, ReactNode } from "react";
import { Component, useCallback, useEffect, useMemo, useState } from "react";
import { AutoTasksPanel } from "@/components/auto-tasks-panel";
import { BorgLogo, PRODUCT_WORD } from "@/components/borg-logo";
import { ChatDrawer } from "@/components/chat-drawer";
import { ChatPanel } from "@/components/chat-panel";
import { ConnectionsPanel } from "@/components/connections-panel";
import { Header } from "@/components/header";
import { LogViewer } from "@/components/log-viewer";
import { LoginPage } from "@/components/login-page";
import { ModeCreatorPanel } from "@/components/mode-creator-panel";
import { ProjectsPanel } from "@/components/projects-panel";
import { ProposalsPanel } from "@/components/proposals-panel";
import { QueuePanel } from "@/components/queue-panel";
import { SettingsPanel } from "@/components/settings-panel";
import { StatusPanel } from "@/components/status-panel";
import { TaskDetail } from "@/components/task-detail";
import { TaskList } from "@/components/task-list";
import type { LinkedCredential, LinkedCredentialConnectSession } from "@/lib/api";
import {
  fetchLinkedCredentialConnectSession,
  startLinkedCredentialConnect,
  submitCredentialConnectCode,
  useLinkedCredentials,
  useLogs,
  useStatus,
} from "@/lib/api";
import { AuthProvider, useAuth } from "@/lib/auth";
import { DashboardModeProvider, useDashboardMode } from "@/lib/dashboard-mode";
import { DomainProvider, useDomain } from "@/lib/domain";
import type { UIMode } from "@/lib/ui-mode";
import { UIModeProvider, useUIMode } from "@/lib/ui-mode";
import { cn } from "@/lib/utils";
import { useVocabulary } from "@/lib/vocabulary";

class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
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
            <button
              onClick={() => this.setState({ error: null })}
              className="text-xs text-[#6b6459] hover:text-[#e8e0d4] underline"
            >
              Try again
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

type View =
  | "tasks"
  | "projects"
  | "connections"
  | "creator"
  | "auto-tasks"
  | "proposals"
  | "logs"
  | "queue"
  | "status"
  | "settings";
type MobileTab = "tasks" | "projects" | "queue" | "chat";
type DashboardNavigateDetail = { view: View };
const SHOW_SETTINGS_NAV = import.meta.env.VITE_SHOW_SETTINGS !== "false";

function useIsMobile() {
  const [mobile, setMobile] = useState(() => (typeof window !== "undefined" ? window.innerWidth < 768 : false));
  useEffect(() => {
    const mq = window.matchMedia("(max-width: 767px)");
    const handler = () => setMobile(mq.matches);
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, []);
  return mobile;
}

const ALL_NAV_ITEMS = [
  { key: "projects" as const, label: "Projects", Icon: FolderOpen, minimalVisible: true },
  { key: "connections" as const, label: "Connections", Icon: Plug, minimalVisible: true },
  { key: "tasks" as const, label: "Tasks", Icon: ListTodo, minimalVisible: false },
  { key: "creator" as const, label: "Pipelines", Icon: Wrench, minimalVisible: true },
  { key: "auto-tasks" as const, label: "Auto Tasks", Icon: Zap, minimalVisible: true },
  { key: "proposals" as const, label: "Proposals", Icon: Lightbulb, minimalVisible: false },
  { key: "status" as const, label: "Status", Icon: Activity, minimalVisible: true },
  { key: "logs" as const, label: "Logs", Icon: Terminal, minimalVisible: false },
  { key: "queue" as const, label: "Queue", Icon: GitMerge, minimalVisible: false },
  { key: "settings" as const, label: "Settings", Icon: Settings, minimalVisible: true },
] as const;

const MOBILE_TABS = [
  { key: "tasks" as const, label: "Tasks", Icon: ListTodo },
  { key: "projects" as const, label: "Projects", Icon: FolderOpen },
  { key: "queue" as const, label: "Queue", Icon: GitMerge },
  { key: "chat" as const, label: "Chat", Icon: MessageSquare },
] as const;

function detectDefaultMode(
  domain: { defaultMode: "minimal" | "advanced" },
  repos?: { mode: string; is_self: boolean }[],
): UIMode {
  if (!repos || repos.length === 0) return domain.defaultMode;
  const primary = repos.find((r) => r.is_self) ?? repos[0];
  if (primary.mode === "sweborg" || primary.mode === "swe") return domain.defaultMode;
  return "minimal";
}

function detectDefaultView(
  _domain: { defaultView: "tasks" | "projects" },
  _repos?: { mode: string; is_self: boolean }[],
): View {
  return "projects";
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

function OnboardingModal() {
  const queryClient = useQueryClient();
  const { data, isLoading } = useLinkedCredentials();
  const [dismissed, setDismissed] = useState(false);
  const [busyProvider, setBusyProvider] = useState<"claude" | "openai" | null>(null);
  const [pending, setPending] = useState<Record<string, LinkedCredentialConnectSession>>({});
  const [codes, setCodes] = useState<Record<string, string>>({});
  const [submittingCode, setSubmittingCode] = useState<string | null>(null);

  useEffect(() => {
    const activeProviders = Object.values(pending)
      .filter((s) => s.status === "pending")
      .map((s) => s.provider);
    if (activeProviders.length === 0) return;
    const timer = window.setInterval(async () => {
      for (const provider of activeProviders) {
        const current = pending[provider];
        if (!current) continue;
        try {
          const updated = await fetchLinkedCredentialConnectSession(current.id);
          if (updated.status === "connected") {
            setPending((prev) => {
              const next = { ...prev };
              delete next[provider];
              return next;
            });
            queryClient.invalidateQueries({ queryKey: ["linked-credentials"] });
          } else {
            setPending((prev) => ({ ...prev, [provider]: updated }));
          }
        } catch {}
      }
    }, 3000);
    return () => window.clearInterval(timer);
  }, [pending, queryClient.invalidateQueries]);

  if (isLoading || dismissed) return null;
  const creds = data?.credentials ?? [];
  const now = new Date();
  const isTokenValid = (c: LinkedCredential) =>
    c.status === "connected" && (!c.expires_at || new Date(c.expires_at) > now);
  const hasValid = creds.some(isTokenValid);
  if (hasValid) return null;
  const hasExpired = creds.some(
    (c) => c.status === "expired" || c.last_error || (c.expires_at && new Date(c.expires_at) <= now),
  );
  const isReconnect = hasExpired && creds.length > 0;

  async function handleConnect(provider: "claude" | "openai") {
    setBusyProvider(provider);
    try {
      const session = await startLinkedCredentialConnect(provider);
      if (session.auth_url) window.open(session.auth_url, "_blank", "noopener,noreferrer");
      setPending((prev) => ({ ...prev, [provider]: session }));
    } finally {
      setBusyProvider(null);
    }
  }

  async function handleSubmitCode(provider: string) {
    const session = pending[provider];
    if (!session) return;
    const code = codes[provider]?.trim();
    if (!code) return;
    setSubmittingCode(provider);
    try {
      await submitCredentialConnectCode(session.id, code);
      setCodes((prev) => {
        const next = { ...prev };
        delete next[provider];
        return next;
      });
    } catch (err) {
      setPending((prev) => ({
        ...prev,
        [provider]: { ...prev[provider]!, status: "failed", error: String(err) },
      }));
    } finally {
      setSubmittingCode(null);
    }
  }

  const providers = [
    { provider: "claude" as const, label: "Claude Pro / Max", hint: "Connects via Claude Code OAuth" },
    { provider: "openai" as const, label: "ChatGPT Plus / Pro", hint: "Connects via Codex device auth" },
  ];

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm">
      <div className="mx-4 w-full max-w-md rounded-2xl border border-[#2a2520] bg-[#151412] p-6 shadow-2xl space-y-5">
        <div>
          <div className="text-[18px] font-semibold text-[#e8e0d4]">
            {isReconnect ? "Reconnect your AI subscription" : "Connect your AI subscription"}
          </div>
          <p className="mt-1 text-[13px] text-[#6b6459]">
            {isReconnect
              ? "Your session has expired. Please reconnect to continue using Borg agents."
              : "Borg uses your Claude or ChatGPT subscription to run agents. Connect one to get started."}
          </p>
          {isReconnect && creds.filter((c) => c.last_error).map((c) => (
            <div key={c.provider} className="mt-2 rounded-lg border border-red-500/20 bg-red-500/5 px-3 py-2 text-[11px] text-red-400">
              {c.provider === "claude" ? "Claude" : "OpenAI"}: {c.last_error}
            </div>
          ))}
        </div>
        <div className="space-y-3">
          {providers.map(({ provider, label, hint }) => {
            const session = pending[provider];
            const isConnected = session?.status === "connected";
            const isPending = session?.status === "pending";
            const isFailed = session?.status === "failed";
            const isBusy = busyProvider === provider;
            const authUrl = session?.auth_url || "";
            const isClaudePending = isPending && provider === "claude";
            const isOpenAIPending = isPending && provider !== "claude";
            return (
              <div key={provider} className="rounded-xl border border-[#2a2520] bg-[#0e0c0a] p-4 space-y-2">
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0">
                    <div className="text-[14px] font-medium text-[#e8e0d4]">{label}</div>
                    <div className="text-[12px] text-[#6b6459]">{hint}</div>
                  </div>
                  {isConnected ? (
                    <span className="rounded-full border border-emerald-500/20 bg-emerald-500/10 px-2.5 py-1 text-[11px] text-emerald-400">
                      Connected
                    </span>
                  ) : (
                    <button
                      onClick={() => handleConnect(provider)}
                      disabled={isBusy || isPending}
                      className="rounded-lg bg-amber-500/15 px-4 py-1.5 text-[12px] font-medium text-amber-300 ring-1 ring-inset ring-amber-500/20 transition-colors hover:bg-amber-500/20 disabled:opacity-50"
                    >
                      {isBusy ? "Opening..." : isPending ? "Waiting..." : "Connect"}
                    </button>
                  )}
                </div>
                {isClaudePending && (
                  <div className="space-y-2">
                    {authUrl && (
                      <a
                        href={authUrl}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="block truncate rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-2 text-[12px] text-amber-400 hover:text-amber-300 transition-colors"
                        title={authUrl}
                      >
                        1. Click here to open Claude authorization
                      </a>
                    )}
                    <p className="text-[11px] text-[#9c9486]">
                      2. Authorize in the browser, then copy the code shown on the page and paste it below.
                    </p>
                    <div className="flex items-center gap-2">
                      <input
                        type="text"
                        value={codes[provider] ?? ""}
                        onChange={(e) => setCodes((prev) => ({ ...prev, [provider]: e.target.value }))}
                        onKeyDown={(e) => e.key === "Enter" && handleSubmitCode(provider)}
                        placeholder="Paste authorization code"
                        className="flex-1 min-w-0 rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-1.5 text-[12px] text-[#e8e0d4] outline-none focus:border-amber-500/30 placeholder:text-[#4a4540]"
                        autoFocus
                      />
                      <button
                        onClick={() => handleSubmitCode(provider)}
                        disabled={submittingCode === provider || !codes[provider]?.trim()}
                        className="shrink-0 rounded-lg bg-amber-500/15 px-3 py-1.5 text-[12px] font-medium text-amber-300 ring-1 ring-inset ring-amber-500/20 transition-colors hover:bg-amber-500/20 disabled:opacity-40"
                      >
                        {submittingCode === provider ? "..." : "Submit"}
                      </button>
                    </div>
                    {session.message && session.message !== "Open the link to authorize your Claude account" && (
                      <div className="rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-2 text-[11px] text-[#9c9486] break-all overflow-hidden max-w-full">
                        {session.message}
                      </div>
                    )}
                  </div>
                )}
                {isOpenAIPending && (
                  <div className="space-y-2">
                    {authUrl ? (
                      <>
                        <a
                          href={authUrl}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="block truncate rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-2 text-[12px] text-amber-400 hover:text-amber-300 transition-colors"
                          title={authUrl}
                        >
                          1. Click here to open OpenAI authorization
                        </a>
                        {session.device_code ? (
                          <>
                            <p className="text-[11px] text-[#9c9486]">2. Enter this code on the page:</p>
                            <div
                              className="flex items-center justify-center gap-2 rounded-lg border border-[#2a2520] bg-[#1c1a17] px-4 py-3 cursor-pointer hover:border-amber-500/30 transition-colors"
                              onClick={() => navigator.clipboard.writeText(session.device_code)}
                              title="Click to copy"
                            >
                              <span className="font-mono text-[18px] font-bold tracking-widest text-[#e8e0d4]">
                                {session.device_code}
                              </span>
                              <span className="text-[11px] text-[#4a4540]">(click to copy)</span>
                            </div>
                            <p className="text-[11px] text-[#6b6459]">
                              3. After authorizing, this dialog will update automatically.
                            </p>
                          </>
                        ) : (
                          <p className="text-[11px] text-[#9c9486]">Waiting for device code...</p>
                        )}
                      </>
                    ) : (
                      <p className="text-[11px] text-[#9c9486]">Waiting for login instructions...</p>
                    )}
                  </div>
                )}
                {isFailed && (
                  <div className="space-y-2">
                    <p className="text-[11px] text-red-400">{session?.error || "Connection failed"}</p>
                    <button
                      onClick={() => {
                        setPending((prev) => {
                          const next = { ...prev };
                          delete next[provider];
                          return next;
                        });
                      }}
                      className="text-[11px] text-[#6b6459] hover:text-[#9c9486] underline"
                    >
                      Try again
                    </button>
                  </div>
                )}
              </div>
            );
          })}
        </div>
        <button
          onClick={() => setDismissed(true)}
          className="w-full rounded-lg py-2 text-[12px] text-[#4a443d] transition-colors hover:text-[#6b6459]"
        >
          Skip for now
        </button>
      </div>
    </div>
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
    <DashboardModeProvider>
      <DomainProvider>
        <OnboardingModal />
        <AppWithDomain />
      </DomainProvider>
    </DashboardModeProvider>
  );
}

function AppWithDomain() {
  const domain = useDomain();
  const { data: status } = useStatus();
  const { isSWE } = useDashboardMode();
  const defaultMode = useMemo(() => {
    if (!isSWE) return "minimal" as UIMode;
    return detectDefaultMode(domain, status?.watched_repos);
  }, [domain, status, isSWE]);

  return (
    <UIModeProvider defaultMode={defaultMode}>
      <AppInner />
    </UIModeProvider>
  );
}

function AppInner() {
  const domain = useDomain();
  const [selectedTaskId, setSelectedTaskId] = useState<number | null>(null);
  const [view, setView] = useState<View>("projects");
  const [repoFilter, setRepoFilter] = useState<string | null>(null);
  const [mobileTab, setMobileTab] = useState<MobileTab>("tasks");
  const [mobileBottomTab, setMobileBottomTab] = useState<"queue" | "proposals">("proposals");
  const { logs, connected } = useLogs();
  const { data: status } = useStatus();
  const { mode: uiMode } = useUIMode();
  const vocab = useVocabulary();
  const isMobile = useIsMobile();
  const defaultView = useMemo(() => detectDefaultView(domain, status?.watched_repos), [domain, status]);
  const sidebarAlert = !!status?.guardrail_alert;
  useEffect(() => {
    setView((curr) => (curr === "projects" || curr === "tasks" ? defaultView : curr));
  }, [defaultView]);

  useEffect(() => {
    function handleNavigate(event: Event) {
      const detail = (event as CustomEvent<DashboardNavigateDetail>).detail;
      if (!detail?.view) return;
      if (detail.view === "settings" && !SHOW_SETTINGS_NAV) return;
      setView(detail.view);
    }
    window.addEventListener("borg:navigate", handleNavigate as EventListener);
    return () => window.removeEventListener("borg:navigate", handleNavigate as EventListener);
  }, []);

  const { isSWE } = useDashboardMode();

  const navLabelOverrides: Record<string, string> = useMemo(
    () => ({
      projects: vocab.projectsLabel,
      tasks: vocab.tasksLabel,
    }),
    [vocab],
  );

  const SWE_ONLY_KEYS = ["tasks", "queue", "proposals", "logs", "auto-tasks"];

  const navItems = useMemo(
    () =>
      ALL_NAV_ITEMS.filter((item) => {
        if (domain.hiddenNavKeys.includes(item.key)) return false;
        if (!SHOW_SETTINGS_NAV && item.key === "settings") return false;
        if (!isSWE && SWE_ONLY_KEYS.includes(item.key)) return false;
        return uiMode === "advanced" || item.minimalVisible;
      }),
    [uiMode, domain, isSWE, SWE_ONLY_KEYS.includes],
  );

  useEffect(() => {
    if (!SHOW_SETTINGS_NAV && view === "settings") {
      setView(defaultView);
    }
  }, [defaultView, view]);

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
                    mobileBottomTab === "proposals" ? "text-[#e8e0d4] border-b-2 border-amber-400" : "text-[#6b6459]"
                  }`}
                >
                  Proposals
                </button>
                <button
                  onClick={() => setMobileBottomTab("queue")}
                  className={`flex-1 py-2.5 text-[13px] font-medium transition-colors ${
                    mobileBottomTab === "queue" ? "text-[#e8e0d4] border-b-2 border-amber-400" : "text-[#6b6459]"
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
          {MOBILE_TABS.filter((t) => isSWE || t.key !== "queue").map(({ key, label: defaultLabel, Icon }) => {
            const label = navLabelOverrides[key] ?? defaultLabel;
            return (
              <button
                key={key}
                onClick={() => setMobileTab(key)}
                className={cn(
                  "flex flex-1 flex-col items-center gap-0.5 pt-2 pb-1.5 active:opacity-70 transition-colors",
                  mobileTab === key ? "text-amber-400" : "text-[#6b6459]",
                )}
              >
                <Icon className="h-5 w-5" strokeWidth={mobileTab === key ? 2 : 1.5} />
                <span className="text-[10px] font-medium">{label}</span>
              </button>
            );
          })}
        </nav>
      </div>
    );
  }

  // Desktop layout
  return (
    <div className="flex h-screen bg-[#0f0e0c] text-foreground antialiased">
      {/* Sidebar nav — 52px in flow, expands to 140px on hover overlaying content */}
      <div className="relative w-[52px] shrink-0">
        <nav
          className={cn(
            "group/nav absolute inset-y-0 left-0 z-40 flex w-[52px] hover:w-[160px] flex-col items-start border-r pb-4 overflow-hidden transition-[width] duration-100 ease-out",
            sidebarAlert
              ? "border-red-500/30 bg-red-950/35"
              : "border-[#2a2520] bg-gradient-to-b from-[#1c1a17] to-[#151412]",
          )}
        >
          <div className={cn("borg-logo mb-2 w-full shrink-0 h-14", domain.accentBg)}>
            <BorgLogo expanded />
            <div
              className="borg-logo-ghost grid grid-cols-2 grid-rows-2 group-hover/nav:grid-cols-4 group-hover/nav:grid-rows-1"
              aria-hidden
            >
              {PRODUCT_WORD.split("").map((c, i) => (
                <span key={i} className="flex items-center justify-center text-[22px]">
                  {c}
                </span>
              ))}
            </div>
          </div>

          <div className="flex flex-1 flex-col items-start gap-0.5 w-full px-2">
            {navItems.map(({ key, label: defaultLabel, Icon }) => {
              const label = navLabelOverrides[key] ?? defaultLabel;
              return (
                <button
                  key={key}
                  onClick={() => setView(key)}
                  title={label}
                  aria-label={label}
                  className={cn(
                    "group relative flex h-10 w-full items-center gap-3 rounded-xl px-[10px]",
                    view === key
                      ? sidebarAlert
                        ? "bg-red-400/20 text-red-50"
                        : "bg-amber-500/[0.08] text-[#e8e0d4]"
                      : sidebarAlert
                        ? "text-red-200/80 hover:bg-red-400/15 hover:text-red-50"
                        : "text-[#6b6459] hover:bg-amber-500/[0.05] hover:text-[#9c9486]",
                  )}
                >
                  <Icon className="h-[18px] w-[18px] shrink-0" strokeWidth={view === key ? 2 : 1.5} />
                  <span className="truncate text-[13px] font-medium opacity-0 group-hover/nav:opacity-100 transition-opacity duration-200">
                    {label}
                  </span>
                  {view === key && (
                    <div
                      className={cn(
                        "absolute left-0 top-1/2 -translate-y-1/2 h-5 w-[3px] rounded-r-full",
                        sidebarAlert ? "bg-red-300" : "bg-amber-400",
                      )}
                    />
                  )}
                </button>
              );
            })}
          </div>

          {/* Status indicator at bottom */}
          <div className="mt-auto flex w-full flex-col items-center gap-3 shrink-0 px-2">
            {(status?.dispatched_agents ?? 0) > 0 && (
              <div
                className="flex h-6 w-6 items-center justify-center rounded-full bg-amber-500/15 ring-1 ring-amber-500/20"
                title={`${status?.dispatched_agents} active agent(s)`}
              >
                <span className="text-[11px] font-bold tabular-nums text-amber-400">{status?.dispatched_agents}</span>
              </div>
            )}
            <div
              className="flex w-full items-center gap-3 px-[10px] py-1"
              title={connected ? "Server connected" : "Server disconnected"}
            >
              <div className="flex h-[18px] w-[18px] shrink-0 items-center justify-center">
                <div
                  className={cn(
                    "h-2.5 w-2.5 rounded-full",
                    connected
                      ? "bg-emerald-500 shadow-[0_0_8px_rgba(200,160,80,0.3)]"
                      : "bg-red-500 shadow-[0_0_8px_rgba(239,68,68,0.3)]",
                  )}
                />
              </div>
              <span className="truncate text-[11px] text-[#6b6459] opacity-0 group-hover/nav:opacity-100 transition-opacity duration-200">
                {connected ? "Connected" : "Offline"}
              </span>
            </div>
          </div>
        </nav>
      </div>

      {/* Main content */}
      <div className="flex min-w-0 flex-1 flex-col">
        {isSWE && (
          <Header connected={connected} view={view} repoFilter={repoFilter} onRepoFilterChange={setRepoFilter} />
        )}

        <div className="min-h-0 flex-1 flex overflow-hidden">
          <div className="min-w-0 flex-1 overflow-hidden" style={{ contain: "strict" }}>
            {view === "tasks" && (
              <div className="flex h-full">
                <div className="w-[320px] shrink-0 overflow-hidden border-r border-[#2a2520]">
                  <TaskList selectedId={selectedTaskId} onSelect={handleSelectTask} repoFilter={repoFilter} />
                </div>
                <div className="min-w-0 flex-1 overflow-hidden">
                  {selectedTaskId !== null ? (
                    <TaskDetail taskId={selectedTaskId} onBack={handleBackFromTask} />
                  ) : (
                    <EmptyState status={status} isSWE={isSWE} />
                  )}
                </div>
              </div>
            )}

            {view === "projects" && <ProjectsPanel />}
            {view === "connections" && <ConnectionsPanel />}
            {view === "creator" && <ModeCreatorPanel />}
            {view === "auto-tasks" && <AutoTasksPanel />}
            {view === "proposals" && <ProposalsPanel repoFilter={repoFilter} />}
            {view === "status" && <StatusPanel />}
            {view === "logs" && <LogViewer logs={logs} />}
            {view === "queue" && <QueuePanel repoFilter={repoFilter} />}
            {view === "settings" && SHOW_SETTINGS_NAV && <SettingsPanel />}
          </div>

          <ChatDrawer view={view} />
        </div>
      </div>
    </div>
  );
}

function EmptyState({
  status,
  isSWE,
}: {
  status?: {
    active_tasks: number;
    merged_tasks: number;
    ai_requests: number;
    failed_tasks: number;
    total_tasks: number;
  } | null;
  isSWE?: boolean;
}) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-8 text-center">
      <div className="flex h-20 w-20 items-center justify-center rounded-3xl bg-gradient-to-br from-amber-500/[0.04] to-amber-500/[0.02] ring-1 ring-amber-900/15">
        <ListTodo className="h-8 w-8 text-[#6b6459]" strokeWidth={1.5} />
      </div>
      <div>
        <p className="text-[15px] font-medium text-[#9c9486]">Select a task to view details</p>
        <p className="mt-2 text-[13px] text-[#6b6459]">or create a new one from the header</p>
      </div>
      {status && isSWE && (
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
