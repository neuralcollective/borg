import { cn } from "@/lib/utils";

const DEFAULT_TOOLS = ["Read", "Glob", "Grep", "Write", "Edit", "Bash", "WebSearch", "WebFetch"];

export function ToolChips({
  value,
  onChange,
  disabled,
  visibleTools,
}: {
  value: string;
  onChange: (v: string) => void;
  disabled?: boolean;
  visibleTools?: string[];
}) {
  const active = new Set(value.split(",").map((t) => t.trim()).filter(Boolean));

  const knownTools = visibleTools ?? DEFAULT_TOOLS;
  const allTools = [...knownTools];
  for (const t of active) {
    if (!allTools.includes(t)) allTools.push(t);
  }

  function toggle(tool: string) {
    if (disabled) return;
    const next = new Set(active);
    if (next.has(tool)) next.delete(tool);
    else next.add(tool);
    const ordered = allTools.filter((t) => next.has(t));
    onChange(ordered.join(","));
  }

  return (
    <div className="flex flex-wrap gap-1.5">
      {allTools.map((tool) => (
        <button
          key={tool}
          type="button"
          disabled={disabled}
          onClick={() => toggle(tool)}
          className={cn(
            "rounded-lg px-2.5 py-1 text-[12px] transition-colors",
            active.has(tool)
              ? "bg-amber-500/15 text-amber-300 ring-1 ring-inset ring-amber-500/20"
              : "bg-[#1c1a17] text-[#6b6459] hover:bg-[#232019] hover:text-[#9c9486]",
            disabled && "cursor-not-allowed opacity-50"
          )}
        >
          {tool}
        </button>
      ))}
    </div>
  );
}
