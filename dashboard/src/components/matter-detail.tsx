import { useEffect, useMemo, useRef, useState } from "react";
import {
  useProjectDetail,
  useProjectTasks,
  useProjectDocuments,
  useUpdateProject,
  getProjectChatMessages,
  sendProjectChat,
  sseUrl,
  tokenReady,
} from "@/lib/api";
import type { Project, ProjectTask, ProjectDocument } from "@/lib/types";
import { StatusBadge } from "./status-badge";
import { PhaseTracker } from "./phase-tracker";
import { BorgingIndicator } from "./borging";
import { ChatMarkdown } from "./chat-markdown";
import { useDictation } from "@/lib/dictation";
import { cn } from "@/lib/utils";
import { retryTask } from "@/lib/api";
import { useQueryClient } from "@tanstack/react-query";
import { ChevronDown, ChevronUp, Edit2, Check, X, FileText, RotateCcw, Mic, MicOff } from "lucide-react";

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

// ── Matter header ─────────────────────────────────────────────────────────────

function MatterHeader({ project }: { project: Project }) {
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
      </div>
    </div>
  );
}

// ── Metadata panel ────────────────────────────────────────────────────────────

function MetadataPanel({ project, projectId }: { project: Project; projectId: number }) {
  const [open, setOpen] = useState(false);
  const { mutate: update } = useUpdateProject(projectId);

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
        <span className="ml-auto text-zinc-600">
          {open ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
        </span>
      </button>
      {open && (
        <div className="grid grid-cols-2 gap-x-6 gap-y-3 px-5 pb-4 sm:grid-cols-3">
          <InlineField label="Client" value={project.client_name} onSave={save("client_name")} placeholder="unset" />
          <InlineField label="Case Number" value={project.case_number} onSave={save("case_number")} placeholder="unset" />
          <InlineField label="Jurisdiction" value={project.jurisdiction} onSave={save("jurisdiction")} placeholder="unset" />
          <InlineField label="Matter Type" value={project.matter_type} onSave={save("matter_type")} placeholder="unset" />
          <InlineField label="Opposing Counsel" value={project.opposing_counsel} onSave={save("opposing_counsel")} placeholder="unset" />
          <InlineField label="Deadline" value={project.deadline} onSave={save("deadline")} placeholder="unset" />
          <InlineField label="Privilege Level" value={project.privilege_level} onSave={save("privilege_level")} placeholder="unset" />
          <InlineField label="Status" value={project.status} onSave={save("status")} placeholder="unset" />
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

// ── Tasks tab ─────────────────────────────────────────────────────────────────

function TasksTab({ projectId }: { projectId: number }) {
  const { data: tasks = [], isLoading } = useProjectTasks(projectId);
  const queryClient = useQueryClient();
  const [retryingId, setRetryingId] = useState<number | null>(null);

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

  return (
    <div className="space-y-2 p-4">
      {tasks.map((task) => (
        <div
          key={task.id}
          className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-3"
        >
          <div className="flex items-start gap-2">
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-2">
                <span className="font-mono text-[10px] text-zinc-600">#{task.id}</span>
                <StatusBadge status={task.status} />
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
            {task.status === "failed" && (
              <button
                onClick={async () => {
                  setRetryingId(task.id);
                  try {
                    await retryTask(task.id);
                    await queryClient.invalidateQueries({ queryKey: ["project_tasks", projectId] });
                  } finally {
                    setRetryingId(null);
                  }
                }}
                disabled={retryingId === task.id}
                className="shrink-0 flex items-center gap-1 rounded border border-white/[0.08] px-2 py-1 text-[11px] text-zinc-400 hover:border-blue-500/30 hover:text-blue-400 disabled:opacity-50 transition-colors"
              >
                <RotateCcw className="h-3 w-3" />
                {retryingId === task.id ? "…" : "Retry"}
              </button>
            )}
          </div>
          <div className="mt-2">
            <PhaseTracker status={task.status} mode={task.mode} />
          </div>
          <div className="mt-1.5 text-[10px] text-zinc-600">
            created {fmtDateTime(task.created_at)}
            {task.attempt > 0 && ` · attempt ${task.attempt}/${task.max_attempts}`}
          </div>
        </div>
      ))}
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

type TabKey = "timeline" | "documents" | "tasks" | "chat";

const TABS: { key: TabKey; label: string }[] = [
  { key: "timeline", label: "Timeline" },
  { key: "documents", label: "Documents" },
  { key: "tasks", label: "Tasks" },
  { key: "chat", label: "Chat" },
];

// ── Main component ────────────────────────────────────────────────────────────

export function MatterDetail({ projectId, onDocumentSelect }: MatterDetailProps) {
  const { data: project, isLoading } = useProjectDetail(projectId);
  const [activeTab, setActiveTab] = useState<TabKey>("timeline");

  if (isLoading || !project) {
    return (
      <div className="flex h-full items-center justify-center text-[12px] text-zinc-600">
        Loading matter...
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <MatterHeader project={project} />
      <MetadataPanel project={project} projectId={projectId} />

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
        {activeTab === "chat" && (
          <div className="flex h-full flex-col">
            <ChatTab projectId={projectId} />
          </div>
        )}
      </div>
    </div>
  );
}
