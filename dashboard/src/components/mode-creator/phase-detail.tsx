import { useState } from "react";
import type { PhaseConfigFull, PhaseType } from "@/lib/types";
import { AutoTextarea } from "./auto-textarea";
import { ToolChips } from "./tool-chips";
import { cn } from "@/lib/utils";

export function PhaseDetail({
  phase,
  phaseNames,
  readOnly,
  onChange,
}: {
  phase: PhaseConfigFull;
  phaseNames: string[];
  readOnly: boolean;
  onChange: (patch: Partial<PhaseConfigFull>) => void;
}) {
  const [showError, setShowError] = useState(!!phase.error_instruction);
  const [showFix, setShowFix] = useState(!!phase.fix_instruction);

  const isAgent = phase.phase_type === "agent" || phase.phase_type === "rebase";

  return (
    <div className="space-y-4 overflow-y-auto">
      {/* Identity */}
      <Section title="Identity">
        <div className="flex gap-3">
          <Field label="Name" className="w-40">
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
              <option value="setup">Setup</option>
              <option value="agent">Agent</option>
              <option value="rebase">Rebase</option>
              <option value="lint_fix">Lint Fix</option>
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
      </Section>

      {/* Agent Configuration */}
      {isAgent && (
        <Section title="Agent Configuration">
          <Field label="System Prompt">
            <AutoTextarea
              value={phase.system_prompt}
              onChange={(v) => onChange({ system_prompt: v })}
              disabled={readOnly}
              placeholder="System prompt for the agent..."
            />
          </Field>
          <Field label="Instruction" className="mt-3">
            <AutoTextarea
              value={phase.instruction}
              onChange={(v) => onChange({ instruction: v })}
              disabled={readOnly}
              placeholder="Task instruction..."
            />
          </Field>

          {/* Collapsible optional fields */}
          <CollapsibleField
            label="Error Instruction"
            value={phase.error_instruction}
            expanded={showError}
            onToggle={() => setShowError(!showError)}
          >
            <AutoTextarea
              value={phase.error_instruction}
              onChange={(v) => onChange({ error_instruction: v })}
              disabled={readOnly}
              placeholder="Instruction when retrying after error..."
              minRows={2}
            />
          </CollapsibleField>

          {phase.phase_type === "rebase" && (
            <CollapsibleField
              label="Fix Instruction"
              value={phase.fix_instruction}
              expanded={showFix}
              onToggle={() => setShowFix(!showFix)}
            >
              <AutoTextarea
                value={phase.fix_instruction}
                onChange={(v) => onChange({ fix_instruction: v })}
                disabled={readOnly}
                placeholder="Instruction for the rebase fix agent..."
                minRows={2}
              />
            </CollapsibleField>
          )}
        </Section>
      )}

      {/* Tools */}
      <Section title="Allowed Tools">
        <ToolChips
          value={phase.allowed_tools}
          onChange={(v) => onChange({ allowed_tools: v })}
          disabled={readOnly}
        />
      </Section>

      {/* Behavior Flags */}
      <Section title="Behavior">
        <div className="grid grid-cols-2 gap-x-4 gap-y-2">
          <FlagToggle label="Use Docker" checked={phase.use_docker} disabled={readOnly}
            onChange={(v) => onChange({ use_docker: v })} />
          <FlagToggle label="Include Task Context" checked={phase.include_task_context} disabled={readOnly}
            onChange={(v) => onChange({ include_task_context: v })} />
          <FlagToggle label="Include File Listing" checked={phase.include_file_listing} disabled={readOnly}
            onChange={(v) => onChange({ include_file_listing: v })} />
          <FlagToggle label="Runs Tests" checked={phase.runs_tests} disabled={readOnly}
            onChange={(v) => onChange({ runs_tests: v })} />
          <FlagToggle label="Commits" checked={phase.commits} disabled={readOnly}
            onChange={(v) => onChange({ commits: v })} />
          <FlagToggle label="Allow No Changes" checked={phase.allow_no_changes} disabled={readOnly}
            onChange={(v) => onChange({ allow_no_changes: v })} />
          <FlagToggle label="QA Fix Routing" checked={phase.has_qa_fix_routing} disabled={readOnly}
            onChange={(v) => onChange({ has_qa_fix_routing: v })} />
          <FlagToggle label="Fresh Session" checked={phase.fresh_session} disabled={readOnly}
            onChange={(v) => onChange({ fresh_session: v })} />
        </div>
      </Section>

      {/* Commit & Artifact */}
      {phase.commits && (
        <Section title="Commit & Artifact">
          <Field label="Commit Message">
            <input
              value={phase.commit_message}
              onChange={(e) => onChange({ commit_message: e.target.value })}
              disabled={readOnly}
              placeholder="impl: implementation from worker agent"
              className={inputCls}
            />
          </Field>
          <Field label="Check Artifact" className="mt-2">
            <input
              value={phase.check_artifact ?? ""}
              onChange={(e) => onChange({ check_artifact: e.target.value || null })}
              disabled={readOnly}
              placeholder="spec.md (optional)"
              className={inputCls}
            />
          </Field>
        </Section>
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

function CollapsibleField({
  label,
  value,
  expanded,
  onToggle,
  children,
}: {
  label: string;
  value: string;
  expanded: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}) {
  const hasContent = value.trim().length > 0;
  return (
    <div className="mt-3">
      <button
        type="button"
        onClick={onToggle}
        className="flex items-center gap-1 text-[11px] text-zinc-500 hover:text-zinc-400"
      >
        <span className="text-[9px]">{expanded ? "\u25BC" : "\u25B6"}</span>
        {label}
        {!expanded && hasContent && (
          <span className="text-zinc-600">({value.split("\n").length} lines)</span>
        )}
        {!expanded && !hasContent && (
          <span className="text-zinc-700">(empty)</span>
        )}
      </button>
      {expanded && <div className="mt-1">{children}</div>}
    </div>
  );
}
