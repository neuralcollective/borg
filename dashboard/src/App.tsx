import { useState, useEffect, useCallback } from "react";
import { useLogs } from "@/lib/api";
import { Header } from "@/components/header";
import { StatsBar } from "@/components/stats-bar";
import { TaskList } from "@/components/task-list";
import { TaskDetail } from "@/components/task-detail";
import { LogViewer } from "@/components/log-viewer";
import { QueuePanel } from "@/components/queue-panel";
import { ProposalsPanel } from "@/components/proposals-panel";
import { ChatPanel } from "@/components/chat-panel";
import { ListTodo, Terminal, GitMerge, MessageSquare } from "lucide-react";
import { cn } from "@/lib/utils";

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

const MOBILE_TABS = [
  { key: "tasks" as const, label: "Tasks", Icon: ListTodo },
  { key: "logs" as const, label: "Logs", Icon: Terminal },
  { key: "queue" as const, label: "Queue", Icon: GitMerge },
  { key: "chat" as const, label: "Chat", Icon: MessageSquare },
] as const;

export default function App() {
  const [selectedTaskId, setSelectedTaskId] = useState<number | null>(null);
  const [chatOpen, setChatOpen] = useState(true);
  const [repoFilter, setRepoFilter] = useState<string | null>(null);
  const [bottomTab, setBottomTab] = useState<"queue" | "proposals">("proposals");
  const [mobileTab, setMobileTab] = useState<MobileTab>("tasks");
  const { logs, connected } = useLogs();
  const isMobile = useIsMobile();

  const handleSelectTask = useCallback((id: number) => setSelectedTaskId(id), []);
  const handleBackFromTask = useCallback(() => setSelectedTaskId(null), []);

  if (isMobile) {
    return (
      <div className="flex flex-col bg-[#0a0a0a] text-foreground antialiased" style={{ height: "100dvh" }}>
        <Header connected={connected} mobile />

        <div className="min-h-0 flex-1 flex flex-col overflow-hidden">
          {mobileTab === "tasks" && (
            <>
              <StatsBar repoFilter={repoFilter} onRepoFilterChange={setRepoFilter} />
              <div className="min-h-0 flex-1 overflow-hidden">
                {selectedTaskId !== null ? (
                  <TaskDetail taskId={selectedTaskId} onBack={handleBackFromTask} />
                ) : (
                  <TaskList selectedId={selectedTaskId} onSelect={handleSelectTask} repoFilter={repoFilter} />
                )}
              </div>
            </>
          )}

          {mobileTab === "logs" && <LogViewer logs={logs} />}

          {mobileTab === "queue" && (
            <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
              <div className="flex shrink-0 border-b border-white/[0.06]">
                <button
                  onClick={() => setBottomTab("proposals")}
                  className={`flex-1 py-2.5 text-[13px] font-medium transition-colors ${
                    bottomTab === "proposals"
                      ? "text-zinc-200 border-b-2 border-blue-400"
                      : "text-zinc-500"
                  }`}
                >
                  Proposals
                </button>
                <button
                  onClick={() => setBottomTab("queue")}
                  className={`flex-1 py-2.5 text-[13px] font-medium transition-colors ${
                    bottomTab === "queue"
                      ? "text-zinc-200 border-b-2 border-blue-400"
                      : "text-zinc-500"
                  }`}
                >
                  Queue
                </button>
              </div>
              <div className="min-h-0 flex-1 overflow-hidden">
                {bottomTab === "queue" ? (
                  <QueuePanel repoFilter={repoFilter} />
                ) : (
                  <ProposalsPanel repoFilter={repoFilter} />
                )}
              </div>
            </div>
          )}

          {mobileTab === "chat" && <ChatPanel />}
        </div>

        {/* Bottom tab bar */}
        <nav
          className="flex shrink-0 border-t border-white/[0.06] bg-[#0a0a0a]"
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
    <div className="flex h-screen flex-col bg-[#0a0a0a] text-foreground antialiased">
      <Header connected={connected} onToggleChat={() => setChatOpen((v) => !v)} chatOpen={chatOpen} />
      <StatsBar repoFilter={repoFilter} onRepoFilterChange={setRepoFilter} />
      <div
        className={`grid min-h-0 flex-1 ${
          chatOpen
            ? "grid-cols-[1fr_1fr_350px]"
            : "grid-cols-[1fr_1fr]"
        } grid-rows-[1fr_auto]`}
      >
        {/* Left: Logs */}
        <div className="row-span-2 overflow-hidden border-r border-white/[0.06]">
          <LogViewer logs={logs} />
        </div>

        {/* Center top: Task list or detail */}
        <div className="overflow-hidden border-b border-white/[0.06]">
          {selectedTaskId !== null ? (
            <TaskDetail taskId={selectedTaskId} onBack={handleBackFromTask} />
          ) : (
            <TaskList selectedId={selectedTaskId} onSelect={handleSelectTask} repoFilter={repoFilter} />
          )}
        </div>

        {/* Center bottom: Queue / Proposals */}
        <div className="flex flex-col overflow-hidden">
          <div className="flex shrink-0 border-b border-white/[0.06]">
            <button
              onClick={() => setBottomTab("proposals")}
              className={`px-4 py-1.5 text-[11px] font-medium transition-colors ${
                bottomTab === "proposals"
                  ? "text-zinc-200 border-b border-zinc-200"
                  : "text-zinc-500 hover:text-zinc-400"
              }`}
            >
              Proposals
            </button>
            <button
              onClick={() => setBottomTab("queue")}
              className={`px-4 py-1.5 text-[11px] font-medium transition-colors ${
                bottomTab === "queue"
                  ? "text-zinc-200 border-b border-zinc-200"
                  : "text-zinc-500 hover:text-zinc-400"
              }`}
            >
              Queue
            </button>
          </div>
          <div className="flex-1 overflow-hidden">
            {bottomTab === "queue" ? (
              <QueuePanel repoFilter={repoFilter} />
            ) : (
              <ProposalsPanel repoFilter={repoFilter} />
            )}
          </div>
        </div>

        {/* Right: Chat (when open) */}
        {chatOpen && (
          <div className="row-span-2 overflow-hidden border-l border-white/[0.06]">
            <ChatPanel />
          </div>
        )}
      </div>
    </div>
  );
}
