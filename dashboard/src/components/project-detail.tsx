import { useQueryClient } from "@tanstack/react-query";
import { Download, Edit2, FileText, RotateCcw, Share2, Trash2 } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { CitationVerification, RevisionHistory } from "@/lib/api";
import {
  apiBase,
  approveTask,
  authHeaders,
  downloadChatArtifact,
  deleteAllProjectFiles,
  deleteProjectFile,
  fetchProjectFileContent,
  getRevisionHistory,
  getTaskCitations,
  getTaskStructuredData,
  patchTask,
  rejectTask,
  requestRevision,
  retryTask,
  type StreamEvent,
  tokenReady,
  useDeleteProject,
  useFullModes,
  useProjectAudit,
  useProjectDetail,
  useProjectDocuments,
  useProjectTasks,
  useSettings,
  useTaskStream,
  useTemplates,
  useUpdateProject,
  verifyTaskCitations,
} from "@/lib/api";
import { useDashboardMode } from "@/lib/dashboard-mode";
import type { Project, ProjectDocument, ProjectTask } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useVocabulary } from "@/lib/vocabulary";
import { CloudStoragePanel } from "./cloud-storage";
import {
  downloadFile,
  FileListItem,
  FileListPagination,
  FilePreviewWrapper,
  FileSearchBar,
  FileUploadArea,
  formatFileSize,
  isPreviewable,
  useFileList,
  useFilePreview,
} from "./file-list-shared";
import { PhaseTracker } from "./phase-tracker";
import { ProjectShareDialog } from "./project-share-dialog";
import { StatusBadge } from "./status-badge";
import { TaskCreator } from "./task-creator";

const AGENT_WORKING_STATUSES = new Set([
  "implement",
  "review",
  "validate",
  "lint_fix",
  "rebase",
  "spec",
  "qa",
  "qa_fix",
  "retry",
]);

function extractFilePaths(events: StreamEvent[]): Set<string> {
  const paths = new Set<string>();
  for (const ev of events) {
    if (ev.type !== "assistant" || !ev.message?.content || !Array.isArray(ev.message.content)) continue;
    for (const block of ev.message.content) {
      if (block.type !== "tool_use") continue;
      const name = block.name || "";
      if (name === "Edit" || name === "Write" || name === "Read") {
        const fp = (block.input as Record<string, unknown>)?.file_path;
        if (typeof fp === "string") {
          const basename = fp.split("/").pop() || fp;
          paths.add(basename);
        }
      }
    }
  }
  return paths;
}

function useActiveFiles(tasks: ProjectTask[]): { activeFiles: Set<string>; activeTaskId: number | null } {
  const activeTask = useMemo(() => tasks.find((t) => AGENT_WORKING_STATUSES.has(t.status)) ?? null, [tasks]);
  const { events } = useTaskStream(activeTask?.id ?? null, !!activeTask);
  const activeFiles = useMemo(() => extractFilePaths(events), [events]);
  return { activeFiles, activeTaskId: activeTask?.id ?? null };
}

