import type { StreamEvent } from "./api";

export interface TermLine {
  type: "system" | "text" | "tool" | "result" | "tool_result" | "phase_result" | "container";
  tool?: string;
  label?: string;
  content: string;
  variant?: "info" | "success" | "error" | "warn";
}

const SEARCH_TOOL_NAMES = new Set(["WebSearch", "ToolSearch", "web_search"]);
const FETCH_TOOL_NAMES = new Set(["WebFetch", "web_fetch"]);

function truncate(value: string, max = 120): string {
  return value.length > max ? `${value.slice(0, max - 3)}...` : value;
}

function safeJson(value: unknown): string {
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function getString(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return "";
}

function collectSearchQueries(value: unknown): string[] {
  if (!value) return [];
  if (typeof value === "string") return value.trim() ? [value.trim()] : [];
  if (Array.isArray(value)) return value.flatMap((item) => collectSearchQueries(item));
  if (typeof value !== "object") return [];

  const obj = value as Record<string, unknown>;
  const directKeys = ["query", "q", "term", "text"];
  for (const key of directKeys) {
    const direct = getString(obj[key]).trim();
    if (direct) return [direct];
  }

  const nestedKeys = ["search_query", "queries", "requests", "items", "payload"];
  for (const key of nestedKeys) {
    if (obj[key] !== undefined) {
      const nested = collectSearchQueries(obj[key]);
      if (nested.length) return nested;
    }
  }

  return [];
}

function formatKeyValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (Array.isArray(value)) {
    const parts = value.map((item) => formatKeyValue(item)).filter(Boolean);
    return parts.join(", ");
  }
  if (value && typeof value === "object") {
    const obj = value as Record<string, unknown>;
    const query = collectSearchQueries(obj);
    if (query.length) return query.join(" | ");
    const scalarEntries = Object.entries(obj)
      .map(([key, item]) => {
        const rendered = getString(item);
        return rendered ? `${key}: ${rendered}` : "";
      })
      .filter(Boolean);
    if (scalarEntries.length) return scalarEntries.slice(0, 2).join("  ");
  }
  return "";
}

function summarizeObjectInput(input: Record<string, unknown>): { label: string; detail: string } {
  const priorityKeys = [
    "description",
    "recipient_name",
    "name",
    "title",
    "query",
    "q",
    "url",
    "command",
    "pattern",
    "path",
    "file_path",
    "prompt",
  ];

  for (const key of priorityKeys) {
    const value = input[key];
    const text = formatKeyValue(value).trim();
    if (text) {
      const remaining = Object.fromEntries(Object.entries(input).filter(([entryKey]) => entryKey !== key));
      const detail = Object.keys(remaining).length === 0 ? "" : truncate(safeJson(remaining), 200);
      return { label: truncate(text), detail };
    }
  }

  const entries = Object.entries(input)
    .map(([key, value]) => {
      const rendered = formatKeyValue(value).trim();
      return rendered ? `${key}: ${rendered}` : "";
    })
    .filter(Boolean);

  if (entries.length) {
    return {
      label: truncate(entries[0]),
      detail: entries.length > 1 ? truncate(entries.slice(1).join("  "), 200) : "",
    };
  }

  return { label: "", detail: truncate(safeJson(input), 200) };
}

export function formatToolInput(tool: string, input: unknown): { label: string; detail: string } {
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
    const suffix = obj.offset ? `  lines ${obj.offset}–${(obj.offset as number) + ((obj.limit as number) || 200)}` : "";
    return { label: fp + suffix, detail: "" };
  }
  if (tool === "Write") return { label: (obj.file_path as string) || "", detail: "" };
  if (tool === "Edit") {
    const fp = (obj.file_path as string) || "";
    const old = (obj.old_string as string) || "";
    const preview = old.length > 80 ? `${old.slice(0, 80)}...` : old;
    return { label: fp, detail: preview ? `replacing: ${preview}` : "" };
  }
  if (tool === "Glob" || tool === "Grep") {
    const pat = (obj.pattern as string) || "";
    const path = (obj.path as string) || "";
    return { label: pat, detail: path || "" };
  }
  if (SEARCH_TOOL_NAMES.has(tool)) {
    const queries = collectSearchQueries(obj);
    if (queries.length) return { label: truncate(queries.join(" | ")), detail: "" };
    return summarizeObjectInput(obj);
  }
  if (FETCH_TOOL_NAMES.has(tool)) return { label: (obj.url as string) || "", detail: "" };
  if (tool === "Task")
    return {
      label: (obj.description as string) || "",
      detail: ((obj.prompt as string) || "").slice(0, 120),
    };
  if (tool === "Agent")
    return {
      label: (obj.description as string) || "",
      detail: ((obj.prompt as string) || "").slice(0, 120),
    };

  if (tool.startsWith("mcp__")) {
    const query = (obj.query as string) || (obj.q as string) || "";
    const docId = (obj.document_id as string) || (obj.id as string) || "";
    const name = (obj.name as string) || "";
    if (query) return { label: query, detail: "" };
    if (docId) return { label: docId, detail: "" };
    if (name) return { label: name, detail: "" };
    return summarizeObjectInput(obj);
  }

  return summarizeObjectInput(obj);
}

