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
    <div className="space-y-2">
      {seeds.length === 0 && (
        <div className="rounded-lg border border-dashed border-white/[0.08] p-6 text-center text-[12px] text-zinc-600">
          No seed modes configured. Seeds auto-generate tasks when the pipeline is idle.
        </div>
      )}

      {seeds.map((seed, i) => {
        const expanded = i === expandedIndex;
        return (
          <div
            key={`${seed.name}-${i}`}
            className={cn(
              "rounded-lg border transition-colors",
              expanded
                ? "border-blue-500/30 bg-blue-500/[0.03]"
                : "border-white/[0.06] bg-white/[0.02]"
            )}
          >
            {/* Summary row */}
            <button
              onClick={() => onExpand(expanded ? null : i)}
              className="flex w-full items-center gap-3 px-3 py-2 text-left"
            >
              <span className="text-[9px] text-zinc-600">
                {expanded ? "\u25BC" : "\u25B6"}
              </span>
              <span className={cn(
                "min-w-[80px] text-[12px] font-medium",
                seed.name ? "text-zinc-200" : "text-zinc-600"
              )}>
                {seed.name || "unnamed"}
              </span>
              <span className="flex-1 truncate text-[11px] text-zinc-500">
                {seed.label || seed.prompt.slice(0, 60) || "—"}
              </span>
              <span className={cn(
                "rounded px-1.5 py-px text-[9px]",
                seed.output_type === "task"
                  ? "bg-blue-500/15 text-blue-400"
                  : "bg-amber-500/15 text-amber-400"
              )}>
                {seed.output_type}
              </span>
              {seed.target_primary_repo && (
                <span className="rounded px-1.5 py-px text-[9px] bg-green-500/15 text-green-400">
                  primary
                </span>
              )}
            </button>

            {/* Expanded editor */}
            {expanded && (
              <div className="border-t border-white/[0.06] px-3 pb-3 pt-2 space-y-3">
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
                      className="ml-auto rounded-md bg-red-500/10 px-2 py-1 text-[11px] text-red-400 hover:bg-red-500/20"
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
          className="rounded-md bg-white/[0.06] px-2 py-1 text-[11px] text-zinc-400 hover:bg-white/[0.1]"
        >
          + Add Seed
        </button>
      )}
    </div>
  );
}

const inputCls =
  "w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-2 py-1.5 text-[12px] text-zinc-200 outline-none focus:border-blue-500/40 disabled:opacity-50 disabled:cursor-not-allowed";

function Field({ label, className, children }: { label: string; className?: string; children: React.ReactNode }) {
  return (
    <div className={className}>
      <div className="mb-1 text-[11px] text-zinc-500">{label}</div>
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
        "flex items-center gap-2 rounded-md px-2 py-1 text-left text-[11px] transition-colors",
        checked ? "text-zinc-300" : "text-zinc-600",
        disabled && "cursor-not-allowed opacity-50"
      )}
    >
      <span className={cn(
        "flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded border",
        checked
          ? "border-blue-500/40 bg-blue-500/20 text-blue-400"
          : "border-white/[0.1] bg-white/[0.03]"
      )}>
        {checked && <span className="text-[8px]">&#10003;</span>}
      </span>
      {label}
    </button>
  );
}
