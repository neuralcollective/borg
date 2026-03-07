import { useState, useRef, useEffect, useCallback } from "react";
import { Clock, ChevronDown, ChevronUp, Mic, MicOff, Send } from "lucide-react";
import { useTaskMessages, useSendTaskMessage } from "@/lib/api";
import type { TaskMessage } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useDictation } from "@/lib/dictation";
import { BorgingIndicator } from "./borging";
import { ChatMarkdown } from "./chat-markdown";

interface TaskChatProps {
  taskId: number;
}

export function TaskChat({ taskId }: TaskChatProps) {
  const [open, setOpen] = useState(false);
  const [input, setInput] = useState("");
  const bottomRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const { data: messages = [] } = useTaskMessages(taskId);
  const { mutateAsync: sendMessage, isPending: sending } = useSendTaskMessage(taskId);

  useEffect(() => {
    if (open) {
      bottomRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [messages.length, open]);

  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  const handleSend = useCallback(async () => {
    const text = input.trim();
    if (!text || sending) return;
    setInput("");
    try {
      await sendMessage(text);
    } catch {
      // error surfaced via mutation state
    }
  }, [input, sending, sendMessage]);

  const dictation = useDictation(input, setInput);

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      handleSend();
    }
  }

  const pendingCount = messages.filter((m) => m.role === "user" && !m.delivered_phase).length;

  return (
    <div className="border-t border-white/[0.07]">
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2.5 px-6 py-3 text-left transition-colors hover:bg-white/[0.02]"
      >
        <span className="text-[12px] font-medium text-zinc-400">Chat</span>
        {pendingCount > 0 && (
          <span className="flex items-center gap-1 rounded-full bg-blue-500/15 px-2 py-0.5 text-[11px] font-medium text-blue-400">
            <Clock className="h-2.5 w-2.5" />
            {pendingCount} pending
          </span>
        )}
        {!open && messages.length > 0 && pendingCount === 0 && (
          <span className="text-[11px] text-zinc-600">{messages.length} message{messages.length !== 1 ? "s" : ""}</span>
        )}
        <span className="ml-auto text-zinc-600">
          {open ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
        </span>
      </button>

      {open && (
        <div className="flex flex-col" style={{ maxHeight: "360px" }}>
          <div className="flex-1 overflow-y-auto overscroll-contain px-5 py-3 space-y-3" style={{ minHeight: 0 }}>
            {messages.length === 0 ? (
              <div className="py-8 text-center text-[12px] text-zinc-600">
                Messages you send here will be seen by the next agent running this task
              </div>
            ) : (
              messages.map((msg) => <MessageBubble key={msg.id} msg={msg} />)
            )}
            {sending && <BorgingIndicator />}
            <div ref={bottomRef} />
          </div>

          <div className="shrink-0 border-t border-white/[0.07] px-5 py-3">
            <div className="flex items-end gap-2 rounded-xl border border-white/[0.08] bg-white/[0.03] px-3 py-2 transition-colors focus-within:border-white/[0.14]">
              <textarea
                ref={inputRef}
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyDown={handleKeyDown}
                placeholder="Message the agent… (Ctrl+Enter)"
                rows={1}
                className="flex-1 resize-none bg-transparent text-[13px] leading-relaxed text-zinc-200 placeholder:text-zinc-600 focus:outline-none"
              />
              <div className="flex items-center gap-1">
                {dictation.supported && (
                  <button
                    onClick={dictation.toggle}
                    title={dictation.listening ? "Stop dictation" : "Start dictation"}
                    className={cn(
                      "shrink-0 rounded-lg p-1.5 transition-colors",
                      dictation.listening
                        ? "bg-red-500/20 text-red-400 hover:bg-red-500/30"
                        : "text-zinc-600 hover:text-zinc-400"
                    )}
                  >
                    {dictation.listening ? <MicOff className="h-3.5 w-3.5" /> : <Mic className="h-3.5 w-3.5" />}
                  </button>
                )}
                <button
                  onClick={handleSend}
                  disabled={!input.trim() || sending}
                  className={cn(
                    "shrink-0 rounded-lg p-1.5 transition-all",
                    input.trim() && !sending
                      ? "bg-blue-500 text-white hover:bg-blue-400"
                      : "cursor-not-allowed text-zinc-700"
                  )}
                >
                  <Send className="h-3.5 w-3.5" />
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function MessageBubble({ msg }: { msg: TaskMessage }) {
  const isUser = msg.role === "user";
  const isPending = isUser && !msg.delivered_phase;

  return (
    <div className={cn("flex gap-2.5", isUser ? "flex-row-reverse" : "flex-row")}>
      <div
        className={cn(
          "flex h-7 w-7 shrink-0 items-center justify-center rounded-full ring-1 ring-white/[0.06]",
          isUser
            ? "bg-gradient-to-br from-blue-500/20 to-cyan-500/20"
            : msg.role === "system"
              ? "bg-white/[0.04]"
              : "bg-gradient-to-br from-violet-500/20 to-fuchsia-500/20"
        )}
      >
        <span className={cn("text-[10px] font-bold", isUser ? "text-blue-300" : msg.role === "system" ? "text-zinc-500" : "text-violet-300")}>
          {isUser ? "U" : msg.role === "system" ? "S" : "B"}
        </span>
      </div>
      <div
        className={cn(
          "max-w-[80%] rounded-2xl px-3.5 py-2.5 text-[13px] leading-relaxed",
          isUser
            ? "bg-blue-500/15 text-zinc-100 rounded-br-md"
            : msg.role === "system"
              ? "bg-white/[0.03] text-zinc-500 ring-1 ring-inset ring-white/[0.06] rounded-bl-md"
              : "bg-white/[0.04] text-zinc-200 rounded-bl-md"
        )}
      >
        {!isUser && (
          <div className={cn(
            "mb-0.5 text-[10px] font-medium",
            msg.role === "system" ? "text-zinc-600" : "text-violet-400/80"
          )}>
            {msg.role}
          </div>
        )}
        {isUser ? (
          <div className="whitespace-pre-wrap break-words">{msg.content}</div>
        ) : (
          <ChatMarkdown text={msg.content} />
        )}
        {isPending && (
          <div className="mt-1.5 flex items-center gap-1 text-[10px] text-zinc-600">
            <Clock className="h-2.5 w-2.5" />
            pending delivery
          </div>
        )}
        {isUser && msg.delivered_phase && (
          <div className="mt-1.5 text-[10px] text-zinc-600">
            delivered at {msg.delivered_phase}
          </div>
        )}
      </div>
    </div>
  );
}
