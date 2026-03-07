import type { SeedConfigFull, SeedOutputType } from "@/lib/types";
import { AutoTextarea } from "./auto-textarea";
import { ToolChips } from "./tool-chips";
import { cn } from "@/lib/utils";

export function SeedList({
  seeds,
  expandedIndex,
  readOnly,
  onExpand,
  onUpdate,
  onAdd,
  onRemove,
}: {
  seeds: SeedConfigFull[];
  expandedIndex: number | null;
  readOnly: boolean;
  onExpand: (index: number | null) => void;
  onUpdate: (index: number, patch: Partial<SeedConfigFull>) => void;
  onAdd: () => void;
  onRemove: (index: number) => void;
}) {
  return (
    <div className="space-y-3">
      {seeds.length === 0 && (
        <div className="flex flex-col items-center rounded-xl border-2 border-dashed border-[#2a2520] py-12 text-center">
          <p className="text-[14px] text-[#9c9486]">No seed modes configured</p>
          <p className="mt-1 text-[12px] text-[#6b6459]">Seeds auto-generate tasks when the pipeline is idle.</p>
        </div>
      )}

      {seeds.map((seed, i) => {
        const expanded = i === expandedIndex;
        return (
          <div
            key={`${seed.name}-${i}`}
            className={cn(
              "rounded-xl border transition-colors",
              expanded
                ? "border-amber-500/30 bg-amber-500/[0.03]"
                : "border-[#2a2520] bg-[#151412] hover:border-amber-900/30"
            )}
          >
            {/* Summary row */}
            <button
              onClick={() => onExpand(expanded ? null : i)}
              className="flex w-full items-center gap-3 px-4 py-3 text-left"
            >
              <span className="text-[10px] text-[#6b6459]">
                {expanded ? "\u25BC" : "\u25B6"}
              </span>
              <span className={cn(
                "min-w-[80px] text-[13px] font-medium",
                seed.name ? "text-[#e8e0d4]" : "text-[#6b6459]"
              )}>
                {seed.name || "unnamed"}
              </span>
              <span className="flex-1 truncate text-[12px] text-[#6b6459]">
                {seed.label || seed.prompt.slice(0, 60) || "\u2014"}
              </span>
              <span className={cn(
                "rounded-lg px-2 py-0.5 text-[10px] font-medium",
                seed.output_type === "task"
                  ? "bg-amber-500/15 text-amber-300"
                  : "bg-violet-500/15 text-violet-300"
              )}>
                {seed.output_type}
              </span>
              {seed.target_primary_repo && (
                <span className="rounded-lg px-2 py-0.5 text-[10px] font-medium bg-emerald-500/15 text-emerald-300">
                  primary
                </span>
              )}
            </button>

            {/* Expanded editor */}
            {expanded && (
              <div className="border-t border-[#2a2520] px-4 pb-4 pt-3 space-y-4">
                <div className="flex gap-3">
                  <Field label="Name" className="w-32">
                    <input
                      value={seed.name}
                      onChange={(e) => onUpdate(i, { name: e.target.value.toLowerCase().replace(/[^a-z0-9_]/g, "") })}
                      disabled={readOnly}
                      placeholder="seed_name"
                      className={inputCls}
                    />
                  </Field>
                  <Field label="Label" className="flex-1">
                    <input
                      value={seed.label}
                      onChange={(e) => onUpdate(i, { label: e.target.value })}
                      disabled={readOnly}
                      placeholder="Human-readable label"
                      className={inputCls}
                    />
                  </Field>
                  <Field label="Output" className="w-28">
                    <select
                      value={seed.output_type}
                      onChange={(e) => onUpdate(i, { output_type: e.target.value as SeedOutputType })}
                      disabled={readOnly}
                      className={inputCls}
                    >
                      <option value="task">Task</option>
                      <option value="proposal">Proposal</option>
                    </select>
                  </Field>
                </div>

                <Field label="Prompt">
                  <AutoTextarea
                    value={seed.prompt}
                    onChange={(v) => onUpdate(i, { prompt: v })}
                    disabled={readOnly}
                    placeholder="Seed prompt..."
                    minRows={3}
                  />
                </Field>

                <Field label="Allowed Tools">
                  <ToolChips
                    value={seed.allowed_tools}
                    onChange={(v) => onUpdate(i, { allowed_tools: v })}
                    disabled={readOnly}
                  />
                </Field>

                <div className="flex items-center gap-4">
                  <FlagToggle
                    label="Target Primary Repo"
                    checked={seed.target_primary_repo}
                    disabled={readOnly}
                    onChange={(v) => onUpdate(i, { target_primary_repo: v })}
                  />
                  {!readOnly && (
                    <button
                      onClick={() => onRemove(i)}
                      className="ml-auto rounded-lg bg-red-500/10 px-3 py-1.5 text-[12px] text-red-400 ring-1 ring-inset ring-red-500/20 transition-colors hover:bg-red-500/20"
                    >
                      Remove Seed
                    </button>
                  )}
                </div>
              </div>
            )}
          </div>
        );
      })}

      {!readOnly && (
        <button
          onClick={onAdd}
          className="rounded-lg bg-[#1c1a17] px-3 py-1.5 text-[12px] text-[#9c9486] ring-1 ring-inset ring-[#2a2520] transition-colors hover:bg-[#232019] hover:text-[#e8e0d4]"
        >
          + Add Seed
        </button>
      )}
    </div>
  );
}

const inputCls =
  "w-full rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-2 text-[13px] text-[#e8e0d4] outline-none transition-colors focus:border-amber-500/30 disabled:opacity-50 disabled:cursor-not-allowed";

function Field({ label, className, children }: { label: string; className?: string; children: React.ReactNode }) {
  return (
    <div className={className}>
      <div className="mb-1.5 text-[12px] font-medium text-[#9c9486]">{label}</div>
      {children}
    </div>
  );
}

function FlagToggle({
  label,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled: boolean;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        "flex items-center gap-2.5 rounded-lg px-3 py-1.5 text-left text-[12px] transition-colors",
        checked ? "text-[#e8e0d4]" : "text-[#6b6459]",
        disabled && "cursor-not-allowed opacity-50"
      )}
    >
      <span className={cn(
        "flex h-4 w-4 shrink-0 items-center justify-center rounded border",
        checked
          ? "border-amber-500/40 bg-amber-500/20 text-amber-400"
          : "border-[#2a2520] bg-[#1c1a17]"
      )}>
        {checked && <span className="text-[9px]">&#10003;</span>}
      </span>
      {label}
    </button>
  );
}
