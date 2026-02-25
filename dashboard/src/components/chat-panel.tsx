import { useState, useRef, useEffect, useCallback } from "react";
import { cn } from "@/lib/utils";

interface ChatMessage {
  role: "user" | "assistant";
  sender?: string;
  text: string;
  ts: string | number;
}

export function ChatPanel() {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const bottomRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const esRef = useRef<EventSource | null>(null);

  useEffect(() => {
    fetch("/api/chat/messages")
      .then((r) => r.json())
      .then((msgs: ChatMessage[]) => setMessages(msgs))
      .catch(() => {});
  }, []);

  const connect = useCallback(() => {
    if (esRef.current) esRef.current.close();
    const es = new EventSource("/api/chat/events");
    esRef.current = es;

    es.onmessage = (e) => {
      try {
        const msg: ChatMessage = JSON.parse(e.data);
        setMessages((prev) => [...prev, msg]);
        if (msg.role === "assistant") setSending(false);
      } catch {
        // ignore
      }
    };

    es.onerror = () => {
      setTimeout(() => connect(), 3000);
    };
  }, []);

  useEffect(() => {
    connect();
    return () => esRef.current?.close();
  }, [connect]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "instant" });
  }, [messages.length]);

  async function handleSend() {
    const text = input.trim();
    if (!text || sending) return;

    setInput("");
    setSending(true);

    try {
      await fetch("/api/chat", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ text, sender: "web-user" }),
      });
    } catch {
      setSending(false);
    }
  }

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-10 shrink-0 items-center border-b border-white/[0.06] px-4">
        <span className="text-[12px] md:text-[11px] font-medium text-zinc-400">Chat</span>
        {sending && (
          <span className="ml-2 text-[11px] md:text-[10px] text-zinc-600 animate-pulse">
            thinking...
          </span>
        )}
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
