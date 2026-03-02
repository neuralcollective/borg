import { useMemo, useState } from "react";
import type { PipelineModeFull } from "@/lib/types";
import { cn } from "@/lib/utils";

export function ModeSidebar({
  builtIn,
  custom,
  activeName,
  onSelect,
  onNew,
  onDelete,
}: {
  builtIn: PipelineModeFull[];
  custom: PipelineModeFull[];
  activeName: string;
  onSelect: (mode: PipelineModeFull, readOnly: boolean) => void;
  onNew: () => void;
  onDelete: (name: string) => void;
}) {
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [search, setSearch] = useState("");

  const filteredBuiltIn = useMemo(() => {
    if (!search.trim()) return builtIn;
    const q = search.toLowerCase();
    return builtIn.filter(
      (m) =>
        m.label.toLowerCase().includes(q) ||
        m.name.toLowerCase().includes(q) ||
        (m.category || "").toLowerCase().includes(q)
    );
  }, [builtIn, search]);

  const filteredCustom = useMemo(() => {
    if (!search.trim()) return custom;
    const q = search.toLowerCase();
    return custom.filter(
      (m) =>
        (m.label || m.name).toLowerCase().includes(q) ||
        m.name.toLowerCase().includes(q)
    );
  }, [custom, search]);

  const categoryGroups = useMemo(() => {
    const groups: { category: string; modes: PipelineModeFull[] }[] = [];
    const seen = new Map<string, number>();
    for (const m of filteredBuiltIn) {
      const cat = m.category || "Other";
      if (seen.has(cat)) {
        groups[seen.get(cat)!].modes.push(m);
      } else {
        seen.set(cat, groups.length);
        groups.push({ category: cat, modes: [m] });
      }
    }
    return groups;
  }, [filteredBuiltIn]);

  const toggle = (cat: string) =>
    setCollapsed((prev) => ({ ...prev, [cat]: !prev[cat] }));

  return (
    <div className="flex h-full w-[240px] shrink-0 flex-col border-r border-white/[0.06]">
      <div className="p-3 pb-2">
        <div className="text-[11px] font-semibold uppercase tracking-wider text-zinc-500">
          Borg Creator
        </div>
        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search modes..."
          className="mt-2 w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-2 py-1 text-[11px] text-zinc-300 placeholder-zinc-600 outline-none focus:border-blue-500/40"
        />
      </div>

      <div className="flex-1 overflow-y-auto px-2 pb-2">
        {categoryGroups.map(({ category, modes }) => (
          <div key={category} className="mb-2">
            <button
              onClick={() => toggle(category)}
              className="flex w-full items-center gap-1.5 rounded-md px-1 py-1 text-left hover:bg-white/[0.03]"
            >
              <span className="text-[9px] text-zinc-600">
                {collapsed[category] ? "\u25B6" : "\u25BC"}
              </span>
              <span className="flex-1 text-[10px] font-medium uppercase tracking-wider text-zinc-500">
                {category}
              </span>
              <span className="text-[9px] text-zinc-700">{modes.length}</span>
            </button>
            {!collapsed[category] &&
              modes.map((m) => (
                <button
                  key={m.name}
                  onClick={() => onSelect(m, true)}
                  className={cn(
                    "flex w-full items-center gap-2 rounded-md px-2 py-1.5 pl-5 text-left transition-colors",
                    activeName === m.name
                      ? "bg-white/[0.08] text-zinc-200"
                      : "text-zinc-500 hover:bg-white/[0.04] hover:text-zinc-300"
                  )}
                >
                  <span className="flex-1 truncate text-[12px]">{m.label}</span>
                  <span className="shrink-0 text-[10px] text-zinc-600">
                    {m.phases.length}p
                  </span>
                </button>
              ))}
          </div>
        ))}

        {filteredCustom.length > 0 && (
          <div className="mb-2">
            <button
              onClick={() => toggle("__custom__")}
              className="flex w-full items-center gap-1.5 rounded-md px-1 py-1 text-left hover:bg-white/[0.03]"
            >
              <span className="text-[9px] text-zinc-600">
                {collapsed["__custom__"] ? "\u25B6" : "\u25BC"}
              </span>
              <span className="flex-1 text-[10px] font-medium uppercase tracking-wider text-zinc-500">
                Custom
              </span>
              <span className="text-[9px] text-zinc-700">{filteredCustom.length}</span>
            </button>
            {!collapsed["__custom__"] &&
              filteredCustom.map((m) => (
                <div
                  key={m.name}
                  className={cn(
                    "group flex items-center gap-1 rounded-md transition-colors",
                    activeName === m.name
                      ? "bg-white/[0.08]"
                      : "hover:bg-white/[0.04]"
                  )}
                >
                  <button
                    onClick={() => onSelect(m, false)}
                    className={cn(
                      "flex flex-1 items-center gap-2 px-2 py-1.5 pl-5 text-left",
                      activeName === m.name ? "text-zinc-200" : "text-zinc-400"
                    )}
                  >
                    <span className="flex-1 truncate text-[12px]">
                      {m.label || m.name}
                    </span>
                    <span className="shrink-0 text-[10px] text-zinc-600">
                      {m.phases.length}p
                    </span>
                  </button>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      onDelete(m.name);
                    }}
                    aria-label={`Delete mode ${m.label || m.name}`}
                    className="mr-1 hidden rounded px-1 py-0.5 text-[10px] text-zinc-600 hover:bg-red-500/20 hover:text-red-400 group-hover:block"
                  >
                    &times;
                  </button>
                </div>
              ))}
          </div>
        )}
      </div>

      <div className="border-t border-white/[0.06] p-2">
        <button
          onClick={onNew}
          className="w-full rounded-md bg-white/[0.06] px-2 py-1.5 text-[12px] text-zinc-300 hover:bg-white/[0.1]"
        >
          + New Mode
        </button>
      </div>
    </div>
  );
}
