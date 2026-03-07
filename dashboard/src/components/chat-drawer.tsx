import { useState, useRef, useEffect, useCallback, useMemo } from "react";
import { Send, Mic, MicOff, ChevronDown, ChevronUp, Plus } from "lucide-react";
import { cn } from "@/lib/utils";
import { useDictation } from "@/lib/dictation";
import { ChatMarkdown } from "./chat-markdown";
import { authHeaders, tokenReady } from "@/lib/api";
import { useChatEvents } from "@/lib/use-chat-events";
import { parseStreamEvents, type TermLine } from "@/lib/stream-utils";
import { TimelineItem } from "./borging";
import type { StreamEvent } from "@/lib/api";
import {
  FileText,
  FilePen,
  Pencil,
  Terminal,
  Search,
  Globe,
  Sparkles,
  Circle,
} from "lucide-react";

interface ChatMessage {
  role: "user" | "assistant";
  sender?: string;
  text: string;
  ts: string | number;
  thread?: string;
}

interface ChatThread {
  id: string;
  last_ts: string;
  message_count: number;
}

const toolIconMap: Record<string, React.ReactNode> = {
  Read: <FileText className="w-3.5 h-3.5 text-amber-400/70" />,
  Write: <FilePen className="w-3.5 h-3.5 text-amber-400/70" />,
  Edit: <Pencil className="w-3.5 h-3.5 text-amber-400/70" />,
  Bash: <Terminal className="w-3.5 h-3.5 text-amber-400/70" />,
  Grep: <Search className="w-3.5 h-3.5 text-amber-400/70" />,
  Glob: <Search className="w-3.5 h-3.5 text-amber-400/70" />,
  WebFetch: <Globe className="w-3.5 h-3.5 text-amber-400/70" />,
  WebSearch: <Globe className="w-3.5 h-3.5 text-amber-400/70" />,
  Task: <Sparkles className="w-3.5 h-3.5 text-amber-400/70" />,
  Agent: <Sparkles className="w-3.5 h-3.5 text-amber-400/70" />,
};

function getToolIcon(tool?: string): React.ReactNode {
  if (!tool) return <Circle className="w-3 h-3 text-[#6b6459]" />;
  return toolIconMap[tool] || <Circle className="w-3 h-3 text-[#6b6459]" />;
}

function threadLabel(id: string): string {
  if (id === "web:dashboard") return "Main";
  return id.replace("web:", "");
}

interface ChatDrawerProps {
  defaultThread?: string;
}