interface ProjectDetailProps {
  projectId: number;
  onDocumentSelect?: (doc: ProjectDocument) => void;
  onDelete?: () => void;
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

function formatRemaining(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const days = Math.floor(totalSeconds / 86400);
  const hours = Math.floor((totalSeconds % 86400) / 3600);
  const mins = Math.floor((totalSeconds % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${mins}m`;
  return `${mins}m`;
}

// ── Project header ─────────────────────────────────────────────────────────────

function ProjectHeader({ project, onDelete }: { project: Project; onDelete?: () => void }) {
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [exportingAll, setExportingAll] = useState(false);
  const [exportMenu, setExportMenu] = useState(false);
  const [exportTemplateId, setExportTemplateId] = useState<number | null>(null);
  const [shareOpen, setShareOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const [editName, setEditName] = useState(project.name);
  const renameInput = useRef<HTMLInputElement>(null);
  const updateProject = useUpdateProject();
  const { data: templates = [] } = useTemplates("template");
  const { isSWE } = useDashboardMode();
  const isLegalProject = project.mode === "lawborg" || project.mode === "legal";

  useEffect(() => {
    if (editing && renameInput.current) {
      renameInput.current.focus();
      renameInput.current.select();
    }
  }, [editing]);

  function commitRename() {
    const trimmed = editName.trim();
    if (trimmed && trimmed !== project.name) {
      updateProject.mutate({ projectId: project.id, body: { name: trimmed } });
    }
    setEditing(false);
  }

  async function exportAll(format: "pdf" | "docx") {
    setExportMenu(false);
    setExportingAll(true);
    try {
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
    <div className="border-b border-white/[0.07] px-5 py-4">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-[15px] font-semibold text-zinc-100 flex items-center gap-1">
              <span className="text-[12px] text-[#6b6459] tabular-nums mr-1.5">#{project.id}</span>
              {editing ? (
                <input
                  ref={renameInput}
                  value={editName}
                  onChange={(e) => setEditName(e.target.value)}
                  onBlur={commitRename}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") commitRename();
                    if (e.key === "Escape") {
                      setEditName(project.name);
                      setEditing(false);
                    }
                  }}
                  className="bg-transparent border-b border-amber-500/40 outline-none text-zinc-100 text-[15px] font-semibold px-0 py-0 min-w-[120px]"
                />
              ) : (
                <span
                  className="cursor-pointer hover:text-amber-400 transition-colors"
                  onDoubleClick={() => {
                    setEditName(project.name);
                    setEditing(true);
                  }}
                  title="Double-click to rename"
                >
                  {project.name}
                </span>
              )}
            </h2>
            {project.jurisdiction && (
              <span className="rounded-lg bg-blue-500/10 px-2 py-0.5 text-[10px] font-medium text-blue-400">
                {project.jurisdiction}
              </span>
            )}
            {project.mode && isSWE && (
              <span className="rounded-lg bg-violet-500/10 px-2 py-0.5 text-[10px] font-medium text-violet-400">
                {project.mode}
              </span>
            )}
          </div>
        </div>
        <div className="flex items-center gap-1.5 shrink-0">
          <TaskCreator
            projectId={project.id}
            projectMode={project.mode}
            hideModePicker={isLegalProject}
            defaultMode={project.mode || "sweborg"}
            buttonLabel={isLegalProject ? "New Matter Task" : "New Task"}
          />
          <button
            onClick={() => setShareOpen(true)}
            className="rounded-lg border border-white/[0.08] px-3 py-1.5 text-[12px] text-zinc-400 hover:border-blue-500/30 hover:text-blue-400 transition-colors flex items-center gap-1.5"
            title="Share"
          >
            <Share2 size={13} />
            Share
          </button>
          {shareOpen && <ProjectShareDialog project={project} onClose={() => setShareOpen(false)} />}
          <div className="relative">
            <button
              onClick={() => setExportMenu((v) => !v)}
              disabled={exportingAll}
              className="rounded-lg border border-white/[0.08] px-3 py-1.5 text-[12px] text-zinc-400 hover:border-blue-500/30 hover:text-blue-400 transition-colors disabled:opacity-50"
              title="Export all documents"
            >
              {exportingAll ? "Exporting..." : "Export All"}
            </button>
            {exportMenu && (
              <div className="absolute right-0 top-full z-50 mt-1.5 w-56 rounded-xl border border-white/[0.1] bg-zinc-900 shadow-xl">
                {templates.length > 0 && (
                  <div className="border-b border-white/[0.07] px-4 py-3">
                    <label className="text-[11px] text-zinc-400 block mb-1">Template</label>
                    <select
                      value={exportTemplateId ?? ""}
                      onChange={(e) => setExportTemplateId(e.target.value ? Number(e.target.value) : null)}
                      className="w-full rounded-lg border border-white/[0.08] bg-zinc-800 px-2 py-1.5 text-[12px] text-zinc-300 outline-none"
                    >
                      <option value="">None (default)</option>
                      {templates.map((t) => (
                        <option key={t.id} value={t.id}>
                          {t.file_name}
                        </option>
                      ))}
                    </select>
                  </div>
                )}
                <button
                  onClick={() => exportAll("docx")}
                  className="flex w-full items-center px-4 py-2.5 text-left text-[13px] text-zinc-300 hover:bg-white/[0.06] transition-colors"
                >
                  Export as DOCX (ZIP)
                </button>
                <button
                  onClick={() => exportAll("pdf")}
                  className="flex w-full items-center px-4 py-2.5 text-left text-[13px] text-zinc-300 hover:bg-white/[0.06] transition-colors"
                >
                  Export as PDF (ZIP)
                </button>
              </div>
            )}
          </div>
          {onDelete &&
            (confirmDelete ? (
              <div className="flex items-center gap-1.5">
                <span className="text-[11px] text-red-400">Delete?</span>
                <button
                  onClick={onDelete}
                  className="rounded-lg px-2 py-1 text-[11px] bg-red-500/20 text-red-400 hover:bg-red-500/30 transition-colors"
                >
                  Yes
                </button>
                <button
                  onClick={() => setConfirmDelete(false)}
                  className="rounded-lg px-2 py-1 text-[11px] bg-zinc-700 text-zinc-400 hover:bg-zinc-600 transition-colors"
                >
                  No
                </button>
              </div>
            ) : (
              <button
                onClick={() => setConfirmDelete(true)}
                className="shrink-0 rounded p-1 text-zinc-600 hover:text-red-400 hover:bg-red-500/10"
                title="Delete matter"
              >
                <Trash2 size={14} />
              </button>
            ))}
        </div>
      </div>
    </div>
  );
}

// ── Activity tab (merged timeline + audit) ───────────────────────────────────

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
  const { data: tasks = [] } = useProjectTasks(projectId);
  const { data: docs = [] } = useProjectDocuments(projectId);
  const { data: auditEvents = [] } = useProjectAudit(projectId);

  type ActivityItem = {
    id: string;
    ts: string;
    label: string;
    sub?: string;
    kind: "task" | "document" | "audit";
    dotColor: string;
  };

  const items = useMemo(() => {
    const list: ActivityItem[] = [];

    for (const t of tasks) {
      list.push({
        id: `task-${t.id}`,
        ts: t.created_at,
        label: t.title,
        sub: `Task #${t.id} created`,
        kind: "task",
        dotColor: "bg-emerald-400/60",
      });
    }

