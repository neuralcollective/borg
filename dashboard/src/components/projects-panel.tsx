import { useEffect, useMemo, useRef, useState } from "react";
import {
  createProject,
  getProjectChatMessages,
  sendProjectChat,
  uploadProjectFiles,
  useModes,
  useProjectFiles,
  useProjects,
} from "@/lib/api";
import { cn } from "@/lib/utils";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";

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

export function ProjectsPanel() {
  const { data: projects = [], refetch: refetchProjects } = useProjects();
  const { data: modes = [] } = useModes();
  const [selectedProjectId, setSelectedProjectId] = useState<number | null>(null);
  const [newProjectName, setNewProjectName] = useState("");
  const [newProjectMode, setNewProjectMode] = useState("general");
  const [creating, setCreating] = useState(false);

  const selectedProject =
    projects.find((p) => p.id === selectedProjectId) ?? projects[0] ?? null;
  const activeProjectId = selectedProject?.id ?? null;

  const {
    data: files = [],
    refetch: refetchFiles,
    isFetching: filesLoading,
  } = useProjectFiles(activeProjectId);

  const [uploading, setUploading] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [messageInput, setMessageInput] = useState("");
  const [sending, setSending] = useState(false);
  const esRef = useRef<EventSource | null>(null);
  const bottomRef = useRef<HTMLDivElement>(null);

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
    if (esRef.current) esRef.current.close();
    const es = new EventSource("/api/chat/events");
    esRef.current = es;
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
    es.onerror = () => {};
    return () => es.close();
  }, [activeProjectId]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "instant" });
  }, [messages.length]);

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
    try {
      await sendProjectChat(activeProjectId, text);
    } catch {
      setSending(false);
    }
  }

  return (
    <div className="flex h-full min-h-0">
      <div className="w-[260px] shrink-0 border-r border-white/[0.06] p-3">
        <div className="mb-3 text-[11px] font-medium uppercase tracking-wide text-zinc-500">
          Projects
        </div>
        <div className="space-y-1 overflow-y-auto" style={{ maxHeight: "calc(100vh - 220px)" }}>
          {projects.map((p) => (
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
          <div className="flex h-full items-center justify-center text-[12px] text-zinc-500">
            Create a project to start.
          </div>
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
                    <span className="shrink-0 text-zinc-600">{formatBytes(f.size_bytes)}</span>
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
                  <div key={idx} className={cn("mb-2 flex", msg.role === "user" ? "justify-end" : "justify-start")}>
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
                        <div className="prose prose-invert prose-sm max-w-none break-words [&_p]:my-1 [&_ul]:my-1 [&_ol]:my-1 [&_li]:my-0.5 [&_h1]:text-[14px] [&_h2]:text-[13px] [&_h3]:text-[12px] [&_h1]:mt-2 [&_h2]:mt-2 [&_h3]:mt-1 [&_code]:text-[11px] [&_code]:bg-white/[0.08] [&_code]:px-1 [&_code]:rounded [&_pre]:bg-white/[0.06] [&_pre]:p-2 [&_pre]:rounded [&_pre]:text-[11px] [&_hr]:border-white/[0.08] [&_strong]:text-zinc-200 [&_a]:text-blue-400">
                          <Markdown remarkPlugins={[remarkGfm]}>{msg.text}</Markdown>
                        </div>
                      )}
                    </div>
                  </div>
                ))}
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
                  <button
                    onClick={handleSendMessage}
                    disabled={sending || !messageInput.trim()}
                    className="rounded bg-blue-500/20 px-3 py-2 text-[12px] font-medium text-blue-300 disabled:cursor-not-allowed disabled:text-zinc-600"
                  >
                    {sending ? "Thinking..." : "Send"}
                  </button>
                </div>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
