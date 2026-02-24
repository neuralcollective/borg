import { useTaskDetail } from "@/lib/api";
import { PhaseTracker } from "./phase-tracker";
import { StatusBadge } from "./status-badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { ScrollArea } from "@/components/ui/scroll-area";
import { ArrowLeft, GitBranch, User, Clock, AlertTriangle, FolderGit2 } from "lucide-react";
import { repoName } from "@/lib/types";

interface TaskDetailProps {
  taskId: number;
  onBack: () => void;
}

export function TaskDetail({ taskId, onBack }: TaskDetailProps) {
  const { data: task, isLoading } = useTaskDetail(taskId);

  if (isLoading || !task) {
    return (
      <div className="flex h-full flex-col">
        <DetailHeader onBack={onBack} />
        <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">Loading...</div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <DetailHeader onBack={onBack} />

      {/* Task info */}
      <div className="space-y-3 border-b border-border px-4 py-3">
        <div className="flex items-start justify-between gap-4">
          <h2 className="text-sm font-medium text-foreground">
            <span className="text-muted-foreground">#{task.id}</span> {task.title}
          </h2>
          <StatusBadge status={task.status} />
        </div>

        <PhaseTracker status={task.status} />

        <div className="flex flex-wrap gap-3 text-[11px] text-muted-foreground">
          {task.repo_path && (
            <span className="flex items-center gap-1" title={task.repo_path}>
              <FolderGit2 size={11} /> {repoName(task.repo_path)}
            </span>
          )}
          {task.branch && (
            <span className="flex items-center gap-1">
              <GitBranch size={11} /> {task.branch}
            </span>
          )}
          {task.attempt > 0 && (
            <span>
              attempt {task.attempt}/{task.max_attempts}
            </span>
          )}
          <span className="flex items-center gap-1">
            <User size={11} /> {task.created_by || "pipeline"}
          </span>
          <span className="flex items-center gap-1">
            <Clock size={11} /> {task.created_at}
          </span>
        </div>
      </div>

      {/* Description */}
      {task.description && (
        <div className="max-h-16 overflow-y-auto border-b border-border px-4 py-2 text-xs text-muted-foreground">
          {task.description}
        </div>
      )}

      {/* Error */}
      {task.last_error && (
        <Card className="mx-3 mt-2 border-red-900 bg-red-950/30">
          <CardContent className="flex items-start gap-2 p-3">
            <AlertTriangle size={14} className="mt-0.5 shrink-0 text-red-400" />
            <pre className="max-h-20 overflow-y-auto whitespace-pre-wrap text-[11px] text-red-400">
              {task.last_error}
            </pre>
          </CardContent>
        </Card>
      )}

      {/* Agent outputs */}
      {task.outputs && task.outputs.length > 0 ? (
        <Tabs defaultValue={task.outputs[0].phase + "-" + task.outputs[0].id} className="flex min-h-0 flex-1 flex-col">
          <TabsList className="h-auto w-full justify-start rounded-none border-b border-border bg-card p-0">
            {task.outputs.map((o) => (
              <TabsTrigger
                key={o.id}
                value={o.phase + "-" + o.id}
                className="rounded-none border-b-2 border-transparent px-4 py-2 text-[11px] uppercase tracking-wide data-[state=active]:border-blue-400 data-[state=active]:text-blue-400"
              >
                {o.phase}
                {o.exit_code === 0 ? (
                  <span className="ml-1 text-green-400">ok</span>
                ) : (
                  <span className="ml-1 text-red-400">x{o.exit_code}</span>
                )}
              </TabsTrigger>
            ))}
          </TabsList>
          {task.outputs.map((o) => (
            <TabsContent key={o.id} value={o.phase + "-" + o.id} className="mt-0 flex-1 data-[state=inactive]:hidden">
              <ScrollArea className="h-full">
                <pre className="p-4 text-[11px] leading-relaxed text-muted-foreground">{o.output}</pre>
              </ScrollArea>
            </TabsContent>
          ))}
        </Tabs>
      ) : (
        <div className="flex flex-1 items-center justify-center text-xs text-muted-foreground">
          No agent outputs recorded yet
        </div>
      )}
    </div>
  );
}

function DetailHeader({ onBack }: { onBack: () => void }) {
  return (
    <div className="flex items-center gap-3 border-b border-border bg-card px-4 py-2">
      <Button variant="outline" size="sm" className="h-7 px-2 text-xs" onClick={onBack}>
        <ArrowLeft size={12} className="mr-1" /> Back
      </Button>
      <span className="text-[10px] uppercase tracking-widest text-muted-foreground">Task Detail</span>
    </div>
  );
}
