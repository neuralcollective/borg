import { useQueryClient } from "@tanstack/react-query";
import { ChevronDown, FolderOpen, Globe, Mic, MicOff, Send, Sparkles } from "lucide-react";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { StreamEvent } from "@/lib/api";
import {
  approveTask,
  authHeaders,
  rejectTask,
  requestRevision,
  retryTask,
  tokenReady,
  useFullModes,
  useProjects,
  useProjectTasks,
} from "@/lib/api";
import { useDictation } from "@/lib/dictation";
import { parseStreamEvents, rawStreamToEvents, type TermLine } from "@/lib/stream-utils";
import { useChatEvents } from "@/lib/use-chat-events";
import { cn } from "@/lib/utils";
import { ActionActivity } from "./action-card";
import { pickWorkingLabel } from "./borging";
import { ChatMarkdown } from "./chat-markdown";

interface ChatMessage {
  role: "user" | "assistant";
  sender?: string;
  text: string;
  ts: string | number;
  thread?: string;
  raw_stream?: string;
}

interface ChatBodyProps {
  thread: string;
  className?: string;
  hideEmptyState?: boolean;
}

export function ChatBody({ thread, className, hideEmptyState }: ChatBodyProps) {
  const { data: projects = [] } = useProjects();
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [streamEvents, setStreamEvents] = useState<StreamEvent[]>([]);
  const [completedStreams, setCompletedStreams] = useState<Map<number, StreamEvent[]>>(new Map());
  const [workingLabel, setWorkingLabel] = useState("Working...");
  const bottomRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const lastTsRef = useRef<number>(0);
  const sendingTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const fetchMessages = useCallback(() => {
    tokenReady.then(() => {
      fetch(`/api/chat/messages?thread=${encodeURIComponent(thread)}`, { headers: authHeaders() })
        .then((r) => {
          if (!r.ok) throw new Error(`${r.status}`);
          return r.json();
        })
        .then((msgs: ChatMessage[]) => {
          setMessages(msgs);
          // Restore completed streams from persisted raw_stream
          const restored = new Map<number, StreamEvent[]>();
          msgs.forEach((m, i) => {
            if (m.role === "assistant" && m.raw_stream) {
              const events = rawStreamToEvents(m.raw_stream);
              if (events.length > 0) restored.set(i, events);
            }
          });
          if (restored.size > 0) setCompletedStreams(restored);
          if (msgs.length > 0) {
            lastTsRef.current = Math.max(...msgs.map((m) => Number(m.ts) || 0));
          }
        })
        .catch(() => {});
    });
  }, [thread]);

  const forceScrollRef = useRef(true);

  useEffect(() => {
    setMessages([]);
    setStreamEvents([]);
    setCompletedStreams(new Map());
    lastTsRef.current = 0;
    forceScrollRef.current = true;
    fetchMessages();
  }, [fetchMessages]);

  const handleSseMessage = useCallback(
    (msg: any) => {
      if ((msg.type === "chat_stream" || msg.type === "task_stream") && msg.thread === thread) {
        try {
          const parsed = JSON.parse(msg.data);
          if (parsed.type) {
            setSending(true);
            setStreamEvents((prev) => [...prev, parsed]);
            // Reset timeout — agent is still active
            if (sendingTimeoutRef.current) clearTimeout(sendingTimeoutRef.current);
            sendingTimeoutRef.current = setTimeout(() => {
              setSending(false);
              sendingTimeoutRef.current = null;
            }, 120000);
          }
        } catch {
          /* skip */
        }
        return;
      }
      if (msg.role === "user") return;
      setMessages((prev) => {
        const next = [...prev, msg];
        if (msg.role === "assistant") {
          // Save stream events for this message so they can be reviewed later
          setStreamEvents((evts) => {
            if (evts.length > 0) {
              setCompletedStreams((m) => {
                const updated = new Map(m);
                updated.set(next.length - 1, evts);
                return updated;
              });
            }
            return [];
          });
          setSending(false);
          if (sendingTimeoutRef.current) {
            clearTimeout(sendingTimeoutRef.current);
            sendingTimeoutRef.current = null;
          }
        }
        return next;
      });
      lastTsRef.current = Math.max(lastTsRef.current, Number(msg.ts) || 0);
    },
    [thread],
  );

  useChatEvents(thread, handleSseMessage);

  useEffect(() => {
    return () => {
      if (sendingTimeoutRef.current) clearTimeout(sendingTimeoutRef.current);
    };
  }, []);

  // Poll fallback — only sync when no active stream to avoid wiping in-progress events
  useEffect(() => {
    const interval = setInterval(() => {
      fetch(`/api/chat/messages?thread=${encodeURIComponent(thread)}`, { headers: authHeaders() })
        .then((r) => r.json())
        .then((msgs: ChatMessage[]) => {
          if (msgs.length === 0) return;
          const newTs = Math.max(...msgs.map((m) => Number(m.ts) || 0));
          if (newTs <= lastTsRef.current) return;

          // Check if poll found a new assistant message we don't have yet
          const lastMsg = msgs[msgs.length - 1];
          const hasNewAssistant = lastMsg?.role === "assistant" && Number(lastMsg.ts || 0) > lastTsRef.current;

          setMessages(msgs);
          lastTsRef.current = newTs;

          // Restore completed streams from persisted raw_stream
          const restored = new Map<number, StreamEvent[]>();
          msgs.forEach((m, i) => {
            if (m.role === "assistant" && m.raw_stream) {
              const events = rawStreamToEvents(m.raw_stream);
              if (events.length > 0) restored.set(i, events);
            }
          });
          if (restored.size > 0) setCompletedStreams(restored);

          // Only clear active stream state when a new assistant message has been persisted
          if (hasNewAssistant) {
            setSending(false);
            setStreamEvents([]);
            if (sendingTimeoutRef.current) {
              clearTimeout(sendingTimeoutRef.current);
              sendingTimeoutRef.current = null;
            }
          }
        })
        .catch(() => {});
    }, 3000);
    return () => clearInterval(interval);
  }, [thread]);

  const scrollContainerRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = scrollContainerRef.current;
    if (!el) return;
    if (forceScrollRef.current && messages.length > 0) {
      forceScrollRef.current = false;
      bottomRef.current?.scrollIntoView({ behavior: "instant" });
      return;
    }
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 150;
    if (nearBottom) {
      bottomRef.current?.scrollIntoView({ behavior: "instant" });
    }
  }, [messages.length]);

  async function handleSend() {
    const text = input.trim();
    if (!text || sending) return;

    setInput("");
    setSending(true);
    setWorkingLabel(pickWorkingLabel());
    setStreamEvents([]);

    if (sendingTimeoutRef.current) clearTimeout(sendingTimeoutRef.current);
    sendingTimeoutRef.current = setTimeout(() => {
      setSending(false);
      sendingTimeoutRef.current = null;
    }, 120000);

    const userMsg: ChatMessage = {
      role: "user",
      sender: "web-user",
      text,
      ts: Math.floor(Date.now() / 1000),
      thread,
    };
    setMessages((prev) => [...prev, userMsg]);
    lastTsRef.current = Number(userMsg.ts);

    try {
      await tokenReady;
      await fetch("/api/chat", {
        method: "POST",
        headers: { "Content-Type": "application/json", ...authHeaders() },
        body: JSON.stringify({ text, sender: "web-user", thread }),
      });
    } catch {
      setSending(false);
      setStreamEvents([]);
      if (sendingTimeoutRef.current) {
        clearTimeout(sendingTimeoutRef.current);
        sendingTimeoutRef.current = null;
      }
    }
  }

  const dictation = useDictation(input, setInput);

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }

  function handleInputChange(e: React.ChangeEvent<HTMLTextAreaElement>) {
    setInput(e.target.value);
    const el = e.target;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }

  const streamLines = useMemo(() => parseStreamEvents(streamEvents), [streamEvents]);

  const scopeLabel = threadLabel(thread, projects);
  const isEmpty = messages.length === 0 && !sending;

  return (
    <div className={cn("flex h-full flex-col overflow-hidden", className)}>
      {/* Messages */}
      <div ref={scrollContainerRef} className="relative min-h-0 flex-1 overflow-y-auto overscroll-contain">
        {/* Centered scope indicator — sits behind messages */}
        {!hideEmptyState && isEmpty && (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
            <div className="flex flex-col items-center text-center">
              <div className="mb-4 flex h-12 w-12 items-center justify-center rounded-2xl bg-[#1c1a17] ring-1 ring-[#2a2520]">
                {thread === "web:dashboard" ? (
                  <Globe className="h-5 w-5 text-amber-400/40" strokeWidth={1.5} />
                ) : (
                  <FolderOpen className="h-5 w-5 text-[#6b6459]" strokeWidth={1.5} />
                )}
              </div>
              <p className="text-[13px] font-medium text-[#9c9486]">{scopeLabel}</p>
              <p className="mt-1 text-[11px] text-[#6b6459]">
                {thread === "web:dashboard" ? "Chat with global knowledge" : "Scoped to this project"}
              </p>
            </div>
          </div>
        )}

        <div className="relative px-3 py-3 space-y-3">
          {/* Inline work item controls */}
          <WorkItemControls thread={thread} />

          {messages.map((msg, i) => (
            <React.Fragment key={`${msg.ts}-${msg.role}-${i}`}>
              {msg.role === "assistant" && completedStreams.has(i) && (
                <CollapsedTimeline events={completedStreams.get(i)!} />
              )}
              <MessageBubble msg={msg} />
            </React.Fragment>
          ))}

          {sending && streamLines.length > 0 && (
            <div className="flex gap-2">
              <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-gradient-to-br from-amber-500/20 to-orange-500/20 ring-1 ring-white/[0.06]">
                <span className="text-[10px] font-bold text-amber-300">B</span>
              </div>
              <div className="min-w-0 flex-1 pt-0.5">
                <AgentTimeline lines={streamLines} streaming />
              </div>
            </div>
          )}

          {sending && streamLines.length === 0 && (
            <div className="flex gap-2">
              <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-gradient-to-br from-amber-500/20 to-orange-500/20 ring-1 ring-white/[0.06]">
                <span className="text-[10px] font-bold text-amber-300">B</span>
              </div>
              <div className="flex items-center gap-2 pt-2">
                <span className="relative flex h-1.5 w-1.5">
                  <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-amber-400 opacity-75" />
                  <span className="relative inline-flex rounded-full h-1.5 w-1.5 bg-amber-400" />
                </span>
                <span className="animate-shimmer-text text-[12px] font-medium text-amber-400">{workingLabel}</span>
              </div>
            </div>
          )}

          <div ref={bottomRef} />
        </div>
      </div>

      {/* Input */}
      <div className="shrink-0 border-t border-[#2a2520] bg-[#0f0e0c]/90 px-3 py-2.5">
        <div className="relative flex items-end gap-1.5 rounded-xl border border-[#2a2520] bg-[#1c1a17] px-3 py-2 transition-colors focus-within:border-amber-500/20 focus-within:bg-[#232019]">
          <textarea
            ref={inputRef}
            value={input}
            onChange={handleInputChange}
            onKeyDown={handleKeyDown}
            placeholder="Message Borg..."
            rows={1}
            className="max-h-[120px] min-h-[20px] flex-1 resize-none bg-transparent text-[13px] leading-relaxed text-[#e8e0d4] placeholder:text-[#6b6459] focus:outline-none"
          />
          <div className="flex shrink-0 items-center gap-0.5">
            {dictation.supported && (
              <button
                onClick={dictation.toggle}
                className={cn(
                  "rounded-lg p-1.5 transition-colors",
                  dictation.listening ? "bg-red-500/20 text-red-400" : "text-[#6b6459] hover:text-[#9c9486]",
                )}
              >
                {dictation.listening ? <MicOff className="h-3.5 w-3.5" /> : <Mic className="h-3.5 w-3.5" />}
              </button>
            )}
            <button
              onClick={handleSend}
              disabled={!input.trim() || sending}
              className={cn(
                "rounded-lg p-1.5 transition-all",
                input.trim() && !sending
                  ? "bg-amber-500 text-white hover:bg-amber-400 shadow-lg shadow-amber-500/25"
                  : "text-[#6b6459] cursor-not-allowed",
              )}
            >
              <Send className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function threadLabel(id: string, projects: { id: number; name: string }[]): string {
  if (id === "web:dashboard") return "Global";
  const match = id.match(/^web:project-(\d+)$/);
  if (match) {
    const proj = projects.find((p) => p.id === Number(match[1]));
    return proj?.name ?? `Project #${match[1]}`;
  }
  return id.replace("web:", "");
}

function MessageBubble({ msg }: { msg: ChatMessage }) {
  const isUser = msg.role === "user";

  return (
    <div className={cn("flex gap-2", isUser && "flex-row-reverse")}>
      <div
        className={cn(
          "flex h-7 w-7 shrink-0 items-center justify-center rounded-full ring-1 ring-white/[0.06]",
          isUser
            ? "bg-gradient-to-br from-amber-400/20 to-yellow-500/20"
            : "bg-gradient-to-br from-amber-500/20 to-orange-500/20",
        )}
      >
        <span className={cn("text-[10px] font-bold", isUser ? "text-amber-200" : "text-amber-300")}>
          {isUser ? "U" : "B"}
        </span>
      </div>
      <div className={cn("min-w-0 max-w-[85%]", isUser && "flex flex-col items-end")}>
        <div
          className={cn(
            "rounded-2xl px-4 py-3 text-[13px] leading-relaxed shadow-[0_12px_28px_rgba(0,0,0,0.14)]",
            isUser
              ? "bg-amber-500/15 text-[#e8e0d4] rounded-br-md"
              : "rounded-bl-md border border-[#2b241d] bg-[#171411] text-[#e8e0d4]",
          )}
        >
          {isUser ? (
            <div className="whitespace-pre-wrap break-words">{msg.text}</div>
          ) : (
            <ChatMarkdown text={msg.text} variant="bubble" />
          )}
        </div>
      </div>
    </div>
  );
}

function WorkItemControls({ thread }: { thread: string }) {
  const match = thread.match(/^web:project-(\d+)$/);
  const projectId = match ? Number(match[1]) : null;
  const { data: tasks = [] } = useProjectTasks(projectId);
  const { data: fullModes = [] } = useFullModes();
  const queryClient = useQueryClient();
  const [revisionId, setRevisionId] = useState<number | null>(null);
  const [feedback, setFeedback] = useState("");

  const reviewTasks = useMemo(
    () =>
      tasks.filter((t) =>
        fullModes.some(
          (m) => m.name === t.mode && m.phases.some((p) => p.name === t.status && p.phase_type === "human_review"),
        ),
      ),
    [tasks, fullModes],
  );

  const failedTasks = useMemo(() => tasks.filter((t) => t.status === "failed"), [tasks]);

  if (reviewTasks.length === 0 && failedTasks.length === 0) return null;

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: ["project_tasks", projectId] });
  };

  return (
    <div className="space-y-2">
      {reviewTasks.map((task) => (
        <div key={task.id} className="rounded-xl border border-emerald-500/20 bg-emerald-500/[0.04] px-3 py-2.5">
          <div className="text-[12px] text-[#e8e0d4] font-medium mb-1.5">
            #{task.id} {task.title}
          </div>
          <div className="text-[11px] text-emerald-400/70 mb-2">Awaiting review</div>
          <div className="flex items-center gap-2">
            <button
              onClick={async () => {
                await approveTask(task.id);
                invalidate();
              }}
              className="rounded-lg bg-emerald-500/15 px-3 py-1.5 text-[11px] font-medium text-emerald-400 hover:bg-emerald-500/25 transition-colors"
            >
              Approve
            </button>
            <button
              onClick={() => setRevisionId(revisionId === task.id ? null : task.id)}
              className="rounded-lg bg-amber-500/10 px-3 py-1.5 text-[11px] font-medium text-amber-400 hover:bg-amber-500/20 transition-colors"
            >
              Revise
            </button>
            <button
              onClick={async () => {
                if (confirm("Reject this task?")) {
                  await rejectTask(task.id, "Rejected by reviewer");
                  invalidate();
                }
              }}
              className="rounded-lg bg-red-500/10 px-3 py-1.5 text-[11px] font-medium text-red-400 hover:bg-red-500/20 transition-colors"
            >
              Reject
            </button>
          </div>
          {revisionId === task.id && (
            <div className="mt-2 space-y-1.5">
              <textarea
                value={feedback}
                onChange={(e) => setFeedback(e.target.value)}
                rows={2}
                className="w-full rounded-lg border border-amber-500/20 bg-black/30 px-2.5 py-1.5 text-[11px] text-[#e8e0d4] outline-none placeholder:text-[#6b6459] focus:border-amber-500/40 resize-y"
                placeholder="What needs to change..."
              />
              <button
                onClick={async () => {
                  if (!feedback.trim()) return;
                  await requestRevision(task.id, feedback.trim());
                  setFeedback("");
                  setRevisionId(null);
                  invalidate();
                }}
                disabled={!feedback.trim()}
                className="rounded-lg bg-amber-500/15 px-3 py-1 text-[11px] font-medium text-amber-400 hover:bg-amber-500/25 disabled:opacity-40 transition-colors"
              >
                Send
              </button>
            </div>
          )}
        </div>
      ))}
      {failedTasks.map((task) => (
        <div key={task.id} className="rounded-xl border border-red-500/20 bg-red-500/[0.04] px-3 py-2.5">
          <div className="text-[12px] text-[#e8e0d4] font-medium mb-1">
            #{task.id} {task.title}
          </div>
          <div className="text-[11px] text-red-400/70 mb-2">Failed</div>
          <button
            onClick={async () => {
              await retryTask(task.id);
              invalidate();
            }}
            className="rounded-lg bg-red-500/10 px-3 py-1.5 text-[11px] font-medium text-red-400 hover:bg-red-500/20 transition-colors"
          >
            Retry
          </button>
        </div>
      ))}
    </div>
  );
}

