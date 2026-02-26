import { useState, useRef, useEffect, useCallback } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Clock, ChevronDown, ChevronUp } from "lucide-react";
import { useTaskMessages, useSendTaskMessage } from "@/lib/api";
import type { TaskMessage } from "@/lib/types";
import { cn } from "@/lib/utils";

interface TaskChatProps {
  taskId: number;
}

export function TaskChat({ taskId }: TaskChatProps) {
  const [open, setOpen] = useState(false);
  const [input, setInput] = useState("");
  const bottomRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const queryClient = useQueryClient();

  const { data: messages = [] } = useTaskMessages(taskId);
  const { mutateAsync: sendMessage, isPending: sending } = useSendTaskMessage(taskId);

  // SSE: listen for task_message events on the existing /api/logs stream
  useEffect(() => {
    if (!open) return;
    const es = new EventSource("/api/logs");

    es.onmessage = (e) => {
      try {
        const d = JSON.parse(e.data);
        if (d.type === "task_message" && d.task_id === taskId) {
          queryClient.invalidateQueries({ queryKey: ["task_messages", taskId] });
        }
      } catch {
        // ignore
      }
    };

    return () => es.close();
  }, [open, taskId, queryClient]);

  // Auto-scroll when messages arrive
  useEffect(() => {
    if (open) {
      bottomRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [messages.length, open]);

  // Focus input when panel opens
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
      // error surfaced via mutation state if needed
    }
  }, [input, sending, sendMessage]);

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      handleSend();
    }
  }

  const pendingCount = messages.filter((m) => m.role === "user" && !m.delivered_phase).length;

  return (
    <div className="border-t border-white/[0.06]">
      {/* Collapsible header */}
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-5 py-2.5 text-left hover:bg-white/[0.02] transition-colors"
      >
        <span className="text-[11px] font-medium text-zinc-400">Chat</span>
        {pendingCount > 0 && (
          <span className="flex items-center gap-1 rounded-full bg-blue-500/15 px-1.5 py-0.5 text-[10px] text-blue-400">
            <Clock className="h-2.5 w-2.5" />
            {pendingCount} pending
          </span>
        )}
        {!open && messages.length > 0 && pendingCount === 0 && (
          <span className="text-[10px] text-zinc-600">{messages.length} message{messages.length !== 1 ? "s" : ""}</span>
        )}
        <span className="ml-auto text-zinc-600">
          {open ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
        </span>
      </button>

      {open && (
        <div className="flex flex-col" style={{ maxHeight: "320px" }}>
          {/* Message list */}
          <div className="flex-1 overflow-y-auto overscroll-contain px-4 py-2 space-y-2" style={{ minHeight: 0 }}>
            {messages.length === 0 ? (
              <div className="py-6 text-center text-[11px] text-zinc-600">
                Messages you send here will be seen by the next agent running this task
              </div>
            ) : (
              messages.map((msg) => <MessageBubble key={msg.id} msg={msg} />)
            )}
            <div ref={bottomRef} />
          </div>

          {/* Input */}
          <div className="shrink-0 border-t border-white/[0.06] px-4 py-2.5">
            <div className="flex gap-2">
              <textarea
                ref={inputRef}
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyDown={handleKeyDown}
                placeholder="Send a message to the agent… (Ctrl+Enter)"
                rows={1}
                className={cn(
                  "flex-1 resize-none rounded-lg border border-white/[0.08] bg-white/[0.03] px-3 py-2",
                  "text-[12px] text-zinc-200 placeholder:text-zinc-600",
                  "focus:border-white/[0.15] focus:outline-none"
                )}
              />
              <button
                onClick={handleSend}
                disabled={!input.trim() || sending}
                className={cn(
                  "shrink-0 rounded-lg px-3 py-2 text-[11px] font-medium transition-colors",
                  input.trim() && !sending
                    ? "bg-blue-500/20 text-blue-300 hover:bg-blue-500/25 active:bg-blue-500/30"
                    : "cursor-not-allowed text-zinc-700"
                )}
              >
                {sending ? "…" : "Send"}
              </button>
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
    <div className={cn("flex", isUser ? "justify-end" : "justify-start")}>
      <div
        className={cn(
          "max-w-[85%] rounded-lg px-3 py-2 text-[12px] leading-relaxed",
          isUser
            ? "bg-blue-500/[0.15] text-zinc-200"
            : msg.role === "system"
              ? "bg-white/[0.03] text-zinc-500 ring-1 ring-inset ring-white/[0.06]"
              : "bg-white/[0.05] text-zinc-300"
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
        <div className="whitespace-pre-wrap break-words">{msg.content}</div>
        {isPending && (
          <div className="mt-1 flex items-center gap-1 text-[10px] text-zinc-600">
            <Clock className="h-2.5 w-2.5" />
            pending delivery
          </div>
        )}
        {isUser && msg.delivered_phase && (
          <div className="mt-1 text-[10px] text-zinc-600">
            delivered at {msg.delivered_phase}
          </div>
        )}
      </div>
    </div>
  );
}
