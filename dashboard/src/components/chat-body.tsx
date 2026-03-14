import { useQueryClient } from "@tanstack/react-query";
import { ArrowDown, Check, ChevronDown, Copy, FolderOpen, Globe, Mic, MicOff, Send, Sparkles } from "lucide-react";
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
  useStatus,
} from "@/lib/api";
import { useDictation } from "@/lib/dictation";
import { parseStreamEvents, rawStreamToEvents, type TermLine } from "@/lib/stream-utils";
import { useChatEvents } from "@/lib/use-chat-events";
import { useChatStream } from "@/lib/use-chat-stream";
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
  const { data: status } = useStatus();
  const availableModels = status?.available_models ?? [];
  const [selectedModel, setSelectedModel] = useState<string>("");
  const [showModelPicker, setShowModelPicker] = useState(false);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [completedStreams, setCompletedStreams] = useState<Map<number, StreamEvent[]>>(new Map());
  const [workingLabel, setWorkingLabel] = useState("Working...");
  const bottomRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const modelDropdownRef = useRef<HTMLDivElement>(null);
  const lastTsRef = useRef<number>(0);
  const sendingRef = useRef(false);
  const sendingTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Per-thread stream — replays full history on connect/reconnect, no gaps
  const {
    streamEvents,
    isStreaming,
    lastEventTimeRef: lastStreamEventRef,
    reset: resetStream,
  } = useChatStream(thread);

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

  useEffect(() => {
    sendingRef.current = sending;
  }, [sending]);

  useEffect(() => {
    if (isStreaming) setSending(true);
  }, [isStreaming]);

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (modelDropdownRef.current && !modelDropdownRef.current.contains(e.target as Node)) {
        setShowModelPicker(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, []);

  const forceScrollRef = useRef(true);

  useEffect(() => {
    setMessages([]);
    resetStream();
    setCompletedStreams(new Map());
    lastTsRef.current = 0;
    forceScrollRef.current = true;
    fetchMessages();
  }, [fetchMessages]);

  // Global SSE for message notifications (user/assistant messages, not stream data)
  const handleSseMessage = useCallback(
    (msg: any) => {
      // Stream events are handled by useChatStream — skip them here
      if (msg.type === "chat_stream" || msg.type === "task_stream") return;
      if (msg.role === "user") return;
      setMessages((prev) => {
        const next = [...prev, msg];
        if (msg.role === "assistant") {
          // Move current stream events to completed streams for this message
          if (streamEvents.length > 0) {
            setCompletedStreams((m) => {
              const updated = new Map(m);
              updated.set(next.length - 1, [...streamEvents]);
              return updated;
            });
          }
          resetStream();
          setSending(false);
          lastStreamEventRef.current = 0;
          if (sendingTimeoutRef.current) {
            clearTimeout(sendingTimeoutRef.current);
            sendingTimeoutRef.current = null;
          }
        }
        return next;
      });
      lastTsRef.current = Math.max(lastTsRef.current, Number(msg.ts) || 0);
    },
    [thread, streamEvents, resetStream],
  );

  useChatEvents(thread, handleSseMessage);

  useEffect(() => {
    return () => {
      if (sendingTimeoutRef.current) clearTimeout(sendingTimeoutRef.current);
    };
  }, []);

  // Poll fallback — recover from edge case where SSE dies at exact moment of completion
  const pollRecoveringRef = useRef(false);
  useEffect(() => {
    if (!sending) return;
    const interval = setInterval(() => {
      if (pollRecoveringRef.current) return;
      fetch(`/api/chat/status?thread=${encodeURIComponent(thread)}`, { headers: authHeaders() })
        .then((r) => r.json())
        .then(async (data: { running: boolean }) => {
          if (!data.running && !pollRecoveringRef.current) {
            pollRecoveringRef.current = true;
            try {
              await tokenReady;
              const r = await fetch(`/api/chat/messages?thread=${encodeURIComponent(thread)}`, {
                headers: authHeaders(),
              });
              if (!r.ok) throw new Error(`${r.status}`);
              const msgs: ChatMessage[] = await r.json();
              const restored = new Map<number, StreamEvent[]>();
              msgs.forEach((m, i) => {
                if (m.role === "assistant" && m.raw_stream) {
                  const events = rawStreamToEvents(m.raw_stream);
                  if (events.length > 0) restored.set(i, events);
                }
              });
              setMessages(msgs);
              if (restored.size > 0) setCompletedStreams(restored);
              if (msgs.length > 0) {
                lastTsRef.current = Math.max(...msgs.map((m) => Number(m.ts) || 0));
              }
              setSending(false);
              resetStream();
              lastStreamEventRef.current = 0;
              if (sendingTimeoutRef.current) {
                clearTimeout(sendingTimeoutRef.current);
                sendingTimeoutRef.current = null;
              }
            } catch {
              setSending(false);
              resetStream();
              lastStreamEventRef.current = 0;
            } finally {
              pollRecoveringRef.current = false;
            }
          }
        })
        .catch(() => {});
    }, 3000);
    return () => clearInterval(interval);
  }, [thread, sending, resetStream]);

  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const [isScrolledUp, setIsScrolledUp] = useState(false);

  useEffect(() => {
    const el = scrollContainerRef.current;
    if (!el) return;
    const onScroll = () => {
      const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 150;
      setIsScrolledUp(!nearBottom);
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, []);

  useEffect(() => {
    const el = scrollContainerRef.current;
    if (!el) return;
    if (forceScrollRef.current && messages.length > 0) {
      forceScrollRef.current = false;
      bottomRef.current?.scrollIntoView({ behavior: "instant" });
      return;
    }
    if (!isScrolledUp) {
      bottomRef.current?.scrollIntoView({ behavior: "instant" });
    }
  }, [messages.length, isScrolledUp]);

  async function handleSend() {
    const text = input.trim();
    if (!text || sending) return;

    setInput("");
    setSending(true);
    setWorkingLabel(pickWorkingLabel());
    resetStream();
    lastStreamEventRef.current = 0;

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

    const postBody = JSON.stringify({
      text,
      sender: "web-user",
      thread,
      ...(selectedModel ? { model: selectedModel } : {}),
    });
    const doPost = async () => {
      await tokenReady;
      const res = await fetch("/api/chat", {
        method: "POST",
        headers: { "Content-Type": "application/json", ...authHeaders() },
        body: postBody,
      });
      if (!res.ok) throw new Error(`${res.status}`);
    };
    try {
      await doPost();
    } catch (err) {
      // Retry once after a short delay
      console.warn("chat POST failed, retrying:", err);
      try {
        await new Promise((r) => setTimeout(r, 1000));
        await doPost();
      } catch (retryErr) {
        console.error("chat POST retry failed:", retryErr);
        setSending(false);
        resetStream();
        if (sendingTimeoutRef.current) {
          clearTimeout(sendingTimeoutRef.current);
          sendingTimeoutRef.current = null;
        }
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
                {thread === "web:dashboard" ? "Chat with global knowledge" : "Scoped to this workspace"}
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
              <PulsingBotAvatar />
              <div className="min-w-0 flex-1 pt-0.5">
                <AgentTimeline lines={streamLines} streaming hideFinalOutput />
              </div>
            </div>
          )}

          {sending && streamLines.length === 0 && (
            <div className="flex gap-2">
              <PulsingBotAvatar />
              <div className="flex items-center pt-2">
                <span className="animate-shimmer-text text-[12px] font-medium text-amber-400">{workingLabel}</span>
              </div>
            </div>
          )}

          <div ref={bottomRef} />
        </div>

        {/* Scroll to live button */}
        {isScrolledUp && sending && (
          <button
            onClick={() => {
              setIsScrolledUp(false);
              bottomRef.current?.scrollIntoView({ behavior: "smooth" });
            }}
            className="absolute bottom-3 left-1/2 z-10 flex -translate-x-1/2 items-center gap-1.5 rounded-full bg-amber-500/90 px-3 py-1.5 text-[11px] font-medium text-white shadow-lg shadow-amber-500/25 transition-all hover:bg-amber-400"
          >
            <ArrowDown className="h-3 w-3" />
            Follow live output
          </button>
        )}
      </div>

      {/* Input */}
      <div className="shrink-0 border-t border-[#2a2520] bg-[#0f0e0c]/90 px-3 py-2.5">
        {availableModels.length > 1 && (
          <div className="mb-1.5 flex items-center" ref={modelDropdownRef}>
            <div className="relative">
              <button
                onClick={() => setShowModelPicker(!showModelPicker)}
                className="flex items-center gap-1.5 rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-1.5 text-[12px] text-[#9c9486] transition-colors hover:border-[#2a2520]/80 hover:text-[#e8e0d4]"
              >
                <Sparkles className="h-3 w-3 text-amber-400/60" />
                <span>{(availableModels.find((m) => m.model === selectedModel) ?? availableModels[0])?.label}</span>
                <ChevronDown className="h-3 w-3" />
              </button>
              {showModelPicker && (
                <div className="absolute bottom-full left-0 z-50 mb-2 min-w-[180px] overflow-hidden rounded-xl border border-[#2a2520] bg-[#1c1a17] shadow-2xl">
                  <div className="px-3 pt-2.5 pb-1.5 text-[10px] font-semibold uppercase tracking-wider text-[#6b6459]">
                    Model
                  </div>
                  <div className="p-1.5 pt-0">
                    {availableModels.map((m) => {
                      const isActive =
                        m.model === selectedModel || (!selectedModel && m.model === availableModels[0]?.model);
                      return (
                        <button
                          key={m.model}
                          onClick={() => {
                            setSelectedModel(m.model);
                            setShowModelPicker(false);
                          }}
                          className={cn(
                            "flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-[13px] transition-colors hover:bg-[#232019]",
                            isActive ? "bg-[#232019] text-[#e8e0d4]" : "text-[#9c9486]",
                          )}
                        >
                          <Sparkles
                            className={cn("h-3.5 w-3.5 shrink-0", isActive ? "text-amber-400/70" : "text-[#6b6459]")}
                          />
                          <span>{m.label}</span>
                        </button>
                      );
                    })}
                  </div>
                </div>
              )}
            </div>
          </div>
        )}
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
    return proj?.name ?? `#${match[1]}`;
  }
  return id.replace("web:", "");
}

function PulsingBotAvatar() {
  return (
    <div className="relative flex h-7 w-7 shrink-0 items-center justify-center">
      <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-amber-400/30" />
      <div className="relative flex h-7 w-7 items-center justify-center rounded-full bg-gradient-to-br from-amber-500/20 to-orange-500/20 ring-1 ring-amber-400/30">
        <span className="text-[13px]">🤖</span>
      </div>
    </div>
  );
}

function MessageBubble({ msg }: { msg: ChatMessage }) {
  const isUser = msg.role === "user";
  const [copied, setCopied] = useState(false);

  function handleCopy() {
    navigator.clipboard.writeText(msg.text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  return (
    <div className={cn("group flex gap-2", isUser && "flex-row-reverse")}>
      {!isUser && (
        <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-gradient-to-br from-amber-500/20 to-orange-500/20 ring-1 ring-white/[0.06]">
          <span className="text-[13px]">🤖</span>
        </div>
      )}
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
        <button
          onClick={handleCopy}
          className="mt-1 flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[10px] text-[#4a443d] opacity-0 transition-opacity hover:text-[#9c9486] group-hover:opacity-100"
        >
          {copied ? <Check className="h-3 w-3" /> : <Copy className="h-3 w-3" />}
          {copied ? "Copied" : "Copy"}
        </button>
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
