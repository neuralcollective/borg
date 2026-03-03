import { useEffect, useMemo, useRef, useState } from "react";
import {
  useProjectDetail,
  useProjectTasks,
  useProjectDocuments,
  useProjectDeadlines,
  useProjectAudit,
  useUpdateProject,
  useDeleteProject,
  useTaskStream,
  getProjectChatMessages,
  sendProjectChat,
  checkConflicts,
  getTaskStructuredData,
  createDeadline,
  updateDeadline,
  deleteDeadline,
  sseUrl,
  tokenReady,
} from "@/lib/api";
import type { ConflictHit, Deadline, AuditEvent } from "@/lib/api";
import type { Project, ProjectTask, ProjectDocument } from "@/lib/types";
import { StatusBadge } from "./status-badge";
import { PhaseTracker } from "./phase-tracker";
import { BorgingIndicator } from "./borging";
import { ChatMarkdown } from "./chat-markdown";
import { useDictation } from "@/lib/dictation";
import { cn } from "@/lib/utils";
import { retryTask, patchTask, approveTask, rejectTask, requestRevision, getRevisionHistory, useFullModes, useTemplates } from "@/lib/api";
import type { RevisionHistory } from "@/lib/api";
import { useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, ChevronDown, ChevronUp, Edit2, Check, X, FileText, RotateCcw, Mic, MicOff, Trash2 } from "lucide-react";

type ChatMessage = {
  role: "user" | "assistant";
  sender?: string;
  text: string;
  ts: string | number;
  thread?: string;
};

interface MatterDetailProps {
  projectId: number;
  onDocumentSelect?: (doc: ProjectDocument) => void;
  onDelete?: () => void;
}

// ── Inline edit field ────────────────────────────────────────────────────────

function InlineField({
  label,
  value,
  onSave,
  placeholder,
}: {
  label: string;
  value: string | undefined;
  onSave: (v: string) => void;
  placeholder?: string;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value ?? "");

  function commit() {
    onSave(draft);
    setEditing(false);
  }

  function cancel() {
    setDraft(value ?? "");
    setEditing(false);
  }

  if (editing) {
    return (
      <div className="flex flex-col gap-0.5">
        <span className="text-[10px] text-zinc-600">{label}</span>
        <div className="flex items-center gap-1">
          <input
            autoFocus
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") commit();
              if (e.key === "Escape") cancel();
            }}
            className="flex-1 rounded border border-white/[0.12] bg-white/[0.04] px-2 py-0.5 text-[12px] text-zinc-200 outline-none focus:border-blue-500/40"
          />
          <button onClick={commit} className="text-emerald-400 hover:text-emerald-300 transition-colors">
            <Check className="h-3 w-3" />
          </button>
          <button onClick={cancel} className="text-zinc-600 hover:text-zinc-400 transition-colors">
            <X className="h-3 w-3" />
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="group flex flex-col gap-0.5">
      <span className="text-[10px] text-zinc-600">{label}</span>
      <div className="flex items-center gap-1.5">
        <span className="text-[12px] text-zinc-300">{value || <span className="text-zinc-600">{placeholder ?? "—"}</span>}</span>
        <button
          onClick={() => { setDraft(value ?? ""); setEditing(true); }}
          className="opacity-0 group-hover:opacity-100 transition-opacity text-zinc-600 hover:text-zinc-400"
        >
          <Edit2 className="h-2.5 w-2.5" />
        </button>
      </div>
    </div>
  );
}

function SelectField({
  label,
  value,
  options,
  onSave,
}: {
  label: string;
  value: string | undefined;
  options: string[];
  onSave: (v: string) => void;
}) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-[10px] text-zinc-600">{label}</span>
      <select
        value={value || ""}
        onChange={(e) => onSave(e.target.value)}
        className="rounded border border-white/[0.12] bg-white/[0.04] px-2 py-0.5 text-[12px] text-zinc-200 outline-none focus:border-blue-500/40"
      >
        <option value="">unset</option>
        {options.map((opt) => (
          <option key={opt} value={opt}>
            {opt.replace(/_/g, " ")}
          </option>
        ))}
      </select>
    </div>
  );
}

// ── Timeline item ─────────────────────────────────────────────────────────────

type TimelineItem = {
  id: string;
  ts: string;
  label: string;
  sub?: string;
  kind: "task_created" | "status_change" | "document";
};

function buildTimeline(tasks: ProjectTask[], docs: ProjectDocument[]): TimelineItem[] {
  const items: TimelineItem[] = [];

  for (const t of tasks) {
    items.push({
      id: `task-${t.id}`,
      ts: t.created_at,
      label: t.title,
      sub: `Task #${t.id} created`,
      kind: "task_created",
    });
  }

  for (const d of docs) {
    items.push({
      id: `doc-${d.task_id}-${d.file_name}`,
      ts: d.created_at,
      label: d.file_name,
      sub: `from task #${d.task_id} · ${d.task_title}`,
      kind: "document",
    });
  }

  items.sort((a, b) => (a.ts < b.ts ? -1 : a.ts > b.ts ? 1 : 0));
  return items;
}

function fmtDate(ts: string): string {
  if (!ts) return "";
  try {
    return new Date(ts).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" });
  } catch {
    return ts;
  }
}

