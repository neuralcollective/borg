import type { StreamEvent } from "./api";

export interface TermLine {
  type: "system" | "text" | "tool" | "result" | "tool_result" | "phase_result";
  tool?: string;
  label?: string;
  content: string;
}

export function formatToolInput(
  tool: string,
  input: unknown
): { label: string; detail: string } {
  if (typeof input === "string") return { label: "", detail: input };
  if (!input || typeof input !== "object") return { label: "", detail: "" };

  const obj = input as Record<string, unknown>;

  if (tool === "Bash") {
    return {
      label: (obj.description as string) || "",
      detail: (obj.command as string) || "",
    };
  }
  if (tool === "Read") {
    const fp = (obj.file_path as string) || "";
    const suffix = obj.offset
      ? `  lines ${obj.offset}–${(obj.offset as number) + ((obj.limit as number) || 200)}`
      : "";
    return { label: fp + suffix, detail: "" };
  }
  if (tool === "Write") return { label: (obj.file_path as string) || "", detail: "" };
  if (tool === "Edit") {
    const fp = (obj.file_path as string) || "";
    const old = (obj.old_string as string) || "";
    const preview = old.length > 80 ? old.slice(0, 80) + "..." : old;
    return { label: fp, detail: preview ? `replacing: ${preview}` : "" };
  }
  if (tool === "Glob" || tool === "Grep") {
    const pat = (obj.pattern as string) || "";
    const path = (obj.path as string) || "";
    return { label: pat, detail: path || "" };
  }
  if (tool === "WebSearch") return { label: (obj.query as string) || "", detail: "" };
  if (tool === "WebFetch") return { label: (obj.url as string) || "", detail: "" };
  if (tool === "Task")
    return {
      label: (obj.description as string) || "",
      detail: ((obj.prompt as string) || "").slice(0, 120),
    };

  const json = JSON.stringify(input);
  return { label: "", detail: json.length > 200 ? json.slice(0, 200) + "..." : json };
}

export function parseStreamEvents(events: StreamEvent[]): TermLine[] {
  const lines: TermLine[] = [];

  for (const ev of events) {
    if (!ev.type) continue;

    if (ev.type === "system") {
      if (ev.session_id) {
        lines.push({ type: "system", content: `session ${ev.session_id}` });
      }
    } else if (ev.type === "assistant") {
      const msg = ev.message;
      if (!msg?.content) continue;
      if (typeof msg.content === "string") {
        if (msg.content.trim()) lines.push({ type: "text", content: msg.content });
      } else if (Array.isArray(msg.content)) {
        for (const block of msg.content) {
          if (block.type === "text" && block.text?.trim()) {
            lines.push({ type: "text", content: block.text });
          } else if (block.type === "tool_use") {
            const name = block.name || "";
            const { label, detail } = formatToolInput(name, block.input);
            lines.push({ type: "tool", tool: name, label, content: detail });
          }
        }
      }
    } else if (ev.type === "tool_result" || ev.type === "tool") {
      const raw = ev.content ?? ev.output ?? "";
      const text =
        typeof raw === "string"
          ? raw
          : Array.isArray(raw)
            ? raw.map((c: { text?: string }) => c.text || "").join("\n")
            : JSON.stringify(raw);
      if (text.trim()) {
        const preview = text.length > 300 ? text.slice(0, 300) + "..." : text;
        lines.push({
          type: "tool_result",
          tool: ev.tool_name || ev.name || "",
          content: preview,
        });
      }
    } else if (ev.type === "result") {
      if (ev.result) {
        lines.push({ type: "result", content: ev.result });
      }
    } else if (ev.type === "phase_result") {
      const content = typeof ev.content === "string" ? ev.content : "";
      if (content.trim()) {
        lines.push({ type: "phase_result", label: ev.phase || "", content });
      }
    }
  }

  return lines;
}

interface ParsedStreamEvent {
  type: string;
  subtype?: string;
  tool?: string;
  label?: string;
  input?: string;
  output?: string;
  content?: string;
}

export function parseRawStream(raw: string): ParsedStreamEvent[] {
  if (!raw) return [];
  const events: ParsedStreamEvent[] = [];
  for (const line of raw.split("\n")) {
    if (!line.trim()) continue;
    try {
      const obj = JSON.parse(line);
      const type = obj.type;
      if (!type) continue;

      if (type === "assistant") {
        const msg = obj.message;
        if (msg?.content) {
          if (typeof msg.content === "string") {
            events.push({ type: "assistant", content: msg.content });
          } else if (Array.isArray(msg.content)) {
            for (const block of msg.content) {
              if (block.type === "text" && block.text) {
                events.push({ type: "assistant", content: block.text });
              } else if (block.type === "tool_use") {
                const { label, detail } = formatToolInput(block.name, block.input);
                events.push({ type: "tool_call", tool: block.name, label, input: detail });
              }
            }
          }
        }
      } else if (type === "tool_result" || type === "tool") {
        const content = obj.content ?? obj.result ?? obj.output ?? "";
        const text =
          typeof content === "string"
            ? content
            : Array.isArray(content)
              ? content.map((c: { text?: string }) => c.text || "").join("\n")
              : JSON.stringify(content);
        if (text) {
          events.push({ type: "tool_result", tool: obj.tool_name || obj.name || "", output: text });
        }
      } else if (type === "result") {
        events.push({ type: "result", content: obj.result || "" });
      } else if (type === "system") {
        if (obj.subtype === "init") {
          events.push({ type: "system", subtype: "init", content: `Session: ${obj.session_id || "?"}` });
        }
      }
    } catch {
      // skip unparseable lines
    }
  }
  return events;
}