export function parseStreamEvents(events: StreamEvent[]): TermLine[] {
  const lines: TermLine[] = [];

  for (const ev of events) {
    if (!ev.type) continue;

    if (ev.type === "system") {
      // Skip system events (session_id, task_progress, etc.) — not useful in chat timeline
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
        const preview = text.length > 300 ? `${text.slice(0, 300)}...` : text;
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
    } else if (ev.type === "container_event") {
      const line = formatContainerEvent(ev as unknown as Record<string, unknown>);
      if (line) lines.push(line);
    }
  }

  return lines;
}

/** Parse a raw NDJSON string (as stored in DB) into StreamEvent[] for parseStreamEvents(). */
export function rawStreamToEvents(raw: string): StreamEvent[] {
  if (!raw) return [];
  const events: StreamEvent[] = [];
  for (const line of raw.split("\n")) {
    if (!line.trim()) continue;
    try {
      const parsed = JSON.parse(line);
      if (parsed?.type) events.push(parsed);
    } catch {
      // skip non-JSON lines
    }
  }
  return events;
}

function formatContainerEvent(ev: Record<string, unknown>): TermLine | null {
  const event = ev.event as string | undefined;
  if (!event) return null;

  switch (event) {
    case "container_starting":
      return {
        type: "container",
        variant: "info",
        content: `Starting container: image=${ev.image ?? "?"} repo=${ev.repo ?? "?"} branch=${ev.branch ?? "?"}`,
      };
    case "agent_started":
      return {
        type: "container",
        variant: "info",
        content: `Agent started: model=${ev.model ?? "?"} repo=${ev.repo ?? "?"}`,
      };
    case "clone_started":
      return {
        type: "container",
        variant: "info",
        content: `Cloning ${ev.repo ?? "?"}${ev.branch ? ` (branch: ${ev.branch})` : ""}...`,
      };
    case "clone_complete":
      return {
        type: "container",
        variant: "success",
        content: `Clone complete${ev.duration_ms !== undefined ? ` (${ev.duration_ms}ms)` : ""}`,
      };
    case "setup_started":
      return { type: "container", variant: "info", content: "Running setup script..." };
    case "setup_complete":
      return { type: "container", variant: "success", content: "Setup complete" };
    case "agent_complete":
      return { type: "container", variant: "success", content: "Agent run complete" };
    case "agent_error":
      return {
        type: "container",
        variant: "error",
        content: `Agent error (exit ${ev.exit_code ?? "?"})${ev.stderr_tail ? `: ${String(ev.stderr_tail).slice(0, 200)}` : ""}`,
      };
    case "commit_complete":
      return {
        type: "container",
        variant: "success",
        content: `Committed: ${ev.message ?? ""}`,
      };
    case "commit_skipped":
      return { type: "container", variant: "info", content: "No changes to commit" };
    case "push_complete":
      return {
        type: "container",
        variant: "success",
        content: `Pushed branch: ${ev.branch ?? "?"}`,
      };
    case "push_failed":
      return {
        type: "container",
        variant: "error",
        content: `Push failed for branch: ${ev.branch ?? "?"}`,
      };
    case "container_id":
      return {
        type: "container",
        variant: "info",
        content: `Container ID: ${String(ev.id ?? "?").slice(0, 12)}`,
      };
    case "container_error":
      return {
        type: "container",
        variant: "error",
        content: `Container error (exit ${ev.exit_code ?? "?"})${ev.stderr_tail ? `: ${String(ev.stderr_tail).slice(0, 200)}` : ""}`,
      };
    case "container_exiting":
      return {
        type: "container",
        variant: ev.exit_code === 0 ? "success" : "warn",
        content: `Container exiting (exit ${ev.exit_code ?? "?"})`,
      };
    default:
      return { type: "container", variant: "info", content: `${event}` };
  }
}

export interface ParsedStreamEvent {
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
      } else if (type === "container_event") {
        const line = formatContainerEvent(obj as Record<string, unknown>);
        if (line) events.push({ type: "container_event", content: line.content, subtype: line.variant });
      }
    } catch {
      // skip unparseable lines
    }
  }
  return events;
}
