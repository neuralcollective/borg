import { useState } from "react";
import { useSettings, useStatus, updateSettings, useRepos, setRepoBackend, type Settings } from "@/lib/api";
import { useUIMode, type UIMode } from "@/lib/ui-mode";
import { useQueryClient } from "@tanstack/react-query";
import { cn } from "@/lib/utils";

function formatUptime(seconds: number) {
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (d > 0) return `${d}d ${h}h ${m}m`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

export function SettingsPanel() {
  const { data: settings, isLoading } = useSettings();
  const { data: status } = useStatus();
  const { mode: uiMode, setMode: setUIMode } = useUIMode();
  const queryClient = useQueryClient();
  const [saving, setSaving] = useState(false);
  const [draft, setDraft] = useState<Partial<Settings>>({});
  const [saved, setSaved] = useState(false);

  const effective = settings ? { ...settings, ...draft } : null;
  const hasDraft = Object.keys(draft).length > 0;

  async function handleSave() {
    if (!hasDraft) return;
    setSaving(true);
    try {
      await updateSettings(draft);
      queryClient.invalidateQueries({ queryKey: ["settings"] });
      queryClient.invalidateQueries({ queryKey: ["status"] });
      setDraft({});
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } finally {
      setSaving(false);
    }
  }

  function update<K extends keyof Settings>(key: K, value: Settings[K]) {
    setDraft((prev) => ({ ...prev, [key]: value }));
    setSaved(false);
  }

  if (isLoading || !effective) {
    return (
      <div className="flex h-full items-center justify-center text-xs text-zinc-600">
        Loading settings...
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-2xl space-y-8 p-6">
        {/* Dashboard Preferences */}
        <Section title="Dashboard">
          <div className="flex items-center justify-between">
            <div>
              <Label>Interface Mode</Label>
              <Desc>Minimal hides technical details (logs, queue, git info). Advanced shows everything.</Desc>
            </div>
            <ToggleGroup
              value={uiMode}
              onChange={(v) => setUIMode(v as UIMode)}
              options={[
                { value: "minimal", label: "Minimal" },
                { value: "advanced", label: "Advanced" },
              ]}
            />
          </div>
        </Section>

        {/* Pipeline Settings */}
        <Section title="Pipeline">
          <ToggleField
            label="Continuous Mode"
            desc="Auto-seed new tasks when pipeline is idle"
            value={effective.continuous_mode}
            onChange={(v) => update("continuous_mode", v)}
          />
          <NumberField
            label="Max Backlog"
            desc="Maximum concurrent pipeline tasks"
            value={effective.pipeline_max_backlog}
            onChange={(v) => update("pipeline_max_backlog", v)}
            min={1}
            max={20}
          />
          <NumberField
            label="Max Agents"
            desc="Maximum concurrent agent processes"
            value={effective.pipeline_max_agents}
            onChange={(v) => update("pipeline_max_agents", v)}
            min={1}
            max={10}
          />
          <NumberField
            label="Release Interval (min)"
            desc="Minutes between release cycles"
            value={effective.release_interval_mins}
            onChange={(v) => update("release_interval_mins", v)}
            min={1}
            max={1440}
          />
          <NumberField
            label="Seed Cooldown (s)"
            desc="Minimum seconds between seed scans"
            value={effective.pipeline_seed_cooldown_s}
            onChange={(v) => update("pipeline_seed_cooldown_s", v)}
            min={60}
            max={86400}
          />
          <NumberField
            label="Tick Interval (s)"
            desc="Main pipeline loop interval"
            value={effective.pipeline_tick_s}
            onChange={(v) => update("pipeline_tick_s", v)}
            min={5}
            max={300}
          />
          <NumberField
            label="Proposal Threshold"
            desc="Minimum triage score (1–10) to auto-promote a proposal to a task"
            value={effective.proposal_promote_threshold}
            onChange={(v) => update("proposal_promote_threshold", v)}
            min={1}
            max={10}
          />
        </Section>

        {/* Agent Settings */}
        <Section title="Agent">
          <SelectField
            label="Backend"
            desc="Default AI provider for pipeline tasks"
            value={effective.backend}
            onChange={(v) => update("backend", v)}
            options={[
              { value: "claude", label: "Claude (Anthropic)" },
              { value: "codex", label: "Codex (OpenAI)" },
              { value: "local", label: "Local (Ollama)" },
            ]}
          />
          <TextField
            label="Model"
            desc="Claude model ID for agent tasks"
            value={effective.model}
            onChange={(v) => update("model", v)}
          />
          <NumberField
            label="Timeout (s)"
            desc="Max seconds per agent run"
            value={effective.agent_timeout_s}
            onChange={(v) => update("agent_timeout_s", v)}
            min={60}
            max={7200}
          />
          <NumberField
            label="Container Memory (MB)"
            desc="Memory limit for Docker containers"
            value={effective.container_memory_mb}
            onChange={(v) => update("container_memory_mb", v)}
            min={256}
            max={16384}
          />
          <TextField
            label="Assistant Name"
            desc="Name used in chat responses"
            value={effective.assistant_name}
            onChange={(v) => update("assistant_name", v)}
          />
        </Section>

        {/* Git Attribution */}
        <Section title="Git Attribution">
          <ToggleField
            label="Claude Co-author"
            desc="Include Claude as Co-Authored-By in pipeline commits"
            value={effective.git_claude_coauthor}
            onChange={(v) => update("git_claude_coauthor", v)}
          />
          <TextField
            label="User Co-author"
            desc="Add as Co-Authored-By in commits (e.g. username <email@example.com>)"
            value={effective.git_user_coauthor}
            onChange={(v) => update("git_user_coauthor", v)}
          />
        </Section>

        {/* Per-Repo Settings */}
        <ReposSection />

        {/* System Info (read-only) */}
        <Section title="System">
          <InfoRow label="Version" value={status?.version ?? "--"} />
          <InfoRow label="Uptime" value={status ? formatUptime(status.uptime_s) : "--"} />
          <InfoRow label="Watched Repos" value={String(status?.watched_repos?.length ?? 0)} />
          <InfoRow label="Active Tasks" value={String(status?.active_tasks ?? 0)} />
          <InfoRow label="Total Tasks" value={String(status?.total_tasks ?? 0)} />
        </Section>

        {/* Save bar */}
        {(hasDraft || saved) && (
          <div className="sticky bottom-4 flex items-center justify-end gap-3 rounded-lg border border-white/[0.08] bg-zinc-900/95 px-4 py-3 backdrop-blur">
            {saved && <span className="text-[11px] text-emerald-400">Settings saved</span>}
            {hasDraft && (
              <>
                <button
                  onClick={() => setDraft({})}
                  className="rounded-md px-3 py-1.5 text-[11px] text-zinc-400 hover:text-zinc-200"
                >
                  Discard
                </button>
                <button
                  onClick={handleSave}
                  disabled={saving}
                  className="rounded-md bg-blue-500/20 px-4 py-1.5 text-[11px] font-medium text-blue-400 ring-1 ring-inset ring-blue-500/20 transition-colors hover:bg-blue-500/30 disabled:opacity-50"
                >
                  {saving ? "Saving..." : "Save Changes"}
                </button>
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function ReposSection() {
  const { data: repos } = useRepos();
  const queryClient = useQueryClient();

  if (!repos || repos.length === 0) return null;

  return (
    <Section title="Repos">
      {repos.map((repo) => (
        <div key={repo.id} className="flex items-center justify-between gap-4">
          <div className="min-w-0 flex-1">
            <Label>{repo.name}</Label>
            <Desc>{repo.mode}{repo.auto_merge ? " · auto-merge" : " · manual"}</Desc>
          </div>
          <select
            value={repo.backend ?? ""}
            onChange={async (e) => {
              await setRepoBackend(repo.id, e.target.value);
              queryClient.invalidateQueries({ queryKey: ["repos"] });
            }}
            className="rounded-md border border-white/[0.08] bg-zinc-900 px-2.5 py-1.5 text-[12px] text-zinc-200 outline-none focus:border-blue-500/40"
          >
            <option value="">default</option>
            <option value="claude">claude</option>
            <option value="codex">codex</option>
            <option value="local">local</option>
          </select>
        </div>
      ))}
    </Section>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <h3 className="mb-3 text-[11px] font-semibold uppercase tracking-wider text-zinc-500">{title}</h3>
      <div className="space-y-4 rounded-lg border border-white/[0.06] bg-white/[0.02] p-4">
        {children}
      </div>
    </div>
  );
}

function Label({ children }: { children: React.ReactNode }) {
  return <div className="text-[12px] font-medium text-zinc-300">{children}</div>;
}

function Desc({ children }: { children: React.ReactNode }) {
  return <div className="mt-0.5 text-[11px] text-zinc-600">{children}</div>;
}

function ToggleField({ label, desc, value, onChange }: {
  label: string;
  desc: string;
  value: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between">
      <div>
        <Label>{label}</Label>
        <Desc>{desc}</Desc>
      </div>
      <button
        onClick={() => onChange(!value)}
        className={cn(
          "relative h-5 w-9 rounded-full transition-colors",
          value ? "bg-blue-500" : "bg-zinc-700"
        )}
      >
        <div
          className={cn(
            "absolute top-0.5 h-4 w-4 rounded-full bg-white transition-transform",
            value ? "left-[18px]" : "left-0.5"
          )}
        />
      </button>
    </div>
  );
}

function NumberField({ label, desc, value, onChange, min, max }: {
  label: string;
  desc: string;
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <div className="min-w-0 flex-1">
        <Label>{label}</Label>
        <Desc>{desc}</Desc>
      </div>
      <input
        type="number"
        value={value}
        min={min}
        max={max}
        onChange={(e) => {
          const v = parseInt(e.target.value);
          if (!isNaN(v)) onChange(v);
        }}
        className="w-24 rounded-md border border-white/[0.08] bg-white/[0.04] px-2.5 py-1.5 text-right text-[12px] tabular-nums text-zinc-200 outline-none focus:border-blue-500/40"
      />
    </div>
  );
}

function TextField({ label, desc, value, onChange }: {
  label: string;
  desc: string;
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <div className="min-w-0 flex-1">
        <Label>{label}</Label>
        <Desc>{desc}</Desc>
      </div>
      <input
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="w-56 rounded-md border border-white/[0.08] bg-white/[0.04] px-2.5 py-1.5 text-[12px] text-zinc-200 outline-none focus:border-blue-500/40"
      />
    </div>
  );
}

function SelectField({ label, desc, value, onChange, options }: {
  label: string;
  desc: string;
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <div className="min-w-0 flex-1">
        <Label>{label}</Label>
        <Desc>{desc}</Desc>
      </div>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="rounded-md border border-white/[0.08] bg-zinc-900 px-2.5 py-1.5 text-[12px] text-zinc-200 outline-none focus:border-blue-500/40"
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>{o.label}</option>
        ))}
      </select>
    </div>
  );
}

function ToggleGroup({ value, onChange, options }: {
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
}) {
  return (
    <div className="flex rounded-md border border-white/[0.08]">
      {options.map((opt, i) => (
        <button
          key={opt.value}
          onClick={() => onChange(opt.value)}
          className={cn(
            "px-3 py-1.5 text-[11px] font-medium transition-colors",
            i > 0 && "border-l border-white/[0.08]",
            value === opt.value
              ? "bg-white/[0.1] text-zinc-200"
              : "text-zinc-500 hover:text-zinc-300"
          )}
        >
          {opt.label}
        </button>
      ))}
    </div>
  );
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between">
      <span className="text-[12px] text-zinc-500">{label}</span>
      <span className="text-[12px] font-medium tabular-nums text-zinc-300">{value}</span>
    </div>
  );
}
