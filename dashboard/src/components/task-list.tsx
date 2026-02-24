import { useTasks } from "@/lib/api";
import { isActiveStatus } from "@/lib/types";
import { StatusBadge } from "./status-badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";

interface TaskListProps {
  selectedId: number | null;
  onSelect: (id: number) => void;
}

export function TaskList({ selectedId, onSelect }: TaskListProps) {
  const { data: tasks } = useTasks();

  const active = tasks?.filter((t) => isActiveStatus(t.status)) ?? [];
  const done = tasks?.filter((t) => !isActiveStatus(t.status)) ?? [];

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-border bg-card px-4 py-2">
        <span className="text-[10px] uppercase tracking-widest text-muted-foreground">Pipeline Tasks</span>
        <span className="text-[10px] text-muted-foreground">{tasks?.length ?? 0}</span>
      </div>
      <ScrollArea className="flex-1">
        <div className="p-1">
          {active.map((t) => (
            <TaskRow key={t.id} task={t} isActive selected={selectedId === t.id} onClick={() => onSelect(t.id)} />
          ))}
          {done.slice(0, 20).map((t) => (
            <TaskRow key={t.id} task={t} selected={selectedId === t.id} onClick={() => onSelect(t.id)} />
          ))}
          {!tasks?.length && <p className="py-8 text-center text-xs text-muted-foreground">No tasks yet</p>}
        </div>
      </ScrollArea>
    </div>
  );
}

function TaskRow({
  task,
  isActive,
  selected,
  onClick,
}: {
  task: { id: number; title: string; status: string; attempt: number; max_attempts: number };
  isActive?: boolean;
  selected?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex w-full items-center gap-2 rounded-md px-3 py-2 text-left transition-colors hover:bg-accent",
        isActive && "border-l-2 border-blue-400 bg-blue-950/20",
        selected && "bg-accent"
      )}
    >
      <span className="min-w-[28px] text-[11px] text-muted-foreground">#{task.id}</span>
      <StatusBadge status={task.status} />
      <span className="flex-1 truncate text-xs text-foreground">{task.title}</span>
      {task.attempt > 0 && (
        <span className="text-[10px] text-muted-foreground">
          {task.attempt}/{task.max_attempts}
        </span>
      )}
    </button>
  );
}
