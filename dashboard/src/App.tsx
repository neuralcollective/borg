import { useState } from "react";
import { useLogs } from "@/lib/api";
import { Header } from "@/components/header";
import { StatsBar } from "@/components/stats-bar";
import { TaskList } from "@/components/task-list";
import { TaskDetail } from "@/components/task-detail";
import { LogViewer } from "@/components/log-viewer";
import { QueuePanel } from "@/components/queue-panel";

export default function App() {
  const [selectedTaskId, setSelectedTaskId] = useState<number | null>(null);
  const { logs, connected } = useLogs();

  return (
    <div className="flex h-screen flex-col bg-[#0a0a0a] text-foreground antialiased">
      <Header connected={connected} />
      <StatsBar />
      <div className="grid min-h-0 flex-1 grid-cols-[1fr_1fr] grid-rows-[1fr_auto]">
        {/* Left: Logs */}
        <div className="row-span-2 overflow-hidden border-r border-white/[0.06]">
          <LogViewer logs={logs} />
        </div>

        {/* Right top: Task list or detail */}
        <div className="overflow-hidden border-b border-white/[0.06]">
          {selectedTaskId !== null ? (
            <TaskDetail taskId={selectedTaskId} onBack={() => setSelectedTaskId(null)} />
          ) : (
            <TaskList selectedId={selectedTaskId} onSelect={setSelectedTaskId} />
          )}
        </div>

        {/* Right bottom: Queue */}
        <div className="overflow-hidden">
          <QueuePanel />
        </div>
      </div>
    </div>
  );
}
