import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import {
  changeUserPassword,
  createUser,
  deleteCacheVolume,
  deleteLinkedCredential,
  deleteUser,
  fetchLinkedCredentialConnectSession,
  type LinkedCredential,
  type LinkedCredentialConnectSession,
  type Settings,
  setRepoBackend,
  startLinkedCredentialConnect,
  updateSettings,
  updateUserSettings,
  useCacheVolumes,
  useHealth,
  useLinkedCredentials,
  useRepos,
  useSettings,
  useStatus,
  useUserSettings,
  useUsers,
} from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { useTheme } from "@/lib/theme";
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
  const { user } = useAuth();
  const { data: settings, isLoading } = useSettings();
  const { data: status } = useStatus();
  const { data: health } = useHealth();
  const { theme, toggle: toggleTheme } = useTheme();
  const queryClient = useQueryClient();
  const [saving, setSaving] = useState(false);
  const [draft, setDraft] = useState<Partial<Settings>>({});
  const [saved, setSaved] = useState(false);
  const [showAdmin, setShowAdmin] = useState(false);
  const savedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const isAdmin = user?.is_admin ?? false;

  useEffect(() => {
    return () => {
      if (savedTimerRef.current) clearTimeout(savedTimerRef.current);
    };
  }, []);

  const effective = settings ? { ...settings, ...draft } : null;
  const hasDraft = Object.keys(draft).length > 0;
  const publicUrlInvalid = !!effective?.public_url && !isValidHttpUrl(effective.public_url);
  const visibleCats = (effective?.visible_categories ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
  const hasCodeMode =
    visibleCats.length === 0 ||
    visibleCats.some((c) => c.toLowerCase().includes("engineering") || c.toLowerCase().includes("data"));

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
    return <div className="flex h-full items-center justify-center text-xs text-[#6b6459]">Loading settings...</div>;
  }

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-2xl space-y-8 px-6 py-8">
        {/* ── Your Settings ──────────────────────────────────────── */}
        <Section title="Your Settings">
          <div className="flex items-center justify-between">
            <div>
              <Label>Theme</Label>
              <Desc>Switch between dark and light appearance.</Desc>
            </div>
            <ToggleGroup
              value={theme}
              onChange={(v) => {
                if (v !== theme) toggleTheme();
              }}
              options={[
                { value: "dark", label: "Dark" },
                { value: "light", label: "Light" },
              ]}
            />
          </div>
          <DashboardModePicker />
          <UserModelPicker />
        </Section>

        {user && (
          <Section title="Account">
            <div className="space-y-4">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <div className="flex h-8 w-8 items-center justify-center rounded-full bg-amber-500/15 text-[13px] font-semibold text-amber-400">
                    {(user.username[0] ?? "?").toUpperCase()}
                  </div>
                  <div>
                    <div className="text-[12px] font-medium text-[#e8e0d4]">{user.username}</div>
                    <div className="text-[11px] text-[#6b6459]">{isAdmin ? "Admin" : "User"}</div>
                  </div>
                </div>
                <ChangeOwnPassword userId={user.id} />
              </div>
              <LinkedAccountsPanel />
            </div>
          </Section>
        )}

        {/* ── Admin Settings ─────────────────────────────────────── */}
        {isAdmin && !showAdmin && (
          <button
            onClick={() => setShowAdmin(true)}
            className="w-full rounded-lg border border-[#2a2520] bg-[#1c1a17]/50 py-3 text-[12px] font-medium text-[#9c9486] transition-colors hover:bg-[#1c1a17] hover:text-[#e8e0d4]"
          >
            Show Admin Settings
          </button>
        )}

        {isAdmin && showAdmin && (
          <>
            {/* User Management */}
            <UserManagement />

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
              <CategoryPicker value={effective.visible_categories} onChange={(v) => update("visible_categories", v)} />
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

            {/* Agent Settings (global defaults) */}
            <Section title="Agent (Global Defaults)">
              <ModelPicker
                model={effective.model}
                backend={effective.backend}
                onChange={(model, backend) => {
                  update("model", model);
                  update("backend", backend);
                }}
              />
              <div className="mt-2">
                <div className="flex items-center justify-between gap-4">
                  <div className="min-w-0 flex-1">
                    <Label>Global Model Override</Label>
                    <Desc>When set, forces this model for all users (disables per-user model choice)</Desc>
                  </div>
                  <select
                    value={effective.model_override}
                    onChange={(e) => update("model_override", e.target.value)}
                    className="rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-1.5 text-[13px] text-[#e8e0d4] outline-none focus:border-amber-500/40 transition-colors"
                  >
                    <option value="">Disabled (users choose)</option>
                    {MODEL_OPTIONS.map((o) => (
                      <option key={o.model} value={o.model}>
                        {o.label}
                      </option>
                    ))}
                  </select>
                </div>
              </div>
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

            {/* Git Attribution — only relevant for code modes */}
            {hasCodeMode && (
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
            )}

            {/* Permissions — hidden, full access by default */}

            {/* Cloud Storage */}
            <Section title="Cloud Storage">
              <TextField
                label="Public URL"
                desc="Public app URL used for OAuth callbacks (for example: https://app.example.com)"
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
                options={[{ value: "vespa", label: "Vespa" }]}
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

            {/* Per-Repo Settings — only for code modes */}
            {hasCodeMode && <ReposSection />}

            {/* Docker Cache Volumes — only for code modes */}
            {hasCodeMode && <CacheSection />}

            {/* System Info (read-only) */}
            <Section title="System">
              <InfoRow label="Version" value={status?.version ?? "--"} />
              <InfoRow label="Uptime" value={status ? formatUptime(status.uptime_s) : "--"} />
              <InfoRow label="Watched Repos" value={String(status?.watched_repos?.length ?? 0)} />
              <InfoRow label="Active Tasks" value={String(status?.active_tasks ?? 0)} />
              <InfoRow label="Total Tasks" value={String(status?.total_tasks ?? 0)} />
            </Section>
            {health?.search && (
              <Section title="BorgSearch">
                <InfoRow
                  label="Status"
                  value={(health.search as Record<string, unknown>).healthy ? "Healthy" : "Unhealthy"}
                />
                <InfoRow label="Backend" value={String((health.search as Record<string, unknown>).backend ?? "none")} />
                <InfoRow
                  label="Indexed Documents"
                  value={String((health.search as Record<string, unknown>).documents ?? "--")}
                />
                <InfoRow
                  label="Indexed Chunks"
                  value={String((health.search as Record<string, unknown>).chunks ?? "--")}
                />
              </Section>
            )}
          </>
        )}

        {/* Save bar */}
        {(hasDraft || saved) && (
          <div className="sticky bottom-4 flex items-center justify-end gap-3 rounded-lg border border-[#2a2520] bg-[#151412]/95 px-4 py-3 backdrop-blur">
            {saved && <span className="text-[11px] text-emerald-400">Settings saved</span>}
            {hasDraft && (
              <>
                <button
                  onClick={() => setDraft({})}
                  className="rounded-md px-3 py-1.5 text-[11px] text-[#9c9486] hover:text-[#e8e0d4]"
                >
                  Discard
                </button>
                <button
                  onClick={handleSave}
                  disabled={saving}
                  className="rounded-md bg-amber-500/20 px-4 py-1.5 text-[11px] font-medium text-amber-400 ring-1 ring-inset ring-amber-500/20 transition-colors hover:bg-amber-500/30 disabled:opacity-50"
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

function DashboardModePicker() {
  const { data: userSettings, refetch } = useUserSettings();
  const queryClient = useQueryClient();
  const [saving, setSaving] = useState(false);

  if (!userSettings) return null;

  const current = userSettings.dashboard_mode || "general";

  async function handleChange(mode: string) {
    setSaving(true);
    try {
      await updateUserSettings({ dashboard_mode: mode });
      await refetch();
      queryClient.invalidateQueries({ queryKey: ["user-settings"] });
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="flex items-center justify-between">
      <div>
        <Label>Mode</Label>
        <Desc>Controls layout, vocabulary, and available pipeline options.</Desc>
      </div>
      <ToggleGroup
        value={current}
        onChange={(v) => {
          if (!saving && v !== current) handleChange(v);
        }}
        options={[
          { value: "general", label: "General" },
          { value: "swe", label: "SWE" },
          { value: "knowledge", label: "Knowledge" },
          { value: "legal", label: "Legal" },
        ]}
      />
    </div>
  );
}

function UserModelPicker() {
  const { data: userSettings, refetch } = useUserSettings();
  const [saving, setSaving] = useState(false);

  if (!userSettings) return null;

  const overrideActive = userSettings.model_override_active;
  const overrideModel = userSettings.model_override;
  const userModel = userSettings.model || "";

  async function handleChange(model: string) {
    if (!model) {
      setSaving(true);
      try {
        await updateUserSettings({ model: "", backend: "" });
        await refetch();
      } finally {
        setSaving(false);
      }
      return;
    }
    const opt = MODEL_OPTIONS.find((o) => o.model === model);
    if (!opt) return;
    setSaving(true);
    try {
      await updateUserSettings({ model: opt.model, backend: opt.backend });
      await refetch();
    } finally {
      setSaving(false);
    }
  }

  if (overrideActive) {
    const label = MODEL_OPTIONS.find((o) => o.model === overrideModel)?.label ?? overrideModel;
    return (
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0 flex-1">
          <Label>Model</Label>
          <Desc>Set by admin — cannot be changed.</Desc>
        </div>
        <span className="rounded-md border border-[#2a2520] bg-[#1c1a17]/50 px-2.5 py-1.5 text-[12px] text-[#6b6459]">
          {label}
        </span>
      </div>
    );
  }

  return (
    <div className="flex items-center justify-between gap-4">
      <div className="min-w-0 flex-1">
        <Label>Model</Label>
        <Desc>Your preferred AI model. Leave blank to use the global default.</Desc>
      </div>
      <select
        value={userModel}
        onChange={(e) => handleChange(e.target.value)}
        disabled={saving}
        className={cn(
          "rounded-md border border-[#2a2520] bg-[#1c1a17] px-2.5 py-1.5 text-[12px] text-[#e8e0d4] outline-none focus:border-amber-500/40",
          saving && "opacity-50 cursor-not-allowed",
        )}
      >
        <option value="">Global default</option>
        {MODEL_OPTIONS.map((o) => (
          <option key={o.model} value={o.model}>
            {o.label}
          </option>
        ))}
      </select>
    </div>
  );
}

function ChangeOwnPassword({ userId }: { userId: number }) {
  const [open, setOpen] = useState(false);
  const [pw, setPw] = useState("");
  const [msg, setMsg] = useState("");
  const [busy, setBusy] = useState(false);

  if (!open) {
    return (
      <button
        onClick={() => setOpen(true)}
        className="rounded-md border border-[#2a2520] bg-[#1c1a17]/50 px-3 py-1.5 text-[11px] text-[#9c9486] transition-colors hover:bg-[#1c1a17] hover:text-[#e8e0d4]"
      >
        Change Password
      </button>
    );
  }

  return (
    <div className="flex items-center gap-2">
      <input
        type="password"
        value={pw}
        onChange={(e) => setPw(e.target.value)}
        placeholder="New password (min 4)"
        className="w-40 rounded-md border border-[#2a2520] bg-[#1c1a17] px-2 py-1.5 text-[11px] text-[#e8e0d4] outline-none focus:border-amber-500/40"
      />
      <button
        disabled={busy || pw.length < 4}
        onClick={async () => {
          setBusy(true);
          const res = await changeUserPassword(userId, pw);
          setMsg(res.error ?? "Password changed");
          setPw("");
          setBusy(false);
        }}
        className="rounded-md bg-amber-500/20 px-3 py-1.5 text-[11px] font-medium text-amber-400 ring-1 ring-inset ring-amber-500/20 disabled:opacity-50"
      >
        Save
      </button>
      <button
        onClick={() => {
          setOpen(false);
          setPw("");
          setMsg("");
        }}
        className="text-[11px] text-[#6b6459] hover:text-[#9c9486]"
      >
        Cancel
      </button>
      {msg && <span className="text-[10px] text-emerald-400">{msg}</span>}
    </div>
  );
}

const LINKED_ACCOUNT_PROVIDERS: Array<{
  provider: "claude" | "openai";
  label: string;
  desc: string;
  connectLabel: string;
}> = [
  {
    provider: "claude",
    label: "Claude Pro/Max",
    desc: "Uses Claude Code account auth. The resulting session is restored into the task sandbox for agent runs.",
    connectLabel: "Connect Claude",
  },
  {
    provider: "openai",
    label: "ChatGPT Plus/Pro",
    desc: "Uses Codex device auth. Borg stores the resulting Codex auth bundle per user and revalidates it every 15 minutes.",
    connectLabel: "Connect OpenAI",
  },
];

function formatCredentialTime(value: string) {
  if (!value) return "Never";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function credentialBadge(credential: LinkedCredential | undefined) {
  const status = credential?.status ?? "disconnected";
  if (status === "connected") {
    return "border-emerald-500/20 bg-emerald-500/10 text-emerald-300";
  }
  if (status === "expired") {
    return "border-amber-500/20 bg-amber-500/10 text-amber-300";
  }
  return "border-[#2a2520] bg-[#1c1a17]/60 text-[#9c9486]";
}

function LinkedAccountsPanel() {
  const queryClient = useQueryClient();
  const { data } = useLinkedCredentials();
  const [busyProvider, setBusyProvider] = useState<"claude" | "openai" | null>(null);
  const [pending, setPending] = useState<Record<string, LinkedCredentialConnectSession>>({});

  useEffect(() => {
    const activeProviders = Object.values(pending)
      .filter((session) => session.status === "pending")
      .map((session) => session.provider);
    if (activeProviders.length === 0) return;
    const timer = window.setInterval(async () => {
      for (const provider of activeProviders) {
        const current = pending[provider];
        if (!current) continue;
        try {
          const updated = await fetchLinkedCredentialConnectSession(current.id);
          if (updated.status === "connected") {
            setPending((prev) => {
              const next = { ...prev };
              delete next[provider];
              return next;
            });
            queryClient.invalidateQueries({ queryKey: ["linked-credentials"] });
            continue;
          }
          setPending((prev) => ({ ...prev, [provider]: updated }));
        } catch {}
      }
    }, 3000);
    return () => window.clearInterval(timer);
  }, [pending, queryClient]);

  async function handleConnect(provider: "claude" | "openai") {
    setBusyProvider(provider);
    try {
      const session = await startLinkedCredentialConnect(provider);
      if (session.auth_url) {
        window.open(session.auth_url, "_blank", "noopener,noreferrer");
      }
      if (session.status === "connected") {
        queryClient.invalidateQueries({ queryKey: ["linked-credentials"] });
        setPending((prev) => {
          const next = { ...prev };
          delete next[provider];
          return next;
        });
      } else {
        setPending((prev) => ({ ...prev, [provider]: session }));
      }
    } finally {
      setBusyProvider(null);
    }
  }

  async function handleDisconnect(provider: "claude" | "openai") {
    setBusyProvider(provider);
    try {
      await deleteLinkedCredential(provider);
      setPending((prev) => {
        const next = { ...prev };
        delete next[provider];
        return next;
      });
      queryClient.invalidateQueries({ queryKey: ["linked-credentials"] });
    } finally {
      setBusyProvider(null);
    }
  }

  const credentialsByProvider = new Map(
    (data?.credentials ?? []).map((credential) => [credential.provider, credential]),
  );

  return (
    <div className="space-y-3 rounded-xl border border-[#2a2520] bg-[#120f0d]/60 p-3">
      <div>
        <Label>Linked AI Accounts</Label>
        <Desc>
          Per-user agent credentials. Borg revalidates linked sessions every 15 minutes and again before use when
          needed.
        </Desc>
      </div>
      {LINKED_ACCOUNT_PROVIDERS.map((meta) => {
        const credential = credentialsByProvider.get(meta.provider);
        const pendingSession = pending[meta.provider];
        const isPending = pendingSession?.status === "pending";
        const detail = credential?.account_email || credential?.account_label || "";
        return (
          <div key={meta.provider} className="space-y-2 rounded-lg border border-[#2a2520] bg-[#1a1714]/70 p-3">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="text-[12px] font-medium text-[#e8e0d4]">{meta.label}</span>
                  <span
                    className={cn(
                      "rounded-full border px-2 py-0.5 text-[10px] uppercase tracking-[0.12em]",
                      credentialBadge(credential),
                    )}
                  >
                    {isPending ? "Connecting" : (credential?.status ?? "Disconnected")}
                  </span>
                </div>
                <div className="mt-1 text-[11px] text-[#6b6459]">{meta.desc}</div>
                {detail && <div className="mt-2 text-[11px] text-[#b8ad9d]">{detail}</div>}
                {credential?.last_validated_at && (
                  <div className="mt-1 text-[10px] text-[#6b6459]">
                    Last checked {formatCredentialTime(credential.last_validated_at)}
                  </div>
                )}
              </div>
              {credential?.status === "connected" ? (
                <button
                  onClick={() => handleDisconnect(meta.provider)}
                  disabled={busyProvider === meta.provider}
                  className="rounded-md border border-[#4a2a24] bg-[#2a1411]/70 px-3 py-1.5 text-[11px] text-[#d49a8f] transition-colors hover:bg-[#331915] disabled:opacity-50"
                >
                  Disconnect
                </button>
              ) : (
                <button
                  onClick={() => handleConnect(meta.provider)}
                  disabled={busyProvider === meta.provider || isPending}
                  className="rounded-md bg-amber-500/20 px-3 py-1.5 text-[11px] font-medium text-amber-300 ring-1 ring-inset ring-amber-500/20 transition-colors hover:bg-amber-500/30 disabled:opacity-50"
                >
                  {busyProvider === meta.provider ? "Starting..." : meta.connectLabel}
                </button>
              )}
            </div>
            {pendingSession && (
              <div className="rounded-md border border-[#2a2520] bg-[#141210] px-3 py-2 text-[11px] text-[#b8ad9d]">
                {pendingSession.message || "Waiting for provider login completion."}
                {pendingSession.auth_url && (
                  <div className="mt-2">
                    <a
                      href={pendingSession.auth_url}
                      target="_blank"
                      rel="noreferrer"
                      className="text-amber-300 underline underline-offset-2"
                    >
                      Open provider sign-in
                    </a>
                  </div>
                )}
                {pendingSession.device_code && (
                  <div className="mt-2 font-mono text-[12px] text-[#f1e7d8]">{pendingSession.device_code}</div>
                )}
                {pendingSession.error && <div className="mt-2 text-[#d49a8f]">{pendingSession.error}</div>}
              </div>
            )}
            {credential?.last_error && credential.status !== "connected" && !pendingSession && (
              <div className="rounded-md border border-[#4a2a24] bg-[#2a1411]/40 px-3 py-2 text-[11px] text-[#d49a8f]">
                {credential.last_error}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function UserManagement() {
  const { data: users, refetch } = useUsers();
  const { user: currentUser, logout } = useAuth();
  const [showCreate, setShowCreate] = useState(false);
  const [newUser, setNewUser] = useState({ username: "", password: "", display_name: "", is_admin: false });
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  return (
    <Section title="Users">
      {users?.map((u) => (
        <div key={u.id} className="flex items-center justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="text-[12px] font-medium text-[#e8e0d4]">
              {u.username}
              {u.is_admin && <span className="ml-1.5 text-[10px] text-amber-400/70">admin</span>}
            </div>
            {u.display_name && u.display_name !== u.username && (
              <div className="text-[11px] text-[#6b6459]">{u.display_name}</div>
            )}
          </div>
          {u.id !== currentUser?.id && (
            <button
              onClick={async () => {
                if (!confirm(`Delete user "${u.username}"?`)) return;
                await deleteUser(u.id);
                await refetch();
              }}
              className="text-[11px] text-red-400/60 hover:text-red-400 transition-colors"
            >
              Delete
            </button>
          )}
        </div>
      ))}

      {!showCreate ? (
        <div className="flex items-center gap-3">
          <button
            onClick={() => setShowCreate(true)}
            className="text-[11px] text-amber-400/70 hover:text-amber-400 transition-colors"
          >
            + Add User
          </button>
          <button
            onClick={logout}
            className="ml-auto text-[11px] text-[#6b6459] hover:text-[#9c9486] transition-colors"
          >
            Sign Out
          </button>
        </div>
      ) : (
        <div className="space-y-2 rounded-md border border-[#2a2520] bg-[#1c1a17]/50 p-3">
          <input
            value={newUser.username}
            onChange={(e) => setNewUser({ ...newUser, username: e.target.value })}
            placeholder="Username"
            className="w-full rounded-md border border-[#2a2520] bg-[#1c1a17] px-2 py-1.5 text-[12px] text-[#e8e0d4] outline-none focus:border-amber-500/40"
          />
          <input
            value={newUser.display_name}
            onChange={(e) => setNewUser({ ...newUser, display_name: e.target.value })}
            placeholder="Display Name (optional)"
            className="w-full rounded-md border border-[#2a2520] bg-[#1c1a17] px-2 py-1.5 text-[12px] text-[#e8e0d4] outline-none focus:border-amber-500/40"
          />
          <input
            type="password"
            value={newUser.password}
            onChange={(e) => setNewUser({ ...newUser, password: e.target.value })}
            placeholder="Password (min 4)"
            className="w-full rounded-md border border-[#2a2520] bg-[#1c1a17] px-2 py-1.5 text-[12px] text-[#e8e0d4] outline-none focus:border-amber-500/40"
          />
          <label className="flex items-center gap-2 text-[11px] text-[#9c9486]">
            <input
              type="checkbox"
              checked={newUser.is_admin}
              onChange={(e) => setNewUser({ ...newUser, is_admin: e.target.checked })}
              className="rounded"
            />
            Admin
          </label>
          <div className="flex items-center gap-2">
            <button
              disabled={busy || !newUser.username.trim() || newUser.password.length < 4}
              onClick={async () => {
                setBusy(true);
                setMsg("");
                const res = await createUser(newUser);
                if (res.error) {
                  setMsg(res.error);
                } else {
                  setNewUser({ username: "", password: "", display_name: "", is_admin: false });
                  setShowCreate(false);
                  await refetch();
                }
                setBusy(false);
              }}
              className="rounded-md bg-amber-500/20 px-3 py-1 text-[11px] text-amber-400 ring-1 ring-inset ring-amber-500/20 disabled:opacity-50"
            >
              Create
            </button>
            <button onClick={() => setShowCreate(false)} className="text-[11px] text-[#6b6459] hover:text-[#9c9486]">
              Cancel
            </button>
            {msg && <span className="text-[10px] text-red-400">{msg}</span>}
          </div>
        </div>
      )}
    </Section>
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
            <Desc>
              {repo.mode}
              {repo.auto_merge ? " · auto-merge" : " · manual"}
            </Desc>
          </div>
          <select
            value={repo.backend ?? ""}
            onChange={async (e) => {
              await setRepoBackend(repo.id, e.target.value);
              queryClient.invalidateQueries({ queryKey: ["repos"] });
            }}
            className="rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-1.5 text-[13px] text-[#e8e0d4] outline-none focus:border-amber-500/40 transition-colors"
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

  async function handleDeleteAll() {
    if (!confirm(`Delete all ${volumes.length} Docker cache volumes?`)) return;
    setDeleting("__all__");
    try {
      for (const vol of volumes) {
        await deleteCacheVolume(vol.name);
      }
      await refetch();
    } finally {
      setDeleting(null);
    }
  }

  const totalSize = volumes.reduce((sum, v) => sum + v.size, 0);

  return (
    <div>
      <div className="mb-3 flex items-center justify-between">
        <h3 className="text-[11px] font-semibold uppercase tracking-wider text-[#6b6459]">
          Docker Cache Volumes
          <span className="ml-2 font-normal normal-case text-[#5a5349]">
            {volumes.length} volumes · {formatBytes(totalSize)}
          </span>
        </h3>
        <button
          onClick={handleDeleteAll}
          disabled={deleting !== null}
          className="rounded-md border border-red-500/20 px-2.5 py-1 text-[11px] font-medium text-red-400/70 hover:border-red-500/40 hover:text-red-400 disabled:opacity-40 transition-colors"
        >
          {deleting === "__all__" ? "Deleting..." : "Delete All"}
        </button>
      </div>
      <div className="max-h-[240px] space-y-3 overflow-y-auto rounded-lg border border-[#2a2520] bg-[#1c1a17]/50 p-4">
        {volumes.map((vol) => (
          <div key={vol.name} className="flex items-center justify-between gap-4">
            <div className="min-w-0 flex-1">
              <div className="text-[12px] font-mono text-[#e8e0d4] truncate">{vol.name}</div>
              <div className="text-[11px] text-[#6b6459]">{formatBytes(vol.size)}</div>
            </div>
            <button
              onClick={() => handleDelete(vol.name)}
              disabled={deleting !== null}
              className="shrink-0 rounded-md border border-red-500/20 px-2.5 py-1 text-[11px] font-medium text-red-400/70 hover:border-red-500/40 hover:text-red-400 disabled:opacity-40 transition-colors"
            >
              {deleting === vol.name ? "Deleting..." : "Delete"}
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <h3 className="mb-3 text-[12px] font-semibold uppercase tracking-wider text-[#6b6459]">{title}</h3>
      <div className="space-y-4 rounded-xl border border-[#2a2520] bg-[#1c1a17]/50 p-5">{children}</div>
    </div>
  );
}

function Label({ children }: { children: React.ReactNode }) {
  return <div className="text-[13px] font-medium text-[#e8e0d4]">{children}</div>;
}

function Desc({ children }: { children: React.ReactNode }) {
  return <div className="mt-0.5 text-[11px] text-[#6b6459]">{children}</div>;
}

function ToggleField({
  label,
  desc,
  value,
  onChange,
}: {
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
        className={cn("relative h-[22px] w-10 rounded-full transition-colors", value ? "bg-amber-500" : "bg-[#3a352f]")}
      >
        <div
          className={cn(
            "absolute top-[3px] h-4 w-4 rounded-full bg-white shadow-sm transition-transform",
            value ? "left-[22px]" : "left-[3px]",
          )}
        />
      </button>
    </div>
  );
}

function NumberField({
  label,
  desc,
  value,
  onChange,
  min,
  max,
}: {
  label: string;
  desc: string;
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
}) {
  const clamp = (v: number) => Math.max(min ?? -Infinity, Math.min(max ?? Infinity, v));
  return (
    <div className="flex items-center justify-between gap-4">
      <div className="min-w-0 flex-1">
        <Label>{label}</Label>
        <Desc>{desc}</Desc>
      </div>
      <div className="flex items-center gap-0 rounded-lg border border-[#2a2520] bg-[#1c1a17]">
        <button
          type="button"
          onClick={() => onChange(clamp(value - 1))}
          className="flex h-7 w-7 items-center justify-center text-[#9c9486] hover:text-[#e8e0d4] transition-colors"
        >
          −
        </button>
        <input
          type="number"
          value={value}
          min={min}
          max={max}
          onChange={(e) => {
            const v = parseInt(e.target.value, 10);
            if (!Number.isNaN(v)) onChange(clamp(v));
          }}
          className="w-14 bg-transparent px-1 py-1 text-center text-[12px] tabular-nums text-[#e8e0d4] outline-none [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none [-moz-appearance:textfield]"
        />
        <button
          type="button"
          onClick={() => onChange(clamp(value + 1))}
          className="flex h-7 w-7 items-center justify-center text-[#9c9486] hover:text-[#e8e0d4] transition-colors"
        >
          +
        </button>
      </div>
    </div>
  );
}

function TextField({
  label,
  desc,
  value,
  onChange,
}: {
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
        className="w-56 rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-1.5 text-[13px] text-[#e8e0d4] outline-none focus:border-amber-500/40 transition-colors"
      />
    </div>
  );
}

function SelectField({
  label,
  desc,
  value,
  onChange,
  options,
}: {
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
        className="rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-1.5 text-[13px] text-[#e8e0d4] outline-none focus:border-amber-500/40 transition-colors"
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </div>
  );
}

function ToggleGroup({
  value,
  onChange,
  options,
}: {
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
}) {
  return (
    <div className="flex rounded-md border border-[#2a2520]">
      {options.map((opt, i) => (
        <button
          key={opt.value}
          onClick={() => onChange(opt.value)}
          className={cn(
            "px-3 py-1.5 text-[11px] font-medium transition-colors",
            i > 0 && "border-l border-[#2a2520]",
            value === opt.value ? "bg-amber-500/[0.08] text-[#e8e0d4]" : "text-[#6b6459] hover:text-[#9c9486]",
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
      <span className="text-[12px] text-[#6b6459]">{label}</span>
      <span className="text-[12px] font-medium tabular-nums text-[#e8e0d4]">{value}</span>
    </div>
  );
}

const MODEL_OPTIONS = [
  { model: "gpt-5.4", backend: "codex", label: "GPT 5.4 (OpenAI)" },
  { model: "claude-opus-4-6", backend: "claude", label: "Claude Opus 4.6 (Anthropic)" },
  { model: "claude-sonnet-4-6", backend: "claude", label: "Claude Sonnet 4.6 (Anthropic)" },
];

function ModelPicker({
  model,
  backend,
  onChange,
}: {
  model: string;
  backend: string;
  onChange: (model: string, backend: string) => void;
}) {
  const match = MODEL_OPTIONS.find((o) => o.model === model);
  const value = match ? match.model : "__custom__";

  return (
    <div>
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0 flex-1">
          <Label>Model</Label>
          <Desc>AI model for pipeline and chat agents. Switching to GPT automatically uses the Codex backend.</Desc>
        </div>
        <select
          value={value}
          onChange={(e) => {
            const opt = MODEL_OPTIONS.find((o) => o.model === e.target.value);
            if (opt) onChange(opt.model, opt.backend);
          }}
          className="rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-1.5 text-[13px] text-[#e8e0d4] outline-none focus:border-amber-500/40 transition-colors"
        >
          {MODEL_OPTIONS.map((o) => (
            <option key={o.model} value={o.model}>
              {o.label}
            </option>
          ))}
          {!match && (
            <option value="__custom__">
              {model} ({backend})
            </option>
          )}
        </select>
      </div>
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
  const active = new Set(
    value
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean),
  );
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
                  ? "bg-amber-500/15 text-amber-400 ring-1 ring-inset ring-amber-500/20"
                  : "bg-[#1c1a17] text-[#6b6459] hover:bg-[#2a2520] hover:text-[#9c9486]",
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
