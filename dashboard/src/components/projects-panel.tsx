import { useEffect, useMemo, useRef, useState } from "react";
import {
  createProject,
  fetchProjectFileText,
  getProjectChatMessages,
  reextractProjectFile,
  sendProjectChat,
  uploadProjectFiles,
  useModes,
  useProjectFiles,
  useProjects,
  searchDocuments,
  sseUrl,
  tokenReady,
} from "@/lib/api";
import type { FtsSearchResult } from "@/lib/api";
import { Eye, FileText, Mic, MicOff, ArrowLeft, Search, RotateCw } from "lucide-react";
import { FilePreviewModal, isPreviewable } from "./file-preview-modal";
import type { ProjectFile, ProjectDocument } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useDictation } from "@/lib/dictation";
import { BorgingIndicator } from "./borging";
import { ChatMarkdown } from "./chat-markdown";
import { MatterDetail } from "./matter-detail";
import { MarkdownLegalViewer } from "./viewers/markdown-legal-viewer";
import { RedlineViewer } from "./viewers/redline-viewer";
import { LegalTaskCreator } from "./legal-task-creator";
import { useProjectDocumentVersions } from "@/lib/api";

type ChatMessage = {
  role: "user" | "assistant";
  sender?: string;
  text: string;
  ts: string | number;
  thread?: string;
};

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function DocumentViewWrapper({
  projectId,
  doc,
  viewMode,
  onBack,
  onToggleMode,
}: {
  projectId: number;
  doc: ProjectDocument;
  viewMode: "view" | "redline";
  onBack: () => void;
  onToggleMode: () => void;
}) {
  const { data: versions = [] } = useProjectDocumentVersions(projectId, doc.task_id, doc.file_name);

  return (
    <div className="flex h-full flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-white/[0.06] px-3 py-2">
        <button
          onClick={onBack}
          className="flex items-center gap-1 text-[11px] text-zinc-500 hover:text-zinc-300 transition-colors"
        >
          <ArrowLeft className="h-3 w-3" />
          Back to matter
        </button>
        <span className="text-[11px] text-zinc-600">·</span>
        <span className="truncate text-[11px] text-zinc-400">{doc.file_name}</span>
        {versions.length >= 2 && (
          <button
            onClick={onToggleMode}
            className={cn(
              "ml-auto rounded border px-2 py-0.5 text-[10px] font-medium transition-colors",
              viewMode === "redline"
                ? "border-blue-500/30 bg-blue-500/10 text-blue-400"
                : "border-white/[0.08] text-zinc-500 hover:border-white/[0.14] hover:text-zinc-300"
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
          />
        )}
      </div>
    </div>
  );
}

export function ProjectsPanel() {
  const { data: projects = [], refetch: refetchProjects } = useProjects();
  const { data: modes = [] } = useModes();
  const [selectedProjectId, setSelectedProjectId] = useState<number | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [ftsQuery, setFtsQuery] = useState("");
  const [ftsResults, setFtsResults] = useState<FtsSearchResult[]>([]);
  const [ftsSearching, setFtsSearching] = useState(false);
  const ftsDebounce = useRef<ReturnType<typeof setTimeout>>(null);
  const [newProjectName, setNewProjectName] = useState("");
  const [newProjectMode, setNewProjectMode] = useState("general");
  const [creating, setCreating] = useState(false);

  const filteredProjects = useMemo(() => {
    if (!searchQuery.trim()) return projects;
    const q = searchQuery.toLowerCase();
    return projects.filter(
      (p) =>
        p.name.toLowerCase().includes(q) ||
        (p.client_name && p.client_name.toLowerCase().includes(q)) ||
        (p.case_number && p.case_number.toLowerCase().includes(q)) ||
        (p.jurisdiction && p.jurisdiction.toLowerCase().includes(q))
    );
  }, [projects, searchQuery]);

  const selectedProject =
    projects.find((p) => p.id === selectedProjectId) ?? projects[0] ?? null;
  const activeProjectId = selectedProject?.id ?? null;

  const {
    data: files = [],
    refetch: refetchFiles,
    isFetching: filesLoading,
  } = useProjectFiles(activeProjectId);

  const [previewFile, setPreviewFile] = useState<ProjectFile | null>(null);
  const [selectedDoc, setSelectedDoc] = useState<ProjectDocument | null>(null);
  const [docViewMode, setDocViewMode] = useState<"view" | "redline">("view");
  const [uploading, setUploading] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const [textViewFile, setTextViewFile] = useState<{ id: number; name: string; text: string } | null>(null);
  const [extracting, setExtracting] = useState<number | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [messageInput, setMessageInput] = useState("");
  const [sending, setSending] = useState(false);
  const dictation = useDictation(messageInput, setMessageInput);
  const esRef = useRef<EventSource | null>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const sseRetriesRef = useRef(0);
  const sseRetryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const totalBytes = useMemo(
    () => files.reduce((sum, f) => sum + f.size_bytes, 0),
    [files]
  );

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
    if (!activeProjectId) {
      setMessages([]);
      return;
    }
    getProjectChatMessages(activeProjectId)
      .then(setMessages)
      .catch(() => setMessages([]));
  }, [activeProjectId]);

  useEffect(() => {
    if (!activeProjectId) return;
    const threadKey = `project:${activeProjectId}`;
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
            // ignore malformed event
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
  }, [activeProjectId]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "instant" });
  }, [messages.length]);

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
      const created = await createProject(name, newProjectMode);
      setNewProjectName("");
      await refetchProjects();
      setSelectedProjectId(created.id);
    } finally {
      setCreating(false);
    }
  }

  async function handleUpload(filesToUpload: FileList | File[]) {
    if (!activeProjectId || uploading) return;
    setUploading(true);
    setUploadError(null);
    try {
      await uploadProjectFiles(activeProjectId, filesToUpload);
      await refetchFiles();
    } catch (err) {
      const msg = err instanceof Error ? err.message : "upload failed";
      setUploadError(msg === "413" ? "Upload exceeds 100MB project limit." : `Upload failed (${msg})`);
    } finally {
      setUploading(false);
      if (fileInputRef.current) fileInputRef.current.value = "";
    }
  }

  async function handleSendMessage() {
    if (!activeProjectId || sending) return;
    const text = messageInput.trim();
    if (!text) return;
    setMessageInput("");
    setSending(true);
    const timeout = setTimeout(() => setSending(false), 60_000);
    try {
      await sendProjectChat(activeProjectId, text);
    } catch {
      setSending(false);
    } finally {
      clearTimeout(timeout);
    }
  }

  return (
    <div className="flex h-full min-h-0">
      <div className="w-[260px] shrink-0 border-r border-white/[0.06] p-3">
        <div className="mb-2 text-[11px] font-medium uppercase tracking-wide text-zinc-500">
          Projects
        </div>
        <div className="relative mb-2">
          <Search className="absolute left-2 top-1/2 h-3 w-3 -translate-y-1/2 text-zinc-600" />
          <input
            value={ftsQuery}
            onChange={(e) => handleFtsSearch(e.target.value)}
            placeholder="Search all documents..."
            className="w-full rounded border border-white/[0.08] bg-black/30 pl-7 pr-2 py-1 text-[11px] text-zinc-300 outline-none placeholder:text-zinc-600 focus:border-blue-500/40"
          />
        </div>
        {ftsQuery.trim() && (ftsSearching || ftsResults.length > 0) ? (
          <div className="space-y-1 overflow-y-auto mb-3" style={{ maxHeight: "calc(100vh - 280px)" }}>
            {ftsSearching && <div className="text-[10px] text-zinc-600 px-1">Searching…</div>}
            {ftsResults.map((r, i) => (
              <button
                key={`${r.task_id}-${r.file_path}-${i}`}
                onClick={() => { setSelectedProjectId(r.project_id); setFtsQuery(""); setFtsResults([]); }}
                className="w-full rounded-md border border-white/[0.04] bg-white/[0.02] px-2 py-1.5 text-left hover:bg-white/[0.06] transition-colors"
              >
                <div className="text-[10px] text-zinc-500 truncate flex items-center gap-1">
                  {r.project_name}
                  {r.source === "semantic" && <span className="px-1 py-0 rounded bg-violet-900/50 text-violet-300 text-[9px]">semantic</span>}
                </div>
                {r.title_snippet && <div className="text-[11px] text-zinc-300 truncate" dangerouslySetInnerHTML={{ __html: r.title_snippet }} />}
                <div className="text-[10px] text-zinc-500 line-clamp-2" dangerouslySetInnerHTML={{ __html: r.content_snippet }} />
              </button>
            ))}
            {!ftsSearching && ftsResults.length === 0 && (
              <div className="text-[10px] text-zinc-600 px-1">No results.</div>
            )}
          </div>
        ) : (
          <>
        {projects.length > 5 && (
          <input
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Filter matters..."
            className="mb-2 w-full rounded border border-white/[0.08] bg-black/30 px-2 py-1 text-[11px] text-zinc-300 outline-none placeholder:text-zinc-600 focus:border-blue-500/40"
          />
        )}
        <div className="space-y-1 overflow-y-auto" style={{ maxHeight: "calc(100vh - 280px)" }}>
          {filteredProjects.map((p) => (
            <button
              key={p.id}
              onClick={() => setSelectedProjectId(p.id)}
              className={cn(
                "w-full rounded-md px-2 py-1.5 text-left text-[12px] transition-colors",
                p.id === activeProjectId
                  ? "bg-white/[0.08] text-zinc-100"
                  : "text-zinc-400 hover:bg-white/[0.04]"
              )}
            >
              {p.name}
            </button>
          ))}
          {projects.length === 0 && (
            <div className="rounded-md border border-dashed border-white/[0.08] px-3 py-2 text-[11px] text-zinc-600">
              No projects yet.
            </div>
          )}
        </div>
          </>
        )}
        <div className="mt-3 border-t border-white/[0.06] pt-3">
          <input
            value={newProjectName}
            onChange={(e) => setNewProjectName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleCreateProject()}
            placeholder="New project name"
            className="w-full rounded border border-white/[0.08] bg-white/[0.03] px-2 py-1.5 text-[12px] text-zinc-200 outline-none"
          />
          <select
            value={newProjectMode}
            onChange={(e) => setNewProjectMode(e.target.value)}
            className="mt-2 w-full rounded border border-white/[0.08] bg-white/[0.03] px-2 py-1.5 text-[12px] text-zinc-300 outline-none"
          >
            <option value="general">general</option>
            {modes.map((mode) => (
              <option key={mode.name} value={mode.name}>
                {mode.name}
              </option>
            ))}
          </select>
          <button
            onClick={handleCreateProject}
            disabled={creating || !newProjectName.trim()}
            className="mt-2 w-full rounded bg-blue-500/20 px-2 py-1.5 text-[12px] font-medium text-blue-300 disabled:cursor-not-allowed disabled:text-zinc-600"
          >
            {creating ? "Creating..." : "Create Project"}
          </button>
        </div>
      </div>

      <div className="flex min-w-0 flex-1 flex-col">
        {!selectedProject ? (
          <div className="flex h-full items-center justify-center">
            <div className="max-w-[320px] text-center">
              <div className="text-[14px] font-medium text-zinc-300">Get Started</div>
              <div className="mt-2 text-[12px] leading-relaxed text-zinc-500">
                Create a matter in the sidebar to start. Each matter gets its own
                dedicated repository for documents and task outputs.
              </div>
              <div className="mt-4 space-y-2 text-left text-[11px] text-zinc-600">
                <div className="rounded border border-white/[0.06] bg-white/[0.02] px-3 py-2">
                  <span className="text-zinc-400">1.</span> Name your matter and select <span className="text-blue-400">lawborg</span> mode
                </div>
                <div className="rounded border border-white/[0.06] bg-white/[0.02] px-3 py-2">
                  <span className="text-zinc-400">2.</span> Upload reference documents (contracts, briefs, filings)
                </div>
                <div className="rounded border border-white/[0.06] bg-white/[0.02] px-3 py-2">
                  <span className="text-zinc-400">3.</span> Create tasks — research memos, contract reviews, case analysis
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
            />
          ) : (
            <div className="flex h-full flex-col">
              <div className="flex shrink-0 items-center justify-end border-b border-white/[0.06] px-4 py-2">
                <LegalTaskCreator />
              </div>
              <div className="min-h-0 flex-1">
                <MatterDetail projectId={selectedProject.id} onDocumentSelect={setSelectedDoc} onDelete={() => setSelectedProjectId(null)} />
              </div>
            </div>
          )
        ) : (
          <>
            <div className="border-b border-white/[0.06] p-3">
              <div className="text-[13px] font-semibold text-zinc-200">{selectedProject.name}</div>
              <div className="mt-1 text-[11px] text-zinc-500">
                Files {files.length} · {formatBytes(totalBytes)} / 100 MB
              </div>
              <div className="mt-3 flex items-center gap-2">
                <input
                  ref={fileInputRef}
                  type="file"
                  multiple
                  onChange={(e) => e.target.files && handleUpload(e.target.files)}
                  className="hidden"
                />
                <button
                  onClick={() => fileInputRef.current?.click()}
                  disabled={uploading}
                  className="rounded bg-white/[0.06] px-3 py-1.5 text-[12px] text-zinc-300 hover:bg-white/[0.1] disabled:cursor-not-allowed disabled:text-zinc-600"
                >
                  {uploading ? "Uploading..." : "Upload Files"}
                </button>
                {filesLoading && <span className="text-[11px] text-zinc-600">refreshing…</span>}
              </div>
              {uploadError && (
                <div className="mt-2 text-[11px] text-red-400">{uploadError}</div>
              )}
              <div className="mt-3 max-h-28 overflow-y-auto rounded border border-white/[0.06] bg-black/20">
                {files.map((f) => (
                  <div key={f.id} className="flex items-center justify-between border-b border-white/[0.04] px-2 py-1 text-[11px] text-zinc-400 last:border-0">
                    <span className="truncate pr-2">{f.file_name}</span>
                    <div className="flex shrink-0 items-center gap-2">
                      {f.has_text && (
                        <button
                          onClick={async () => {
                            if (!activeProjectId) return;
                            const data = await fetchProjectFileText(activeProjectId, f.id);
                            setTextViewFile({ id: f.id, name: data.file_name, text: data.extracted_text });
                          }}
                          className="text-emerald-600 transition-colors hover:text-emerald-400"
                          title={`View extracted text (${(f.text_chars / 1000).toFixed(1)}k chars)`}
                        >
                          <FileText className="h-3 w-3" />
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
                          className="text-zinc-600 transition-colors hover:text-zinc-300 disabled:animate-spin"
                          title="Extract text"
                        >
                          <RotateCw className="h-3 w-3" />
                        </button>
                      )}
                      {isPreviewable(f) && (
                        <button
                          onClick={() => setPreviewFile(f)}
                          className="text-zinc-600 transition-colors hover:text-zinc-300"
                          title="Preview"
                        >
                          <Eye className="h-3 w-3" />
                        </button>
                      )}
                      <span className="text-zinc-600">{formatBytes(f.size_bytes)}</span>
                    </div>
                  </div>
                ))}
                {files.length === 0 && (
                  <div className="px-2 py-2 text-[11px] text-zinc-600">No files uploaded yet.</div>
                )}
              </div>
            </div>

            <div className="flex min-h-0 flex-1 flex-col">
              <div className="flex-1 overflow-y-auto p-3">
                {messages.map((msg, idx) => (
                  <div key={`${msg.ts}-${msg.role}-${idx}`} className={cn("mb-2 flex", msg.role === "user" ? "justify-end" : "justify-start")}>
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

              <div className="border-t border-white/[0.06] p-3">
                <div className="flex gap-2">
                  <textarea
                    value={messageInput}
                    onChange={(e) => setMessageInput(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && !e.shiftKey) {
                        e.preventDefault();
                        handleSendMessage();
                      }
                    }}
                    placeholder="Message the director about this project..."
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
                    onClick={handleSendMessage}
                    disabled={sending || !messageInput.trim()}
                    className="rounded bg-blue-500/20 px-3 py-2 text-[12px] font-medium text-blue-300 disabled:cursor-not-allowed disabled:text-zinc-600"
                  >
                    Send
                  </button>
                </div>
              </div>
            </div>
          </>
        )}
      </div>
      {previewFile && activeProjectId && (
        <FilePreviewModal
          file={previewFile}
          projectId={activeProjectId}
          onClose={() => setPreviewFile(null)}
        />
      )}
      {textViewFile && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm" onClick={() => setTextViewFile(null)}>
          <div className="mx-4 flex max-h-[80vh] w-full max-w-3xl flex-col rounded-lg border border-white/10 bg-zinc-900 shadow-xl" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between border-b border-white/10 px-4 py-3">
              <span className="text-sm font-medium text-zinc-200">{textViewFile.name} — Extracted Text</span>
              <button onClick={() => setTextViewFile(null)} className="text-zinc-500 hover:text-zinc-300">✕</button>
            </div>
            <pre className="flex-1 overflow-auto whitespace-pre-wrap p-4 font-mono text-[12px] leading-relaxed text-zinc-300">{textViewFile.text}</pre>
          </div>
        </div>
      )}
    </div>
  );
}
