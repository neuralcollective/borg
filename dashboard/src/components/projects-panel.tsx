import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  browseProjectCloudFiles,
  completeProjectUploadSession,
  createProjectUploadSession,
  createProject,
  deleteProjectCloudConnection,
  fetchProjectFileText,
  getProjectUploadSessionStatus,
  importProjectCloudFiles,
  listProjectUploadSessions,
  reextractProjectFile,
  retryProjectUploadSession,
  uploadProjectUploadChunk,
  useProjectCloudConnections,
  useSettings,
  useStatus,
  useModes,
  useProjectFiles,
  useProjects,
  searchDocuments,
} from "@/lib/api";
import type { CloudBrowseItem, CloudConnection, FtsSearchResult } from "@/lib/api";
import type { UploadSession } from "@/lib/api";
import { Eye, FileText, ArrowLeft, Search, RotateCw, Folder, Upload, X } from "lucide-react";
import { FilePreviewModal, isPreviewable } from "./file-preview-modal";
import type { ProjectFile, ProjectDocument } from "@/lib/types";
import { cn } from "@/lib/utils";
import { MatterDetail } from "./matter-detail";
import { MarkdownLegalViewer } from "./viewers/markdown-legal-viewer";
import { RedlineViewer } from "./viewers/redline-viewer";
import { useProjectDocumentVersions } from "@/lib/api";
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

const CLOUD_PROVIDERS = [
  { id: "dropbox", label: "Dropbox", clientIdKey: "dropbox_client_id", clientSecretKey: "dropbox_client_secret" },
  { id: "google_drive", label: "Google Drive", clientIdKey: "google_client_id", clientSecretKey: "google_client_secret" },
  { id: "onedrive", label: "OneDrive", clientIdKey: "ms_client_id", clientSecretKey: "ms_client_secret" },
] as const;
const MAX_CLOUD_IMPORT_SELECTION = 1000;
const RESUMABLE_UPLOAD_CHUNK_SIZE = 8 * 1024 * 1024;
const RESUMABLE_UPLOAD_PARALLEL_CHUNKS = 4;
const RESUMABLE_UPLOAD_CHUNK_RETRIES = 3;
const UPLOAD_SESSION_KEY_PREFIX = "borg-upload-session";

type FileUploadProgress = {
  id: string;
  fileName: string;
  uploadedBytes: number;
  totalBytes: number;
  status: "starting" | "uploading" | "processing" | "done" | "failed";
  sessionId?: number;
  error?: string;
};

function cloudProviderLabel(provider: string): string {
  return CLOUD_PROVIDERS.find((p) => p.id === provider)?.label ?? provider;
}

function DropboxIcon() {
  return (
    <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden>
      <path fill="#0D63D6" d="m6.1 3.2-4.7 3 4.7 3 4.7-3-4.7-3Zm11.8 0-4.7 3 4.7 3 4.7-3-4.7-3ZM6.1 10.7l-4.7 3 4.7 3 4.7-3-4.7-3Zm11.8 0-4.7 3 4.7 3 4.7-3-4.7-3ZM12 14.9l-4.7 3 4.7 3 4.7-3-4.7-3Z" />
    </svg>
  );
}

function GoogleDriveIcon() {
  return (
    <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden>
      <path fill="#0F9D58" d="M6.5 20.3h11l-2.7-4.7h-11l2.7 4.7Z" />
      <path fill="#FFC107" d="m12 3.7 5.5 9.5h5.4L17.4 3.7H12Z" />
      <path fill="#4285F4" d="M1.1 13.2h5.4L12 3.7H6.6L1.1 13.2Z" />
    </svg>
  );
}

function OneDriveIcon() {
  return (
    <svg viewBox="0 0 24 24" className="h-4 w-4" aria-hidden>
      <path fill="#0078D4" d="M10.2 9a5.4 5.4 0 0 1 10.2 2.4h.2a3.4 3.4 0 1 1 0 6.8H6.5a4.5 4.5 0 0 1-.8-8.9A5.7 5.7 0 0 1 10.2 9Z" />
    </svg>
  );
}

function CloudProviderIcon({ provider }: { provider: string }) {
  if (provider === "dropbox") return <DropboxIcon />;
  if (provider === "google_drive") return <GoogleDriveIcon />;
  return <OneDriveIcon />;
}

function DocumentViewWrapper({
  projectId,
  doc,
  viewMode,
  onBack,
  onToggleMode,
  defaultTemplateId,
}: {
  projectId: number;
  doc: ProjectDocument;
  viewMode: "view" | "redline";
  onBack: () => void;
  onToggleMode: () => void;
  defaultTemplateId?: number | null;
}) {
  const { data: versions = [] } = useProjectDocumentVersions(projectId, doc.task_id, doc.file_name);

  return (
    <div className="flex h-full flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-white/[0.07] px-4 py-3">
        <button
          onClick={onBack}
          className="flex items-center gap-1.5 text-[12px] text-zinc-400 hover:text-zinc-200 transition-colors"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
          Back to matter
        </button>
        <span className="text-[12px] text-zinc-600">·</span>
        <span className="truncate text-[12px] text-zinc-400">{doc.file_name}</span>
        {versions.length >= 2 && (
          <button
            onClick={onToggleMode}
            className={cn(
              "ml-auto rounded-lg border px-3 py-1 text-[12px] font-medium transition-colors",
              viewMode === "redline"
                ? "border-blue-500/30 bg-blue-500/10 text-blue-400"
                : "border-white/[0.08] text-zinc-400 hover:border-white/[0.14] hover:text-zinc-200"
            )}
          >
            {viewMode === "redline" ? "Document View" : "Compare Versions"}
          </button>
        )}
      </div>
      <div className="min-h-0 flex-1">
        {viewMode === "redline" && versions.length >= 2 ? (
          <RedlineViewer
            projectId={projectId}
            taskId={doc.task_id}
            path={doc.file_name}
            versions={versions}
          />
        ) : (
          <MarkdownLegalViewer
            projectId={projectId}
            taskId={doc.task_id}
            path={doc.file_name}
            defaultTemplateId={defaultTemplateId}
          />
        )}
      </div>
    </div>
  );
}

