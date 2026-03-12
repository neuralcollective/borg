import { ChevronDown, FolderOpen, Globe, Mic, MicOff, Send } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { authHeaders, tokenReady, useProjects } from "@/lib/api";
import { useDictation } from "@/lib/dictation";
import { useChatEvents } from "@/lib/use-chat-events";
import { cn } from "@/lib/utils";
import { BorgingIndicator } from "./borging";
import { ChatMarkdown } from "./chat-markdown";

interface ChatMessage {
  role: "user" | "assistant";
  sender?: string;
  text: string;
  ts: string | number;
  thread?: string;
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

export function ChatPanel() {
  const { data: projects = [] } = useProjects();
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [thread, setThread] = useState("web:dashboard");
  const [showThreads, setShowThreads] = useState(false);
  const bottomRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const lastTsRef = useRef<number>(0);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const abortRef = useRef<AbortController | null>(null);
  const sendingTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const fetchMessages = useCallback(() => {
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;
    tokenReady.then(() => {
      fetch(`/api/chat/messages?thread=${encodeURIComponent(thread)}`, {
        signal: controller.signal,
        headers: authHeaders(),
      })
        .then((r) => {
          if (!r.ok) throw new Error(`${r.status}`);
          return r.json();
        })
        .then((msgs: ChatMessage[]) => {
          if (controller.signal.aborted) return;
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
    lastTsRef.current = 0;
    fetchMessages();
  }, [fetchMessages]);

  const handleSseMessage = useCallback((msg: ChatMessage) => {
    if (msg.role === "user") return;
    setMessages((prev) => [...prev, msg]);
    lastTsRef.current = Math.max(lastTsRef.current, Number(msg.ts) || 0);
    if (msg.role === "assistant") {
      setSending(false);
      if (sendingTimeoutRef.current) {
        clearTimeout(sendingTimeoutRef.current);
        sendingTimeoutRef.current = null;
      }
    }
  }, []);
  useChatEvents<ChatMessage>(thread, handleSseMessage, () => setSending(false));

  useEffect(() => {
    return () => {
      if (sendingTimeoutRef.current) clearTimeout(sendingTimeoutRef.current);
    };
  }, []);

  // Poll only when sending — recover from missed SSE completion events
  useEffect(() => {
    if (!sending) return;
    const interval = setInterval(() => {
      fetch(`/api/chat/status?thread=${encodeURIComponent(thread)}`, { headers: authHeaders() })
        .then((r) => r.json())
        .then((data: { running: boolean }) => {
          if (!data.running) {
            fetchMessages();
            setSending(false);
            if (sendingTimeoutRef.current) {
              clearTimeout(sendingTimeoutRef.current);
              sendingTimeoutRef.current = null;
            }
          }
        })
        .catch(() => {});
    }, 3000);
    return () => clearInterval(interval);
  }, [thread, sending, fetchMessages]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "instant" });
  }, []);

  // Close dropdown on outside click
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

    if (sendingTimeoutRef.current) clearTimeout(sendingTimeoutRef.current);
    sendingTimeoutRef.current = setTimeout(() => {
      setSending(false);
      sendingTimeoutRef.current = null;
    }, 60000);

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

  // Auto-resize textarea
  function handleInputChange(e: React.ChangeEvent<HTMLTextAreaElement>) {
    setInput(e.target.value);
    const el = e.target;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }

  return (
    <div className="flex h-full flex-col">
      {/* Header */}
      <div className="flex h-14 shrink-0 items-center justify-between border-b border-[#2a2520] px-5">
        <h2 className="text-[14px] font-semibold text-[#e8e0d4]">Chat</h2>
        <div className="relative" ref={dropdownRef}>
          <button
            onClick={() => setShowThreads(!showThreads)}
            className="flex items-center gap-1.5 rounded-lg border border-[#2a2520] bg-[#1c1a17] px-3 py-1.5 text-[12px] text-[#9c9486] transition-colors hover:border-[#2a2520]/80 hover:text-[#e8e0d4]"
          >
            {thread === "web:dashboard" ? (
              <Globe className="h-3 w-3 text-amber-400/60" />
            ) : (
              <FolderOpen className="h-3 w-3 text-amber-400/60" />
            )}
            <span>{threadLabel(thread, projects)}</span>
            <ChevronDown className="h-3 w-3" />
          </button>
          {showThreads && (
            <div className="absolute right-0 top-full z-50 mt-2 min-w-[220px] overflow-hidden rounded-xl border border-[#2a2520] bg-[#1c1a17] shadow-2xl">
              <div className="px-3 pt-2.5 pb-1.5 text-[10px] font-semibold uppercase tracking-wider text-[#6b6459]">
                Scope
              </div>
              <div className="p-1.5 pt-0">
                <button
                  onClick={() => {
                    setThread("web:dashboard");
                    setShowThreads(false);
                  }}
                  className={cn(
                    "flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-[13px] transition-colors hover:bg-[#232019]",
                    thread === "web:dashboard" ? "bg-[#232019] text-[#e8e0d4]" : "text-[#9c9486]",
                  )}
                >
                  <Globe className="h-3.5 w-3.5 shrink-0 text-amber-400/50" />
                  <div>
                    <div>Global</div>
                    <div className="text-[10px] text-[#6b6459]">Uses global knowledge base</div>
                  </div>
                </button>
              </div>
              {projects.length > 0 && (
                <>
                  <div className="px-3 pt-1 pb-1.5 text-[10px] font-semibold uppercase tracking-wider text-[#6b6459]">
                    Projects
                  </div>
                  <div className="p-1.5 pt-0 max-h-[240px] overflow-y-auto">
                    {projects.map((p) => {
                      const tid = `web:project-${p.id}`;
                      return (
                        <button
                          key={p.id}
                          onClick={() => {
                            setThread(tid);
                            setShowThreads(false);
                          }}
                          className={cn(
                            "flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-left text-[13px] transition-colors hover:bg-[#232019]",
                            thread === tid ? "bg-[#232019] text-[#e8e0d4]" : "text-[#9c9486]",
                          )}
                        >
                          <FolderOpen className="h-3.5 w-3.5 shrink-0 text-[#6b6459]" />
                          <span className="truncate">{p.name}</span>
                        </button>
                      );
                    })}
                  </div>
                </>
              )}
            </div>
          )}
        </div>
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl px-4 py-6">
          {messages.length === 0 && !sending && (
            <div className="flex flex-col items-center justify-center py-24 text-center">
              <div className="mb-5 flex h-14 w-14 items-center justify-center rounded-2xl bg-[#232019] ring-1 ring-[#2a2520]">
                {thread === "web:dashboard" ? (
                  <Globe className="h-6 w-6 text-amber-400/40" strokeWidth={1.5} />
                ) : (
                  <FolderOpen className="h-6 w-6 text-[#6b6459]" strokeWidth={1.5} />
                )}
              </div>
              <p className="text-[15px] font-medium text-[#9c9486]">
                {thread === "web:dashboard" ? "Global Chat" : threadLabel(thread, projects)}
              </p>
              <p className="mt-1.5 text-[13px] text-[#6b6459]">
                {thread === "web:dashboard"
                  ? "Chat with access to the global knowledge base"
                  : "Chat scoped to this workspace's documents"}
              </p>
            </div>
          )}

          <div className="space-y-5">
            {messages.map((msg, i) => (
              <MessageBubble key={`${msg.ts}-${msg.role}-${i}`} msg={msg} />
            ))}
          </div>

          {sending && (
            <div className="mt-5 flex gap-3">
              <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-gradient-to-br from-amber-500/20 to-orange-500/20 ring-1 ring-white/[0.06]">
                <span className="text-[11px] font-bold text-amber-300">B</span>
              </div>
              <div className="pt-1">
                <BorgingIndicator />
              </div>
            </div>
          )}
          <div ref={bottomRef} />
        </div>
      </div>

      {/* Input */}
      <div className="shrink-0 border-t border-[#2a2520] bg-[#0f0e0c]/80 backdrop-blur-sm">
        <div className="mx-auto max-w-3xl px-4 py-4">
          <div className="relative flex items-end gap-2 rounded-2xl border border-[#2a2520] bg-[#1c1a17] px-4 py-3 transition-colors focus-within:border-amber-500/20 focus-within:bg-[#232019]">
            <textarea
              ref={inputRef}
              value={input}
              onChange={handleInputChange}
              onKeyDown={handleKeyDown}
              placeholder="Message Borg..."
              rows={1}
              className="max-h-[200px] min-h-[24px] flex-1 resize-none bg-transparent text-[14px] leading-relaxed text-zinc-100 placeholder:text-zinc-600 focus:outline-none"
            />
            <div className="flex shrink-0 items-center gap-1">
              {dictation.supported && (
                <button
                  onClick={dictation.toggle}
                  title={dictation.listening ? "Stop dictation" : "Start dictation"}
                  className={cn(
                    "rounded-lg p-2 transition-colors",
                    dictation.listening
                      ? "bg-red-500/20 text-red-400 hover:bg-red-500/30"
                      : "text-[#6b6459] hover:text-[#9c9486] hover:bg-[#232019]",
                  )}
                >
                  {dictation.listening ? <MicOff className="h-4 w-4" /> : <Mic className="h-4 w-4" />}
                </button>
              )}
              <button
                onClick={handleSend}
                disabled={!input.trim() || sending}
                className={cn(
                  "rounded-lg p-2 transition-all",
                  input.trim() && !sending
                    ? "bg-amber-500 text-white hover:bg-amber-400 shadow-lg shadow-amber-500/25"
                    : "text-[#6b6459] cursor-not-allowed",
                )}
              >
                <Send className="h-4 w-4" />
              </button>
            </div>
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
      {/* Avatar */}
      <div
        className={cn(
          "flex h-8 w-8 shrink-0 items-center justify-center rounded-full ring-1 ring-white/[0.06]",
          isUser
            ? "bg-gradient-to-br from-amber-400/20 to-yellow-500/20"
            : "bg-gradient-to-br from-amber-500/20 to-orange-500/20",
        )}
      >
        <span className={cn("text-[11px] font-bold", isUser ? "text-amber-200" : "text-amber-300")}>
          {isUser ? "U" : "B"}
        </span>
      </div>

      {/* Content */}
      <div className={cn("min-w-0 max-w-[85%]", isUser && "flex flex-col items-end")}>
        {!isUser && <div className="mb-1 text-[11px] font-medium text-zinc-500">{msg.sender ?? "Borg"}</div>}
        <div
          className={cn(
            "rounded-2xl px-4 py-3.5 text-[14px] leading-relaxed shadow-[0_12px_28px_rgba(0,0,0,0.14)]",
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
