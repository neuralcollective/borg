import { useState, useRef, useEffect, useCallback, useMemo } from "react";
import { Send, Mic, MicOff, ChevronRight, FolderOpen } from "lucide-react";
import { cn } from "@/lib/utils";
import { useDictation } from "@/lib/dictation";
import { ChatMarkdown } from "./chat-markdown";
import { authHeaders, tokenReady, useProjects } from "@/lib/api";
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
  MessageSquare,
} from "lucide-react";

interface ChatMessage {
  role: "user" | "assistant";
  sender?: string;
  text: string;
  ts: string | number;
  thread?: string;
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

function threadLabel(id: string, projects: { id: number; name: string }[]): string {
  if (id === "web:dashboard") return "Global";
  const match = id.match(/^web:project-(\d+)$/);
  if (match) {
    const proj = projects.find((p) => p.id === Number(match[1]));
    return proj?.name ?? `Project #${match[1]}`;
  }
  return id.replace("web:", "");
}

interface ChatDrawerProps {
  defaultThread?: string;
}

export function ChatDrawer({ defaultThread = "web:dashboard" }: ChatDrawerProps) {
  const { data: projects = [] } = useProjects();
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [thread, setThread] = useState(defaultThread);
  const [open, setOpen] = useState(false);
  const [streamEvents, setStreamEvents] = useState<StreamEvent[]>([]);
  const bottomRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const lastTsRef = useRef<number>(0);
  const sendingTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Auto-switch thread when a project is selected
  useEffect(() => {
    function handleProjectSelected(e: Event) {
      const id = (e as CustomEvent).detail;
      if (typeof id === "number") {
        setThread(`web:project-${id}`);
      }
    }
    window.addEventListener("borg:project-selected", handleProjectSelected);
    return () => window.removeEventListener("borg:project-selected", handleProjectSelected);
  }, []);

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

  useEffect(() => {
    setMessages([]);
    setStreamEvents([]);
    lastTsRef.current = 0;
    fetchMessages();
  }, [thread, fetchMessages]);

  const handleSseMessage = useCallback((msg: any) => {
    if (msg.type === "chat_stream" && msg.thread === thread) {
      try {
        const parsed = JSON.parse(msg.data);
        if (parsed.type) {
          setStreamEvents((prev) => [...prev, parsed]);
        }
      } catch { /* skip */ }
      return;
    }
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
    if (open) {
      bottomRef.current?.scrollIntoView({ behavior: "instant" });
    }
  }, [messages.length, streamEvents.length, open]);

  async function handleSend() {
    const text = input.trim();
    if (!text || sending) return;

    setInput("");
    setSending(true);
    setOpen(true);
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
    el.style.height = Math.min(el.scrollHeight, 160) + "px";
  }

  const streamLines = useMemo(
    () => parseStreamEvents(streamEvents),
    [streamEvents]
  );

  const hasContent = messages.length > 0 || sending;
  const scopeLabel = threadLabel(thread, projects);

  return (
    <div
      className={cn(
        "flex h-full shrink-0 flex-col border-l border-[#2a2520] bg-[#0f0e0c] transition-[width] duration-200 ease-out overflow-hidden",
        open ? "w-[380px]" : "w-12"
      )}
    >
      {/* Header — always visible */}
      <div className="flex h-12 shrink-0 items-center border-b border-[#2a2520]">
        {open ? (
          <div className="flex w-full items-center gap-2 px-3">
            <button
              onClick={() => setOpen(false)}
              className="rounded-lg p-1.5 text-[#6b6459] transition-colors hover:text-[#9c9486]"
              title="Collapse"
            >
              <ChevronRight className="h-4 w-4" />
            </button>
            <div className="flex min-w-0 flex-1 items-center gap-1.5">
              {thread === "web:dashboard" ? (
                <Globe className="h-3 w-3 shrink-0 text-amber-400/60" />
              ) : (
                <FolderOpen className="h-3 w-3 shrink-0 text-amber-400/60" />
              )}
              <span className="truncate text-[12px] font-medium text-[#9c9486]">{scopeLabel}</span>
            </div>
            {thread !== "web:dashboard" && (
              <button
                onClick={() => setThread("web:dashboard")}
                className="rounded-lg p-1 text-[#6b6459] transition-colors hover:text-[#9c9486]"
                title="Switch to Global"
              >
                <Globe className="h-3.5 w-3.5" />
              </button>
            )}
          </div>
        ) : (
          <button
            onClick={() => setOpen(true)}
            className="flex h-full w-full items-center justify-center text-[#6b6459] transition-colors hover:text-amber-400"
            title="Open chat"
          >
            <MessageSquare className="h-[18px] w-[18px]" />
          </button>
        )}
      </div>

      {/* Messages — only when open */}
      {open && (
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain">
          <div className="px-3 py-3 space-y-3">
            {!hasContent && (
              <div className="flex flex-col items-center justify-center py-12 text-center">
                <div className="mb-4 flex h-12 w-12 items-center justify-center rounded-2xl bg-[#1c1a17] ring-1 ring-[#2a2520]">
                  {thread === "web:dashboard" ? (
                    <Globe className="h-5 w-5 text-amber-400/40" strokeWidth={1.5} />
                  ) : (
                    <FolderOpen className="h-5 w-5 text-[#6b6459]" strokeWidth={1.5} />
                  )}
                </div>
                <p className="text-[13px] font-medium text-[#9c9486]">{scopeLabel}</p>
                <p className="mt-1 text-[11px] text-[#6b6459]">
                  {thread === "web:dashboard"
                    ? "Chat with global knowledge"
                    : "Scoped to this project"}
                </p>
              </div>
            )}

            {messages.map((msg, i) => (
              <MessageBubble key={`${msg.ts}-${msg.role}-${i}`} msg={msg} />
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
                  <span className="animate-shimmer-text text-[12px] font-medium text-amber-400">
                    Working...
                  </span>
                </div>
              </div>
            )}

            <div ref={bottomRef} />
          </div>
        </div>
      )}

      {/* Input — only when open */}
      {open && (
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
        </div>
      )}
    </div>
  );
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
            : "bg-gradient-to-br from-amber-500/20 to-orange-500/20"
        )}
      >
        <span className={cn("text-[10px] font-bold", isUser ? "text-amber-200" : "text-amber-300")}>
          {isUser ? "U" : "B"}
        </span>
      </div>
      <div className={cn("min-w-0 max-w-[85%]", isUser && "flex flex-col items-end")}>
        <div
          className={cn(
            "rounded-2xl px-3 py-2 text-[13px] leading-relaxed",
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
  const visible = lines.slice(-15);

  return (
    <div className="space-y-0.5 text-[12px]">
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
            <div key={i} className="text-[#e8e0d4] text-[12px] whitespace-pre-wrap break-words pl-7 py-0.5 leading-relaxed">
              {line.content.length > 150 ? line.content.slice(0, 150) + "..." : line.content}
            </div>
          );
        }
        if (line.type === "tool_result") {
          return (
            <div key={i} className="pl-7 py-0.5 text-[10px] text-[#6b6459] break-all">
              {line.content.length > 80 ? line.content.slice(0, 80) + "..." : line.content}
            </div>
          );
        }
        return null;
      })}
      {streaming && (
        <div className="flex items-center gap-2 pl-7 pt-1">
          <span className="relative flex h-1.5 w-1.5">
            <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-amber-400 opacity-75" />
            <span className="relative inline-flex rounded-full h-1.5 w-1.5 bg-amber-400" />
          </span>
        </div>
      )}
    </div>
  );
}
