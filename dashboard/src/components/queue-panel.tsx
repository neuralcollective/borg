import { useQueue } from "@/lib/api";
import { StatusBadge } from "./status-badge";
import { ScrollArea } from "@/components/ui/scroll-area";

export function QueuePanel() {
  const { data: queue } = useQueue();

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-border bg-card px-4 py-2">
        <span className="text-[10px] uppercase tracking-widest text-muted-foreground">Integration Queue</span>
        <span className="text-[10px] text-muted-foreground">{queue?.length ?? 0}</span>
      </div>
      <ScrollArea className="flex-1">
        <div className="p-2">
          {queue?.map((e) => (
            <div key={e.id} className="flex items-center gap-2 px-2 py-1.5 text-xs">
              <StatusBadge status={e.status} />
              <span className="flex-1 truncate text-foreground">{e.branch}</span>
              <span className="text-[10px] text-muted-foreground">#{e.task_id}</span>
            </div>
          ))}
          {!queue?.length && <p className="py-6 text-center text-xs text-muted-foreground">Queue empty</p>}
        </div>
      </ScrollArea>
    </div>
  );
}