export function ProjectsPanel() {
  const { data: projects = [], refetch: refetchProjects } = useProjects();
  const { data: status } = useStatus();
  const { data: modes = [] } = useModes();
  const { data: settings } = useSettings();
  const [selectedProjectId, setSelectedProjectId] = useState<number | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [ftsQuery, setFtsQuery] = useState("");
  const [ftsResults, setFtsResults] = useState<FtsSearchResult[]>([]);
  const [ftsSearching, setFtsSearching] = useState(false);
  const ftsDebounce = useRef<ReturnType<typeof setTimeout>>(null);
  const [newProjectName, setNewProjectName] = useState("");
  const [newProjectMode, setNewProjectMode] = useState("");
  const [newProjectJurisdiction, setNewProjectJurisdiction] = useState("");
  const [creating, setCreating] = useState(false);
  const [jurisdictionFilter, setJurisdictionFilter] = useState<string>("all");

  const jurisdictions = useMemo(() => {
    const set = new Set<string>();
    for (const p of projects) {
      if (p.jurisdiction?.trim()) set.add(p.jurisdiction.trim());
    }
    return [...set].sort();
  }, [projects]);

  const isLegalMode = useMemo(() => {
    const repos = status?.watched_repos;
    if (repos?.length) {
      const primary = repos.find((r) => r.is_self) ?? repos[0];
      return primary.mode === "lawborg" || primary.mode === "legal";
    }
    return projects.some((p) => p.mode === "lawborg" || p.mode === "legal");
  }, [status, projects]);

  const filteredProjects = useMemo(() => {
    let filtered = projects;
    if (jurisdictionFilter !== "all") {
      filtered = filtered.filter((p) => p.jurisdiction?.trim() === jurisdictionFilter);
    }
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      filtered = filtered.filter(
        (p) =>
          p.name.toLowerCase().includes(q) ||
          (p.jurisdiction && p.jurisdiction.toLowerCase().includes(q))
      );
    }
    return filtered;
  }, [projects, searchQuery, jurisdictionFilter]);

  const selectedProject =
    projects.find((p) => p.id === selectedProjectId) ?? projects[0] ?? null;
  const activeProjectId = selectedProject?.id ?? null;
  const [fileSearch, setFileSearch] = useState("");
  const [filePageStack, setFilePageStack] = useState<Array<{ cursor: string | null; offset: number }>>([
    { cursor: null, offset: 0 },
  ]);
  const currentFilePage = filePageStack[filePageStack.length - 1] ?? { cursor: null, offset: 0 };

  const {
    data: filePage,
    refetch: refetchFiles,
    isFetching: filesLoading,
  } = useProjectFiles(activeProjectId, {
    limit: 50,
    offset: currentFilePage.offset,
    cursor: currentFilePage.cursor,
    q: fileSearch,
  });
  const files = filePage?.items ?? [];
  const fileSummary = filePage?.summary;
  const {
    data: cloudConnections = [],
    refetch: refetchCloudConnections,
    isFetching: cloudConnectionsLoading,
  } = useProjectCloudConnections(activeProjectId);

  const [previewFile, setPreviewFile] = useState<ProjectFile | null>(null);
  const [selectedDoc, setSelectedDoc] = useState<ProjectDocument | null>(null);
  const [docViewMode, setDocViewMode] = useState<"view" | "redline">("view");
  const [uploading, setUploading] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const [textViewFile, setTextViewFile] = useState<{ id: number; name: string; text: string } | null>(null);
  const [extracting, setExtracting] = useState<number | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [cloudMessage, setCloudMessage] = useState<{ type: "success" | "error"; text: string } | null>(null);
  const [cloudModalOpen, setCloudModalOpen] = useState(false);
  const [cloudModalConn, setCloudModalConn] = useState<CloudConnection | null>(null);
  const [cloudItems, setCloudItems] = useState<CloudBrowseItem[]>([]);
  const [cloudLoading, setCloudLoading] = useState(false);
  const [cloudLoadError, setCloudLoadError] = useState<string | null>(null);
  const [cloudCursor, setCloudCursor] = useState<string | null>(null);
  const [cloudHasMore, setCloudHasMore] = useState(false);
  const [cloudSelected, setCloudSelected] = useState<Record<string, CloudBrowseItem>>({});
  const [cloudImporting, setCloudImporting] = useState(false);
  const [cloudBreadcrumbs, setCloudBreadcrumbs] = useState<Array<{ id?: string; name: string }>>([{ name: "Root" }]);
  const [uploadSessions, setUploadSessions] = useState<UploadSession[]>([]);
  const [uploadSessionCounts, setUploadSessionCounts] = useState<Record<string, number>>({});
  const [uploadSessionsLoading, setUploadSessionsLoading] = useState(false);
  const [uploadProgress, setUploadProgress] = useState<FileUploadProgress[]>([]);
  const [dragOver, setDragOver] = useState(false);
  const dropRef = useRef<HTMLDivElement>(null);


  const totalBytes = fileSummary?.total_bytes ?? 0;
  const currentCloudFolderId = cloudBreadcrumbs[cloudBreadcrumbs.length - 1]?.id;
  const publicUrl = settings?.public_url?.trim() || "";
  const publicUrlValid = useMemo(() => {
    if (!publicUrl) return false;
    try {
      const parsed = new URL(publicUrl);
      return parsed.protocol === "http:" || parsed.protocol === "https:";
    } catch {
      return false;
    }
  }, [publicUrl]);
  const maxCloudImportSelection = Math.max(
    1,
    settings?.cloud_import_max_batch_files ?? MAX_CLOUD_IMPORT_SELECTION
  );
  const projectMaxBytes = Math.max(1, settings?.project_max_bytes ?? 100 * 1024 * 1024);

  const updateUploadProgress = useCallback((id: string, patch: Partial<FileUploadProgress>) => {
    setUploadProgress((prev) =>
      prev.map((entry) => (entry.id === id ? { ...entry, ...patch } : entry))
    );
  }, []);

  const refreshUploadSessions = useCallback(async () => {
    if (!activeProjectId) return;
    const data = await listProjectUploadSessions(activeProjectId, 30);
    setUploadSessions(data.sessions || []);
    setUploadSessionCounts(data.counts || {});
  }, [activeProjectId]);

  useEffect(() => {
    if (!selectedProjectId && projects.length > 0) {
      setSelectedProjectId(projects[0].id);
    }
  }, [projects, selectedProjectId]);

  useEffect(() => {
    setSelectedDoc(null);
    setDocViewMode("view");
  }, [selectedProjectId]);

  useEffect(() => {
    setFilePageStack([{ cursor: null, offset: 0 }]);
    setFileSearch("");
  }, [activeProjectId]);

  useEffect(() => {
    if (!activeProjectId) return;
    let cancelled = false;
    const load = async () => {
      setUploadSessionsLoading(true);
      try {
        const data = await listProjectUploadSessions(activeProjectId, 30);
        if (cancelled) return;
        setUploadSessions(data.sessions || []);
        setUploadSessionCounts(data.counts || {});
      } finally {
        if (!cancelled) setUploadSessionsLoading(false);
      }
    };
    load();
    const t = setInterval(load, 5000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, [activeProjectId]);

  useEffect(() => {
    const hash = window.location.hash || "";
    const queryIdx = hash.indexOf("?");
    if (queryIdx < 0) return;

    const params = new URLSearchParams(hash.slice(queryIdx + 1));
    const connected = params.get("cloud_connected");
    const error = params.get("cloud_error");
    const provider = params.get("provider");
    const projectIdParam = params.get("project_id");
    if (!connected && !error) return;

    if (projectIdParam) {
      const pid = Number(projectIdParam);
      if (Number.isFinite(pid)) setSelectedProjectId(pid);
    }

    if (connected) {
      setCloudMessage({ type: "success", text: `${cloudProviderLabel(connected)} connected.` });
      refetchCloudConnections();
    } else if (error) {
      const prefix = provider ? `${cloudProviderLabel(provider)}: ` : "";
      if (error === "access_denied") {
        setCloudMessage({ type: "error", text: `${prefix}authorization was denied.` });
      } else if (error === "token_exchange") {
        setCloudMessage({ type: "error", text: `${prefix}token exchange failed. Check client ID/secret and callback URL.` });
      } else if (error === "missing_public_url") {
        setCloudMessage({ type: "error", text: "Set a valid Public URL in Settings before connecting cloud providers." });
      } else if (error === "missing_credentials") {
        setCloudMessage({ type: "error", text: `${prefix}credentials are missing in Settings > Cloud Storage.` });
      } else {
        setCloudMessage({ type: "error", text: `${prefix}connection failed (${error}).` });
      }
    }

    const cleanHash = hash.slice(0, queryIdx) || "#/projects";
    window.history.replaceState(null, "", `${window.location.pathname}${window.location.search}${cleanHash}`);
  }, [refetchCloudConnections]);

  useEffect(() => {
    if (!activeProjectId) {
      setCloudModalOpen(false);
      setCloudModalConn(null);
      setCloudItems([]);
      setCloudSelected({});
      setCloudBreadcrumbs([{ name: "Root" }]);
    }
  }, [activeProjectId]);

  function handleFtsSearch(q: string) {
    setFtsQuery(q);
    if (ftsDebounce.current) clearTimeout(ftsDebounce.current);
    if (!q.trim()) { setFtsResults([]); return; }
    ftsDebounce.current = setTimeout(async () => {
      setFtsSearching(true);
      try {
        const results = await searchDocuments(q.trim());
        setFtsResults(results);
      } catch { setFtsResults([]); }
      finally { setFtsSearching(false); }
    }, 300);
  }

  async function handleCreateProject() {
    const name = newProjectName.trim();
    if (!name || creating) return;
    setCreating(true);
    try {
      const opts = newProjectJurisdiction.trim()
        ? { jurisdiction: newProjectJurisdiction.trim() }
        : {};
      const effectiveMode = isLegalMode ? "lawborg" : (newProjectMode || "general");
      const created = await createProject(name, effectiveMode, opts);
      setNewProjectName("");
      setNewProjectJurisdiction("");
      await refetchProjects();
      setSelectedProjectId(created.id);
    } finally {
      setCreating(false);
    }
  }

  function uploadSessionStorageKey(projectId: number, file: File): string {
    return `${UPLOAD_SESSION_KEY_PREFIX}:${projectId}:${file.name}:${file.size}:${file.lastModified}`;
  }

  function buildChunkQueueFromRanges(ranges: Array<[number, number]>, totalChunks: number): number[] {
    const queue: number[] = [];
    if (ranges.length === 0) {
      for (let idx = 0; idx < totalChunks; idx += 1) queue.push(idx);
      return queue;
    }
    for (const [startRaw, endRaw] of ranges) {
      const start = Math.max(0, startRaw);
      const end = Math.min(totalChunks - 1, endRaw);
      for (let idx = start; idx <= end; idx += 1) queue.push(idx);
    }
    return queue;
  }

async function uploadChunkQueue(
  projectId: number,
  sessionId: number,
  file: File,
    chunkSize: number,
    queue: number[],
    onChunkUploaded: (bytes: number) => void,
  ) {
    const workerCount = Math.min(RESUMABLE_UPLOAD_PARALLEL_CHUNKS, queue.length);
  await Promise.all(
    Array.from({ length: workerCount }, async () => {
      while (true) {
        const chunkIndex = queue.shift();
        if (chunkIndex === undefined) return;
        const start = chunkIndex * chunkSize;
        const end = Math.min(start + chunkSize, file.size);
        const blob = file.slice(start, end);
        let uploaded = false;
        let lastErr: unknown = null;
        for (let attempt = 1; attempt <= RESUMABLE_UPLOAD_CHUNK_RETRIES; attempt += 1) {
          try {
            await uploadProjectUploadChunk(projectId, sessionId, chunkIndex, blob);
            uploaded = true;
            break;
          } catch (err) {
            lastErr = err;
            if (attempt < RESUMABLE_UPLOAD_CHUNK_RETRIES) {
              await new Promise((resolve) => setTimeout(resolve, attempt * 500));
            }
          }
        }
        if (!uploaded) {
          throw lastErr instanceof Error ? lastErr : new Error("chunk upload failed");
        }
        onChunkUploaded(blob.size);
      }
    }),
  );
}

  async function handleUpload(filesToUpload: FileList | File[]) {
    if (!activeProjectId || uploading) return;
    setUploading(true);
    setUploadError(null);
    const files = Array.from(filesToUpload).filter((file) => file.size > 0);
    if (files.length === 0) {
      setUploadError("No non-empty files selected.");
      setUploading(false);
      return;
    }
    const startingProgress: FileUploadProgress[] = files.map((file, idx) => ({
      id: `${Date.now()}-${idx}-${file.name}`,
      fileName: file.name,
      totalBytes: file.size,
      uploadedBytes: 0,
      status: "starting",
    }));
    setUploadProgress(startingProgress);
    const fileFailures: Array<{ fileName: string; error: string }> = [];
    try {
      for (let fileIndex = 0; fileIndex < files.length; fileIndex += 1) {
        const file = files[fileIndex];
        const progressId = startingProgress[fileIndex]?.id ?? `${Date.now()}-${fileIndex}-${file.name}`;
        try {
          const chunkSize = RESUMABLE_UPLOAD_CHUNK_SIZE;
          const totalChunks = Math.max(1, Math.ceil(file.size / chunkSize));
          const sessionKey = uploadSessionStorageKey(activeProjectId, file);
          let sessionId = Number(localStorage.getItem(sessionKey) || "");
          let status = null as Awaited<ReturnType<typeof getProjectUploadSessionStatus>> | null;

          if (!(Number.isFinite(sessionId) && sessionId > 0)) {
            sessionId = 0;
          } else {
            try {
              status = await getProjectUploadSessionStatus(activeProjectId, sessionId);
              if (status.session.status !== "uploading") {
                localStorage.removeItem(sessionKey);
              }
            } catch {
              sessionId = 0;
              status = null;
              localStorage.removeItem(sessionKey);
            }
          }

          if (!status) {
            const created = await createProjectUploadSession(activeProjectId, {
              file_name: file.name,
              mime_type: file.type || "application/octet-stream",
              file_size: file.size,
              chunk_size: chunkSize,
              total_chunks: totalChunks,
              is_zip: file.name.toLowerCase().endsWith(".zip"),
            });
            sessionId = created.session_id;
            localStorage.setItem(sessionKey, String(sessionId));
            status = await getProjectUploadSessionStatus(activeProjectId, sessionId);
          }

          updateUploadProgress(progressId, {
            sessionId,
            uploadedBytes: status.session.uploaded_bytes,
            status: status.session.status === "uploading" ? "uploading" : status.session.status,
          });

          if (status.session.status === "uploading") {
            const queue = buildChunkQueueFromRanges(status.missing_ranges, status.total_chunks);
            await uploadChunkQueue(activeProjectId, sessionId, file, status.session.chunk_size, queue, (bytes) => {
              setUploadProgress((prev) =>
                prev.map((entry) =>
                  entry.id === progressId
                    ? { ...entry, uploadedBytes: Math.min(entry.uploadedBytes + bytes, entry.totalBytes), status: "uploading" }
                    : entry
                ),
              );
            });
            await completeProjectUploadSession(activeProjectId, sessionId);
            localStorage.removeItem(sessionKey);
            updateUploadProgress(progressId, {
              uploadedBytes: file.size,
              status: "processing",
            });
          } else if (status.session.status === "done") {
            localStorage.removeItem(sessionKey);
            updateUploadProgress(progressId, {
              uploadedBytes: file.size,
              status: "done",
            });
          } else if (status.session.status === "failed") {
            setUploadProgress((prev) =>
              prev.map((entry) =>
                entry.id === progressId
                  ? { ...entry, status: "failed", error: status.session.error || "upload processing failed" }
                  : entry
              ),
            );
          } else {
            updateUploadProgress(progressId, {
              uploadedBytes: file.size,
              status: "processing",
            });
          }
        } catch (err) {
          const msg = err instanceof Error ? err.message : "upload failed";
          fileFailures.push({ fileName: file.name, error: msg });
          updateUploadProgress(progressId, {
            status: "failed",
            error: msg,
          });
        }
      }
      setFilePageStack([{ cursor: null, offset: 0 }]);
      await refetchFiles();
      await refreshUploadSessions();
      if (fileFailures.length > 0) {
        const sample = fileFailures[0];
        const summary = fileFailures.length === 1
          ? `Upload failed for ${sample.fileName}: ${sample.error}`
          : `${fileFailures.length} files failed (first: ${sample.fileName}: ${sample.error})`;
        setUploadError(summary);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : "upload failed";
      setUploadError(msg === "413" ? `Upload exceeds project limit (${formatBytes(projectMaxBytes)}).` : `Upload failed (${msg})`);
      setUploadProgress((prev) =>
        prev.map((entry) => (entry.status === "done" ? entry : { ...entry, status: "failed", error: msg })),
      );
    } finally {
      setUploading(false);
      if (fileInputRef.current) fileInputRef.current.value = "";
    }
  }

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    const droppedFiles = e.dataTransfer.files;
    if (droppedFiles.length > 0) handleUpload(droppedFiles);
  }, [activeProjectId, uploading]);

  async function retryUploadSession(sessionId: number) {
    if (!activeProjectId) return;
    try {
      await retryProjectUploadSession(activeProjectId, sessionId);
      await refreshUploadSessions();
    } catch {
      // no-op
    }
  }

  function hasCloudCredentials(provider: (typeof CLOUD_PROVIDERS)[number]) {
    if (!settings) return false;
    const id = settings[provider.clientIdKey] ?? "";
    const secret = settings[provider.clientSecretKey] ?? "";
    return id.trim().length > 0 && secret.trim().length > 0;
  }

  async function loadCloudFolder(connection: CloudConnection, folderId?: string, opts?: { append?: boolean; cursor?: string }) {
    if (!activeProjectId) return;
    setCloudLoading(true);
    setCloudLoadError(null);
    try {
      const data = await browseProjectCloudFiles(activeProjectId, connection.id, {
        folder_id: folderId,
        cursor: opts?.cursor,
      });
      setCloudItems((prev) => (opts?.append ? [...prev, ...(data.items || [])] : (data.items || [])));
      const nextCursor = data.cursor ?? data.next_page_token ?? null;
      setCloudCursor(nextCursor);
      setCloudHasMore(Boolean(data.has_more || data.next_page_token));
    } catch (err) {
      const msg = err instanceof Error ? err.message : "Failed to browse cloud files";
      setCloudLoadError(msg);
    } finally {
      setCloudLoading(false);
    }
  }

  function connectCloudProvider(provider: (typeof CLOUD_PROVIDERS)[number]["id"]) {
    if (!activeProjectId) return;
    if (!publicUrlValid) {
      setCloudMessage({ type: "error", text: "Set a valid Public URL in Settings before connecting cloud providers." });
      return;
    }
    window.location.href = `/api/cloud/${provider}/auth?project_id=${activeProjectId}`;
  }

  async function openCloudBrowser(connection: CloudConnection) {
    setCloudModalConn(connection);
    setCloudModalOpen(true);
    setCloudSelected({});
    setCloudBreadcrumbs([{ name: "Root" }]);
    setCloudCursor(null);
    setCloudHasMore(false);
    await loadCloudFolder(connection);
  }

  async function disconnectCloudConnection(connection: CloudConnection) {
    if (!activeProjectId) return;
    if (!confirm(`Disconnect ${cloudProviderLabel(connection.provider)} account ${connection.account_email || connection.id}?`)) return;
    try {
      await deleteProjectCloudConnection(activeProjectId, connection.id);
      setCloudMessage({ type: "success", text: `${cloudProviderLabel(connection.provider)} disconnected.` });
      await refetchCloudConnections();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "disconnect failed";
      setCloudMessage({ type: "error", text: `Failed to disconnect (${msg}).` });
    }
  }

  async function importSelectedCloudFiles() {
    if (!activeProjectId || !cloudModalConn || cloudImporting) return;
    const filesToImport = Object.values(cloudSelected)
      .filter((item) => item.type === "file")
      .map((item) => ({ id: item.id, name: item.name, size: item.size }));
    if (filesToImport.length === 0) return;
    if (filesToImport.length > maxCloudImportSelection) {
      setCloudLoadError(`Please select at most ${maxCloudImportSelection} files per import.`);
      return;
    }

    setCloudImporting(true);
    try {
      await importProjectCloudFiles(activeProjectId, cloudModalConn.id, filesToImport);
      setCloudMessage({ type: "success", text: `Imported ${filesToImport.length} file(s).` });
      setCloudModalOpen(false);
      setCloudSelected({});
      setFilePageStack([{ cursor: null, offset: 0 }]);
      await refetchFiles();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "import failed";
      setCloudLoadError(`Import failed (${msg}).`);
    } finally {
      setCloudImporting(false);
    }
  }

  return (
    <div className="flex h-full min-h-0">
      <div className="w-[270px] shrink-0 border-r border-[#2a2520] bg-[#0f0e0c] p-4">
        <div className="mb-3 flex items-center justify-between">
          <span className="text-[12px] font-semibold uppercase tracking-wide text-[#6b6459]">
            Projects
          </span>
          {jurisdictions.length > 0 && (
            <select
              value={jurisdictionFilter}
              onChange={(e) => setJurisdictionFilter(e.target.value)}
              className="rounded-lg border border-[#2a2520] bg-[#1c1a17] px-2 py-1 text-[11px] text-[#e8e0d4] outline-none focus:border-amber-500/30"
            >
              <option value="all">All jurisdictions</option>
              {jurisdictions.map((j) => (
                <option key={j} value={j}>{j}</option>
              ))}
            </select>
          )}
        </div>
        <div className="relative mb-3">
          <Search className="absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-[#6b6459]" />
          <input
            value={ftsQuery}
            onChange={(e) => handleFtsSearch(e.target.value)}
            placeholder="Search all documents..."
            className="w-full rounded-xl border border-[#2a2520] bg-[#1c1a17] pl-8 pr-3 py-2.5 text-[13px] text-[#e8e0d4] outline-none placeholder:text-[#6b6459] focus:border-amber-500/30"
          />
        </div>
        {ftsQuery.trim() && (ftsSearching || ftsResults.length > 0) ? (
          <div className="space-y-1.5 overflow-y-auto mb-3" style={{ maxHeight: "calc(100vh - 280px)" }}>
            {ftsSearching && <div className="text-[11px] text-[#6b6459] px-1">Searching…</div>}
            {ftsResults.map((r, i) => (
              <button
                key={`${r.task_id}-${r.file_path}-${i}`}
                onClick={() => { setSelectedProjectId(r.project_id); setFtsQuery(""); setFtsResults([]); }}
                className="w-full rounded-xl border border-[#2a2520] bg-[#1c1a17] px-3 py-2.5 text-left hover:bg-[#232019] transition-colors"
              >
                <div className="text-[11px] text-[#6b6459] truncate flex items-center gap-1.5">
                  {r.project_name}
                  {r.source === "semantic" && <span className="px-1.5 py-0.5 rounded-lg bg-violet-900/50 text-violet-300 text-[10px]">semantic</span>}
                </div>
                {r.title_snippet && <div className="text-[12px] text-[#e8e0d4] truncate mt-0.5" dangerouslySetInnerHTML={{ __html: r.title_snippet }} />}
                <div className="text-[11px] text-[#9c9486] line-clamp-2 mt-0.5" dangerouslySetInnerHTML={{ __html: r.content_snippet }} />
              </button>
            ))}
            {!ftsSearching && ftsResults.length === 0 && (
              <div className="text-[11px] text-[#6b6459] px-1">No results.</div>
            )}
          </div>
        ) : (
          <>
        {projects.length > 5 && (
          <input
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Filter projects..."
            className="mb-3 w-full rounded-xl border border-[#2a2520] bg-[#1c1a17] px-3 py-2 text-[13px] text-[#e8e0d4] outline-none placeholder:text-[#6b6459] focus:border-amber-500/30"
          />
        )}
        <div className="space-y-1 overflow-y-auto" style={{ maxHeight: "calc(100vh - 300px)" }}>
          {filteredProjects.map((p) => (
            <button
              key={p.id}
              onClick={() => setSelectedProjectId(p.id)}
              className={cn(
                "w-full rounded-xl px-3 py-2.5 text-left text-[13px] transition-colors",
                p.id === activeProjectId
                  ? "bg-amber-500/[0.08] text-[#e8e0d4] font-medium"
                  : "text-[#9c9486] hover:bg-[#1c1a17]"
              )}
            >
              <span className="truncate">{p.name}</span>
              {p.jurisdiction?.trim() && jurisdictionFilter === "all" && (
                <span className="mt-0.5 block truncate text-[10px] text-[#6b6459]">{p.jurisdiction}</span>
              )}
            </button>
          ))}
          {projects.length === 0 && (
            <div className="flex flex-col items-center justify-center rounded-xl border border-dashed border-[#2a2520] px-4 py-6 text-center">
              <Folder className="h-6 w-6 text-[#6b6459] mb-2" />
              <div className="text-[12px] text-[#9c9486]">No projects yet</div>
              <div className="text-[11px] text-[#6b6459] mt-0.5">Create one below to get started</div>
            </div>
          )}
        </div>
          </>
        )}
        <div className="mt-4 border-t border-[#2a2520] pt-4">
          <input
            value={newProjectName}
            onChange={(e) => setNewProjectName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleCreateProject()}
            placeholder="New project name"
            className="w-full rounded-xl border border-[#2a2520] bg-[#1c1a17] px-4 py-2.5 text-[14px] text-[#e8e0d4] outline-none placeholder:text-[#6b6459] focus:border-amber-500/30"
          />
          {!isLegalMode && (
            <select
              value={newProjectMode}
              onChange={(e) => setNewProjectMode(e.target.value)}
              className="mt-2.5 w-full rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-2 text-[13px] text-[#e8e0d4] outline-none"
            >
              <option value="general">general</option>
              {modes.map((mode) => (
                <option key={mode.name} value={mode.name}>
                  {mode.experimental
                    ? `${mode.label ?? mode.name} (experimental)`
                    : (mode.label ?? mode.name)}
                </option>
              ))}
            </select>
          )}
          {(isLegalMode || ["lawborg", "legal"].includes(newProjectMode)) && (
            <input
              value={newProjectJurisdiction}
              onChange={(e) => setNewProjectJurisdiction(e.target.value)}
              placeholder="Jurisdiction (e.g. England & Wales)"
              className="mt-2.5 w-full rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-2 text-[13px] text-[#e8e0d4] outline-none placeholder:text-[#6b6459] focus:border-amber-500/30"
            />
          )}
          <button
            onClick={handleCreateProject}
            disabled={creating || !newProjectName.trim()}
            className="mt-2.5 w-full rounded-lg bg-amber-500/20 px-3 py-2.5 text-[13px] font-medium text-amber-300 hover:bg-amber-500/30 transition-colors disabled:cursor-not-allowed disabled:text-[#6b6459]"
          >
            {creating ? "Creating..." : "Create Project"}
          </button>
        </div>
      </div>

      <div className="flex min-w-0 flex-1 flex-col">
        {!selectedProject ? (
          <div className="flex h-full items-center justify-center">
            <div className="max-w-[360px] text-center">
              <div className="mx-auto mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-[#1c1a17] ring-1 ring-amber-900/20">
                <Folder className="h-7 w-7 text-[#6b6459]" />
              </div>
              <div className="text-[16px] font-semibold text-[#e8e0d4]">Get Started</div>
              <div className="mt-2 text-[13px] leading-relaxed text-[#9c9486]">
                Create a project in the sidebar to start. Each project gets its own
                document store and can be chatted with via the Chat tab.
              </div>
              <div className="mt-5 space-y-2.5 text-left text-[13px] text-[#9c9486]">
                <div className="rounded-xl border border-[#2a2520] bg-[#151412] px-4 py-3">
                  <span className="text-[#e8e0d4] font-medium">1.</span> Name your project and select a mode
                </div>
                <div className="rounded-xl border border-[#2a2520] bg-[#151412] px-4 py-3">
                  <span className="text-[#e8e0d4] font-medium">2.</span> Upload reference documents
                </div>
                <div className="rounded-xl border border-[#2a2520] bg-[#151412] px-4 py-3">
                  <span className="text-[#e8e0d4] font-medium">3.</span> Create tasks from the Tasks tab or chat about your docs
                </div>
              </div>
            </div>
          </div>
        ) : (selectedProject.mode === "lawborg" || selectedProject.mode === "legal") ? (
          selectedDoc ? (
            <DocumentViewWrapper
              projectId={selectedProject.id}
              doc={selectedDoc}
              viewMode={docViewMode}
              onBack={() => { setSelectedDoc(null); setDocViewMode("view"); }}
              onToggleMode={() => setDocViewMode(docViewMode === "view" ? "redline" : "view")}
              defaultTemplateId={undefined}
            />
          ) : (
            <MatterDetail projectId={selectedProject.id} onDocumentSelect={setSelectedDoc} onDelete={() => setSelectedProjectId(null)} />
          )
        ) : (
          <div className="flex flex-col h-full overflow-y-auto">
            <div className="mx-auto w-full max-w-3xl px-6 py-8 space-y-6">
              {/* Header */}
              <div className="flex items-center gap-3">
                <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-[#1c1a17] ring-1 ring-amber-900/20">
                  <Folder className="h-6 w-6 text-amber-400/60" />
                </div>
                <div>
                  <h2 className="text-[20px] font-semibold text-[#e8e0d4]">{selectedProject.name}</h2>
                  <p className="text-[13px] text-[#6b6459]">
                    Project documents — scoped to this project only.
                    Chat with these docs via the Chat tab.
                  </p>
                </div>
              </div>

              {/* Search & stats */}
              <div className="flex items-center gap-3">
                <div className="relative flex-1">
                  <Search className="pointer-events-none absolute left-3.5 top-1/2 h-4 w-4 -translate-y-1/2 text-[#6b6459]" />
                  <input
                    type="text"
                    value={fileSearch}
                    onChange={(e) => {
                      setFileSearch(e.target.value);
                      setFilePageStack([{ cursor: null, offset: 0 }]);
                    }}
                    placeholder="Search project files..."
                    className="w-full rounded-xl border border-[#2a2520] bg-[#151412] py-2.5 pl-10 pr-4 text-[14px] text-[#e8e0d4] outline-none transition-colors focus:border-amber-500/30 placeholder:text-[#6b6459]"
                  />
                </div>
                <div className="text-[12px] text-[#6b6459] tabular-nums whitespace-nowrap">
                  {fileSummary?.total_files ?? files.length} files
                  <span className="ml-1">· {formatBytes(totalBytes)} / {formatBytes(projectMaxBytes)}</span>
                </div>
              </div>

              {/* Drag-and-drop upload area */}
              <div
                ref={dropRef}
                onDragOver={(e) => { e.preventDefault(); setDragOver(true); }}
                onDragLeave={() => setDragOver(false)}
                onDrop={handleDrop}
                className={cn(
                  "rounded-xl border-2 border-dashed p-6 transition-colors",
                  dragOver
                    ? "border-amber-500/40 bg-amber-500/[0.04]"
                    : "border-[#2a2520] bg-[#151412]"
                )}
              >
                <div className="flex flex-col items-center gap-3 text-center">
                  <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-[#1c1a17]">
                    <Upload className="h-5 w-5 text-[#6b6459]" />
                  </div>
                  <div>
                    <p className="text-[14px] font-medium text-[#e8e0d4]">
                      Drop files here or{" "}
                      <button onClick={() => fileInputRef.current?.click()} className="text-amber-400 hover:text-amber-300">
                        browse
                      </button>
                    </p>
                    <p className="mt-1 text-[12px] text-[#6b6459]">Supports any file type. Multiple files allowed.</p>
                  </div>
                  <input
                    ref={fileInputRef}
                    type="file"
                    multiple
                    onChange={(e) => e.target.files && handleUpload(e.target.files)}
                    className="hidden"
                  />
                </div>
              </div>

              {uploadError && <p className="text-[12px] text-red-400">{uploadError}</p>}

              {/* Upload progress */}
              {uploadProgress.length > 0 && (
                <div className="space-y-2 rounded-xl border border-[#2a2520] bg-[#151412] p-4">
                  {uploadProgress.map((entry) => {
                    const pct = entry.totalBytes > 0 ? Math.round((entry.uploadedBytes / entry.totalBytes) * 100) : 0;
                    return (
                      <div key={entry.id} className="text-[12px]">
                        <div className="flex items-center justify-between gap-2 text-[#e8e0d4]">
                          <span className="truncate">{entry.fileName}</span>
                          <span className="shrink-0 text-[#6b6459]">
                            {entry.status} {["uploading", "processing", "done"].includes(entry.status) ? `${pct}%` : ""}
                          </span>
                        </div>
                        <div className="mt-1 h-1.5 w-full overflow-hidden rounded bg-[#1c1a17]">
                          <div
                            className={cn(
                              "h-full transition-all",
                              entry.status === "failed" ? "bg-red-500/70" : "bg-amber-500/70"
                            )}
                            style={{ width: `${Math.max(0, Math.min(100, pct))}%` }}
                          />
                        </div>
                        {entry.error && <div className="mt-0.5 text-[10px] text-red-400">{entry.error}</div>}
                      </div>
                    );
                  })}
                </div>
              )}

              {/* File list */}
              <div className="space-y-3">
                {filesLoading && files.length === 0 && (
                  <div className="flex items-center justify-center py-12">
                    <div className="h-6 w-6 animate-spin rounded-full border-2 border-[#2a2520] border-t-amber-400" />
                  </div>
                )}
                {!filesLoading && files.length === 0 && (
                  <div className="flex flex-col items-center py-12 text-center">
                    <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-[#1c1a17] ring-1 ring-amber-900/20">
                      <FileText className="h-6 w-6 text-[#6b6459]" />
                    </div>
                    <p className="text-[14px] text-[#9c9486]">
                      {filePage && filePage.total > 0 ? "No files match your search" : "No files uploaded yet"}
                    </p>
                    <p className="mt-1 text-[12px] text-[#6b6459]">
                      {filePage && filePage.total > 0 ? "Try a different search term" : "Upload files to make them available for this project"}
                    </p>
                  </div>
                )}
                {files.map((f) => (
                  <div key={f.id} className="group rounded-xl border border-[#2a2520] bg-[#151412] p-4 transition-colors hover:border-amber-900/30">
                    <div className="flex items-start justify-between gap-3">
                      <div className="flex items-start gap-3 min-w-0">
                        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-[#1c1a17] ring-1 ring-amber-900/20">
                          <FileText className="h-4 w-4 text-[#6b6459]" />
                        </div>
                        <div className="min-w-0">
                          <div className="text-[13px] font-medium text-[#e8e0d4] truncate">{f.file_name}</div>
                          <div className="mt-0.5 text-[12px] text-[#6b6459]">
                            {formatBytes(f.size_bytes)}
                            {f.source_path && f.source_path !== f.file_name && (
                              <span className="ml-1.5">· {f.source_path}</span>
                            )}
                          </div>
                        </div>
                      </div>
                      <div className="flex gap-1.5 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
                        {f.has_text && (
                          <button
                            onClick={async () => {
                              if (!activeProjectId) return;
                              const data = await fetchProjectFileText(activeProjectId, f.id);
                              setTextViewFile({ id: f.id, name: data.file_name, text: data.extracted_text });
                            }}
                            className="rounded-lg p-2 text-[#6b6459] transition-colors hover:bg-[#232019] hover:text-emerald-400"
                            title={`View extracted text (${(f.text_chars / 1000).toFixed(1)}k chars)`}
                          >
                            <FileText className="h-3.5 w-3.5" />
                          </button>
                        )}
                        {!f.has_text && (
                          <button
                            onClick={async () => {
                              if (!activeProjectId) return;
                              setExtracting(f.id);
                              try {
                                await reextractProjectFile(activeProjectId, f.id);
                                refetchFiles();
                              } finally { setExtracting(null); }
                            }}
                            disabled={extracting === f.id}
                            className="rounded-lg p-2 text-[#6b6459] transition-colors hover:bg-[#232019] hover:text-[#e8e0d4] disabled:animate-spin"
                            title="Extract text"
                          >
                            <RotateCw className="h-3.5 w-3.5" />
                          </button>
                        )}
                        {isPreviewable(f) && (
                          <button
                            onClick={() => setPreviewFile(f)}
                            className="rounded-lg p-2 text-[#6b6459] transition-colors hover:bg-[#232019] hover:text-amber-400"
                            title="Preview"
                          >
                            <Eye className="h-3.5 w-3.5" />
                          </button>
                        )}
                      </div>
                    </div>
                    {f.has_text && (
                      <div className="mt-3 flex items-center gap-2">
                        <span className="rounded-full bg-emerald-500/15 px-2.5 py-0.5 text-[11px] font-medium text-emerald-300 ring-1 ring-inset ring-emerald-500/20">
                          Extracted
                        </span>
                        <span className="text-[11px] text-[#6b6459]">{(f.text_chars / 1000).toFixed(1)}k chars</span>
                      </div>
                    )}
                  </div>
                ))}

                {/* Pagination */}
                {filePage && filePage.total > filePage.limit && (
                  <div className="flex items-center justify-between pt-2">
                    <span className="text-[12px] text-[#6b6459]">
                      {filePage.total === 0 ? 0 : currentFilePage.offset + 1}–{Math.min(currentFilePage.offset + files.length, filePage.total)} of {filePage.total}
                    </span>
                    <div className="flex gap-2">
                      <button
                        onClick={() => setFilePageStack((prev) => (prev.length > 1 ? prev.slice(0, -1) : prev))}
                        disabled={filePageStack.length <= 1}
                        className="rounded-lg border border-[#2a2520] px-3 py-1.5 text-[12px] text-[#9c9486] transition-colors hover:border-amber-900/30 hover:text-[#e8e0d4] disabled:opacity-40"
                      >
                        Prev
                      </button>
                      <button
                        onClick={() => {
                          if (!filePage.next_cursor) return;
                          setFilePageStack((prev) => [
                            ...prev,
                            { cursor: filePage.next_cursor ?? null, offset: currentFilePage.offset + files.length },
                          ]);
                        }}
                        disabled={!filePage.has_more || !filePage.next_cursor}
                        className="rounded-lg border border-[#2a2520] px-3 py-1.5 text-[12px] text-[#9c9486] transition-colors hover:border-amber-900/30 hover:text-[#e8e0d4] disabled:opacity-40"
                      >
                        Next
                      </button>
                    </div>
                  </div>
                )}
              </div>

              {/* Upload sessions (compact) */}
              {(uploadSessions.length > 0 || uploadSessionsLoading) && (
                <div className="rounded-xl border border-[#2a2520] bg-[#151412] p-4">
                  <div className="mb-2 flex items-center justify-between">
                    <span className="text-[12px] font-semibold text-[#e8e0d4]">Upload Sessions</span>
                    <span className="text-[11px] text-[#6b6459] tabular-nums">
                      {uploadSessionCounts.uploading ?? 0} uploading · {uploadSessionCounts.processing ?? 0} processing · {uploadSessionCounts.done ?? 0} done
                    </span>
                  </div>
                  <div className="space-y-1.5 max-h-32 overflow-y-auto">
                    {uploadSessions.slice(0, 8).map((s) => (
                      <div key={s.id} className="flex items-center justify-between rounded-lg border border-[#2a2520] px-3 py-2 text-[12px]">
                        <span className="truncate pr-2 text-[#e8e0d4]">#{s.id} {s.file_name}</span>
                        <div className="flex items-center gap-2">
                          <span className="text-[#6b6459]">{s.status}</span>
                          {s.status === "failed" && (
                            <button
                              onClick={() => retryUploadSession(s.id)}
                              className="rounded-lg border border-amber-500/30 px-2 py-1 text-[11px] text-amber-300 hover:bg-amber-500/10"
                            >
                              Retry
                            </button>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              )}

              {/* Cloud storage */}
              <div className="rounded-xl border border-[#2a2520] bg-[#151412] p-4">
                <div className="mb-3 text-[12px] font-semibold text-[#e8e0d4]">Cloud Storage</div>
                {!publicUrlValid && (
                  <div className="mb-3 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] text-amber-300">
                    Configure a valid Public URL in Settings before connecting cloud accounts.
                  </div>
                )}
                {cloudMessage && (
                  <div
                    className={cn(
                      "mb-3 flex items-start justify-between gap-2 rounded-lg border px-3 py-2 text-[11px]",
                      cloudMessage.type === "success"
                        ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-400"
                        : "border-red-500/30 bg-red-500/10 text-red-400"
                    )}
                  >
                    <span>{cloudMessage.text}</span>
                    <button onClick={() => setCloudMessage(null)} className="shrink-0 text-[#6b6459] hover:text-[#e8e0d4]">
                      <X className="h-3 w-3" />
                    </button>
                  </div>
                )}
                <div className="mb-3 flex flex-wrap gap-1.5">
                  {CLOUD_PROVIDERS.map((provider) => {
                    const configured = hasCloudCredentials(provider);
                    return (
                      <button
                        key={provider.id}
                        onClick={() => connectCloudProvider(provider.id)}
                        disabled={!configured || !activeProjectId || !publicUrlValid}
                        title={
                          !publicUrlValid
                            ? "Set a valid Public URL in Settings > Cloud Storage"
                            : configured
                            ? `Connect ${provider.label}`
                            : `Configure ${provider.label} credentials in Settings > Cloud Storage`
                        }
                        className="inline-flex items-center gap-1.5 rounded-lg border border-[#2a2520] px-3 py-1.5 text-[12px] text-[#e8e0d4] transition-colors hover:bg-[#232019] disabled:cursor-not-allowed disabled:opacity-40"
                      >
                        <CloudProviderIcon provider={provider.id} />
                        {provider.label}
                      </button>
                    );
                  })}
                </div>
                <div className="space-y-1.5 max-h-36 overflow-y-auto">
                  {cloudConnections.map((conn) => (
                    <div key={conn.id} className="flex items-center justify-between rounded-lg border border-[#2a2520] px-3 py-2 text-[12px]">
                      <div className="min-w-0 flex items-center gap-1.5 text-[#e8e0d4]">
                        <CloudProviderIcon provider={conn.provider} />
                        <span className="truncate">{conn.account_email || cloudProviderLabel(conn.provider)}</span>
                      </div>
                      <div className="flex shrink-0 items-center gap-1.5">
                        <button
                          onClick={() => openCloudBrowser(conn)}
                          className="inline-flex items-center gap-1.5 rounded-lg border border-[#2a2520] px-2.5 py-1 text-[12px] text-[#e8e0d4] transition-colors hover:bg-[#232019]"
                        >
                          <Folder className="h-3 w-3" />
                          Browse
                        </button>
                        <button
                          onClick={() => disconnectCloudConnection(conn)}
                          className="rounded-lg p-1.5 text-[#6b6459] transition-colors hover:bg-red-500/10 hover:text-red-400"
                          title="Disconnect"
                        >
                          <X className="h-3 w-3" />
                        </button>
                      </div>
                    </div>
                  ))}
                  {!cloudConnectionsLoading && cloudConnections.length === 0 && (
                    <div className="text-[12px] text-[#6b6459]">No connected cloud accounts.</div>
                  )}
                </div>
              </div>
            </div>
          </div>
        )}
      </div>
      {previewFile && activeProjectId && (
        <FilePreviewModal
          file={previewFile}
          projectId={activeProjectId}
          onClose={() => setPreviewFile(null)}
        />
      )}
      {cloudModalOpen && cloudModalConn && activeProjectId && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
          onClick={() => setCloudModalOpen(false)}
        >
          <div
            className="mx-4 flex max-h-[82vh] w-full max-w-4xl flex-col rounded-xl border border-white/10 bg-zinc-900 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between border-b border-white/10 px-5 py-4">
              <div className="min-w-0">
                <div className="text-[15px] font-semibold text-zinc-100">
                  {cloudProviderLabel(cloudModalConn.provider)} - {cloudModalConn.account_email || "Account"}
                </div>
                <div className="mt-1.5 flex items-center gap-1 overflow-x-auto text-[12px] text-zinc-400">
                  {cloudBreadcrumbs.map((crumb, idx) => (
                    <button
                      key={`${crumb.id ?? "root"}-${idx}`}
                      onClick={async () => {
                        const next = cloudBreadcrumbs.slice(0, idx + 1);
                        setCloudBreadcrumbs(next);
                        setCloudSelected({});
                        setCloudCursor(null);
                        setCloudHasMore(false);
                        await loadCloudFolder(cloudModalConn, next[next.length - 1]?.id);
                      }}
                      className="shrink-0 hover:text-zinc-300"
                    >
                      {idx > 0 ? "/" : ""}
                      {crumb.name}
                    </button>
                  ))}
                </div>
              </div>
              <button onClick={() => setCloudModalOpen(false)} className="text-zinc-500 hover:text-zinc-300">x</button>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto p-4">
              {cloudLoadError && (
                <div className="mb-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-[12px] text-red-400">
                  {cloudLoadError}
                </div>
              )}
              <div className="overflow-hidden rounded-xl border border-white/[0.08]">
                {cloudItems.map((item) => {
                  const selected = Boolean(cloudSelected[item.id]);
                  return (
                    <div key={item.id} className="flex items-center justify-between border-b border-white/[0.07] px-3 py-2.5 text-[13px] last:border-b-0">
                      <label className="flex min-w-0 flex-1 items-center gap-2 text-zinc-300">
                        {item.type === "file" ? (
                          <input
                            type="checkbox"
                            checked={selected}
                            onChange={(e) => {
                              setCloudSelected((prev) => {
                                const next = { ...prev };
                                if (e.target.checked) next[item.id] = item;
                                else delete next[item.id];
                                return next;
                              });
                            }}
                          />
                        ) : (
                          <span className="inline-block w-4" />
                        )}
                        <button
                          disabled={item.type !== "folder"}
                          onClick={async () => {
                            if (item.type !== "folder") return;
                            setCloudBreadcrumbs((prev) => [...prev, { id: item.id, name: item.name }]);
                            setCloudSelected({});
                            setCloudCursor(null);
                            setCloudHasMore(false);
                            await loadCloudFolder(cloudModalConn, item.id);
                          }}
                          className={cn(
                            "truncate text-left",
                            item.type === "folder" ? "text-blue-400 hover:text-blue-300" : "text-zinc-300"
                          )}
                        >
                          {item.type === "folder" ? "[DIR] " : "[FILE] "}
                          {item.name}
                        </button>
                      </label>
                      <div className="ml-2 shrink-0 text-[12px] text-zinc-500">
                        {item.type === "file" ? formatBytes(item.size || 0) : "folder"}
                      </div>
                    </div>
                  );
                })}
                {!cloudLoading && cloudItems.length === 0 && (
                  <div className="px-4 py-6 text-[13px] text-zinc-500 text-center">This folder is empty.</div>
                )}
              </div>
              {cloudLoading && <div className="mt-3 text-[12px] text-zinc-500">Loading...</div>}
              {!cloudLoading && cloudHasMore && cloudCursor && (
                <button
                  onClick={() => loadCloudFolder(cloudModalConn, currentCloudFolderId, { append: true, cursor: cloudCursor })}
                  className="mt-3 rounded-lg border border-white/[0.08] px-3 py-1.5 text-[12px] text-zinc-300 hover:bg-white/[0.06] transition-colors"
                >
                  Load more
                </button>
              )}
            </div>
            <div className="flex items-center justify-between border-t border-white/10 px-5 py-4">
              <div className="text-[12px] text-zinc-400">
                Selected: {Object.values(cloudSelected).filter((i) => i.type === "file").length} file(s)
              </div>
              <button
                onClick={importSelectedCloudFiles}
                disabled={cloudImporting || Object.values(cloudSelected).every((i) => i.type !== "file")}
                className="rounded-lg bg-blue-500/20 px-4 py-2 text-[13px] font-medium text-blue-300 hover:bg-blue-500/30 transition-colors disabled:cursor-not-allowed disabled:text-zinc-600"
              >
                {cloudImporting ? "Importing..." : "Import Selected"}
              </button>
            </div>
          </div>
        </div>
      )}
      {textViewFile && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick={() => setTextViewFile(null)}>
          <div className="mx-4 flex max-h-[80vh] w-full max-w-3xl flex-col rounded-xl border border-white/10 bg-zinc-900 shadow-xl" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between border-b border-white/10 px-5 py-4">
              <span className="text-[15px] font-semibold text-zinc-100">{textViewFile.name} — Extracted Text</span>
              <button onClick={() => setTextViewFile(null)} className="text-zinc-500 hover:text-zinc-300">✕</button>
            </div>
            <pre className="flex-1 overflow-auto whitespace-pre-wrap p-5 font-mono text-[13px] leading-relaxed text-zinc-300">{textViewFile.text}</pre>
          </div>
        </div>
      )}
    </div>
  );
}
