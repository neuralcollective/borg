import { useStatus } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Circle } from "lucide-react";

function formatUptime(seconds: number) {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  return `${h}h ${m}m ${s}s`;
}

export function Header({ connected }: { connected: boolean }) {
  const { data: status } = useStatus();

  return (
    <header className="flex items-center gap-4 border-b border-border bg-card px-5 py-3">
      <h1 className="text-sm font-bold tracking-wider text-primary">BORG</h1>

      <div className="h-5 w-px bg-border" />

      {status?.continuous_mode ? (
        <Badge variant="outline" className="border-green-800 bg-green-950/50 text-green-400 text-xs">
          CONTINUOUS
        </Badge>
      ) : (
        <Badge variant="outline" className="border-amber-800 bg-amber-950/50 text-amber-400 text-xs">
          EVERY {status?.release_interval_mins ?? "?"}M
        </Badge>
      )}

      <div className="h-5 w-px bg-border" />

      <span className="text-xs text-muted-foreground">
        uptime{" "}
        <span className="text-foreground">{status ? formatUptime(status.uptime_s) : "--"}</span>
      </span>

      <div className="h-5 w-px bg-border" />

      <span className="text-xs text-muted-foreground">
        model <span className="text-foreground">{status?.model ?? "--"}</span>
      </span>

      <div className="ml-auto flex items-center gap-2">
        <Circle
          className={connected ? "fill-green-500 text-green-500" : "fill-red-500 text-red-500"}
          size={8}
        />
        <span className="text-[10px] text-muted-foreground">{connected ? "live" : "disconnected"}</span>
      </div>
    </header>
  );
}
