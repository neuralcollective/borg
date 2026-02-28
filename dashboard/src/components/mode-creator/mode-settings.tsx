import type { PipelineModeFull, IntegrationType } from "@/lib/types";
import { cn } from "@/lib/utils";

export function ModeSettings({
  mode,
  readOnly,
  onChange,
  onFork,
}: {
  mode: PipelineModeFull;
  readOnly: boolean;
  onChange: (key: keyof PipelineModeFull, value: unknown) => void;
  onFork: () => void;
}) {
  return (
    <div className="space-y-3 rounded-lg border border-white/[0.06] bg-white/[0.02] p-3">
      {/* Row 1: Identity */}
      <div className="flex items-end gap-3">
        <Field label="Name" className="w-36">
          <input
            value={mode.name}
            onChange={(e) => onChange("name", e.target.value.toLowerCase().replace(/[^a-z0-9_-]/g, ""))}
            disabled={readOnly}
            placeholder="myborg"
            className={inputCls}
          />
        </Field>
        <Field label="Label" className="flex-1">
          <input
            value={mode.label}
            onChange={(e) => onChange("label", e.target.value)}
            disabled={readOnly}
            placeholder="My Pipeline"
            className={inputCls}
          />
        </Field>
        <Field label="Category" className="w-40">
          <input
            value={mode.category || ""}
            onChange={(e) => onChange("category", e.target.value)}
            disabled={readOnly}
            placeholder="Engineering"
            className={inputCls}
            list="mode-categories"
          />
          <datalist id="mode-categories">
            <option value="Engineering" />
            <option value="Professional Services" />
            <option value="People & Ops" />
            <option value="Data & Analytics" />
          </datalist>
        </Field>
        <Field label="Integration" className="w-28">
          <select
            value={mode.integration}
            onChange={(e) => onChange("integration", e.target.value as IntegrationType)}
            disabled={readOnly}
            className={inputCls}
          >
            <option value="git_pr">Git PR</option>
            <option value="none">None</option>
          </select>
        </Field>
        <Field label="Max Attempts" className="w-20">
          <input
            type="number"
            min={1}
            max={20}
            value={mode.default_max_attempts}
            onChange={(e) => onChange("default_max_attempts", Math.max(1, Number(e.target.value)))}
            disabled={readOnly}
            className={inputCls}
          />
        </Field>
        {readOnly && (
          <button
            onClick={onFork}
            className="shrink-0 rounded-md bg-blue-500/20 px-3 py-1.5 text-[12px] font-medium text-blue-400 ring-1 ring-inset ring-blue-500/20 hover:bg-blue-500/30"
          >
            Fork to Edit
          </button>
        )}
      </div>

      {/* Row 2: Flags */}
      <div className="flex items-center gap-4">
        <Toggle label="Docker" checked={mode.uses_docker} disabled={readOnly}
          onChange={(v) => onChange("uses_docker", v)} />
        <Toggle label="Test Cmd" checked={mode.uses_test_cmd} disabled={readOnly}
          onChange={(v) => onChange("uses_test_cmd", v)} />
        <Toggle label="Git Worktrees" checked={mode.uses_git_worktrees} disabled={readOnly}
          onChange={(v) => onChange("uses_git_worktrees", v)} />
      </div>
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

function Toggle({
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
        "flex items-center gap-1.5 rounded-md px-2 py-1 text-[11px] transition-colors",
        checked
          ? "bg-blue-500/15 text-blue-400 ring-1 ring-inset ring-blue-500/20"
          : "bg-white/[0.04] text-zinc-600",
        disabled && "cursor-not-allowed opacity-50"
      )}
    >
      <span className={cn("h-2 w-2 rounded-sm", checked ? "bg-blue-400" : "bg-zinc-700")} />
      {label}
    </button>
  );
}