function AgentTimeline({
  lines,
  streaming,
  hideFinalOutput,
}: {
  lines: TermLine[];
  streaming: boolean;
  hideFinalOutput?: boolean;
}) {
  return <ActionActivity lines={lines} streaming={streaming} compact hideFinalOutput={hideFinalOutput} />;
}

function extractMcpStatus(events: StreamEvent[]): { name: string; status: string }[] | null {
  for (const ev of events) {
    const raw = ev as unknown as Record<string, unknown>;
    if (raw.type === "system" && raw.subtype === "init" && Array.isArray(raw.mcp_servers)) {
      return raw.mcp_servers as { name: string; status: string }[];
    }
  }
  return null;
}

function CollapsedTimeline({ events }: { events: StreamEvent[] }) {
  const [expanded, setExpanded] = useState(false);
  const lines = useMemo(() => parseStreamEvents(events), [events]);
  const toolCount = lines.filter((l) => l.type === "tool").length;
  const mcpServers = useMemo(() => extractMcpStatus(events), [events]);
  const failedMcp = mcpServers?.filter((s) => s.status === "failed") ?? [];
  if (toolCount === 0 && failedMcp.length === 0) return null;

  return (
    <div>
      {failedMcp.length > 0 && (
        <div className="mb-1.5 flex items-center gap-1.5 rounded-lg border border-amber-500/15 bg-amber-500/[0.04] px-3 py-1.5 text-[11px] text-amber-400/70">
          <span className="inline-block h-1.5 w-1.5 rounded-full bg-amber-400/50" />
          MCP {failedMcp.map((s) => s.name).join(", ")} failed to load
        </div>
      )}
      {toolCount > 0 && (
        <div className="rounded-xl border border-[#2a2520] bg-[#1c1a17]">
          <button
            onClick={() => setExpanded(!expanded)}
            className="flex w-full items-center gap-2.5 px-3.5 py-2.5 text-left transition-colors hover:bg-amber-500/[0.03]"
          >
            <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-lg bg-amber-500/10 text-amber-400/80">
              <Sparkles className="h-3.5 w-3.5" />
            </span>
            <span className="min-w-0 flex-1 text-[13px] font-medium text-[#9c9486]">
              Borg performed {toolCount} action{toolCount !== 1 ? "s" : ""}
            </span>
            <ChevronDown
              className={cn(
                "h-3.5 w-3.5 shrink-0 text-[#6b6459] transition-transform duration-200",
                expanded && "rotate-180",
              )}
            />
          </button>
          {expanded && (
            <div className="border-t border-[#2a2520] p-2.5">
              <AgentTimeline lines={lines} streaming={false} hideFinalOutput />
            </div>
          )}
        </div>
      )}
    </div>
  );
}
