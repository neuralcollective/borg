import { useQueryClient } from "@tanstack/react-query";
import { Check, Copy, Link, Trash2, UserPlus, X } from "lucide-react";
import { useEffect, useState } from "react";
import {
  addProjectShare,
  apiBase,
  createProjectShareLink,
  removeProjectShare,
  revokeProjectShareLink,
  useProjectShareLinks,
  useProjectShares,
} from "@/lib/api";
import type { Project } from "@/lib/types";

function timeUntil(dateStr: string): string {
  const expires = new Date(`${dateStr}Z`);
  const now = new Date();
  const diff = expires.getTime() - now.getTime();
  if (diff <= 0) return "expired";
  const hours = Math.floor(diff / 3_600_000);
  if (hours < 1) return `${Math.ceil(diff / 60_000)}m left`;
  if (hours < 48) return `${hours}h left`;
  return `${Math.floor(hours / 24)}d left`;
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      onClick={() => {
        navigator.clipboard.writeText(text);
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      }}
      className="shrink-0 rounded p-1 text-zinc-500 hover:text-zinc-300 hover:bg-white/[0.06]"
      title="Copy to clipboard"
    >
      {copied ? <Check size={13} className="text-green-400" /> : <Copy size={13} />}
    </button>
  );
}

// ── People tab ────────────────────────────────────────────────────────────

