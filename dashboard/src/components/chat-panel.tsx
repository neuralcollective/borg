import { useState, useRef, useEffect, useCallback } from "react";
import { Mic, MicOff } from "lucide-react";
import { cn } from "@/lib/utils";
import { useDictation } from "@/lib/dictation";

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
  const esRef = useRef<EventSource | null>(null);
  const lastTsRef = useRef<number>(0);

  const fetchMessages = useCallback(() => {
    fetch(`/api/chat/messages?thread=${encodeURIComponent(thread)}`)
      .then((r) => r.json())
      .then((msgs: ChatMessage[]) => {
        setMessages(msgs);
        if (msgs.length > 0) {
          lastTsRef.current = Math.max(...msgs.map((m) => Number(m.ts) || 0));
        }
      })
      .catch(() => {});
  }, [thread]);

  const fetchThreads = useCallback(() => {
    fetch("/api/chat/threads")
      .then((r) => r.json())
      .then((t: ChatThread[]) => setThreads(t))
      .catch(() => {});
  }, []);

  // Load messages when thread changes
  useEffect(() => {
    setMessages([]);
    lastTsRef.current = 0;
    fetchMessages();
    fetchThreads();
  }, [thread, fetchMessages, fetchThreads]);

  // SSE for real-time updates
  const connect = useCallback(() => {
    if (esRef.current) esRef.current.close();
    const es = new EventSource("/api/chat/events");
    esRef.current = es;

    es.onmessage = (e) => {
      try {
        const msg: ChatMessage = JSON.parse(e.data);
        // Only show messages for the current thread
        const msgThread = msg.thread || "web:dashboard";
        if (msgThread !== thread) return;
        // Skip user echoes
        if (msg.role === "user") return;
        setMessages((prev) => [...prev, msg]);
        lastTsRef.current = Math.max(lastTsRef.current, Number(msg.ts) || 0);
        if (msg.role === "assistant") setSending(false);
      } catch {
        // ignore
      }
    };

    es.onerror = () => {
      setTimeout(() => connect(), 3000);
    };
  }, [thread]);

  useEffect(() => {
    connect();
    return () => esRef.current?.close();
  }, [connect]);

  // Poll fallback
  useEffect(() => {
    const interval = setInterval(() => {
      fetch(`/api/chat/messages?thread=${encodeURIComponent(thread)}`)
        .then((r) => r.json())
        .then((msgs: ChatMessage[]) => {
          if (msgs.length === 0) return;
          const newTs = Math.max(...msgs.map((m) => Number(m.ts) || 0));
          if (newTs > lastTsRef.current) {
            setMessages(msgs);
            lastTsRef.current = newTs;
            if (msgs[msgs.length - 1]?.role === "assistant") setSending(false);
          }
        })
        .catch(() => {});
    }, 3000);
    return () => clearInterval(interval);
  }, [thread]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "instant" });
  }, [messages.length]);

  async function handleSend() {
    const text = input.trim();
    if (!text || sending) return;

    setInput("");
    setSending(true);

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
      await fetch("/api/chat", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ text, sender: "web-user", thread }),
      });
    } catch {
      setSending(false);
    }
  }

  function handleNewThread() {
    const id = `web:thread-${Date.now()}`;
    setThread(id);
    setShowThreads(false);
  }

  const dictation = useDictation((transcript) => {
    setInput((prev) => (prev ? prev + " " + transcript : transcript));
  });

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-10 shrink-0 items-center justify-between border-b border-white/[0.06] px-4">
        <div className="flex items-center gap-2">
          <span className="text-[12px] md:text-[11px] font-medium text-zinc-400">Chat</span>
          {sending && (
            <span className="text-[11px] md:text-[10px] text-zinc-600 animate-pulse">
              thinking...
            </span>
          )}
        </div>
        <div className="flex items-center gap-1.5">
          <div className="relative">
            <button
              onClick={() => setShowThreads(!showThreads)}
              className="rounded-md bg-white/[0.04] px-2 py-0.5 text-[10px] font-medium text-zinc-500 ring-1 ring-inset ring-white/[0.06] transition-colors hover:bg-white/[0.08]"
            >
              {threadLabel(thread)}
            </button>
            {showThreads && (
              <div className="absolute right-0 top-full z-50 mt-1 min-w-[160px] rounded-lg border border-white/[0.08] bg-zinc-900 py-1 shadow-xl">
                {threads.map((t) => (
                  <button
                    key={t.id}
                    onClick={() => { setThread(t.id); setShowThreads(false); }}
                    className={cn(
                      "flex w-full items-center justify-between px-3 py-1.5 text-left text-[11px] transition-colors hover:bg-white/[0.06]",
                      t.id === thread ? "text-zinc-200" : "text-zinc-500"
                    )}
                  >
                    <span className="truncate">{threadLabel(t.id)}</span>
                    <span className="ml-2 text-[9px] tabular-nums text-zinc-600">{t.message_count}</span>
                  </button>
                ))}
                {threads.length > 0 && <div className="mx-2 my-1 h-px bg-white/[0.06]" />}
                <button
                  onClick={handleNewThread}
                  className="flex w-full items-center px-3 py-1.5 text-[11px] text-violet-400 transition-colors hover:bg-white/[0.06]"
                >
                  + New Thread
                </button>
              </div>
            )}
          </div>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto overscroll-contain p-3 space-y-2">
        {messages.map((msg, i) => (
          <MessageBubble key={i} msg={msg} />
        ))}
        <div ref={bottomRef} />
      </div>

      <div className="shrink-0 border-t border-white/[0.06] p-3">
        <div className="flex gap-2">
          <textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Message the director..."
            rows={1}
            className={cn(
              "flex-1 resize-none rounded-lg border border-white/[0.08] bg-white/[0.03] px-3 py-2.5 md:py-2",
              "text-[14px] md:text-[12px] text-zinc-200 placeholder:text-zinc-600",
              "focus:border-white/[0.15] focus:outline-none"
            )}
          />
          {dictation.supported && (
            <button
              onClick={dictation.toggle}
              title={dictation.listening ? "Stop dictation" : "Start dictation"}
              className={cn(
                "shrink-0 rounded-lg px-2.5 py-2.5 md:py-2 transition-colors",
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
            disabled={!input.trim() || sending}
            className={cn(
              "rounded-lg px-4 md:px-3 py-2.5 md:py-2 text-[13px] md:text-[11px] font-medium transition-colors",
              input.trim() && !sending
                ? "bg-blue-500/20 text-blue-300 active:bg-blue-500/30 hover:bg-blue-500/25"
                : "text-zinc-700 cursor-not-allowed"
            )}
          >
            Send
          </button>
        </div>
      </div>
    </div>
  );
}

function MessageBubble({ msg }: { msg: ChatMessage }) {
  const isUser = msg.role === "user";
  return (
    <div className={cn("flex", isUser ? "justify-end" : "justify-start")}>
      <div
        className={cn(
          "max-w-[85%] rounded-lg px-3 py-2 text-[13px] md:text-[12px] leading-relaxed",
          isUser
            ? "bg-blue-500/[0.15] text-zinc-200"
            : "bg-white/[0.05] text-zinc-300"
        )}
      >
        {!isUser && (
          <div className="mb-0.5 text-[10px] font-medium text-zinc-500">
            {msg.sender ?? "borg"}
          </div>
        )}
        <div className="whitespace-pre-wrap break-words">{msg.text}</div>
      </div>
    </div>
  );
}