function fmtDateTime(ts: string): string {
  if (!ts) return "";
  try {
    return new Date(ts).toLocaleString("en-US", { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
  } catch {
    return ts;
  }
}

function formatDuration(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  const rm = m % 60;
  return rm > 0 ? `${h}h ${rm}m` : `${h}h`;
}

// ── Matter header ─────────────────────────────────────────────────────────────

function MatterHeader({ project, onDelete }: { project: Project; onDelete?: () => void }) {
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [exportingAll, setExportingAll] = useState(false);
  const [exportMenu, setExportMenu] = useState(false);
  const [exportTemplateId, setExportTemplateId] = useState<number | null>(project.default_template_id ?? null);
  const { data: templates = [] } = useTemplates("template");

  async function exportAll(format: "pdf" | "docx") {
    setExportMenu(false);
    setExportingAll(true);
    try {
      const { apiBase, authHeaders, tokenReady } = await import("@/lib/api");
      await tokenReady;
      const params = new URLSearchParams({ format, toc: "true" });
      if (exportTemplateId) params.set("template_id", String(exportTemplateId));
      const res = await fetch(`${apiBase()}/api/projects/${project.id}/export-all?${params}`, {
        headers: authHeaders(),
      });
      if (!res.ok) {
        alert(`Export failed: ${await res.text()}`);
        return;
      }
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${project.name.replace(/[^a-zA-Z0-9 -]/g, "").trim()}-export.zip`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
    } finally {
      setExportingAll(false);
    }
  }

  return (
    <div className="border-b border-white/[0.06] px-5 py-3">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-[14px] font-semibold text-zinc-100">{project.name}</h2>
            {project.status && <StatusBadge status={project.status} />}
            {project.matter_type && (
              <span className="rounded bg-violet-500/10 px-1.5 py-0.5 text-[9px] font-medium text-violet-400">
                {project.matter_type}
              </span>
            )}
          </div>
          <div className="mt-1.5 flex flex-wrap gap-x-4 gap-y-0.5 text-[11px] text-zinc-500">
            {project.case_number && (
              <span>
                <span className="text-zinc-600">case</span>{" "}
                <span className="font-mono">{project.case_number}</span>
              </span>
            )}
            {project.client_name && (
              <span>
                <span className="text-zinc-600">client</span> {project.client_name}
              </span>
            )}
            {project.jurisdiction && (
              <span>
                <span className="text-zinc-600">jurisdiction</span> {project.jurisdiction}
              </span>
            )}
            {project.deadline && (
              <span>
                <span className="text-zinc-600">deadline</span> {fmtDate(project.deadline)}
              </span>
            )}
          </div>
        </div>
        <div className="flex items-center gap-1.5 shrink-0">
          <div className="relative">
            <button
              onClick={() => setExportMenu(v => !v)}
              disabled={exportingAll}
              className="rounded border border-white/[0.08] px-2 py-1 text-[10px] text-zinc-500 hover:border-blue-500/30 hover:text-blue-400 transition-colors disabled:opacity-50"
              title="Export all documents"
            >
              {exportingAll ? "Exporting..." : "Export All"}
            </button>
            {exportMenu && (
              <div className="absolute right-0 top-full z-50 mt-1 w-52 rounded border border-white/[0.1] bg-zinc-900 shadow-xl">
                {templates.length > 0 && (
                  <div className="border-b border-white/[0.06] px-3 py-2">
                    <label className="text-[9px] text-zinc-500 block mb-0.5">Template</label>
                    <select
                      value={exportTemplateId ?? ""}
                      onChange={(e) => setExportTemplateId(e.target.value ? Number(e.target.value) : null)}
                      className="w-full rounded border border-white/[0.08] bg-zinc-800 px-1.5 py-1 text-[10px] text-zinc-300 outline-none"
                    >
                      <option value="">None (default)</option>
                      {templates.map((t) => (
                        <option key={t.id} value={t.id}>{t.file_name}</option>
                      ))}
                    </select>
                  </div>
                )}
                <button onClick={() => exportAll("docx")} className="flex w-full items-center px-3 py-2 text-left text-[11px] text-zinc-300 hover:bg-white/[0.06]">
                  Export as DOCX (ZIP)
                </button>
                <button onClick={() => exportAll("pdf")} className="flex w-full items-center px-3 py-2 text-left text-[11px] text-zinc-300 hover:bg-white/[0.06]">
                  Export as PDF (ZIP)
                </button>
              </div>
            )}
          </div>
          {onDelete && (
            confirmDelete ? (
              <div className="flex items-center gap-1.5">
                <span className="text-[10px] text-red-400">Delete?</span>
                <button onClick={onDelete} className="rounded px-1.5 py-0.5 text-[10px] bg-red-500/20 text-red-400 hover:bg-red-500/30">Yes</button>
                <button onClick={() => setConfirmDelete(false)} className="rounded px-1.5 py-0.5 text-[10px] bg-zinc-700 text-zinc-400 hover:bg-zinc-600">No</button>
              </div>
            ) : (
              <button onClick={() => setConfirmDelete(true)} className="shrink-0 rounded p-1 text-zinc-600 hover:text-red-400 hover:bg-red-500/10" title="Delete matter">
                <Trash2 size={14} />
              </button>
            )
          )}
        </div>
      </div>
    </div>
  );
}

// ── Metadata panel ────────────────────────────────────────────────────────────

function MetadataPanel({ project, projectId }: { project: Project; projectId: number }) {
  const [open, setOpen] = useState(false);
  const [conflicts, setConflicts] = useState<ConflictHit[]>([]);
  const { mutate: update } = useUpdateProject(projectId);
  const { data: templates = [] } = useTemplates("template");

  useEffect(() => {
    if (!project.client_name && !project.opposing_counsel) return;
    checkConflicts(project.client_name || "", project.opposing_counsel || "", projectId)
      .then(setConflicts)
      .catch(() => setConflicts([]));
  }, [project.client_name, project.opposing_counsel, projectId]);

  function save(field: keyof Project) {
    return (value: string) => update({ [field]: value });
  }

  return (
    <div className="border-b border-white/[0.06]">
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-5 py-2 text-left hover:bg-white/[0.02] transition-colors"
      >
        <span className="text-[11px] font-medium text-zinc-500">Matter Details</span>
        {conflicts.length > 0 && (
          <AlertTriangle className="h-3 w-3 text-amber-400" />
        )}
        <span className="ml-auto text-zinc-600">
          {open ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
        </span>
      </button>
      {open && (
        <>
          {conflicts.length > 0 && (
            <div className="mx-5 mb-3 rounded-md border border-amber-500/30 bg-amber-500/10 p-2.5">
              <div className="mb-1 flex items-center gap-1.5 text-[11px] font-medium text-amber-400">
                <AlertTriangle className="h-3 w-3" />
                Potential Conflict
              </div>
              {conflicts.map((c, i) => (
                <p key={i} className="text-[10px] text-amber-300/80">
                  <span className="font-medium">{c.party_name}</span>
                  {" "}({c.party_role === "opposing_counsel" ? "opposing" : c.party_role})
                  {" in "}<span className="font-medium">{c.project_name}</span>
                </p>
              ))}
            </div>
          )}
          <div className="grid grid-cols-2 gap-x-6 gap-y-3 px-5 pb-4 sm:grid-cols-3">
            <InlineField label="Client" value={project.client_name} onSave={save("client_name")} placeholder="unset" />
            <InlineField label="Case Number" value={project.case_number} onSave={save("case_number")} placeholder="unset" />
            <InlineField label="Jurisdiction" value={project.jurisdiction} onSave={save("jurisdiction")} placeholder="unset" />
            <InlineField label="Matter Type" value={project.matter_type} onSave={save("matter_type")} placeholder="unset" />
            <InlineField label="Opposing Counsel" value={project.opposing_counsel} onSave={save("opposing_counsel")} placeholder="unset" />
            <InlineField label="Deadline" value={project.deadline} onSave={save("deadline")} placeholder="unset" />
            <InlineField label="Privilege Level" value={project.privilege_level} onSave={save("privilege_level")} placeholder="unset" />
            <SelectField
              label="Status"
              value={project.status}
              options={["active", "pending", "on_hold", "closed", "archived"]}
              onSave={save("status")}
            />
            {templates.length > 0 && (
              <div className="flex flex-col gap-0.5">
                <span className="text-[10px] text-zinc-600">Default Template</span>
                <select
                  value={project.default_template_id ?? ""}
                  onChange={(e) => {
                    const v = e.target.value ? Number(e.target.value) : null;
                    update({ default_template_id: v });
                  }}
                  className="rounded border border-white/[0.12] bg-white/[0.04] px-2 py-0.5 text-[12px] text-zinc-200 outline-none focus:border-blue-500/40"
                >
                  <option value="">none</option>
                  {templates.map((t) => (
                    <option key={t.id} value={t.id}>{t.file_name}</option>
                  ))}
                </select>
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
}

// ── Deadlines panel ──────────────────────────────────────────────────────────

function deadlineUrgency(dueDate: string): "overdue" | "urgent" | "normal" {
  const due = new Date(dueDate + "T23:59:59");
  const now = new Date();
  if (due < now) return "overdue";
  const diff = due.getTime() - now.getTime();
  if (diff < 7 * 24 * 60 * 60 * 1000) return "urgent";
  return "normal";
}

const urgencyStyle: Record<string, string> = {
  overdue: "border-red-500/30 bg-red-500/10 text-red-400",
  urgent: "border-amber-500/30 bg-amber-500/10 text-amber-400",
  normal: "border-white/[0.06] bg-white/[0.02] text-zinc-300",
};

function DeadlinesPanel({ projectId }: { projectId: number }) {
  const { data: deadlines = [], refetch } = useProjectDeadlines(projectId);
  const [open, setOpen] = useState(false);
  const [adding, setAdding] = useState(false);
  const [newLabel, setNewLabel] = useState("");
  const [newDate, setNewDate] = useState("");
  const [newBasis, setNewBasis] = useState("");

  const pending = deadlines.filter(d => d.status === "pending");
  const completed = deadlines.filter(d => d.status !== "pending");
  const hasOverdue = pending.some(d => deadlineUrgency(d.due_date) === "overdue");

  async function handleAdd() {
    if (!newLabel.trim() || !newDate.trim()) return;
    await createDeadline(projectId, newLabel.trim(), newDate.trim(), newBasis.trim());
    setNewLabel(""); setNewDate(""); setNewBasis(""); setAdding(false);
    refetch();
  }

  async function handleComplete(d: Deadline) {
    await updateDeadline(projectId, d.id, { status: "completed" });
    refetch();
  }

  async function handleDelete(d: Deadline) {
    await deleteDeadline(projectId, d.id);
    refetch();
  }

  return (
    <div className="border-b border-white/[0.06]">
      <button
        onClick={() => setOpen(v => !v)}
        className="flex w-full items-center gap-2 px-5 py-2 text-left hover:bg-white/[0.02] transition-colors"
      >
        <span className="text-[11px] font-medium text-zinc-500">Deadlines</span>
        {pending.length > 0 && (
          <span className={cn("rounded-full px-1.5 py-0.5 text-[9px] font-medium", hasOverdue ? "bg-red-500/20 text-red-400" : "bg-zinc-700 text-zinc-400")}>
            {pending.length}
          </span>
        )}
        <span className="ml-auto text-zinc-600">
          {open ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
        </span>
      </button>
      {open && (
        <div className="px-5 pb-4 space-y-1.5">
          {pending.map(d => {
            const urg = deadlineUrgency(d.due_date);
            return (
              <div key={d.id} className={cn("flex items-center gap-2 rounded border px-2.5 py-1.5 text-[11px]", urgencyStyle[urg])}>
                <button onClick={() => handleComplete(d)} className="shrink-0 opacity-60 hover:opacity-100" title="Mark complete">
                  <Check className="h-3 w-3" />
                </button>
                <span className="font-medium">{d.label}</span>
                <span className="font-mono text-[10px] opacity-70">{d.due_date}</span>
                {d.rule_basis && <span className="text-[10px] opacity-60 truncate">{d.rule_basis}</span>}
                <button onClick={() => handleDelete(d)} className="ml-auto shrink-0 opacity-40 hover:opacity-100 text-zinc-500 hover:text-red-400" title="Delete">
                  <X className="h-3 w-3" />
                </button>
              </div>
            );
          })}
          {completed.length > 0 && (
            <div className="mt-2 space-y-1">
              {completed.map(d => (
                <div key={d.id} className="flex items-center gap-2 rounded border border-white/[0.04] px-2.5 py-1 text-[10px] text-zinc-600 line-through">
                  <span>{d.label}</span>
                  <span className="font-mono">{d.due_date}</span>
                  <button onClick={() => handleDelete(d)} className="ml-auto shrink-0 opacity-40 hover:opacity-100 hover:text-red-400" title="Delete">
                    <X className="h-2.5 w-2.5" />
                  </button>
                </div>
              ))}
            </div>
          )}
          {adding ? (
            <div className="mt-2 space-y-1.5 rounded border border-white/[0.08] bg-white/[0.02] p-2">
              <input
                autoFocus
                value={newLabel}
                onChange={e => setNewLabel(e.target.value)}
                placeholder="Label (e.g. Motion to Dismiss)"
                className="w-full rounded border border-white/[0.08] bg-black/30 px-2 py-1 text-[11px] text-zinc-200 outline-none focus:border-blue-500/40"
              />
              <div className="flex gap-1.5">
                <input
                  type="date"
                  value={newDate}
                  onChange={e => setNewDate(e.target.value)}
                  className="flex-1 rounded border border-white/[0.08] bg-black/30 px-2 py-1 text-[11px] text-zinc-200 outline-none focus:border-blue-500/40"
                />
                <input
                  value={newBasis}
                  onChange={e => setNewBasis(e.target.value)}
                  placeholder="Rule basis (optional)"
                  className="flex-1 rounded border border-white/[0.08] bg-black/30 px-2 py-1 text-[11px] text-zinc-200 outline-none focus:border-blue-500/40"
                />
              </div>
              <div className="flex gap-1.5">
                <button onClick={handleAdd} className="rounded bg-blue-500/20 px-2 py-0.5 text-[10px] text-blue-300 hover:bg-blue-500/30">Add</button>
                <button onClick={() => setAdding(false)} className="rounded bg-zinc-700 px-2 py-0.5 text-[10px] text-zinc-400 hover:bg-zinc-600">Cancel</button>
              </div>
            </div>
          ) : (
            <button
              onClick={() => setAdding(true)}
              className="mt-1 text-[10px] text-zinc-600 hover:text-zinc-400 transition-colors"
            >
              + Add deadline
            </button>
          )}
        </div>
      )}
    </div>
  );
}

// ── Timeline tab ──────────────────────────────────────────────────────────────

function TimelineTab({ projectId }: { projectId: number }) {
  const { data: tasks = [] } = useProjectTasks(projectId);
  const { data: docs = [] } = useProjectDocuments(projectId);
  const items = useMemo(() => buildTimeline(tasks, docs), [tasks, docs]);

  if (items.length === 0) {
    return (
      <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">
        No activity yet.
      </div>
    );
  }

  return (
    <div className="space-y-0 overflow-y-auto p-4">
      {items.map((item, idx) => (
        <div key={item.id} className="flex gap-3">
          <div className="flex flex-col items-center">
            <div
              className={cn(
                "mt-1 h-2 w-2 shrink-0 rounded-full",
                item.kind === "document"
                  ? "bg-blue-400/60"
                  : item.kind === "task_created"
                    ? "bg-emerald-400/60"
                    : "bg-zinc-500/60"
              )}
            />
            {idx < items.length - 1 && (
              <div className="mt-1 w-px flex-1 bg-white/[0.06]" style={{ minHeight: "24px" }} />
            )}
          </div>
          <div className="pb-4 min-w-0">
            <div className="text-[12px] font-medium text-zinc-300 truncate">{item.label}</div>
            <div className="mt-0.5 text-[11px] text-zinc-600">
              {item.sub} · {fmtDateTime(item.ts)}
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

// ── Documents tab ─────────────────────────────────────────────────────────────

function DocumentsTab({
  projectId,
  onDocumentSelect,
}: {
  projectId: number;
  onDocumentSelect?: (doc: ProjectDocument) => void;
}) {
  const { data: docs = [], isLoading } = useProjectDocuments(projectId);

  if (isLoading) {
    return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">Loading...</div>;
  }

  if (docs.length === 0) {
    return (
      <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">
        No documents yet. Run a task to generate research or drafts.
      </div>
    );
  }

  return (
    <div className="grid grid-cols-1 gap-2 p-4 sm:grid-cols-2">
      {docs.map((doc) => (
        <button
          key={`${doc.task_id}-${doc.file_name}`}
          onClick={() => onDocumentSelect?.(doc)}
          className="flex flex-col gap-1.5 rounded-lg border border-white/[0.06] bg-white/[0.02] p-3 text-left transition-colors hover:border-white/[0.1] hover:bg-white/[0.04]"
        >
          <div className="flex items-center gap-2">
            <FileText className="h-3.5 w-3.5 shrink-0 text-blue-400/60" />
            <span className="text-[12px] font-medium text-zinc-200 truncate">{doc.file_name}</span>
            <StatusBadge status={doc.task_status} />
          </div>
          <div className="text-[11px] text-zinc-600 truncate">
            #{doc.task_id} · {doc.task_title}
          </div>
          {doc.branch && (
            <div className="font-mono text-[10px] text-zinc-700 truncate">{doc.branch}</div>
          )}
        </button>
      ))}
    </div>
  );
}

// ── Task stream mini-panel ────────────────────────────────────────────────────

function TaskStreamMini({ taskId }: { taskId: number }) {
  const isActive = true;
  const { events, streaming } = useTaskStream(taskId, isActive);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [events.length]);

  const lines = useMemo(() => {
    return events
      .filter((e) => e.type === "assistant" && e.message?.content)
      .map((e) => {
        const content = e.message!.content;
        if (typeof content === "string") return content;
        if (Array.isArray(content)) {
          return content
            .filter((b): b is { type: string; text?: string } => b.type === "text" && !!b.text)
            .map((b) => b.text!)
            .join("");
        }
        return "";
      })
      .filter(Boolean);
  }, [events]);

  if (!streaming && lines.length === 0) return null;

  return (
    <div className="mt-2 rounded border border-white/[0.06] bg-black/30">
      <div className="flex items-center gap-2 border-b border-white/[0.04] px-2.5 py-1.5">
        {streaming && (
          <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-blue-400" />
        )}
        <span className="text-[10px] font-medium text-zinc-500">
          {streaming ? "Live output" : "Output"}
        </span>
      </div>
      <div
        ref={scrollRef}
        className="max-h-[200px] overflow-y-auto p-2.5 font-mono text-[10px] leading-relaxed text-zinc-500 whitespace-pre-wrap"
      >
        {lines.length > 0 ? lines[lines.length - 1].slice(-500) : (
          <span className="text-zinc-700">Waiting for output…</span>
        )}
      </div>
    </div>
  );
}

// ── Structured data panel ─────────────────────────────────────────────────────

function StructuredDataPanel({ taskId }: { taskId: number }) {
  const [data, setData] = useState<Record<string, unknown> | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    getTaskStructuredData(taskId)
      .then(setData)
      .catch(() => setData(null))
      .finally(() => setLoading(false));
  }, [taskId]);

  if (loading) return <div className="mt-2 text-[10px] text-zinc-600">Loading results…</div>;
  if (!data) return <div className="mt-2 text-[10px] text-zinc-600">No structured data.</div>;

  const summary = data.summary as string | undefined;
  const riskFlags = data.risk_flags as { severity: string; issue: string; section?: string; recommendation?: string }[] | undefined;
  const keyObligations = data.key_obligations as { party: string; obligation: string; section?: string }[] | undefined;
  const parties = data.parties as string[] | undefined;
  const complianceItems = data.compliance_items as { requirement: string; status: string; evidence?: string; recommendation?: string }[] | undefined;
  const deadlines = data.deadlines as { date: string; description: string; authority?: string }[] | undefined;
  const regulations = data.regulations as { name: string; jurisdiction?: string; status?: string }[] | undefined;

  const severityColor: Record<string, string> = {
    high: "text-red-400 bg-red-500/10 border-red-500/20",
    medium: "text-amber-400 bg-amber-500/10 border-amber-500/20",
    low: "text-emerald-400 bg-emerald-500/10 border-emerald-500/20",
  };

  const complianceColor: Record<string, string> = {
    compliant: "text-emerald-400",
    non_compliant: "text-red-400",
    partial: "text-amber-400",
    unknown: "text-zinc-500",
  };

  return (
    <div className="mt-2 rounded border border-white/[0.06] bg-black/20 p-3 space-y-3">
      {summary && (
        <p className="text-[11px] text-zinc-300 leading-relaxed">{summary}</p>
      )}

      {parties && parties.length > 0 && (
        <div>
          <div className="text-[10px] font-medium text-zinc-500 mb-1">Parties</div>
          <div className="flex flex-wrap gap-1.5">
            {parties.map((p, i) => (
              <span key={i} className="rounded bg-white/[0.06] px-1.5 py-0.5 text-[10px] text-zinc-300">{p}</span>
            ))}
          </div>
        </div>
      )}

      {keyObligations && keyObligations.length > 0 && (
        <div>
          <div className="text-[10px] font-medium text-zinc-500 mb-1">Key Obligations</div>
          <div className="space-y-1">
            {keyObligations.map((o, i) => (
              <div key={i} className="rounded bg-white/[0.03] px-2 py-1.5 text-[10px]">
                <span className="text-zinc-400 font-medium">{o.party}</span>
                <span className="text-zinc-500"> — </span>
                <span className="text-zinc-300">{o.obligation}</span>
                {o.section && <span className="text-zinc-600 ml-1">§{o.section}</span>}
              </div>
            ))}
          </div>
        </div>
      )}

      {riskFlags && riskFlags.length > 0 && (
        <div>
          <div className="text-[10px] font-medium text-zinc-500 mb-1">Risk Flags</div>
          <div className="space-y-1">
            {riskFlags.map((r, i) => (
              <div key={i} className={cn("rounded border px-2 py-1.5 text-[10px]", severityColor[r.severity] || severityColor.low)}>
                <div className="flex items-center gap-1.5">
                  <span className="font-medium uppercase text-[9px]">{r.severity}</span>
                  <span className="text-zinc-300">{r.issue}</span>
                  {r.section && <span className="text-zinc-600 ml-auto">§{r.section}</span>}
                </div>
                {r.recommendation && <div className="mt-0.5 text-zinc-400">{r.recommendation}</div>}
              </div>
            ))}
          </div>
        </div>
      )}

      {regulations && regulations.length > 0 && (
        <div>
          <div className="text-[10px] font-medium text-zinc-500 mb-1">Regulations</div>
          <div className="space-y-1">
            {regulations.map((r, i) => (
              <div key={i} className="flex items-center gap-2 rounded bg-white/[0.03] px-2 py-1.5 text-[10px]">
                <span className="text-zinc-300 font-medium">{r.name}</span>
                {r.jurisdiction && <span className="text-zinc-500">{r.jurisdiction}</span>}
                {r.status && <span className="ml-auto text-zinc-500">{r.status}</span>}
              </div>
            ))}
          </div>
        </div>
      )}

      {complianceItems && complianceItems.length > 0 && (
        <div>
          <div className="text-[10px] font-medium text-zinc-500 mb-1">Compliance</div>
          <div className="space-y-1">
            {complianceItems.map((c, i) => (
              <div key={i} className="rounded bg-white/[0.03] px-2 py-1.5 text-[10px]">
                <div className="flex items-center gap-2">
                  <span className={cn("font-medium", complianceColor[c.status] || "text-zinc-500")}>
                    {c.status === "compliant" ? "✓" : c.status === "non_compliant" ? "✗" : "○"}
                  </span>
                  <span className="text-zinc-300">{c.requirement}</span>
                </div>
                {c.evidence && <div className="mt-0.5 pl-4 text-zinc-500">{c.evidence}</div>}
                {c.recommendation && <div className="mt-0.5 pl-4 text-zinc-400">{c.recommendation}</div>}
              </div>
            ))}
          </div>
        </div>
      )}

      {deadlines && deadlines.length > 0 && (
        <div>
          <div className="text-[10px] font-medium text-zinc-500 mb-1">Deadlines</div>
          <div className="space-y-1">
            {deadlines.map((d, i) => (
              <div key={i} className="flex items-center gap-2 rounded bg-white/[0.03] px-2 py-1.5 text-[10px]">
                <span className="font-mono text-zinc-400">{d.date}</span>
                <span className="text-zinc-300">{d.description}</span>
                {d.authority && <span className="ml-auto text-zinc-500">{d.authority}</span>}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

// ── Tasks tab ─────────────────────────────────────────────────────────────────

const ACTIVE_STATUSES = new Set(["implement", "review", "validate", "lint_fix", "rebase", "spec", "qa", "qa_fix", "retry"]);

function TasksTab({ projectId }: { projectId: number }) {
  const { data: tasks = [], isLoading } = useProjectTasks(projectId);
  const { data: fullModes = [] } = useFullModes();
  const queryClient = useQueryClient();
  const [retryingId, setRetryingId] = useState<number | null>(null);
  const [expandedStream, setExpandedStream] = useState<number | null>(null);
  const [expandedResults, setExpandedResults] = useState<number | null>(null);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [editTitle, setEditTitle] = useState("");
  const [reviewingId, setReviewingId] = useState<number | null>(null);
  const [revisionFeedback, setRevisionFeedback] = useState("");
  const [citationsId, setCitationsId] = useState<number | null>(null);
  const [revisionsId, setRevisionsId] = useState<number | null>(null);
  const [editDesc, setEditDesc] = useState("");

  if (isLoading) {
    return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">Loading...</div>;
  }

  if (tasks.length === 0) {
    return (
      <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">
        No tasks linked to this matter.
      </div>
    );
  }

  const totalSecs = tasks.reduce((sum, t) => sum + (t.duration_secs ?? 0), 0);

  return (
    <div className="space-y-2 p-4">
      {totalSecs > 0 && (
        <div className="text-[11px] text-zinc-500 pb-1">
          Total time: <span className="text-zinc-300">{formatDuration(totalSecs)}</span>
          {" · "}{tasks.filter(t => t.duration_secs != null).length} tracked
        </div>
      )}
      {tasks.map((task) => {
        const isActive = ACTIVE_STATUSES.has(task.status);
        const isHumanReview = fullModes.some((m) =>
          m.name === task.mode &&
          m.phases.some((p) => p.name === task.status && p.phase_type === "human_review")
        );
        const reviewPhaseInstruction = isHumanReview
          ? fullModes.find((m) => m.name === task.mode)
              ?.phases.find((p) => p.name === task.status)?.instruction
          : undefined;
        return (
          <div
            key={task.id}
            className={cn(
              "rounded-lg border p-3",
              isHumanReview
                ? "border-emerald-500/20 bg-emerald-500/[0.03]"
                : "border-white/[0.06] bg-white/[0.02]"
            )}
          >
            <div className="flex items-start gap-2">
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-mono text-[10px] text-zinc-600">#{task.id}</span>
                  <StatusBadge status={task.status} />
                  {isHumanReview && (
                    <span className="rounded bg-emerald-500/15 px-1.5 py-0.5 text-[9px] font-medium text-emerald-400">
                      awaiting review
                    </span>
                  )}
                  {isActive && !isHumanReview && (
                    <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-blue-400" title="Running" />
                  )}
                  {task.revision_count != null && task.revision_count > 0 && (
                    <span className="text-[9px] text-amber-500/80">rev {task.revision_count}</span>
                  )}
                  {task.mode && task.mode !== "lawborg" && task.mode !== "legal" && (
                    <span className="rounded bg-violet-500/10 px-1.5 py-0.5 text-[9px] font-medium text-violet-400">
                      {task.mode}
                    </span>
                  )}
                </div>
                <div className="mt-1 text-[12px] font-medium text-zinc-200">{task.title}</div>
                {task.description && (
                  <div className="mt-0.5 line-clamp-2 text-[11px] text-zinc-600">{task.description}</div>
                )}
              </div>
              <div className="flex shrink-0 items-center gap-1">
                {(task.status === "done" || task.status === "merged") && (
                  <button
                    onClick={() => setExpandedResults(expandedResults === task.id ? null : task.id)}
                    className="flex items-center gap-1 rounded border border-white/[0.08] px-2 py-1 text-[10px] text-zinc-500 hover:border-emerald-500/30 hover:text-emerald-400 transition-colors"
                  >
                    {expandedResults === task.id ? "Hide" : "Results"}
                  </button>
                )}
                {(task.status === "done" || task.status === "merged" || isHumanReview) && (
                  <button
                    onClick={() => setCitationsId(citationsId === task.id ? null : task.id)}
                    className="flex items-center gap-1 rounded border border-white/[0.08] px-2 py-1 text-[10px] text-zinc-500 hover:border-blue-500/30 hover:text-blue-400 transition-colors"
                  >
                    {citationsId === task.id ? "Hide" : "Citations"}
                  </button>
                )}
                {(task.revision_count ?? 0) > 0 && (
                  <button
                    onClick={() => setRevisionsId(revisionsId === task.id ? null : task.id)}
                    className="flex items-center gap-1 rounded border border-white/[0.08] px-2 py-1 text-[10px] text-amber-500/60 hover:border-amber-500/30 hover:text-amber-400 transition-colors"
                  >
                    {revisionsId === task.id ? "Hide" : `Revisions (${task.revision_count})`}
                  </button>
                )}
                {isActive && (
                  <button
                    onClick={() => setExpandedStream(expandedStream === task.id ? null : task.id)}
                    className="flex items-center gap-1 rounded border border-white/[0.08] px-2 py-1 text-[10px] text-zinc-500 hover:border-blue-500/30 hover:text-blue-400 transition-colors"
                  >
                    {expandedStream === task.id ? "Hide" : "Stream"}
                  </button>
                )}
                {task.status === "failed" && (
                  <>
                    <button
                      onClick={() => {
                        if (editingId === task.id) { setEditingId(null); } else {
                          setEditTitle(task.title);
                          setEditDesc(task.description || "");
                          setEditingId(task.id);
                        }
                      }}
                      className="flex items-center gap-1 rounded border border-white/[0.08] px-2 py-1 text-[10px] text-zinc-500 hover:border-amber-500/30 hover:text-amber-400 transition-colors"
                    >
                      <Edit2 className="h-3 w-3" />
                      Edit
                    </button>
                    <button
                      onClick={async () => {
                        setRetryingId(task.id);
                        try {
                          if (editingId === task.id) {
                            await patchTask(task.id, { title: editTitle, description: editDesc });
                            setEditingId(null);
                          }
                          await retryTask(task.id);
                          await queryClient.invalidateQueries({ queryKey: ["project_tasks", projectId] });
                        } finally {
                          setRetryingId(null);
                        }
                      }}
                      disabled={retryingId === task.id}
                      className="flex items-center gap-1 rounded border border-white/[0.08] px-2 py-1 text-[11px] text-zinc-400 hover:border-blue-500/30 hover:text-blue-400 disabled:opacity-50 transition-colors"
                    >
                      <RotateCcw className="h-3 w-3" />
                      {retryingId === task.id ? "…" : "Retry"}
                    </button>
                  </>
                )}
              </div>
            </div>
            {editingId === task.id && (
              <div className="mt-2 space-y-1.5 rounded border border-amber-500/20 bg-amber-500/5 p-2">
                <input
                  value={editTitle}
                  onChange={(e) => setEditTitle(e.target.value)}
                  className="w-full rounded border border-white/[0.08] bg-black/30 px-2 py-1 text-[12px] text-zinc-200 outline-none focus:border-amber-500/40"
                  placeholder="Title"
                />
                <textarea
                  value={editDesc}
                  onChange={(e) => setEditDesc(e.target.value)}
                  rows={4}
                  className="w-full rounded border border-white/[0.08] bg-black/30 px-2 py-1 text-[11px] text-zinc-300 outline-none focus:border-amber-500/40 resize-y"
                  placeholder="Description / instructions"
                />
              </div>
            )}
            {/* Human review panel */}
            {isHumanReview && (
              <div className="mt-2 rounded-lg border border-emerald-500/20 bg-emerald-500/[0.04] p-3 space-y-2">
                {reviewPhaseInstruction && (
                  <div className="text-[11px] text-emerald-400/70 leading-relaxed">
                    {reviewPhaseInstruction}
                  </div>
                )}
                <div className="flex items-center gap-2">
                  <button
                    onClick={async () => {
                      await approveTask(task.id);
                      queryClient.invalidateQueries({ queryKey: ["project_tasks", projectId] });
                    }}
                    className="rounded-md bg-emerald-500/15 px-3 py-1.5 text-[11px] font-medium text-emerald-400 hover:bg-emerald-500/25 transition-colors"
                  >
                    Approve
                  </button>
                  <button
                    onClick={() => setReviewingId(reviewingId === task.id ? null : task.id)}
                    className="rounded-md bg-amber-500/10 px-3 py-1.5 text-[11px] font-medium text-amber-400 hover:bg-amber-500/20 transition-colors"
                  >
                    Request Revision
                  </button>
                  <button
                    onClick={async () => {
                      if (confirm("Reject this task? It will be marked as failed.")) {
                        await rejectTask(task.id, "Rejected by reviewer");
                        queryClient.invalidateQueries({ queryKey: ["project_tasks", projectId] });
                      }
                    }}
                    className="rounded-md bg-red-500/10 px-3 py-1.5 text-[11px] font-medium text-red-400 hover:bg-red-500/20 transition-colors"
                  >
                    Reject
                  </button>
                </div>
                {reviewingId === task.id && (
                  <div className="space-y-1.5">
                    <textarea
                      value={revisionFeedback}
                      onChange={(e) => setRevisionFeedback(e.target.value)}
                      rows={3}
                      className="w-full rounded-md border border-amber-500/20 bg-black/30 px-2.5 py-1.5 text-[11px] text-zinc-200 outline-none focus:border-amber-500/40 resize-y placeholder:text-zinc-600"
                      placeholder="Describe what needs to change..."
                    />
                    <div className="flex items-center gap-2">
                      <button
                        onClick={async () => {
                          if (!revisionFeedback.trim()) return;
                          await requestRevision(task.id, revisionFeedback.trim());
                          setRevisionFeedback("");
                          setReviewingId(null);
                          queryClient.invalidateQueries({ queryKey: ["project_tasks", projectId] });
                        }}
                        disabled={!revisionFeedback.trim()}
                        className="rounded-md bg-amber-500/15 px-3 py-1 text-[11px] font-medium text-amber-400 hover:bg-amber-500/25 disabled:opacity-40 transition-colors"
                      >
                        Send Revision Request
                      </button>
                      <button
                        onClick={() => { setReviewingId(null); setRevisionFeedback(""); }}
                        className="text-[11px] text-zinc-600 hover:text-zinc-400"
                      >
                        Cancel
                      </button>
                    </div>
                  </div>
                )}
              </div>
            )}
            <div className="mt-2">
              <PhaseTracker status={task.status} mode={task.mode} />
            </div>
            <div className="mt-1.5 text-[10px] text-zinc-600">
              created {fmtDateTime(task.created_at)}
              {task.attempt > 0 && ` · attempt ${task.attempt}/${task.max_attempts}`}
              {task.duration_secs != null && ` · ${formatDuration(task.duration_secs)}`}
            </div>
            {(isActive && expandedStream === task.id) && (
              <TaskStreamMini taskId={task.id} />
            )}
            {expandedResults === task.id && (
              <StructuredDataPanel taskId={task.id} />
            )}
            {citationsId === task.id && (
              <CitationPanel taskId={task.id} />
            )}
            {revisionsId === task.id && (
              <RevisionHistoryPanel taskId={task.id} />
            )}
          </div>
        );
      })}
    </div>
  );
}

// ── Citation panel ───────────────────────────────────────────────────────────

function CitationPanel({ taskId }: { taskId: number }) {
  const [citations, setCitations] = useState<import("@/lib/api").CitationVerification[]>([]);
  const [verifying, setVerifying] = useState(false);

  useEffect(() => {
    import("@/lib/api").then(({ getTaskCitations }) => {
      getTaskCitations(taskId).then(setCitations).catch(() => {});
    });
  }, [taskId]);

  const statusColor = (s: string) => {
    if (s === "verified") return "text-emerald-400 bg-emerald-500/10";
    if (s === "flagged" || s === "error") return "text-red-400 bg-red-500/10";
    if (s === "format_valid") return "text-blue-400 bg-blue-500/10";
    return "text-zinc-400 bg-zinc-500/10";
  };

  return (
    <div className="mt-2 rounded-lg border border-white/[0.06] bg-white/[0.02] p-3 space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-[11px] font-medium text-zinc-400">
          Citations {citations.length > 0 && `(${citations.length})`}
        </span>
        <button
          onClick={async () => {
            setVerifying(true);
            try {
              const { verifyTaskCitations } = await import("@/lib/api");
              const result = await verifyTaskCitations(taskId);
              setCitations(result.citations);
            } finally {
              setVerifying(false);
            }
          }}
          disabled={verifying}
          className="rounded-md bg-blue-500/10 px-2 py-0.5 text-[10px] text-blue-400 hover:bg-blue-500/20 disabled:opacity-50 transition-colors"
        >
          {verifying ? "Verifying..." : citations.length > 0 ? "Re-verify" : "Verify All"}
        </button>
      </div>
      {citations.length > 0 && (
        <div className="space-y-1">
          {citations.map((c) => (
            <div key={c.id} className="flex items-start gap-2 text-[10px]">
              <span className={cn("shrink-0 rounded px-1.5 py-0.5 font-medium", statusColor(c.status))}>
                {c.status}
              </span>
              <span className="font-mono text-zinc-300 min-w-0 break-all">{c.citation_text}</span>
              {c.source && (
                <span className="shrink-0 text-zinc-600">{c.source}</span>
              )}
            </div>
          ))}
          <div className="text-[9px] text-zinc-600 pt-1">
            {citations.filter(c => c.status === "verified").length} verified
            {" · "}{citations.filter(c => c.status === "unverified").length} unverified
            {citations.some(c => c.status === "error") && ` · ${citations.filter(c => c.status === "error").length} errors`}
          </div>
        </div>
      )}
    </div>
  );
}

// ── Revision history panel ───────────────────────────────────────────────────

function RevisionHistoryPanel({ taskId }: { taskId: number }) {
  const [history, setHistory] = useState<RevisionHistory | null>(null);

  useEffect(() => {
    getRevisionHistory(taskId).then(setHistory).catch(() => {});
  }, [taskId]);

  if (!history || history.rounds.length === 0) {
    return (
      <div className="mt-2 rounded-lg border border-white/[0.06] bg-white/[0.02] p-3">
        <span className="text-[11px] text-zinc-600">No revision history</span>
      </div>
    );
  }

  return (
    <div className="mt-2 rounded-lg border border-white/[0.06] bg-white/[0.02] p-3 space-y-3">
      <div className="flex items-center gap-2">
        <span className="text-[11px] font-medium text-zinc-400">
          Revision History
        </span>
        <span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-[9px] font-medium text-amber-400">
          {history.revision_count} revision{history.revision_count !== 1 ? "s" : ""}
        </span>
        {history.review_status && (
          <span className={cn(
            "rounded px-1.5 py-0.5 text-[9px] font-medium",
            history.review_status === "approved" ? "bg-emerald-500/10 text-emerald-400" :
            history.review_status === "rejected" ? "bg-red-500/10 text-red-400" :
            "bg-amber-500/10 text-amber-400"
          )}>
            {history.review_status.replace("_", " ")}
          </span>
        )}
      </div>
      <div className="relative space-y-0">
        {history.rounds.map((round, i) => (
          <div key={round.round} className="relative pl-5">
            {i < history.rounds.length - 1 && (
              <div className="absolute left-[7px] top-4 bottom-0 w-px bg-white/[0.06]" />
            )}
            <div className="absolute left-0 top-1 h-3.5 w-3.5 rounded-full border border-white/10 bg-zinc-900 flex items-center justify-center">
              <span className="text-[7px] text-zinc-500">{round.round}</span>
            </div>
            <div className="pb-3">
              <div className="text-[10px] font-medium text-zinc-300">
                {round.round === 0 ? "Initial Draft" : `Draft ${round.round + 1}`}
              </div>
              {round.feedback && (
                <div className="mt-1 rounded border border-amber-500/10 bg-amber-500/[0.03] px-2 py-1.5">
                  <div className="text-[9px] text-amber-500/60 mb-0.5">Reviewer feedback</div>
                  <div className="text-[11px] text-zinc-300 whitespace-pre-wrap">{round.feedback}</div>
                  {round.feedback_at && (
                    <div className="text-[9px] text-zinc-600 mt-1">{new Date(round.feedback_at).toLocaleString()}</div>
                  )}
                </div>
              )}
              {round.phases.length > 0 && (
                <div className="mt-1 space-y-1">
                  {round.phases.map((p, j) => (
                    <div key={j} className="flex items-center gap-2 text-[10px]">
                      <span className={cn(
                        "shrink-0 rounded px-1.5 py-0.5 font-medium",
                        p.exit_code === 0 ? "bg-emerald-500/10 text-emerald-400" : "bg-red-500/10 text-red-400"
                      )}>
                        {p.phase}
                      </span>
                      <span className="text-zinc-600 truncate">{p.output_preview.slice(0, 100)}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

// ── Activity tab ──────────────────────────────────────────────────────────────

const AUDIT_KIND_LABELS: Record<string, string> = {
  "matter.created": "Matter created",
  "matter.updated": "Matter updated",
  "matter.deleted": "Matter deleted",
  "task.created": "Task created",
  "task.completed": "Task completed",
  "task.failed": "Task failed",
  "deadline.created": "Deadline added",
  "document.exported": "Document exported",
  "file.uploaded": "File uploaded",
  "conflict.acknowledged": "Conflict acknowledged",
};

function ActivityTab({ projectId }: { projectId: number }) {
  const { data: events = [], isLoading } = useProjectAudit(projectId);

  if (isLoading) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">Loading...</div>;
  if (events.length === 0) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">No activity logged yet.</div>;

  return (
    <div className="space-y-0 overflow-y-auto p-4">
      {events.map((ev, idx) => {
        let detail = "";
        try {
          const p = JSON.parse(ev.payload);
          if (p.title) detail = p.title;
          else if (p.name) detail = p.name;
          else if (p.label) detail = p.label;
        } catch { /* ignore */ }
        return (
          <div key={ev.id} className="flex gap-3">
            <div className="flex flex-col items-center">
              <div className="mt-1 h-2 w-2 shrink-0 rounded-full bg-zinc-500/60" />
              {idx < events.length - 1 && <div className="mt-1 w-px flex-1 bg-white/[0.06]" style={{ minHeight: "24px" }} />}
            </div>
            <div className="pb-3 min-w-0">
              <div className="text-[11px] font-medium text-zinc-300">
                {AUDIT_KIND_LABELS[ev.kind] || ev.kind}
              </div>
              {detail && <div className="text-[11px] text-zinc-500 truncate">{detail}</div>}
              <div className="mt-0.5 text-[10px] text-zinc-600">
                {ev.actor && <span>{ev.actor} · </span>}
                {fmtDateTime(ev.created_at)}
                {ev.task_id && <span className="ml-1 font-mono">#{ev.task_id}</span>}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

// ── Activity tab ──────────────────────────────────────────────────────────────

const AUDIT_KIND_LABELS: Record<string, string> = {
  "matter.created": "Matter created",
  "matter.updated": "Matter updated",
  "matter.deleted": "Matter deleted",
  "task.created": "Task created",
  "task.completed": "Task completed",
  "task.failed": "Task failed",
  "deadline.created": "Deadline added",
  "document.exported": "Document exported",
  "file.uploaded": "File uploaded",
  "conflict.acknowledged": "Conflict acknowledged",
};

function ActivityTab({ projectId }: { projectId: number }) {
  const { data: events = [], isLoading } = useProjectAudit(projectId);

  if (isLoading) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">Loading...</div>;
  if (events.length === 0) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">No activity logged yet.</div>;

  return (
    <div className="space-y-0 overflow-y-auto p-4">
      {events.map((ev, idx) => {
        let detail = "";
        try {
          const p = JSON.parse(ev.payload);
          if (p.title) detail = p.title;
          else if (p.name) detail = p.name;
          else if (p.label) detail = p.label;
        } catch { /* ignore */ }
        return (
          <div key={ev.id} className="flex gap-3">
            <div className="flex flex-col items-center">
              <div className="mt-1 h-2 w-2 shrink-0 rounded-full bg-zinc-500/60" />
              {idx < events.length - 1 && <div className="mt-1 w-px flex-1 bg-white/[0.06]" style={{ minHeight: "24px" }} />}
            </div>
            <div className="pb-3 min-w-0">
              <div className="text-[11px] font-medium text-zinc-300">
                {AUDIT_KIND_LABELS[ev.kind] || ev.kind}
              </div>
              {detail && <div className="text-[11px] text-zinc-500 truncate">{detail}</div>}
              <div className="mt-0.5 text-[10px] text-zinc-600">
                {ev.actor && <span>{ev.actor} · </span>}
                {fmtDateTime(ev.created_at)}
                {ev.task_id && <span className="ml-1 font-mono">#{ev.task_id}</span>}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

// ── Activity tab ──────────────────────────────────────────────────────────────

const AUDIT_KIND_LABELS: Record<string, string> = {
  "matter.created": "Matter created",
  "matter.updated": "Matter updated",
  "matter.deleted": "Matter deleted",
  "task.created": "Task created",
  "task.completed": "Task completed",
  "task.failed": "Task failed",
  "deadline.created": "Deadline added",
  "document.exported": "Document exported",
  "file.uploaded": "File uploaded",
  "conflict.acknowledged": "Conflict acknowledged",
};

function ActivityTab({ projectId }: { projectId: number }) {
  const { data: events = [], isLoading } = useProjectAudit(projectId);

  if (isLoading) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">Loading...</div>;
  if (events.length === 0) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">No activity logged yet.</div>;

  return (
    <div className="space-y-0 overflow-y-auto p-4">
      {events.map((ev, idx) => {
        let detail = "";
        try {
          const p = JSON.parse(ev.payload);
          if (p.title) detail = p.title;
          else if (p.name) detail = p.name;
          else if (p.label) detail = p.label;
        } catch { /* ignore */ }
        return (
          <div key={ev.id} className="flex gap-3">
            <div className="flex flex-col items-center">
              <div className="mt-1 h-2 w-2 shrink-0 rounded-full bg-zinc-500/60" />
              {idx < events.length - 1 && <div className="mt-1 w-px flex-1 bg-white/[0.06]" style={{ minHeight: "24px" }} />}
            </div>
            <div className="pb-3 min-w-0">
              <div className="text-[11px] font-medium text-zinc-300">
                {AUDIT_KIND_LABELS[ev.kind] || ev.kind}
              </div>
              {detail && <div className="text-[11px] text-zinc-500 truncate">{detail}</div>}
              <div className="mt-0.5 text-[10px] text-zinc-600">
                {ev.actor && <span>{ev.actor} · </span>}
                {fmtDateTime(ev.created_at)}
                {ev.task_id && <span className="ml-1 font-mono">#{ev.task_id}</span>}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

// ── Activity tab ──────────────────────────────────────────────────────────────

const AUDIT_KIND_LABELS: Record<string, string> = {
  "matter.created": "Matter created",
  "matter.updated": "Matter updated",
  "matter.deleted": "Matter deleted",
  "task.created": "Task created",
  "task.completed": "Task completed",
  "task.failed": "Task failed",
  "deadline.created": "Deadline added",
  "document.exported": "Document exported",
  "file.uploaded": "File uploaded",
  "conflict.acknowledged": "Conflict acknowledged",
};

function ActivityTab({ projectId }: { projectId: number }) {
  const { data: events = [], isLoading } = useProjectAudit(projectId);

  if (isLoading) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">Loading...</div>;
  if (events.length === 0) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">No activity logged yet.</div>;

  return (
    <div className="space-y-0 overflow-y-auto p-4">
      {events.map((ev, idx) => {
        let detail = "";
        try {
          const p = JSON.parse(ev.payload);
          if (p.title) detail = p.title;
          else if (p.name) detail = p.name;
          else if (p.label) detail = p.label;
        } catch { /* ignore */ }
        return (
          <div key={ev.id} className="flex gap-3">
            <div className="flex flex-col items-center">
              <div className="mt-1 h-2 w-2 shrink-0 rounded-full bg-zinc-500/60" />
              {idx < events.length - 1 && <div className="mt-1 w-px flex-1 bg-white/[0.06]" style={{ minHeight: "24px" }} />}
            </div>
            <div className="pb-3 min-w-0">
              <div className="text-[11px] font-medium text-zinc-300">
                {AUDIT_KIND_LABELS[ev.kind] || ev.kind}
              </div>
              {detail && <div className="text-[11px] text-zinc-500 truncate">{detail}</div>}
              <div className="mt-0.5 text-[10px] text-zinc-600">
                {ev.actor && <span>{ev.actor} · </span>}
                {fmtDateTime(ev.created_at)}
                {ev.task_id && <span className="ml-1 font-mono">#{ev.task_id}</span>}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

// ── Activity tab ──────────────────────────────────────────────────────────────

const AUDIT_KIND_LABELS: Record<string, string> = {
  "matter.created": "Matter created",
  "matter.updated": "Matter updated",
  "matter.deleted": "Matter deleted",
  "task.created": "Task created",
  "task.completed": "Task completed",
  "task.failed": "Task failed",
  "deadline.created": "Deadline added",
  "document.exported": "Document exported",
  "file.uploaded": "File uploaded",
  "conflict.acknowledged": "Conflict acknowledged",
};

function ActivityTab({ projectId }: { projectId: number }) {
  const { data: events = [], isLoading } = useProjectAudit(projectId);

  if (isLoading) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">Loading...</div>;
  if (events.length === 0) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">No activity logged yet.</div>;

  return (
    <div className="space-y-0 overflow-y-auto p-4">
      {events.map((ev, idx) => {
        let detail = "";
        try {
          const p = JSON.parse(ev.payload);
          if (p.title) detail = p.title;
          else if (p.name) detail = p.name;
          else if (p.label) detail = p.label;
        } catch { /* ignore */ }
        return (
          <div key={ev.id} className="flex gap-3">
            <div className="flex flex-col items-center">
              <div className="mt-1 h-2 w-2 shrink-0 rounded-full bg-zinc-500/60" />
              {idx < events.length - 1 && <div className="mt-1 w-px flex-1 bg-white/[0.06]" style={{ minHeight: "24px" }} />}
            </div>
            <div className="pb-3 min-w-0">
              <div className="text-[11px] font-medium text-zinc-300">
                {AUDIT_KIND_LABELS[ev.kind] || ev.kind}
              </div>
              {detail && <div className="text-[11px] text-zinc-500 truncate">{detail}</div>}
              <div className="mt-0.5 text-[10px] text-zinc-600">
                {ev.actor && <span>{ev.actor} · </span>}
                {fmtDateTime(ev.created_at)}
                {ev.task_id && <span className="ml-1 font-mono">#{ev.task_id}</span>}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

// ── Activity tab ──────────────────────────────────────────────────────────────

const AUDIT_KIND_LABELS: Record<string, string> = {
  "matter.created": "Matter created",
  "matter.updated": "Matter updated",
  "matter.deleted": "Matter deleted",
  "task.created": "Task created",
  "task.completed": "Task completed",
  "task.failed": "Task failed",
  "deadline.created": "Deadline added",
  "document.exported": "Document exported",
  "file.uploaded": "File uploaded",
  "conflict.acknowledged": "Conflict acknowledged",
};

function ActivityTab({ projectId }: { projectId: number }) {
  const { data: events = [], isLoading } = useProjectAudit(projectId);

  if (isLoading) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">Loading...</div>;
  if (events.length === 0) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">No activity logged yet.</div>;

  return (
    <div className="space-y-0 overflow-y-auto p-4">
      {events.map((ev, idx) => {
        let detail = "";
        try {
          const p = JSON.parse(ev.payload);
          if (p.title) detail = p.title;
          else if (p.name) detail = p.name;
          else if (p.label) detail = p.label;
        } catch { /* ignore */ }
        return (
          <div key={ev.id} className="flex gap-3">
            <div className="flex flex-col items-center">
              <div className="mt-1 h-2 w-2 shrink-0 rounded-full bg-zinc-500/60" />
              {idx < events.length - 1 && <div className="mt-1 w-px flex-1 bg-white/[0.06]" style={{ minHeight: "24px" }} />}
            </div>
            <div className="pb-3 min-w-0">
              <div className="text-[11px] font-medium text-zinc-300">
                {AUDIT_KIND_LABELS[ev.kind] || ev.kind}
              </div>
              {detail && <div className="text-[11px] text-zinc-500 truncate">{detail}</div>}
              <div className="mt-0.5 text-[10px] text-zinc-600">
                {ev.actor && <span>{ev.actor} · </span>}
                {fmtDateTime(ev.created_at)}
                {ev.task_id && <span className="ml-1 font-mono">#{ev.task_id}</span>}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

// ── Activity tab ──────────────────────────────────────────────────────────────

const AUDIT_KIND_LABELS: Record<string, string> = {
  "matter.created": "Matter created",
  "matter.updated": "Matter updated",
  "matter.deleted": "Matter deleted",
  "task.created": "Task created",
  "task.completed": "Task completed",
  "task.failed": "Task failed",
  "deadline.created": "Deadline added",
  "document.exported": "Document exported",
  "file.uploaded": "File uploaded",
  "conflict.acknowledged": "Conflict acknowledged",
};

function ActivityTab({ projectId }: { projectId: number }) {
  const { data: events = [], isLoading } = useProjectAudit(projectId);

  if (isLoading) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">Loading...</div>;
  if (events.length === 0) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">No activity logged yet.</div>;

  return (
    <div className="space-y-0 overflow-y-auto p-4">
      {events.map((ev, idx) => {
        let detail = "";
        try {
          const p = JSON.parse(ev.payload);
          if (p.title) detail = p.title;
          else if (p.name) detail = p.name;
          else if (p.label) detail = p.label;
        } catch { /* ignore */ }
        return (
          <div key={ev.id} className="flex gap-3">
            <div className="flex flex-col items-center">
              <div className="mt-1 h-2 w-2 shrink-0 rounded-full bg-zinc-500/60" />
              {idx < events.length - 1 && <div className="mt-1 w-px flex-1 bg-white/[0.06]" style={{ minHeight: "24px" }} />}
            </div>
            <div className="pb-3 min-w-0">
              <div className="text-[11px] font-medium text-zinc-300">
                {AUDIT_KIND_LABELS[ev.kind] || ev.kind}
              </div>
              {detail && <div className="text-[11px] text-zinc-500 truncate">{detail}</div>}
              <div className="mt-0.5 text-[10px] text-zinc-600">
                {ev.actor && <span>{ev.actor} · </span>}
                {fmtDateTime(ev.created_at)}
                {ev.task_id && <span className="ml-1 font-mono">#{ev.task_id}</span>}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

// ── Activity tab ──────────────────────────────────────────────────────────────

const AUDIT_KIND_LABELS: Record<string, string> = {
  "matter.created": "Matter created",
  "matter.updated": "Matter updated",
  "matter.deleted": "Matter deleted",
  "task.created": "Task created",
  "task.completed": "Task completed",
  "task.failed": "Task failed",
  "deadline.created": "Deadline added",
  "document.exported": "Document exported",
  "file.uploaded": "File uploaded",
  "conflict.acknowledged": "Conflict acknowledged",
};

function ActivityTab({ projectId }: { projectId: number }) {
  const { data: events = [], isLoading } = useProjectAudit(projectId);

  if (isLoading) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">Loading...</div>;
  if (events.length === 0) return <div className="flex h-32 items-center justify-center text-[12px] text-zinc-600">No activity logged yet.</div>;

  return (
    <div className="space-y-0 overflow-y-auto p-4">
      {events.map((ev, idx) => {
        let detail = "";
        try {
          const p = JSON.parse(ev.payload);
          if (p.title) detail = p.title;
          else if (p.name) detail = p.name;
          else if (p.label) detail = p.label;
        } catch { /* ignore */ }
        return (
          <div key={ev.id} className="flex gap-3">
            <div className="flex flex-col items-center">
              <div className="mt-1 h-2 w-2 shrink-0 rounded-full bg-zinc-500/60" />
              {idx < events.length - 1 && <div className="mt-1 w-px flex-1 bg-white/[0.06]" style={{ minHeight: "24px" }} />}
            </div>
            <div className="pb-3 min-w-0">
              <div className="text-[11px] font-medium text-zinc-300">
                {AUDIT_KIND_LABELS[ev.kind] || ev.kind}
              </div>
              {detail && <div className="text-[11px] text-zinc-500 truncate">{detail}</div>}
              <div className="mt-0.5 text-[10px] text-zinc-600">
                {ev.actor && <span>{ev.actor} · </span>}
                {fmtDateTime(ev.created_at)}
                {ev.task_id && <span className="ml-1 font-mono">#{ev.task_id}</span>}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

// ── Chat tab ──────────────────────────────────────────────────────────────────

function ChatTab({ projectId }: { projectId: number }) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [messageInput, setMessageInput] = useState("");
  const [sending, setSending] = useState(false);
  const dictation = useDictation(messageInput, setMessageInput);
  const esRef = useRef<EventSource | null>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const sseRetriesRef = useRef(0);
  const sseRetryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const threadKey = `project:${projectId}`;

  useEffect(() => {
    getProjectChatMessages(projectId)
      .then(setMessages)
      .catch(() => setMessages([]));
  }, [projectId]);

  useEffect(() => {
    sseRetriesRef.current = 0;

    function connectSSE() {
      if (esRef.current) esRef.current.close();
      tokenReady.then(() => {
        const es = new EventSource(sseUrl("/api/chat/events"));
        esRef.current = es;

        es.onopen = () => { sseRetriesRef.current = 0; };

        es.onmessage = (e) => {
          try {
            const msg: ChatMessage = JSON.parse(e.data);
            if ((msg.thread ?? "") !== threadKey) return;
            setMessages((prev) => [...prev, msg]);
            if (msg.role === "assistant") setSending(false);
          } catch {
            // ignore malformed events
          }
        };

        es.onerror = () => {
          es.close();
          esRef.current = null;
          setSending(false);
          if (sseRetriesRef.current < 5) {
            const delay = Math.min(1000 * Math.pow(2, sseRetriesRef.current), 30000);
            sseRetriesRef.current++;
            sseRetryTimerRef.current = setTimeout(connectSSE, delay);
          }
        };
      });
    }

    connectSSE();
    return () => {
      esRef.current?.close();
      if (sseRetryTimerRef.current) clearTimeout(sseRetryTimerRef.current);
    };
  }, [projectId, threadKey]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "instant" });
  }, [messages.length]);

  async function handleSend() {
    if (sending) return;
    const text = messageInput.trim();
    if (!text) return;
    setMessageInput("");
    setSending(true);
    const timeout = setTimeout(() => setSending(false), 60_000);
    try {
      await sendProjectChat(projectId, text);
    } catch {
      setSending(false);
    } finally {
      clearTimeout(timeout);
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex-1 overflow-y-auto p-4">
        {messages.length === 0 && !sending && (
          <div className="py-8 text-center text-[12px] text-zinc-600">
            Chat with the director about this matter.
          </div>
        )}
        {messages.map((msg, idx) => (
          <div
            key={`${msg.ts}-${msg.role}-${idx}`}
            className={cn("mb-2 flex", msg.role === "user" ? "justify-end" : "justify-start")}
          >
            <div
              className={cn(
                "max-w-[85%] rounded-lg px-3 py-2 text-[12px] leading-relaxed",
                msg.role === "user"
                  ? "bg-blue-500/[0.15] text-zinc-200"
                  : "bg-white/[0.05] text-zinc-300"
              )}
            >
              {msg.role !== "user" && (
                <div className="mb-1 text-[10px] text-zinc-500">{msg.sender ?? "director"}</div>
              )}
              {msg.role === "user" ? (
                <div className="whitespace-pre-wrap break-words">{msg.text}</div>
              ) : (
                <ChatMarkdown text={msg.text} />
              )}
            </div>
          </div>
        ))}
        {sending && <BorgingIndicator />}
        <div ref={bottomRef} />
      </div>

      <div className="shrink-0 border-t border-white/[0.06] p-3">
        <div className="flex gap-2">
          <textarea
            value={messageInput}
            onChange={(e) => setMessageInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                handleSend();
              }
            }}
            placeholder="Message the director about this matter..."
            rows={2}
            className="flex-1 resize-none rounded border border-white/[0.08] bg-white/[0.03] px-3 py-2 text-[12px] text-zinc-200 outline-none placeholder:text-zinc-600"
          />
          {dictation.supported && (
            <button
              onClick={dictation.toggle}
              title={dictation.listening ? "Stop dictation" : "Start dictation"}
              className={cn(
                "shrink-0 rounded px-2.5 py-2 transition-colors",
                dictation.listening
                  ? "bg-red-500/20 text-red-400 hover:bg-red-500/30"
                  : "text-zinc-600 hover:text-zinc-400 hover:bg-white/[0.06]"
              )}
            >
              {dictation.listening ? <MicOff className="h-4 w-4" /> : <Mic className="h-4 w-4" />}
            </button>
          )}
          <button
            onClick={handleSend}
            disabled={sending || !messageInput.trim()}
            className="rounded bg-blue-500/20 px-3 py-2 text-[12px] font-medium text-blue-300 disabled:cursor-not-allowed disabled:text-zinc-600"
          >
            Send
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Tab bar ───────────────────────────────────────────────────────────────────

type TabKey = "timeline" | "documents" | "tasks" | "activity" | "chat";

const TABS: { key: TabKey; label: string }[] = [
  { key: "timeline", label: "Timeline" },
  { key: "documents", label: "Documents" },
  { key: "tasks", label: "Tasks" },
  { key: "activity", label: "Activity" },
  { key: "chat", label: "Chat" },
];

// ── Main component ────────────────────────────────────────────────────────────

export function MatterDetail({ projectId, onDocumentSelect, onDelete }: MatterDetailProps) {
  const { data: project, isLoading } = useProjectDetail(projectId);
  const [activeTab, setActiveTab] = useState<TabKey>("timeline");
  const deleteMut = useDeleteProject();

  const handleDelete = () => {
    deleteMut.mutate(projectId, { onSuccess: () => onDelete?.() });
  };

  if (isLoading || !project) {
    return (
      <div className="flex h-full items-center justify-center text-[12px] text-zinc-600">
        Loading matter...
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <MatterHeader project={project} onDelete={handleDelete} />
      <MetadataPanel project={project} projectId={projectId} />
      <DeadlinesPanel projectId={projectId} />

      <div className="shrink-0 flex gap-0 border-b border-white/[0.06] px-5">
        {TABS.map((tab) => (
          <button
            key={tab.key}
            onClick={() => setActiveTab(tab.key)}
            className={cn(
              "border-b-2 px-3 py-2.5 text-[12px] font-medium transition-colors",
              activeTab === tab.key
                ? "border-blue-500 text-zinc-200"
                : "border-transparent text-zinc-500 hover:text-zinc-300"
            )}
          >
            {tab.label}
          </button>
        ))}
      </div>

      <div className="min-h-0 flex-1 overflow-hidden">
        {activeTab === "timeline" && (
          <div className="h-full overflow-y-auto">
            <TimelineTab projectId={projectId} />
          </div>
        )}
        {activeTab === "documents" && (
          <div className="h-full overflow-y-auto">
            <DocumentsTab projectId={projectId} onDocumentSelect={onDocumentSelect} />
          </div>
        )}
        {activeTab === "tasks" && (
          <div className="h-full overflow-y-auto">
            <TasksTab projectId={projectId} />
          </div>
        )}
        {activeTab === "activity" && (
          <div className="h-full overflow-y-auto">
            <ActivityTab projectId={projectId} />
          </div>
        )}
        {activeTab === "chat" && (
          <div className="flex h-full flex-col">
            <ChatTab projectId={projectId} />
          </div>
        )}
      </div>
    </div>
  );
}
