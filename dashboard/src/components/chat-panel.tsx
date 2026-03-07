import { useState, useRef, useEffect, useCallback } from "react";
import { MessageSquare, Mic, MicOff, Send, ChevronDown, Plus } from "lucide-react";
import { cn } from "@/lib/utils";
import { useDictation } from "@/lib/dictation";
import { BorgingIndicator } from "./borging";
import { ChatMarkdown } from "./chat-markdown";
import { authHeaders, tokenReady } from "@/lib/api";
import { useChatEvents } from "@/lib/use-chat-events";

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

function threadLabel(id: string): string {
  if (id === "web:dashboard") return "Main";
  return id.replace("web:", "");
}

export function ChatPanel() {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [thread, setThread] = useState("web:dashboard");
  const [threads, setThreads] = useState<ChatThread[]>([]);
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
      fetch(`/api/chat/messages?thread=${encodeURIComponent(thread)}`, { signal: controller.signal, headers: authHeaders() })
        .then((r) => { if (!r.ok) throw new Error(`${r.status}`); return r.json(); })
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
    lastTsRef.current = 0;
    fetchMessages();
    fetchThreads();
  }, [thread, fetchMessages, fetchThreads]);

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

  // Poll fallback
  useEffect(() => {
    let pollAbort: AbortController | null = null;
    const interval = setInterval(() => {
      pollAbort?.abort();
      const ctrl = new AbortController();
      pollAbort = ctrl;
      fetch(`/api/chat/messages?thread=${encodeURIComponent(thread)}`, { signal: ctrl.signal, headers: authHeaders() })
        .then((r) => r.json())
        .then((msgs: ChatMessage[]) => {
          if (ctrl.signal.aborted || msgs.length === 0) return;
          const newTs = Math.max(...msgs.map((m) => Number(m.ts) || 0));
          if (newTs > lastTsRef.current) {
            setMessages(msgs);
            lastTsRef.current = newTs;
            if (msgs[msgs.length - 1]?.role === "assistant") {
              setSending(false);
              if (sendingTimeoutRef.current) {
                clearTimeout(sendingTimeoutRef.current);
                sendingTimeoutRef.current = null;
              }
            }
          }
        })
        .catch(() => {});
    }, 3000);
    return () => {
      clearInterval(interval);
      pollAbort?.abort();
    };
  }, [thread]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "instant" });
  }, [messages.length]);

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

  // Auto-resize textarea
  function handleInputChange(e: React.ChangeEvent<HTMLTextAreaElement>) {
    setInput(e.target.value);
    const el = e.target;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, 200) + "px";
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
            <span>{threadLabel(thread)}</span>
            <ChevronDown className="h-3 w-3" />
          </button>
          {showThreads && (
            <div className="absolute right-0 top-full z-50 mt-2 min-w-[200px] overflow-hidden rounded-xl border border-[#2a2520] bg-[#1c1a17] shadow-2xl">
              <div className="p-1.5">
                {threads.map((t) => (
                  <button
                    key={t.id}
                    onClick={() => { setThread(t.id); setShowThreads(false); }}
                    className={cn(
                      "flex w-full items-center justify-between rounded-lg px-3 py-2 text-left text-[13px] transition-colors hover:bg-[#232019]",
                      t.id === thread ? "bg-[#232019] text-[#e8e0d4]" : "text-[#9c9486]"
                    )}
                  >
                    <span className="truncate">{threadLabel(t.id)}</span>
                    <span className="ml-3 text-[11px] tabular-nums text-zinc-600">{t.message_count}</span>
                  </button>
                ))}
              </div>
              <div className="border-t border-[#2a2520] p-1.5">
                <button
                  onClick={handleNewThread}
                  className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-[13px] text-amber-400 transition-colors hover:bg-amber-500/10"
                >
                  <Plus className="h-3.5 w-3.5" />
                  New Thread
                </button>
              </div>
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
                <MessageSquare className="h-6 w-6 text-[#6b6459]" strokeWidth={1.5} />
              </div>
              <p className="text-[15px] font-medium text-[#9c9486]">Start a conversation with Borg</p>
              <p className="mt-1.5 text-[13px] text-[#6b6459]">Messages are processed by the active agent</p>
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
                      : "text-[#6b6459] hover:text-[#9c9486] hover:bg-[#232019]"
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
                    : "text-[#6b6459] cursor-not-allowed"
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
            : "bg-gradient-to-br from-amber-500/20 to-orange-500/20"
        )}
      >
        <span className={cn("text-[11px] font-bold", isUser ? "text-amber-200" : "text-amber-300")}>
          {isUser ? "U" : "B"}
        </span>
      </div>

      {/* Content */}
      <div className={cn("min-w-0 max-w-[85%]", isUser && "flex flex-col items-end")}>
        {!isUser && (
          <div className="mb-1 text-[11px] font-medium text-zinc-500">
            {msg.sender ?? "Borg"}
          </div>
        )}
        <div
          className={cn(
            "rounded-2xl px-4 py-3 text-[14px] leading-relaxed",
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
