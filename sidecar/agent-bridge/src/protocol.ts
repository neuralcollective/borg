// Wire protocol types for Rust <-> Bridge communication over JSON-lines stdio.

// ── Rust -> Bridge requests ─────────────────────────────────────────────

export interface QueryRequest {
  type: "query";
  id: string;
  prompt: string;
  options: {
    cwd?: string;
    systemPrompt?: string;
    allowedTools?: string[];
    disallowedTools?: string[];
    mcpServers?: Record<
      string,
      { command: string; args: string[]; env?: Record<string, string> }
    >;
    model?: string;
    maxTurns?: number;
    maxBudgetUsd?: number;
    permissionMode?: "default" | "bypassPermissions";
    resume?: string;
    env?: Record<string, string>;
  };
}

// ── Bridge -> Rust events ───────────────────────────────────────────────

export interface TextDeltaEvent {
  type: "text_delta";
  id: string;
  content: string;
}

export interface ToolUseEvent {
  type: "tool_use";
  id: string;
  tool: string;
  input: Record<string, unknown>;
  timestamp: number;
}

export interface ToolResultEvent {
  type: "tool_result";
  id: string;
  tool: string;
  output: string;
  duration_ms: number;
  success: boolean;
}

export interface ResultEvent {
  type: "result";
  id: string;
  text: string;
  session_id: string;
  usage: { input_tokens: number; output_tokens: number };
  cost_usd: number;
}

export interface ErrorEvent {
  type: "error";
  id: string;
  message: string;
  code: "rate_limit" | "auth" | "context_overflow" | "unknown";
}

export interface StopEvent {
  type: "stop";
  id: string;
  reason: "end_turn" | "max_turns" | "budget";
  usage: { input_tokens: number; output_tokens: number };
}

export type BridgeEvent =
  | TextDeltaEvent
  | ToolUseEvent
  | ToolResultEvent
  | ResultEvent
  | ErrorEvent
  | StopEvent;
