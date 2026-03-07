import { useState, useEffect } from "react";
import type { PhaseConfigFull, PhaseType } from "@/lib/types";
import { AutoTextarea } from "./auto-textarea";
import { ToolChips } from "./tool-chips";
import { cn } from "@/lib/utils";
import type { CategoryProfile } from "./category-profiles";

const PHASE_TYPE_LABELS: Record<PhaseType, string> = {
  setup: "Setup",
  agent: "Agent",
  validate: "Validate",
  rebase: "Rebase",
  lint_fix: "Lint Fix",
  human_review: "Human Review",
  compliance_check: "Compliance",
};

const FLAG_LABELS: Record<string, string> = {
  commits: "Commits",
  runs_tests: "Runs Tests",
  use_docker: "Use Docker",
  include_task_context: "Include Context",
  include_file_listing: "File Listing",
  allow_no_changes: "Allow No Changes",
  fresh_session: "Fresh Session",
};

export function PhaseDetail({
  phase,
  phaseNames,
  readOnly,
  onChange,
  profile,
}: {
  phase: PhaseConfigFull;
  phaseNames: string[];
  readOnly: boolean;
  onChange: (patch: Partial<PhaseConfigFull>) => void;
  profile: CategoryProfile;
}) {
  const [showAdvanced, setShowAdvanced] = useState(false);

  useEffect(() => {
    setShowAdvanced(false);
  }, [phase.name]);

  const isAgent = phase.phase_type === "agent" || phase.phase_type === "rebase";
  const isHumanReview = phase.phase_type === "human_review";
  const isCompliance = phase.phase_type === "compliance_check";

  const advancedCount = [
    phase.system_prompt,
    phase.error_instruction,
    phase.fix_instruction,
  ].filter((s) => s.trim()).length;

  return (
    <div className="space-y-3 overflow-y-auto">
      {/* Identity — compact inline row */}
      <div className="flex items-end gap-2">
        <Field label="Name" className="w-36">
          <input
            value={phase.name}
            onChange={(e) => onChange({ name: e.target.value.toLowerCase().replace(/[^a-z0-9_]/g, "") })}
            disabled={readOnly}
            className={inputCls}
          />
        </Field>
        <Field label="Label" className="flex-1">
          <input
            value={phase.label}
            onChange={(e) => onChange({ label: e.target.value })}
            disabled={readOnly}
            className={inputCls}
          />
        </Field>
        <Field label="Type" className="w-28">
          <select
            value={phase.phase_type}
            onChange={(e) => onChange({ phase_type: e.target.value as PhaseType })}
            disabled={readOnly}
            className={inputCls}
          >
            {profile.phaseTypes.map((pt) => (
              <option key={pt} value={pt}>{PHASE_TYPE_LABELS[pt]}</option>
            ))}
            {/* Keep current type visible even if profile hides it */}
            {!profile.phaseTypes.includes(phase.phase_type) && (
              <option value={phase.phase_type}>{PHASE_TYPE_LABELS[phase.phase_type]}</option>
            )}
          </select>
        </Field>
        <Field label="Next" className="w-28">
          <select
            value={phase.next}
            onChange={(e) => onChange({ next: e.target.value })}
            disabled={readOnly}
            className={inputCls}
          >
            {phaseNames.filter((n) => n !== phase.name).map((n) => (
              <option key={n} value={n}>{n}</option>
            ))}
            <option value="done">done</option>
          </select>
        </Field>
      </div>

      {/* Human Review */}
      {isHumanReview && (
        <Section title="Human Review">
          <div className="mb-2 rounded bg-emerald-500/5 border border-emerald-500/20 px-3 py-2 text-[11px] text-emerald-400/80">
            Pauses the pipeline until a human approves, rejects, or requests revision.
          </div>
          <Field label="Reviewer Guidance">
            <AutoTextarea
              value={phase.instruction}
              onChange={(v) => onChange({ instruction: v })}
              disabled={readOnly}
              placeholder="Instructions for the human reviewer..."
              minRows={3}
            />
          </Field>
        </Section>
      )}

      {/* Compliance */}
      {isCompliance && (
        <Section title="Compliance Check">
          <div className="grid grid-cols-2 gap-3">
            <Field label="Profile">
              <select
                value={phase.compliance_profile || "uk_sra"}
                onChange={(e) => onChange({ compliance_profile: e.target.value })}
                disabled={readOnly}
                className={inputCls}
              >
                <option value="uk_sra">UK SRA / Law Society</option>
                <option value="us_prof_resp">US Professional Responsibility</option>
              </select>
            </Field>
            <Field label="Enforcement">
              <select
                value={phase.compliance_enforcement || "warn"}
                onChange={(e) => onChange({ compliance_enforcement: e.target.value })}
                disabled={readOnly}
                className={inputCls}
              >
                <option value="warn">warn</option>
                <option value="block">block</option>
              </select>
            </Field>
          </div>
        </Section>
      )}

      {/* Main instruction — the primary thing users edit */}
      {isAgent && (
        <Section title="Instruction">
          <AutoTextarea
            value={phase.instruction}
            onChange={(v) => onChange({ instruction: v })}
            disabled={readOnly}
            placeholder="What should the agent do in this phase?"
            minRows={4}
          />
        </Section>
      )}

      {/* Tools — compact toggles */}
      {isAgent && (
        <Section title="Tools">
          <ToolChips
            value={phase.allowed_tools}
            onChange={(v) => onChange({ allowed_tools: v })}
            disabled={readOnly}
            visibleTools={profile.tools}
          />
        </Section>
      )}

      {/* Behavior — filtered by category profile */}
      {isAgent && (
        <Section title="Behavior">
          <div className="grid grid-cols-3 gap-x-4 gap-y-1.5">
            {profile.behaviorFlags.map((flag) => {
              const key = flag as keyof PhaseConfigFull;
              return (
                <FlagToggle
                  key={flag}
                  label={FLAG_LABELS[flag] || flag}
                  checked={!!phase[key]}
                  disabled={readOnly}
                  onChange={(v) => onChange({ [key]: v })}
                />
              );
            })}
          </div>
          {/* Commit settings inline when enabled */}
          {phase.commits && (
            <div className="mt-2.5 flex gap-3 border-t border-white/[0.04] pt-2.5">
              <Field label="Commit Message" className="flex-1">
                <input
                  value={phase.commit_message}
                  onChange={(e) => onChange({ commit_message: e.target.value })}
                  disabled={readOnly}
                  placeholder="feat: implementation from agent"
                  className={inputCls}
                />
              </Field>
              <Field label="Required Artifact" className="w-40">
                <input
                  value={phase.check_artifact ?? ""}
                  onChange={(e) => onChange({ check_artifact: e.target.value || null })}
                  disabled={readOnly}
                  placeholder="(optional)"
                  className={inputCls}
                />
              </Field>
            </div>
          )}
        </Section>
      )}

      {/* Advanced — collapsed by default, for power users */}
      {isAgent && (
        <div className="rounded-lg border border-white/[0.04]">
          <button
            type="button"
            onClick={() => setShowAdvanced(!showAdvanced)}
            className="flex w-full items-center gap-2 px-3 py-2 text-[11px] text-zinc-500 hover:text-zinc-400"
          >
            <span className="text-[9px]">{showAdvanced ? "\u25BC" : "\u25B6"}</span>
            <span className="font-medium">Advanced</span>
            {!showAdvanced && advancedCount > 0 && (
              <span className="rounded bg-white/[0.06] px-1.5 py-0.5 text-[10px] text-zinc-500">
                {advancedCount} configured
              </span>
            )}
          </button>
          {showAdvanced && (
            <div className="space-y-3 border-t border-white/[0.04] px-3 pb-3 pt-2">
              <Field label="System Prompt">
                <AutoTextarea
                  value={phase.system_prompt}
                  onChange={(v) => onChange({ system_prompt: v })}
                  disabled={readOnly}
                  placeholder="Override the default system prompt..."
                  minRows={2}
                />
              </Field>
              <Field label="Error Instruction">
                <AutoTextarea
                  value={phase.error_instruction}
                  onChange={(v) => onChange({ error_instruction: v })}
                  disabled={readOnly}
                  placeholder="Instruction when retrying after error (use {ERROR} placeholder)..."
                  minRows={2}
                />
              </Field>
              {phase.phase_type === "rebase" && (
                <Field label="Fix Instruction">
                  <AutoTextarea
                    value={phase.fix_instruction}
                    onChange={(v) => onChange({ fix_instruction: v })}
                    disabled={readOnly}
                    placeholder="Instruction for the rebase fix agent..."
                    minRows={2}
                  />
                </Field>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

const inputCls =
  "w-full rounded-md border border-white/[0.08] bg-white/[0.04] px-2 py-1.5 text-[12px] text-zinc-200 outline-none focus:border-blue-500/40 disabled:opacity-50 disabled:cursor-not-allowed";

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-3">
      <div className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-zinc-500">{title}</div>
      {children}
    </div>
  );
}

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
