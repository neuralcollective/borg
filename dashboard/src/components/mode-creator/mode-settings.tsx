import type { PipelineModeFull, IntegrationType } from "@/lib/types";
import type { CategoryProfile } from "./category-profiles";
import { cn } from "@/lib/utils";

export function ModeSettings({
  mode,
  readOnly,
  onChange,
  profile,
}: {
  mode: PipelineModeFull;
  readOnly: boolean;
  onChange: (key: keyof PipelineModeFull, value: unknown) => void;
  profile: CategoryProfile;
}) {
  return (
    <div className="space-y-4 rounded-xl border border-[#2a2520] bg-[#151412] p-4">
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
            {profile.integrations.map((opt) => (
              <option key={opt.value} value={opt.value}>{opt.label}</option>
            ))}
            {!profile.integrations.some((o) => o.value === mode.integration) && (
              <option value={mode.integration}>{mode.integration}</option>
            )}
          </select>
        </Field>
        <Field label="Max Attempts" className="w-24">
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
      </div>

      {/* Row 2: Flags */}
      {(profile.showDocker || profile.showTestCmd) && (
        <div className="flex items-center gap-3">
          {profile.showDocker && (
            <Toggle label="Docker" checked={mode.uses_docker} disabled={readOnly}
              onChange={(v) => onChange("uses_docker", v)} />
          )}
          {profile.showTestCmd && (
            <Toggle label="Test Cmd" checked={mode.uses_test_cmd} disabled={readOnly}
              onChange={(v) => onChange("uses_test_cmd", v)} />
          )}
        </div>
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
        "flex items-center gap-2 rounded-lg px-3 py-1.5 text-[12px] transition-colors",
        checked
          ? "bg-amber-500/15 text-amber-300 ring-1 ring-inset ring-amber-500/20"
          : "bg-[#1c1a17] text-[#6b6459]",
        disabled && "cursor-not-allowed opacity-50"
      )}
    >
      <span className={cn("h-2.5 w-2.5 rounded-sm", checked ? "bg-amber-400" : "bg-[#3d3830]")} />
      {label}
    </button>
  );
}