export function ChatDrawer({ defaultThread = "web:dashboard" }: ChatDrawerProps) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [thread, setThread] = useState(defaultThread);
  const [threads, setThreads] = useState<ChatThread[]>([]);
  const [showThreads, setShowThreads] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [streamEvents, setStreamEvents] = useState<StreamEvent[]>([]);
  const bottomRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const lastTsRef = useRef<number>(0);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const sendingTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const fetchMessages = useCallback(() => {
    tokenReady.then(() => {
      fetch(`/api/chat/messages?thread=${encodeURIComponent(thread)}`, { headers: authHeaders() })
        .then((r) => { if (!r.ok) throw new Error(`${r.status}`); return r.json(); })
        .then((msgs: ChatMessage[]) => {
          setMessages(msgs);
          if (msgs.length > 0) {
            lastTsRef.current = Math.max(...msgs.map((m) => Number(m.ts) || 0));
          }
        })
        .catch(() => {});
    });
  }, [thread]);

  const fetchThreads = useCallback(() => {
    tokenReady.then(() => {
      fetch("/api/chat/threads", { headers: authHeaders() })
        .then((r) => r.json())
        .then((t: ChatThread[]) => setThreads(t))
        .catch(() => {});
    });
  }, []);

  useEffect(() => {
    setMessages([]);
    setStreamEvents([]);
    lastTsRef.current = 0;
    fetchMessages();
    fetchThreads();
  }, [thread, fetchMessages, fetchThreads]);

  const handleSseMessage = useCallback((msg: any) => {
    // Handle stream events (agentic breakdown)
    if (msg.type === "chat_stream" && msg.thread === thread) {
      try {
        const parsed = JSON.parse(msg.data);
        if (parsed.type) {
          setStreamEvents((prev) => [...prev, parsed]);
        }
      } catch { /* skip */ }
      return;
    }
    // Handle regular chat messages
    if (msg.role === "user") return;
    setMessages((prev) => [...prev, msg]);
    lastTsRef.current = Math.max(lastTsRef.current, Number(msg.ts) || 0);
    if (msg.role === "assistant") {
      setSending(false);
      setStreamEvents([]);
      if (sendingTimeoutRef.current) {
        clearTimeout(sendingTimeoutRef.current);
        sendingTimeoutRef.current = null;
      }
    }
  }, [thread]);

  useChatEvents(thread, handleSseMessage, () => setSending(false));

  useEffect(() => {
    return () => {
      if (sendingTimeoutRef.current) clearTimeout(sendingTimeoutRef.current);
    };
  }, []);

  // Poll fallback
  useEffect(() => {
    const interval = setInterval(() => {
      fetch(`/api/chat/messages?thread=${encodeURIComponent(thread)}`, { headers: authHeaders() })
        .then((r) => r.json())
        .then((msgs: ChatMessage[]) => {
          if (msgs.length === 0) return;
          const newTs = Math.max(...msgs.map((m) => Number(m.ts) || 0));
          if (newTs > lastTsRef.current) {
            setMessages(msgs);
            lastTsRef.current = newTs;
            if (msgs[msgs.length - 1]?.role === "assistant") {
              setSending(false);
              setStreamEvents([]);
              if (sendingTimeoutRef.current) {
                clearTimeout(sendingTimeoutRef.current);
                sendingTimeoutRef.current = null;
              }
            }
          }
        })
        .catch(() => {});
    }, 3000);
    return () => clearInterval(interval);
  }, [thread]);

  useEffect(() => {
    if (expanded) {
      bottomRef.current?.scrollIntoView({ behavior: "instant" });
    }
  }, [messages.length, streamEvents.length, expanded]);

  useEffect(() => {
    if (!showThreads) return;
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setShowThreads(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [showThreads]);

  async function handleSend() {
    const text = input.trim();
    if (!text || sending) return;

    setInput("");
    setSending(true);
    setExpanded(true);
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

  function handleNewThread() {
    const id = `web:thread-${Date.now()}`;
    setThread(id);
    setShowThreads(false);
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
    el.style.height = Math.min(el.scrollHeight, 160) + "px";
  }

  function handleInputFocus() {
    if (messages.length > 0 || sending) {
      setExpanded(true);
    }
  }

  // Parse stream events into timeline lines
  const streamLines = useMemo(
    () => parseStreamEvents(streamEvents),
    [streamEvents]
  );

  const hasContent = messages.length > 0 || sending;

  return (
    <div className="flex flex-col border-t border-[#2a2520] bg-[#0f0e0c]">
      {/* Expandable message area */}
      {expanded && hasContent && (
        <div className="max-h-[50vh] overflow-y-auto overscroll-contain">
          <div className="mx-auto max-w-3xl px-4 py-4 space-y-4">
            {messages.map((msg, i) => (
              <MessageBubble key={`${msg.ts}-${msg.role}-${i}`} msg={msg} />
            ))}

            {/* Agentic breakdown while working */}
            {sending && streamLines.length > 0 && (
              <div className="flex gap-3">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-gradient-to-br from-amber-500/20 to-orange-500/20 ring-1 ring-white/[0.06]">
                  <span className="text-[11px] font-bold text-amber-300">B</span>
                </div>
                <div className="min-w-0 flex-1 pt-1">
                  <AgentTimeline lines={streamLines} streaming />
                </div>
              </div>
            )}

            {/* Simple working indicator if no stream events yet */}
            {sending && streamLines.length === 0 && (
              <div className="flex gap-3">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-gradient-to-br from-amber-500/20 to-orange-500/20 ring-1 ring-white/[0.06]">
                  <span className="text-[11px] font-bold text-amber-300">B</span>
                </div>
                <div className="flex items-center gap-2 pt-2.5">
                  <span className="relative flex h-2 w-2">
                    <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-amber-400 opacity-75" />
                    <span className="relative inline-flex rounded-full h-2 w-2 bg-amber-400" />
                  </span>
                  <span className="animate-shimmer-text text-[13px] font-medium text-amber-400">
                    Working...
                  </span>
                </div>
              </div>
            )}

            <div ref={bottomRef} />
          </div>
        </div>
      )}

      {/* Input bar — always visible */}
      <div className="shrink-0 bg-[#0f0e0c]/90 backdrop-blur-sm">
        <div className="mx-auto max-w-3xl px-4 py-3">
          <div className="flex items-center gap-2">
            {/* Thread selector */}
            <div className="relative" ref={dropdownRef}>
              <button
                onClick={() => setShowThreads(!showThreads)}
                className="flex items-center gap-1 rounded-lg px-2 py-1.5 text-[11px] text-[#6b6459] transition-colors hover:text-[#9c9486]"
                title="Switch thread"
              >
                <span className="max-w-[60px] truncate">{threadLabel(thread)}</span>
                <ChevronDown className="h-3 w-3" />
              </button>
              {showThreads && (
                <div className="absolute left-0 bottom-full z-50 mb-2 min-w-[180px] overflow-hidden rounded-xl border border-[#2a2520] bg-[#1c1a17] shadow-2xl">
                  <div className="p-1.5 max-h-[200px] overflow-y-auto">
                    {threads.map((t) => (
                      <button
                        key={t.id}
                        onClick={() => { setThread(t.id); setShowThreads(false); }}
                        className={cn(
                          "flex w-full items-center justify-between rounded-lg px-3 py-1.5 text-left text-[12px] transition-colors hover:bg-[#232019]",
                          t.id === thread ? "bg-[#232019] text-[#e8e0d4]" : "text-[#9c9486]"
                        )}
                      >
                        <span className="truncate">{threadLabel(t.id)}</span>
                        <span className="ml-2 text-[10px] tabular-nums text-[#6b6459]">{t.message_count}</span>
                      </button>
                    ))}
                  </div>
                  <div className="border-t border-[#2a2520] p-1.5">
                    <button
                      onClick={handleNewThread}
                      className="flex w-full items-center gap-1.5 rounded-lg px-3 py-1.5 text-[12px] text-amber-400 transition-colors hover:bg-amber-500/10"
                    >
                      <Plus className="h-3 w-3" />
                      New Thread
                    </button>
                  </div>
                </div>
              )}
            </div>

            {/* Input */}
            <div className="relative flex min-w-0 flex-1 items-end gap-2 rounded-2xl border border-[#2a2520] bg-[#1c1a17] px-3 py-2 transition-colors focus-within:border-amber-500/20 focus-within:bg-[#232019]">
              <textarea
                ref={inputRef}
                value={input}
                onChange={handleInputChange}
                onKeyDown={handleKeyDown}
                onFocus={handleInputFocus}
                placeholder="Message Borg..."
                rows={1}
                className="max-h-[160px] min-h-[22px] flex-1 resize-none bg-transparent text-[13px] leading-relaxed text-[#e8e0d4] placeholder:text-[#6b6459] focus:outline-none"
              />
              <div className="flex shrink-0 items-center gap-1">
                {dictation.supported && (
                  <button
                    onClick={dictation.toggle}
                    className={cn(
                      "rounded-lg p-1.5 transition-colors",
                      dictation.listening
                        ? "bg-red-500/20 text-red-400"
                        : "text-[#6b6459] hover:text-[#9c9486]"
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
                      : "text-[#6b6459] cursor-not-allowed"
                  )}
                >
                  <Send className="h-3.5 w-3.5" />
                </button>
              </div>
            </div>

            {/* Expand/collapse */}
            {hasContent && (
              <button
                onClick={() => setExpanded(!expanded)}
                className="rounded-lg p-1.5 text-[#6b6459] transition-colors hover:text-[#9c9486]"
              >
                {expanded ? <ChevronDown className="h-4 w-4" /> : <ChevronUp className="h-4 w-4" />}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function MessageBubble({ msg }: { msg: ChatMessage }) {
  const isUser = msg.role === "user";

  return (
    <div className={cn("flex gap-3", isUser && "flex-row-reverse")}>
      <div
        className={cn(
          "flex h-8 w-8 shrink-0 items-center justify-center rounded-full ring-1 ring-white/[0.06]",
          isUser
            ? "bg-gradient-to-br from-amber-400/20 to-yellow-500/20"
            : "bg-gradient-to-br from-amber-500/20 to-orange-500/20"
        )}
      >
        <span className={cn("text-[11px] font-bold", isUser ? "text-amber-200" : "text-amber-300")}>
          {isUser ? "U" : "B"}
        </span>
      </div>
      <div className={cn("min-w-0 max-w-[85%]", isUser && "flex flex-col items-end")}>
        <div
          className={cn(
            "rounded-2xl px-4 py-2.5 text-[13px] leading-relaxed",
            isUser
              ? "bg-amber-500/15 text-[#e8e0d4] rounded-br-md"
              : "bg-[#1c1a17] text-[#e8e0d4] rounded-bl-md"
          )}
        >
          {isUser ? (
            <div className="whitespace-pre-wrap break-words">{msg.text}</div>
          ) : (
            <ChatMarkdown text={msg.text} />
          )}
        </div>
      </div>
    </div>
  );
}

function AgentTimeline({ lines, streaming }: { lines: TermLine[]; streaming: boolean }) {
  // Show only the last N lines to keep it compact
  const visible = lines.slice(-20);

  return (
    <div className="space-y-0.5 text-[13px]">
      {visible.map((line, i) => {
        if (line.type === "tool") {
          return (
            <TimelineItem
              key={i}
              icon={getToolIcon(line.tool)}
              label={line.tool || "Tool"}
              detail={line.label || line.content || undefined}
              isActive
              isFirst={i === 0}
              isLast={i === visible.length - 1 && !streaming}
            />
          );
        }
        if (line.type === "text") {
          return (
            <div key={i} className="text-[#e8e0d4] text-[13px] whitespace-pre-wrap break-words pl-9 py-0.5 leading-relaxed">
              {line.content.length > 200 ? line.content.slice(0, 200) + "..." : line.content}
            </div>
          );
        }
        if (line.type === "tool_result") {
          return (
            <div key={i} className="pl-9 py-0.5 text-[11px] text-[#6b6459] break-all">
              {line.content.length > 100 ? line.content.slice(0, 100) + "..." : line.content}
            </div>
          );
        }
        return null;
      })}
      {streaming && (
        <div className="flex items-center gap-2 pl-9 pt-1">
          <span className="relative flex h-1.5 w-1.5">
            <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-amber-400 opacity-75" />
            <span className="relative inline-flex rounded-full h-1.5 w-1.5 bg-amber-400" />
          </span>
        </div>
      )}
    </div>
  );
}
