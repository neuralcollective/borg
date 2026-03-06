import { useState, useRef, useEffect } from "react";
import {
  useSettings,
  useStatus,
  updateSettings,
  useRepos,
  setRepoBackend,
  useCacheVolumes,
  deleteCacheVolume,
  type Settings,
} from "@/lib/api";
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

function isValidHttpUrl(url: string): boolean {
  const raw = url.trim();
  if (!raw) return false;
  try {
    const parsed = new URL(raw);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}

export function SettingsPanel() {
  const { data: settings, isLoading } = useSettings();
  const { data: status } = useStatus();
  const { mode: uiMode, setMode: setUIMode } = useUIMode();
  const queryClient = useQueryClient();
  const [saving, setSaving] = useState(false);
  const [draft, setDraft] = useState<Partial<Settings>>({});
  const [saved, setSaved] = useState(false);
  const savedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => { if (savedTimerRef.current) clearTimeout(savedTimerRef.current); };
  }, []);

  const effective = settings ? { ...settings, ...draft } : null;
  const hasDraft = Object.keys(draft).length > 0;
  const publicUrlInvalid = !!effective?.public_url && !isValidHttpUrl(effective.public_url);

  async function handleSave() {
    if (!hasDraft) return;
    setSaving(true);
    try {
      await updateSettings(draft);
      queryClient.invalidateQueries({ queryKey: ["settings"] });
      queryClient.invalidateQueries({ queryKey: ["status"] });
      setDraft({});
      setSaved(true);
      if (savedTimerRef.current) clearTimeout(savedTimerRef.current);
      savedTimerRef.current = setTimeout(() => setSaved(false), 2000);
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
          <ToggleField
            label="Experimental Domains"
            desc="Enable non-core mode presets and runtime integrations."
            value={effective.experimental_domains}
            onChange={(v) => update("experimental_domains", v)}
          />
          <CategoryPicker
            value={effective.visible_categories}
            onChange={(v) => update("visible_categories", v)}
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
              { value: "gemini", label: "Gemini (Google)" },
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

        {/* Permissions */}
        <Section title="Permissions">
          <TextField
            label="Chat Disallowed Tools"
            desc="Comma-separated tools to block for chat agents (empty = all allowed)"
            value={effective.chat_disallowed_tools}
            onChange={(v) => update("chat_disallowed_tools", v)}
          />
          <TextField
            label="Pipeline Disallowed Tools"
            desc="Comma-separated tools to block for pipeline agents (empty = all allowed)"
            value={effective.pipeline_disallowed_tools}
            onChange={(v) => update("pipeline_disallowed_tools", v)}
          />
        </Section>

        {/* Cloud Storage */}
        <Section title="Cloud Storage">
          <TextField
            label="Public URL"
            desc="Public app URL used for OAuth callbacks (for example: https://app.borg.legal)"
            value={effective.public_url}
            onChange={(v) => update("public_url", v)}
          />
          {publicUrlInvalid && (
            <div className="rounded border border-amber-500/30 bg-amber-500/10 px-2.5 py-2 text-[11px] text-amber-300">
              Public URL should be a valid http(s) URL.
            </div>
          )}
          <TextField
            label="Dropbox Client ID"
            desc="OAuth app client ID for Dropbox"
            value={effective.dropbox_client_id}
            onChange={(v) => update("dropbox_client_id", v)}
          />
          <TextField
            label="Dropbox Client Secret"
            desc="OAuth app client secret for Dropbox"
            value={effective.dropbox_client_secret}
            onChange={(v) => update("dropbox_client_secret", v)}
          />
          <TextField
            label="Google Client ID"
            desc="OAuth app client ID for Google Drive"
            value={effective.google_client_id}
            onChange={(v) => update("google_client_id", v)}
          />
          <TextField
            label="Google Client Secret"
            desc="OAuth app client secret for Google Drive"
            value={effective.google_client_secret}
            onChange={(v) => update("google_client_secret", v)}
          />
          <TextField
            label="Microsoft Client ID"
            desc="OAuth app client ID for OneDrive"
            value={effective.ms_client_id}
            onChange={(v) => update("ms_client_id", v)}
          />
          <TextField
            label="Microsoft Client Secret"
            desc="OAuth app client secret for OneDrive"
            value={effective.ms_client_secret}
            onChange={(v) => update("ms_client_secret", v)}
          />
          <SelectField
            label="File Storage Backend"
            desc="Where uploaded files are stored. Use an S3-compatible endpoint for SeaweedFS."
            value={effective.storage_backend}
            onChange={(v) => update("storage_backend", v)}
            options={[
              { value: "local", label: "Local Disk" },
              { value: "s3", label: "S3-Compatible" },
            ]}
          />
          <TextField
            label="S3 Bucket"
            desc="Bucket name for project file storage"
            value={effective.s3_bucket}
            onChange={(v) => update("s3_bucket", v)}
          />
          <TextField
            label="S3 Region"
            desc="AWS region (for example us-east-1)"
            value={effective.s3_region}
            onChange={(v) => update("s3_region", v)}
          />
          <TextField
            label="S3 Endpoint"
            desc="Optional custom endpoint (for example SeaweedFS, Backblaze B2, or AWS-compatible storage)"
            value={effective.s3_endpoint}
            onChange={(v) => update("s3_endpoint", v)}
          />
          <TextField
            label="S3 Prefix"
            desc="Object key prefix (for example borg/)"
            value={effective.s3_prefix}
            onChange={(v) => update("s3_prefix", v)}
          />
          <SelectField
            label="Backup Backend"
            desc="Offsite backup target for active work artifacts"
            value={effective.backup_backend}
            onChange={(v) => update("backup_backend", v)}
            options={[
              { value: "disabled", label: "Disabled" },
              { value: "s3", label: "S3-Compatible" },
            ]}
          />
          <SelectField
            label="Backup Mode"
            desc="Default is to protect active work only. Upload backup is opt-in because it is the expensive path."
            value={effective.backup_mode}
            onChange={(v) => update("backup_mode", v)}
            options={[
              { value: "active_work_only", label: "Active Work Only" },
              { value: "include_uploads", label: "Include Uploads" },
            ]}
          />
          <TextField
            label="Backup Bucket"
            desc="Bucket used for offsite backup snapshots"
            value={effective.backup_bucket}
            onChange={(v) => update("backup_bucket", v)}
          />
          <TextField
            label="Backup Region"
            desc="Region for the backup target"
            value={effective.backup_region}
            onChange={(v) => update("backup_region", v)}
          />
          <TextField
            label="Backup Endpoint"
            desc="Custom endpoint for backup target (for example Backblaze B2 S3 API)"
            value={effective.backup_endpoint}
            onChange={(v) => update("backup_endpoint", v)}
          />
          <TextField
            label="Backup Prefix"
            desc="Object key prefix for backup snapshots"
            value={effective.backup_prefix}
            onChange={(v) => update("backup_prefix", v)}
          />
          <NumberField
            label="Backup Poll Interval"
            desc="Seconds between active-work backup snapshots"
            value={effective.backup_poll_interval_s}
            onChange={(v) => update("backup_poll_interval_s", v)}
            min={30}
          />
          <NumberField
            label="Project Max Bytes"
            desc="Maximum bytes allowed per project file corpus"
            value={effective.project_max_bytes}
            onChange={(v) => update("project_max_bytes", v)}
            min={1}
          />
          <NumberField
            label="Knowledge Max Bytes"
            desc="Maximum bytes allowed in global knowledge corpus"
            value={effective.knowledge_max_bytes}
            onChange={(v) => update("knowledge_max_bytes", v)}
            min={1}
          />
          <NumberField
            label="Cloud Import Batch Max"
            desc="Maximum files per cloud import request"
            value={effective.cloud_import_max_batch_files}
            onChange={(v) => update("cloud_import_max_batch_files", v)}
            min={1}
          />
          <SelectField
            label="Ingestion Queue Backend"
            desc="Background ingestion queue transport"
            value={effective.ingestion_queue_backend}
            onChange={(v) => update("ingestion_queue_backend", v)}
            options={[
              { value: "disabled", label: "Disabled" },
              { value: "sqs", label: "AWS SQS" },
            ]}
          />
          <TextField
            label="SQS Queue URL"
            desc="Queue URL for ingestion job messages"
            value={effective.sqs_queue_url}
            onChange={(v) => update("sqs_queue_url", v)}
          />
          <TextField
            label="SQS Region"
            desc="AWS region for SQS client"
            value={effective.sqs_region}
            onChange={(v) => update("sqs_region", v)}
          />
          <SelectField
            label="Search Backend"
            desc="External retrieval engine for project documents"
            value={effective.search_backend}
            onChange={(v) => update("search_backend", v)}
            options={[
              { value: "vespa", label: "Vespa" },
            ]}
          />
          <TextField
            label="Vespa URL"
            desc="Base URL for Vespa query/document API"
            value={effective.vespa_url}
            onChange={(v) => update("vespa_url", v)}
          />
          <TextField
            label="Vespa Namespace"
            desc="Vespa document namespace"
            value={effective.vespa_namespace}
            onChange={(v) => update("vespa_namespace", v)}
          />
          <TextField
            label="Vespa Document Type"
            desc="Vespa document type for indexed project files"
            value={effective.vespa_document_type}
            onChange={(v) => update("vespa_document_type", v)}
          />
        </Section>

        {/* Per-Repo Settings */}
        <ReposSection />

        {/* Docker Cache Volumes */}
        <CacheSection />

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

function CacheSection() {
  const { data, refetch } = useCacheVolumes();
  const [deleting, setDeleting] = useState<string | null>(null);
  const volumes = data?.volumes ?? [];

  if (volumes.length === 0) return null;

  function formatBytes(n: number) {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  }

  async function handleDelete(name: string) {
    if (!confirm(`Delete Docker volume "${name}"?`)) return;
    setDeleting(name);
    try {
      await deleteCacheVolume(name);
      await refetch();
    } finally {
      setDeleting(null);
    }
  }

  return (
    <Section title="Docker Cache Volumes">
      {volumes.map((vol) => (
        <div key={vol.name} className="flex items-center justify-between gap-4">
          <div className="min-w-0 flex-1">
            <div className="text-[12px] font-mono text-zinc-300 truncate">{vol.name}</div>
            <div className="text-[11px] text-zinc-600">{formatBytes(vol.size)}</div>
          </div>
          <button
            onClick={() => handleDelete(vol.name)}
            disabled={deleting === vol.name}
            className="shrink-0 rounded-md border border-red-500/20 px-2.5 py-1 text-[11px] font-medium text-red-400/70 hover:border-red-500/40 hover:text-red-400 disabled:opacity-40 transition-colors"
          >
            {deleting === vol.name ? "Deleting..." : "Delete"}
          </button>
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

const ALL_CATEGORIES = [
  { value: "Engineering", label: "Engineering" },
  { value: "Professional Services", label: "Legal / Professional" },
  { value: "People & Ops", label: "People & Ops" },
  { value: "Data & Analytics", label: "Data & Analytics" },
];

function CategoryPicker({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const active = new Set(value.split(",").map((s) => s.trim()).filter(Boolean));
  const allSelected = active.size === 0;

  function toggle(cat: string) {
    const next = new Set(active);
    if (next.has(cat)) {
      next.delete(cat);
    } else {
      next.add(cat);
    }
    onChange([...next].join(","));
  }

  return (
    <div>
      <div className="flex items-center justify-between">
        <div>
          <Label>Visible Domains</Label>
          <Desc>Which mode categories appear in the pipeline editor. Empty means all.</Desc>
        </div>
      </div>
      <div className="mt-2 flex flex-wrap gap-1.5">
        {ALL_CATEGORIES.map((cat) => {
          const on = allSelected || active.has(cat.value);
          return (
            <button
              key={cat.value}
              onClick={() => toggle(cat.value)}
              className={cn(
                "rounded-md px-2.5 py-1 text-[11px] transition-colors",
                on
                  ? "bg-blue-500/15 text-blue-400 ring-1 ring-inset ring-blue-500/20"
                  : "bg-white/[0.04] text-zinc-600 hover:bg-white/[0.08] hover:text-zinc-400"
              )}
            >
              {cat.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}
