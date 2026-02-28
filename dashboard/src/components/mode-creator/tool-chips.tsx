import { cn } from "@/lib/utils";

const KNOWN_TOOLS = ["Read", "Glob", "Grep", "Write", "Edit", "Bash", "WebSearch", "WebFetch"];

export function ToolChips({
  value,
  onChange,
  disabled,
}: {
  value: string;
  onChange: (v: string) => void;
  disabled?: boolean;
}) {
  const active = new Set(value.split(",").map((t) => t.trim()).filter(Boolean));

  // Include any unknown tools from the current value
  const allTools = [...KNOWN_TOOLS];
  for (const t of active) {
    if (!allTools.includes(t)) allTools.push(t);
  }

  function toggle(tool: string) {
    if (disabled) return;
    const next = new Set(active);
    if (next.has(tool)) next.delete(tool);
    else next.add(tool);
    // Preserve KNOWN_TOOLS ordering
    const ordered = allTools.filter((t) => next.has(t));
    onChange(ordered.join(","));
  }

  return (
    <div className="flex flex-wrap gap-1">
      {allTools.map((tool) => (
        <button
          key={tool}
          type="button"
          disabled={disabled}
          onClick={() => toggle(tool)}
          className={cn(
            "rounded-md px-2 py-0.5 text-[11px] transition-colors",
            active.has(tool)
              ? "bg-blue-500/15 text-blue-400 ring-1 ring-inset ring-blue-500/20"
              : "bg-white/[0.04] text-zinc-600 hover:bg-white/[0.08] hover:text-zinc-400",
            disabled && "cursor-not-allowed opacity-50"
          )}
        >
          {tool}
        </button>
      ))}
    </div>
  );
}