function PeopleTab({ project }: { project: Project }) {
  const qc = useQueryClient();
  const { data: shares = [], isLoading } = useProjectShares(project.id);
  const [username, setUsername] = useState("");
  const [role, setRole] = useState<"viewer" | "editor">("viewer");
  const [error, setError] = useState("");
  const [adding, setAdding] = useState(false);

  async function handleAdd() {
    if (!username.trim()) return;
    setAdding(true);
    setError("");
    try {
      await addProjectShare(project.id, username.trim(), role);
      qc.invalidateQueries({ queryKey: ["project_shares", project.id] });
      setUsername("");
    } catch (e: any) {
      setError(e.message === "404" ? "User not found" : "Failed to share");
    } finally {
      setAdding(false);
    }
  }

  async function handleRemove(userId: number) {
    await removeProjectShare(project.id, userId);
    qc.invalidateQueries({ queryKey: ["project_shares", project.id] });
  }

  return (
    <div className="space-y-4">
      <div className="flex gap-2">
        <input
          type="text"
          value={username}
          onChange={(e) => setUsername(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleAdd()}
          placeholder="Username"
          className="flex-1 rounded-lg border border-white/[0.08] bg-zinc-800 px-3 py-1.5 text-[13px] text-zinc-200 outline-none placeholder:text-zinc-600 focus:border-blue-500/40"
        />
        <select
          value={role}
          onChange={(e) => setRole(e.target.value as "viewer" | "editor")}
          className="rounded-lg border border-white/[0.08] bg-zinc-800 px-2 py-1.5 text-[12px] text-zinc-300 outline-none"
        >
          <option value="viewer">Viewer</option>
          <option value="editor">Editor</option>
        </select>
        <button
          onClick={handleAdd}
          disabled={adding || !username.trim()}
          className="flex items-center gap-1.5 rounded-lg bg-blue-500/15 px-3 py-1.5 text-[12px] text-blue-300 ring-1 ring-inset ring-blue-500/20 hover:bg-blue-500/25 disabled:opacity-50"
        >
          <UserPlus size={13} />
          Add
        </button>
      </div>
      {error && <p className="text-[12px] text-red-400">{error}</p>}

      {isLoading ? (
        <p className="text-[12px] text-zinc-500">Loading...</p>
      ) : shares.length === 0 ? (
        <p className="text-[12px] text-zinc-500">No one has been given direct access yet.</p>
      ) : (
        <div className="space-y-1">
          {shares.map((s) => (
            <div key={s.id} className="flex items-center justify-between rounded-lg px-3 py-2 hover:bg-white/[0.03]">
              <div className="min-w-0">
                <span className="text-[13px] text-zinc-200">{s.display_name || s.username}</span>
                {s.display_name && s.display_name !== s.username && (
                  <span className="ml-1.5 text-[11px] text-zinc-500">{s.username}</span>
                )}
                <span className="ml-2 rounded bg-zinc-800 px-1.5 py-0.5 text-[10px] text-zinc-400">{s.role}</span>
              </div>
              <button
                onClick={() => handleRemove(s.user_id)}
                className="shrink-0 rounded p-1 text-zinc-600 hover:text-red-400 hover:bg-red-500/10"
                title="Remove access"
              >
                <Trash2 size={13} />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Links tab ─────────────────────────────────────────────────────────────

function LinksTab({ project }: { project: Project }) {
  const qc = useQueryClient();
  const { data: links = [], isLoading } = useProjectShareLinks(project.id);
  const [label, setLabel] = useState("");
  const [hours, setHours] = useState(72);
  const [creating, setCreating] = useState(false);
  const [newToken, setNewToken] = useState<string | null>(null);

  async function handleCreate() {
    setCreating(true);
    try {
      const result = await createProjectShareLink(project.id, label.trim(), hours);
      setNewToken(result.token);
      setLabel("");
      qc.invalidateQueries({ queryKey: ["project_share_links", project.id] });
    } finally {
      setCreating(false);
    }
  }

  async function handleRevoke(linkId: number) {
    await revokeProjectShareLink(project.id, linkId);
    qc.invalidateQueries({ queryKey: ["project_share_links", project.id] });
  }

  const shareUrl = (token: string) => `${window.location.origin}${apiBase()}/#/shared/${token}`;
  const activeLinks = links.filter((l) => !l.revoked);

  return (
    <div className="space-y-4">
      <div className="flex gap-2">
        <input
          type="text"
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          placeholder="Label (optional)"
          className="flex-1 rounded-lg border border-white/[0.08] bg-zinc-800 px-3 py-1.5 text-[13px] text-zinc-200 outline-none placeholder:text-zinc-600 focus:border-blue-500/40"
        />
        <select
          value={hours}
          onChange={(e) => setHours(Number(e.target.value))}
          className="rounded-lg border border-white/[0.08] bg-zinc-800 px-2 py-1.5 text-[12px] text-zinc-300 outline-none"
        >
          <option value={24}>24 hours</option>
          <option value={72}>3 days</option>
          <option value={168}>7 days</option>
          <option value={720}>30 days</option>
        </select>
        <button
          onClick={handleCreate}
          disabled={creating}
          className="flex items-center gap-1.5 rounded-lg bg-blue-500/15 px-3 py-1.5 text-[12px] text-blue-300 ring-1 ring-inset ring-blue-500/20 hover:bg-blue-500/25 disabled:opacity-50"
        >
          <Link size={13} />
          Create
        </button>
      </div>

      {newToken && (
        <div className="rounded-lg border border-green-500/20 bg-green-500/5 px-3 py-2">
          <p className="mb-1 text-[11px] text-green-400">Link created! Copy it now:</p>
          <div className="flex items-center gap-2">
            <code className="flex-1 truncate text-[11px] text-green-300">{shareUrl(newToken)}</code>
            <CopyButton text={shareUrl(newToken)} />
          </div>
        </div>
      )}

      {isLoading ? (
        <p className="text-[12px] text-zinc-500">Loading...</p>
      ) : activeLinks.length === 0 ? (
        <p className="text-[12px] text-zinc-500">No active share links.</p>
      ) : (
        <div className="space-y-1">
          {activeLinks.map((l) => (
            <div key={l.id} className="flex items-center justify-between rounded-lg px-3 py-2 hover:bg-white/[0.03]">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <Link size={12} className="shrink-0 text-zinc-500" />
                  <span className="truncate text-[13px] text-zinc-300">{l.label || "Untitled link"}</span>
                  <span className="shrink-0 text-[10px] text-zinc-500">{timeUntil(l.expires_at)}</span>
                </div>
                <div className="mt-0.5 flex items-center gap-1">
                  <code className="truncate text-[10px] text-zinc-600">{shareUrl(l.token)}</code>
                  <CopyButton text={shareUrl(l.token)} />
                </div>
              </div>
              <button
                onClick={() => handleRevoke(l.id)}
                className="shrink-0 rounded p-1 text-zinc-600 hover:text-red-400 hover:bg-red-500/10"
                title="Revoke link"
              >
                <Trash2 size={13} />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Main dialog ───────────────────────────────────────────────────────────

export function ProjectShareDialog({ project, onClose }: { project: Project; onClose: () => void }) {
  const [tab, setTab] = useState<"people" | "links">("people");

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="flex max-h-[70vh] w-full max-w-lg flex-col overflow-hidden rounded-2xl border border-white/[0.08] bg-[#0e0e10] shadow-2xl mx-4">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-white/[0.07] px-5 py-4">
          <h3 className="text-[14px] font-semibold text-zinc-100">Share "{project.name}"</h3>
          <button onClick={onClose} className="shrink-0 rounded-lg p-2 text-zinc-500 hover:bg-white/[0.06]">
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Tabs */}
        <div className="flex border-b border-white/[0.07] px-5">
          {(["people", "links"] as const).map((t) => (
            <button
              key={t}
              onClick={() => setTab(t)}
              className={`px-3 py-2.5 text-[12px] font-medium transition-colors ${
                tab === t ? "border-b-2 border-blue-400 text-blue-400" : "text-zinc-500 hover:text-zinc-300"
              }`}
            >
              {t === "people" ? "People" : "Share Links"}
            </button>
          ))}
        </div>

        {/* Content */}
        <div className="min-h-0 flex-1 overflow-auto p-5">
          {tab === "people" ? <PeopleTab project={project} /> : <LinksTab project={project} />}
        </div>
      </div>
    </div>
  );
}