    for (const d of docs) {
      list.push({
        id: `doc-${d.task_id}-${d.file_name}`,
        ts: d.created_at,
        label: d.file_name,
        sub: `from task #${d.task_id} · ${d.task_title}`,
        kind: "document",
        dotColor: "bg-blue-400/60",
      });
    }

    for (const ev of auditEvents) {
      let detail = "";
      try {
        const p = JSON.parse(ev.payload);
        detail = p.title || p.name || p.label || "";
      } catch {
        /* skip */
      }
      list.push({
        id: `audit-${ev.id}`,
        ts: ev.created_at,
        label: AUDIT_KIND_LABELS[ev.kind] || ev.kind,
        sub: [detail, ev.actor, ev.task_id ? `#${ev.task_id}` : ""].filter(Boolean).join(" · "),
        kind: "audit",
        dotColor: ev.kind.startsWith("file.") ? "bg-amber-400/60" : "bg-zinc-500/60",
      });
    }

    list.sort((a, b) => (a.ts < b.ts ? 1 : a.ts > b.ts ? -1 : 0));
    return list;
  }, [tasks, docs, auditEvents]);

  if (items.length === 0) {
    return (
      <div className="flex h-32 flex-col items-center justify-center text-center">
        <FileText className="h-6 w-6 text-zinc-600 mb-2" />
        <div className="text-[13px] text-zinc-400">No activity yet</div>
      </div>
    );
  }

  return (
    <div className="space-y-0 overflow-y-auto p-5">
      {items.map((item, idx) => (
        <div key={item.id} className="flex gap-3">
          <div className="flex flex-col items-center">
            <div className={cn("mt-1.5 h-2.5 w-2.5 shrink-0 rounded-full", item.dotColor)} />
            {idx < items.length - 1 && (
              <div className="mt-1 w-px flex-1 bg-white/[0.07]" style={{ minHeight: "28px" }} />
            )}
          </div>
          <div className="pb-4 min-w-0">
            <div className="text-[13px] font-medium text-zinc-300 truncate">{item.label}</div>
            {item.sub && (
              <div className="mt-0.5 text-[12px] text-zinc-400">
                {item.sub} · {fmtDateTime(item.ts)}
              </div>
            )}
            {!item.sub && <div className="mt-0.5 text-[12px] text-zinc-400">{fmtDateTime(item.ts)}</div>}
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
  const vocab = useVocabulary();
  const queryClient = useQueryClient();
  const { data: settings = null } = useSettings();
  const { data: docs = [], isLoading } = useProjectDocuments(projectId);
  const { data: tasks = [] } = useProjectTasks(projectId);
  const { activeFiles } = useActiveFiles(tasks);
  const fl = useFileList(projectId);
  const { previewFile, setPreviewFile } = useFilePreview();
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [deletingAll, setDeletingAll] = useState(false);
  const activeTasks = useMemo(() => tasks.filter((t) => AGENT_WORKING_STATUSES.has(t.status)), [tasks]);
  const activeTasksWithoutDocs = useMemo(() => {
    const docTaskIds = new Set(docs.map((d) => d.task_id));
    return activeTasks.filter((t) => !docTaskIds.has(t.id));
  }, [activeTasks, docs]);

  if (isLoading) {
    return <div className="flex h-32 items-center justify-center text-[13px] text-zinc-400">Loading...</div>;
  }

  const hasFiles = (fl.filePage?.summary.total_files ?? 0) > 0;
  const hasDocs = docs.length > 0;
  const hasAgentWork = hasDocs || activeTasksWithoutDocs.length > 0;

  async function handleDeleteAllFiles() {
    if (deletingAll) return;
    if (
      !confirm(
        `Delete all documents in this ${vocab.projectSingular}? This removes every source file in the ${vocab.projectSingular}, not just the current search results.`,
      )
    ) {
      return;
    }
    setDeleteError(null);
    setDeletingAll(true);
    try {
      await deleteAllProjectFiles(projectId);
      setPreviewFile(null);
      fl.resetPagination();
      await Promise.all([
        fl.refetchFiles(),
        queryClient.invalidateQueries({ queryKey: ["project_documents", projectId] }),
      ]);
    } catch (err) {
      setDeleteError(err instanceof Error ? err.message : `Failed to delete ${vocab.projectDocsLabel.toLowerCase()}`);
    } finally {
      setDeletingAll(false);
    }
  }

  return (
    <div className="flex h-full flex-col">
      {/* Sticky top: agent work + upload + search + pagination */}
      <div className="shrink-0 space-y-3 p-5 pb-3">
        {/* Agent work — above upload so it's the first thing visible */}
        {hasAgentWork && (
          <div>
            <div className="mb-2 text-[13px] font-medium text-[#e8e0d4]">
              Agent Work
              {hasDocs && <span className="ml-1.5 text-[12px] text-[#6b6459]">({docs.length})</span>}
            </div>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              {activeTasksWithoutDocs.map((task) => (
                <div
                  key={`active-${task.id}`}
                  className="flex flex-col gap-2 rounded-xl border border-amber-500/30 bg-[#1a1814] p-4"
                >
                  <div className="flex items-center gap-2">
                    <span className="h-2 w-2 shrink-0 rounded-full bg-amber-400 animate-pulse" />
                    <span className="text-[13px] font-medium text-[#e8e0d4] truncate">{task.title}</span>
                    <StatusBadge status={task.status} />
                  </div>
                  <div className="text-[12px] text-[#6b6459] truncate">#{task.id} · working...</div>
                </div>
              ))}
              {docs.map((doc) => {
                const docBasename = doc.file_name.split("/").pop() || doc.file_name;
                const isActive = activeFiles.has(docBasename);
                const isChatArtifact = doc.source === "chat";
                const isBinary = /\.(docx|pdf|xlsx|pptx|png|jpg|jpeg|gif|svg)$/i.test(doc.file_name);
                const handleClick = () => {
                  if (isChatArtifact && isBinary) {
                    downloadChatArtifact(projectId, doc.file_name);
                  } else {
                    onDocumentSelect?.(doc);
                  }
                };
                return (
                  <button
                    key={`${doc.task_id}-${doc.file_name}`}
                    onClick={handleClick}
                    className={cn(
                      "flex flex-col gap-2 rounded-xl border p-4 text-left transition-colors hover:border-amber-900/30 hover:bg-[#1c1a17]",
                      isActive ? "border-amber-500/30 bg-[#1a1814]" : "border-[#2a2520] bg-[#151412]",
                    )}
                  >
                    <div className="flex items-center gap-2">
                      {isChatArtifact && isBinary ? (
                        <Download className="h-4 w-4 shrink-0 text-emerald-400/60" />
                      ) : (
                        <FileText className="h-4 w-4 shrink-0 text-blue-400/60" />
                      )}
                      <span className="text-[13px] font-medium text-[#e8e0d4] truncate">{doc.file_name}</span>
                      {isActive && (
                        <span className="flex items-center gap-1 shrink-0">
                          <span className="h-2 w-2 rounded-full bg-amber-400 animate-pulse" />
                          <span className="text-[10px] text-amber-400">editing</span>
                        </span>
                      )}
                      <StatusBadge status={doc.task_status} />
                    </div>
                    <div className="text-[12px] text-[#6b6459] truncate">
                      {isChatArtifact ? doc.task_title : `#${doc.task_id} · ${doc.task_title}`}
                    </div>
                  </button>
                );
              })}
            </div>
          </div>
        )}

        <FileUploadArea
          projectId={projectId}
          onUploaded={() => {
            fl.resetPagination();
            fl.refetchFiles();
          }}
          subtitle={vocab.uploadSubtitle}
        />

        <CloudStoragePanel
          projectId={projectId}
          settings={settings}
          onImported={() => {
            fl.resetPagination();
            fl.refetchFiles();
          }}
        />

        {deleteError && <div className="text-[12px] text-red-400">{deleteError}</div>}

        {hasFiles && (
          <>
            <FileSearchBar
              value={fl.fileSearch}
              onChange={(v) => {
                fl.setFileSearch(v);
                fl.resetPagination();
              }}
              placeholder="Search source files..."
              stats={
                <>
                  {fl.filePage?.summary.total_files ?? fl.files.length} files{" "}
                  {formatFileSize(fl.filePage?.summary.total_bytes ?? 0)}
                </>
              }
            />
            {fl.filePage && (
              <FileListPagination
                filePage={fl.filePage}
                currentOffset={fl.currentFilePage.offset}
                fileCount={fl.files.length}
                pageSize={fl.pageSize}
                onPageSizeChange={(s) => {
                  fl.setPageSize(s);
                  fl.resetPagination();
                }}
                canGoPrev={fl.filePageStack.length > 1}
                onPrev={() => fl.setFilePageStack((prev) => (prev.length > 1 ? prev.slice(0, -1) : prev))}
                canGoNext={!!(fl.filePage?.has_more && fl.filePage?.next_cursor)}
                onNext={() => {
                  if (!fl.filePage?.next_cursor) return;
                  fl.setFilePageStack((prev) => [
                    ...prev,
                    {
                      cursor: fl.filePage?.next_cursor ?? null,
                      offset: fl.currentFilePage.offset + fl.files.length,
                    },
                  ]);
                }}
                actions={
                  <button
                    type="button"
                    onClick={handleDeleteAllFiles}
                    disabled={deletingAll}
                    className="inline-flex items-center gap-1.5 rounded-lg border border-red-500/20 bg-red-500/[0.08] px-3 py-1.5 text-[12px] font-medium text-red-300 transition-colors hover:bg-red-500/[0.14] disabled:cursor-not-allowed disabled:opacity-60"
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                    {deletingAll ? "Deleting..." : "Delete All"}
                  </button>
                }
              />
            )}
          </>
        )}
      </div>

      {/* Scrollable content */}
      <div className="min-h-0 flex-1 overflow-y-auto px-5 pb-5 space-y-4">
        {/* Source files */}
        {hasFiles && (
          <div className="space-y-2">
            {fl.files.map((f, i) => {
              const fileBasename = f.file_name.split("/").pop() || f.file_name;
              const isFileActive = activeFiles.has(fileBasename);
              return (
                <FileListItem
                  key={f.id}
                  file={f}
                  index={fl.currentFilePage.offset + i + 1}
                  isActive={isFileActive}
                  onClick={isPreviewable(f) ? () => setPreviewFile(f) : undefined}
                  onDownload={() => downloadFile((id) => fetchProjectFileContent(projectId, id), f)}
                  onDelete={async () => {
                    if (!confirm(`Delete "${f.file_name}"?`)) return;
                    await deleteProjectFile(projectId, f.id);
                    fl.refetchFiles();
                  }}
                  extraBadges={
                    f.privileged ? (
                      <span className="rounded-full bg-rose-500/15 px-2 py-0.5 text-[10px] font-medium text-rose-300 ring-1 ring-inset ring-rose-500/20">
                        privileged
                      </span>
                    ) : undefined
                  }
                />
              );
            })}
            {fl.files.length === 0 && (
              <div className="rounded-xl border border-dashed border-[#2a2520] px-4 py-4 text-[12px] text-[#6b6459] text-center">
                No files match the current filter.
              </div>
            )}
          </div>
        )}

        {!hasFiles && !hasAgentWork && (
          <div className="flex flex-col items-center py-12 text-center">
            <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-[#1c1a17] ring-1 ring-amber-900/20">
              <FileText className="h-6 w-6 text-[#6b6459]" />
            </div>
            <p className="text-[14px] text-[#9c9486]">No documents yet</p>
            <p className="mt-1 text-[12px] text-[#6b6459]">Upload sources or run a task to generate drafts</p>
          </div>
        )}
      </div>

      <FilePreviewWrapper
        file={previewFile}
        fetchContent={(fileId) => fetchProjectFileContent(projectId, fileId)}
        onClose={() => setPreviewFile(null)}
        isActive={
          previewFile ? activeFiles.has(previewFile.file_name.split("/").pop() || previewFile.file_name) : false
        }
      />
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
  }, []);

  const lines = useMemo(() => {
    return events
      .filter((e) => e.type === "assistant" && e.message?.content)
      .map((e) => {
        const content = e.message?.content;
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
    <div className="mt-2 rounded-xl border border-white/[0.07] bg-black/30">
      <div className="flex items-center gap-2 border-b border-white/[0.07] px-3 py-2">
        {streaming && <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-blue-400" />}
        <span className="text-[11px] font-medium text-zinc-400">{streaming ? "Live output" : "Output"}</span>
      </div>
      <div
        ref={scrollRef}
        className="max-h-[200px] overflow-y-auto p-3 font-mono text-[11px] leading-relaxed text-zinc-400 whitespace-pre-wrap"
      >
        {lines.length > 0 ? (
          lines[lines.length - 1].slice(-500)
        ) : (
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

  if (loading) return <div className="mt-2 text-[11px] text-zinc-500">Loading results…</div>;
  if (!data) return <div className="mt-2 text-[11px] text-zinc-500">No structured data.</div>;

  const summary = data.summary as string | undefined;
  const riskFlags = data.risk_flags as
    | { severity: string; issue: string; section?: string; recommendation?: string }[]
    | undefined;
  const keyObligations = data.key_obligations as { party: string; obligation: string; section?: string }[] | undefined;
  const parties = data.parties as string[] | undefined;
  const complianceItems = data.compliance_items as
    | { requirement: string; status: string; evidence?: string; recommendation?: string }[]
    | undefined;
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
    <div className="mt-2 rounded-xl border border-white/[0.07] bg-white/[0.03] p-4 space-y-3">
      {summary && <p className="text-[12px] text-zinc-300 leading-relaxed">{summary}</p>}

      {parties && parties.length > 0 && (
        <div>
          <div className="text-[11px] font-semibold text-zinc-400 mb-1.5">Parties</div>
          <div className="flex flex-wrap gap-1.5">
            {parties.map((p, i) => (
              <span key={i} className="rounded-lg bg-white/[0.06] px-2 py-0.5 text-[11px] text-zinc-300">
                {p}
              </span>
            ))}
          </div>
        </div>
      )}

      {keyObligations && keyObligations.length > 0 && (
        <div>
          <div className="text-[11px] font-semibold text-zinc-400 mb-1.5">Key Obligations</div>
          <div className="space-y-1">
            {keyObligations.map((o, i) => (
              <div key={i} className="rounded-lg bg-white/[0.03] px-2.5 py-2 text-[11px]">
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
          <div className="text-[11px] font-semibold text-zinc-400 mb-1.5">Risk Flags</div>
          <div className="space-y-1">
            {riskFlags.map((r, i) => (
              <div
                key={i}
                className={cn(
                  "rounded-lg border px-2.5 py-2 text-[11px]",
                  severityColor[r.severity] || severityColor.low,
                )}
              >
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
          <div className="text-[11px] font-semibold text-zinc-400 mb-1.5">Regulations</div>
          <div className="space-y-1">
            {regulations.map((r, i) => (
              <div key={i} className="flex items-center gap-2 rounded-lg bg-white/[0.03] px-2.5 py-2 text-[11px]">
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
          <div className="text-[11px] font-semibold text-zinc-400 mb-1.5">Compliance</div>
          <div className="space-y-1">
            {complianceItems.map((c, i) => (
              <div key={i} className="rounded-lg bg-white/[0.03] px-2.5 py-2 text-[11px]">
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
          <div className="text-[11px] font-semibold text-zinc-400 mb-1.5">Deadlines</div>
          <div className="space-y-1">
            {deadlines.map((d, i) => (
              <div key={i} className="flex items-center gap-2 rounded-lg bg-white/[0.03] px-2.5 py-2 text-[11px]">
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

const ACTIVE_STATUSES = new Set([
  "implement",
  "review",
  "validate",
  "lint_fix",
  "rebase",
  "spec",
  "qa",
  "qa_fix",
  "retry",
]);

function TasksTab({ projectId }: { projectId: number }) {
  const { data: tasks = [], isLoading } = useProjectTasks(projectId);
  const { data: fullModes = [] } = useFullModes();
  const { isSWE } = useDashboardMode();
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
    return <div className="flex h-32 items-center justify-center text-[13px] text-zinc-400">Loading...</div>;
  }

  if (tasks.length === 0) {
    return (
      <div className="flex h-32 flex-col items-center justify-center text-center">
        <FileText className="h-6 w-6 text-zinc-600 mb-2" />
        <div className="text-[13px] text-zinc-400">No tasks linked to this matter</div>
      </div>
    );
  }

  const totalSecs = tasks.reduce((sum, t) => sum + (t.duration_secs ?? 0), 0);

  return (
    <div className="space-y-2.5 p-4">
      {totalSecs > 0 && (
        <div className="text-[12px] text-zinc-400 pb-1">
          Total time: <span className="text-zinc-300">{formatDuration(totalSecs)}</span>
          {" · "}
          {tasks.filter((t) => t.duration_secs != null).length} tracked
        </div>
      )}
      {tasks.map((task) => {
        const isActive = ACTIVE_STATUSES.has(task.status);
        const isHumanReview = fullModes.some(
          (m) =>
            m.name === task.mode && m.phases.some((p) => p.name === task.status && p.phase_type === "human_review"),
        );
        const reviewPhaseInstruction = isHumanReview
          ? fullModes.find((m) => m.name === task.mode)?.phases.find((p) => p.name === task.status)?.instruction
          : undefined;
        const purgeEtaMs =
          task.status === "purge" && task.updated_at
            ? new Date(task.updated_at).getTime() + 7 * 24 * 60 * 60 * 1000
            : null;
        const purgeRemainingMs = purgeEtaMs != null ? purgeEtaMs - Date.now() : null;
        return (
          <div
            key={task.id}
            className={cn(
              "rounded-xl border p-4",
              isHumanReview ? "border-emerald-500/20 bg-emerald-500/[0.03]" : "border-white/[0.07] bg-white/[0.03]",
            )}
          >
            <div className="flex items-start gap-2">
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-mono text-[11px] text-zinc-500">#{task.id}</span>
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
                  {task.status === "purge" && (
                    <span className="rounded bg-red-500/10 px-1.5 py-0.5 text-[9px] font-medium text-red-300">
                      purge{" "}
                      {purgeRemainingMs != null && purgeRemainingMs > 0
                        ? `in ${formatRemaining(purgeRemainingMs)}`
                        : "pending"}
                    </span>
                  )}
                  {task.status === "purged" && (
                    <span className="rounded bg-red-500/10 px-1.5 py-0.5 text-[9px] font-medium text-red-300">
                      purged
                    </span>
                  )}
                  {task.mode && isSWE && (
                    <span className="rounded bg-violet-500/10 px-1.5 py-0.5 text-[9px] font-medium text-violet-400">
                      {task.mode}
                    </span>
                  )}
                </div>
                <div className="mt-1 text-[13px] font-medium text-zinc-200">{task.title}</div>
                {task.description && (
                  <div className="mt-0.5 line-clamp-2 text-[12px] text-zinc-400">{task.description}</div>
                )}
              </div>
              <div className="flex shrink-0 items-center gap-1">
                {(task.status === "done" || task.status === "merged") && (
                  <button
                    onClick={() => setExpandedResults(expandedResults === task.id ? null : task.id)}
                    className="flex items-center gap-1 rounded-lg border border-white/[0.08] px-2.5 py-1 text-[11px] text-zinc-400 hover:border-emerald-500/30 hover:text-emerald-400 transition-colors"
                  >
                    {expandedResults === task.id ? "Hide" : "Results"}
                  </button>
                )}
                {(task.status === "done" || task.status === "merged" || isHumanReview) && (
                  <button
                    onClick={() => setCitationsId(citationsId === task.id ? null : task.id)}
                    className="flex items-center gap-1 rounded-lg border border-white/[0.08] px-2.5 py-1 text-[11px] text-zinc-400 hover:border-blue-500/30 hover:text-blue-400 transition-colors"
                  >
                    {citationsId === task.id ? "Hide" : "Citations"}
                  </button>
                )}
                {(task.revision_count ?? 0) > 0 && (
                  <button
                    onClick={() => setRevisionsId(revisionsId === task.id ? null : task.id)}
                    className="flex items-center gap-1 rounded-lg border border-white/[0.08] px-2.5 py-1 text-[11px] text-amber-500/60 hover:border-amber-500/30 hover:text-amber-400 transition-colors"
                  >
                    {revisionsId === task.id ? "Hide" : `Revisions (${task.revision_count})`}
                  </button>
                )}
                {isActive && (
                  <button
                    onClick={() => setExpandedStream(expandedStream === task.id ? null : task.id)}
                    className="flex items-center gap-1 rounded-lg border border-white/[0.08] px-2.5 py-1 text-[11px] text-zinc-400 hover:border-blue-500/30 hover:text-blue-400 transition-colors"
                  >
                    {expandedStream === task.id ? "Hide" : "Stream"}
                  </button>
                )}
                {task.status === "failed" && (
                  <>
                    <button
                      onClick={() => {
                        if (editingId === task.id) {
                          setEditingId(null);
                        } else {
                          setEditTitle(task.title);
                          setEditDesc(task.description || "");
                          setEditingId(task.id);
                        }
                      }}
                      className="flex items-center gap-1 rounded-lg border border-white/[0.08] px-2.5 py-1 text-[11px] text-zinc-400 hover:border-amber-500/30 hover:text-amber-400 transition-colors"
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
                      className="flex items-center gap-1 rounded-lg border border-white/[0.08] px-2.5 py-1 text-[12px] text-zinc-400 hover:border-blue-500/30 hover:text-blue-400 disabled:opacity-50 transition-colors"
                    >
                      <RotateCcw className="h-3 w-3" />
                      {retryingId === task.id ? "…" : "Retry"}
                    </button>
                  </>
                )}
              </div>
            </div>
            {editingId === task.id && (
              <div className="mt-2.5 space-y-2 rounded-xl border border-amber-500/20 bg-amber-500/5 p-3">
                <input
                  value={editTitle}
                  onChange={(e) => setEditTitle(e.target.value)}
                  className="w-full rounded-lg border border-white/[0.08] bg-black/30 px-3 py-1.5 text-[13px] text-zinc-200 outline-none focus:border-amber-500/40"
                  placeholder="Title"
                />
                <textarea
                  value={editDesc}
                  onChange={(e) => setEditDesc(e.target.value)}
                  rows={4}
                  className="w-full rounded-lg border border-white/[0.08] bg-black/30 px-3 py-1.5 text-[12px] text-zinc-300 outline-none focus:border-amber-500/40 resize-y"
                  placeholder="Description / instructions"
                />
              </div>
            )}
            {/* Human review panel */}
            {isHumanReview && (
              <div className="mt-2.5 rounded-xl border border-emerald-500/20 bg-emerald-500/[0.04] p-4 space-y-2.5">
                {reviewPhaseInstruction && (
                  <div className="text-[12px] text-emerald-400/70 leading-relaxed">{reviewPhaseInstruction}</div>
                )}
                <div className="flex items-center gap-2">
                  <button
                    onClick={async () => {
                      await approveTask(task.id);
                      queryClient.invalidateQueries({ queryKey: ["project_tasks", projectId] });
                    }}
                    className="rounded-lg bg-emerald-500/15 px-3 py-1.5 text-[12px] font-medium text-emerald-400 hover:bg-emerald-500/25 transition-colors"
                  >
                    Approve
                  </button>
                  <button
                    onClick={() => setReviewingId(reviewingId === task.id ? null : task.id)}
                    className="rounded-lg bg-amber-500/10 px-3 py-1.5 text-[12px] font-medium text-amber-400 hover:bg-amber-500/20 transition-colors"
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
                    className="rounded-lg bg-red-500/10 px-3 py-1.5 text-[12px] font-medium text-red-400 hover:bg-red-500/20 transition-colors"
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
                      className="w-full rounded-xl border border-amber-500/20 bg-black/30 px-3 py-2 text-[13px] text-zinc-200 outline-none focus:border-amber-500/40 resize-y placeholder:text-zinc-500"
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
                        className="rounded-lg bg-amber-500/15 px-3 py-1.5 text-[12px] font-medium text-amber-400 hover:bg-amber-500/25 disabled:opacity-40 transition-colors"
                      >
                        Send Revision Request
                      </button>
                      <button
                        onClick={() => {
                          setReviewingId(null);
                          setRevisionFeedback("");
                        }}
                        className="text-[12px] text-zinc-500 hover:text-zinc-300"
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
            <div className="mt-2 text-[11px] text-zinc-500">
              created {fmtDateTime(task.created_at)}
              {task.attempt > 0 && ` · attempt ${task.attempt}/${task.max_attempts}`}
              {task.duration_secs != null && ` · ${formatDuration(task.duration_secs)}`}
            </div>
            {isActive && expandedStream === task.id && <TaskStreamMini taskId={task.id} />}
            {expandedResults === task.id && <StructuredDataPanel taskId={task.id} />}
            {citationsId === task.id && <CitationPanel taskId={task.id} />}
            {revisionsId === task.id && <RevisionHistoryPanel taskId={task.id} />}
          </div>
        );
      })}
    </div>
  );
}

// ── Citation panel ───────────────────────────────────────────────────────────

function CitationPanel({ taskId }: { taskId: number }) {
  const [citations, setCitations] = useState<CitationVerification[]>([]);
  const [verifying, setVerifying] = useState(false);

  useEffect(() => {
    getTaskCitations(taskId)
      .then(setCitations)
      .catch(() => {});
  }, [taskId]);

  const statusColor = (s: string) => {
    if (s === "verified") return "text-emerald-400 bg-emerald-500/10";
    if (s === "flagged" || s === "error") return "text-red-400 bg-red-500/10";
    if (s === "format_valid") return "text-blue-400 bg-blue-500/10";
    return "text-zinc-400 bg-zinc-500/10";
  };

  return (
    <div className="mt-2 rounded-xl border border-white/[0.07] bg-white/[0.03] p-4 space-y-2.5">
      <div className="flex items-center justify-between">
        <span className="text-[12px] font-semibold text-zinc-300">
          Citations {citations.length > 0 && `(${citations.length})`}
        </span>
        <button
          onClick={async () => {
            setVerifying(true);
            try {
              const result = await verifyTaskCitations(taskId);
              setCitations(result.citations);
            } finally {
              setVerifying(false);
            }
          }}
          disabled={verifying}
          className="rounded-lg bg-blue-500/10 px-2.5 py-1 text-[11px] text-blue-400 hover:bg-blue-500/20 disabled:opacity-50 transition-colors"
        >
          {verifying ? "Verifying..." : citations.length > 0 ? "Re-verify" : "Verify All"}
        </button>
      </div>
      {citations.length > 0 && (
        <div className="space-y-1">
          {citations.map((c) => (
            <div key={c.id} className="flex items-start gap-2 text-[11px]">
              <span className={cn("shrink-0 rounded px-1.5 py-0.5 font-medium", statusColor(c.status))}>
                {c.status}
              </span>
              <span className="font-mono text-zinc-300 min-w-0 break-all">{c.citation_text}</span>
              {c.source && <span className="shrink-0 text-zinc-600">{c.source}</span>}
            </div>
          ))}
          <div className="text-[9px] text-zinc-600 pt-1">
            {citations.filter((c) => c.status === "verified").length} verified
            {" · "}
            {citations.filter((c) => c.status === "unverified").length} unverified
            {citations.some((c) => c.status === "error") &&
              ` · ${citations.filter((c) => c.status === "error").length} errors`}
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
    getRevisionHistory(taskId)
      .then(setHistory)
      .catch(() => {});
  }, [taskId]);

  if (!history || history.rounds.length === 0) {
    return (
      <div className="mt-2 rounded-xl border border-white/[0.07] bg-white/[0.03] p-4">
        <span className="text-[12px] text-zinc-500">No revision history</span>
      </div>
    );
  }

  return (
    <div className="mt-2 rounded-xl border border-white/[0.07] bg-white/[0.03] p-4 space-y-3">
      <div className="flex items-center gap-2">
        <span className="text-[12px] font-semibold text-zinc-300">Revision History</span>
        <span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-[9px] font-medium text-amber-400">
          {history.revision_count} revision{history.revision_count !== 1 ? "s" : ""}
        </span>
        {history.review_status && (
          <span
            className={cn(
              "rounded px-1.5 py-0.5 text-[9px] font-medium",
              history.review_status === "approved"
                ? "bg-emerald-500/10 text-emerald-400"
                : history.review_status === "rejected"
                  ? "bg-red-500/10 text-red-400"
                  : "bg-amber-500/10 text-amber-400",
            )}
          >
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
              <div className="text-[11px] font-medium text-zinc-300">
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
                    <div key={j} className="flex items-center gap-2 text-[11px]">
                      <span
                        className={cn(
                          "shrink-0 rounded px-1.5 py-0.5 font-medium",
                          p.exit_code === 0 ? "bg-emerald-500/10 text-emerald-400" : "bg-red-500/10 text-red-400",
                        )}
                      >
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

// ── Tab bar ───────────────────────────────────────────────────────────────────

type TabKey = "activity" | "documents" | "tasks";

const ALL_TABS: { key: TabKey; label: string; sweOnly?: boolean }[] = [
  { key: "documents", label: "Documents" },
  { key: "tasks", label: "Tasks", sweOnly: true },
  { key: "activity", label: "Activity", sweOnly: true },
];

// ── Main component ────────────────────────────────────────────────────────────

export function ProjectDetail({ projectId, onDocumentSelect, onDelete }: ProjectDetailProps) {
  const { data: project, isLoading } = useProjectDetail(projectId);
  const { isSWE } = useDashboardMode();
  const tabs = useMemo(() => ALL_TABS.filter((t) => !t.sweOnly || isSWE), [isSWE]);
  const [activeTab, setActiveTab] = useState<TabKey>("documents");
  const deleteMut = useDeleteProject();

  const handleDelete = () => {
    deleteMut.mutate(projectId, { onSuccess: () => onDelete?.() });
  };

  if (isLoading || !project) {
    return <div className="flex h-full items-center justify-center text-[13px] text-zinc-400">Loading...</div>;
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ProjectHeader project={project} onDelete={handleDelete} />
      <div className="shrink-0 flex gap-0 border-b border-white/[0.07] px-5">
        {tabs.map((tab) => (
          <button
            key={tab.key}
            onClick={() => setActiveTab(tab.key)}
            className={cn(
              "border-b-2 px-4 py-3 text-[13px] font-medium transition-colors",
              activeTab === tab.key
                ? "border-blue-500 text-zinc-200"
                : "border-transparent text-zinc-400 hover:text-zinc-200",
            )}
          >
            {tab.label}
          </button>
        ))}
      </div>

      <div className="min-h-0 flex-1 overflow-hidden">
        {activeTab === "activity" && (
          <div className="h-full overflow-y-auto">
            <ActivityTab projectId={projectId} />
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
      </div>
    </div>
  );
}
