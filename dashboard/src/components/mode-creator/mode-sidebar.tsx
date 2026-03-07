import { useMemo, useState } from "react";
import type { PipelineModeFull } from "@/lib/types";
import { cn } from "@/lib/utils";
import { Search, Plus } from "lucide-react";

export function ModeSidebar({
  builtIn,
  custom,
  allowExperimental,
  activeName,
  onSelect,
  onNew,
  onDelete,
}: {
  builtIn: PipelineModeFull[];
  custom: PipelineModeFull[];
  allowExperimental: boolean;
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
    <div className="flex h-full w-[260px] shrink-0 flex-col border-r border-[#2a2520] bg-[#0f0e0c]">
      <div className="p-4 pb-3">
        <h3 className="text-[13px] font-semibold text-[#e8e0d4]">Pipeline Modes</h3>
        <div className="relative mt-3">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-[#6b6459]" />
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search modes..."
            className="w-full rounded-lg border border-[#2a2520] bg-[#151412] py-2 pl-9 pr-3 text-[13px] text-[#e8e0d4] placeholder-[#6b6459] outline-none transition-colors focus:border-amber-500/30"
          />
        </div>
      </div>

      <div className="flex-1 overflow-y-auto px-3 pb-3">
        {categoryGroups.map(({ category, modes }) => (
          <div key={category} className="mb-3">
            <button
              onClick={() => toggle(category)}
              className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-[#1c1a17]"
            >
              <span className="text-[10px] text-[#6b6459]">
                {collapsed[category] ? "\u25B6" : "\u25BC"}
              </span>
              <span className="flex-1 text-[11px] font-semibold uppercase tracking-wider text-[#6b6459]">
                {category}
              </span>
              <span className="text-[11px] text-[#3d3830]">{modes.length}</span>
            </button>
            {!collapsed[category] &&
              modes.map((m) => (
                <button
                  key={m.name}
                  onClick={() => onSelect(m, true)}
                  className={cn(
                    "flex w-full items-center gap-2 rounded-lg px-3 py-2 pl-7 text-left transition-colors",
                    activeName === m.name
                      ? "bg-amber-500/[0.08] text-[#e8e0d4] ring-1 ring-inset ring-amber-500/20"
                      : "text-[#9c9486] hover:bg-[#1c1a17] hover:text-[#e8e0d4]"
                  )}
                >
                  <span className="flex-1 truncate text-[13px]">{m.label}</span>
                  <span className="shrink-0 text-[11px] text-[#6b6459]">
                    {m.phases.length}p
                  </span>
                </button>
              ))}
          </div>
        ))}

        {filteredCustom.length > 0 && (
          <div className="mb-3">
            <button
              onClick={() => toggle("__custom__")}
              className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-[#1c1a17]"
            >
              <span className="text-[10px] text-[#6b6459]">
                {collapsed["__custom__"] ? "\u25B6" : "\u25BC"}
              </span>
              <span className="flex-1 text-[11px] font-semibold uppercase tracking-wider text-[#6b6459]">
                Custom
              </span>
              <span className="text-[11px] text-[#3d3830]">{filteredCustom.length}</span>
            </button>
            {!collapsed["__custom__"] &&
              filteredCustom.map((m) => (
                <div
                  key={m.name}
                  className={cn(
                    "group flex items-center gap-1 rounded-lg transition-colors",
                    activeName === m.name
                      ? "bg-amber-500/[0.08] ring-1 ring-inset ring-amber-500/20"
                      : "hover:bg-[#1c1a17]"
                  )}
                >
                  <button
                    onClick={() => onSelect(m, false)}
                    className={cn(
                      "flex flex-1 items-center gap-2 px-3 py-2 pl-7 text-left",
                      activeName === m.name ? "text-[#e8e0d4]" : "text-[#9c9486]"
                    )}
                  >
                    <span className="flex-1 truncate text-[13px]">
                      {m.label || m.name}
                    </span>
                    <span className="shrink-0 text-[11px] text-[#6b6459]">
                      {m.phases.length}p
                    </span>
                  </button>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      onDelete(m.name);
                    }}
                    aria-label={`Delete mode ${m.label || m.name}`}
                    className="mr-2 hidden rounded-lg px-1.5 py-1 text-[11px] text-[#6b6459] hover:bg-red-500/15 hover:text-red-400 group-hover:block"
                  >
                    &times;
                  </button>
                </div>
              ))}
          </div>
        )}
      </div>

      <div className="border-t border-[#2a2520] p-3">
        <button
          onClick={onNew}
          className="flex w-full items-center justify-center gap-2 rounded-lg bg-amber-500/15 px-3 py-2.5 text-[13px] font-medium text-amber-300 ring-1 ring-inset ring-amber-500/20 transition-colors hover:bg-amber-500/20"
        >
          <Plus className="h-4 w-4" />
          New Mode
        </button>
        {!allowExperimental && (
          <p className="mt-2 text-center text-[11px] text-[#6b6459]">
            Enable Experimental Domains in Settings for more modes.
          </p>
        )}
      </div>
    </div>
  );
}
