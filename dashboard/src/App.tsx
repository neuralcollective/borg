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
    <div className="flex h-screen flex-col bg-background text-foreground">
      <Header connected={connected} />
      <StatsBar />
      <div className="grid min-h-0 flex-1 grid-cols-2 gap-px bg-border">
        {/* Left: Logs */}
        <div className="row-span-2 bg-background">
          <LogViewer logs={logs} />
        </div>

        {/* Right top: Task list or detail */}
        <div className="bg-background">
          {selectedTaskId !== null ? (
            <TaskDetail taskId={selectedTaskId} onBack={() => setSelectedTaskId(null)} />
          ) : (
            <TaskList selectedId={selectedTaskId} onSelect={setSelectedTaskId} />
          )}
        </div>

        {/* Right bottom: Queue */}
        <div className="bg-background">
          <QueuePanel />
        </div>
      </div>
    </div>
  );
}
