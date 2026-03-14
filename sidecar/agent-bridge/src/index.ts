#!/usr/bin/env bun
// Agent bridge: long-lived bun process that wraps @anthropic-ai/claude-agent-sdk
// and communicates with the Rust borg server via JSON lines over stdio.

import { query } from "@anthropic-ai/claude-agent-sdk";
import type {
  QueryRequest,
  BridgeEvent,
  TextDeltaEvent,
  ToolUseEvent,
  ToolResultEvent,
  ResultEvent,
  ErrorEvent,
  StopEvent,
} from "./protocol.js";

// ── Health check ────────────────────────────────────────────────────────

if (process.argv.includes("--health")) {
  console.log(JSON.stringify({ status: "ok" }));
  process.exit(0);
}

// ── Emit a bridge event as a JSON line to stdout ────────────────────────

function emit(event: BridgeEvent): void {
  process.stdout.write(JSON.stringify(event) + "\n");
}

// ── Classify SDK errors into protocol error codes ───────────────────────

function classifyError(err: unknown): ErrorEvent["code"] {
  const msg = err instanceof Error ? err.message : String(err);
  const lower = msg.toLowerCase();
  if (lower.includes("rate_limit") || lower.includes("rate limit") || lower.includes("429"))
    return "rate_limit";
  if (
    lower.includes("authentication") ||
    lower.includes("auth") ||
    lower.includes("401") ||
    lower.includes("api key")
  )
    return "auth";
  if (lower.includes("context") || lower.includes("too long") || lower.includes("token"))
    return "context_overflow";
  return "unknown";
}

// ── Map SDK SDKAssistantMessageError to our error codes ─────────────────

function mapSdkError(
  sdkError: string | undefined,
): ErrorEvent["code"] | null {
  if (!sdkError) return null;
  switch (sdkError) {
    case "rate_limit":
      return "rate_limit";
    case "authentication_failed":
    case "billing_error":
      return "auth";
    case "invalid_request":
    case "server_error":
    case "unknown":
    default:
      return "unknown";
  }
}

// ── Process a single query request ──────────────────────────────────────

async function handleQuery(req: QueryRequest): Promise<void> {
  const { id, prompt, options } = req;

  // Inject env vars from request (for auth passthrough: ANTHROPIC_API_KEY, etc.)
  if (options.env) {
    for (const [k, v] of Object.entries(options.env)) {
      process.env[k] = v;
    }
  }

  // Build MCP server configs for the SDK
  const mcpServers: Record<string, { type?: "stdio"; command: string; args?: string[]; env?: Record<string, string> }> =
    {};
  if (options.mcpServers) {
    for (const [name, cfg] of Object.entries(options.mcpServers)) {
      mcpServers[name] = {
        type: "stdio",
        command: cfg.command,
        args: cfg.args,
        env: cfg.env,
      };
    }
  }

  // Track tool call start times for duration_ms
  const toolStartTimes = new Map<string, number>();

  // Build hooks to capture tool use events
  const hooks: Record<string, Array<{ hooks: Array<(input: any, toolUseID: string | undefined, opts: { signal: AbortSignal }) => Promise<any>> }>> = {
    PreToolUse: [
      {
        hooks: [
          async (input: any, _toolUseID: string | undefined) => {
            const toolUseId = input.tool_use_id as string;
            toolStartTimes.set(toolUseId, Date.now());
            const event: ToolUseEvent = {
              type: "tool_use",
              id,
              tool: input.tool_name as string,
              input: (input.tool_input as Record<string, unknown>) ?? {},
              timestamp: Date.now(),
            };
            emit(event);
            return { continue: true };
          },
        ],
      },
    ],
    PostToolUse: [
      {
        hooks: [
          async (input: any, _toolUseID: string | undefined) => {
            const toolUseId = input.tool_use_id as string;
            const startTime = toolStartTimes.get(toolUseId) ?? Date.now();
            toolStartTimes.delete(toolUseId);

            const response = input.tool_response;
            let output: string;
            if (typeof response === "string") {
              output = response;
            } else {
              try {
                output = JSON.stringify(response);
              } catch {
                output = String(response);
              }
            }

            // Truncate large tool outputs to keep the pipe manageable
            if (output.length > 50_000) {
              output = output.slice(0, 50_000) + "... [truncated]";
            }

            const event: ToolResultEvent = {
              type: "tool_result",
              id,
              tool: input.tool_name as string,
              output,
              duration_ms: Date.now() - startTime,
              success: true,
            };
            emit(event);
            return { continue: true };
          },
        ],
      },
    ],
    PostToolUseFailure: [
      {
        hooks: [
          async (input: any, _toolUseID: string | undefined) => {
            const toolUseId = input.tool_use_id as string;
            const startTime = toolStartTimes.get(toolUseId) ?? Date.now();
            toolStartTimes.delete(toolUseId);

            const event: ToolResultEvent = {
              type: "tool_result",
              id,
              tool: input.tool_name as string,
              output: (input.error as string) ?? "unknown error",
              duration_ms: Date.now() - startTime,
              success: false,
            };
            emit(event);
            return { continue: true };
          },
        ],
      },
    ],
  };

  try {
    const q = query({
      prompt,
      options: {
        cwd: options.cwd ?? process.cwd(),
        systemPrompt: options.systemPrompt ?? undefined,
        allowedTools: options.allowedTools,
        disallowedTools: options.disallowedTools,
        mcpServers: Object.keys(mcpServers).length > 0 ? mcpServers : undefined,
        model: options.model,
        maxTurns: options.maxTurns,
        maxBudgetUsd: options.maxBudgetUsd,
        permissionMode: options.permissionMode === "bypassPermissions"
          ? "bypassPermissions"
          : "default",
        allowDangerouslySkipPermissions: options.permissionMode === "bypassPermissions",
        resume: options.resume,
        includePartialMessages: true,
        hooks,
      },
    });

    let finalText = "";
    let sessionId = "";
    let totalInputTokens = 0;
    let totalOutputTokens = 0;
    let costUsd = 0;
    let stopReason: StopEvent["reason"] = "end_turn";

    for await (const message of q) {
      switch (message.type) {
        case "system": {
          if (message.subtype === "init") {
            sessionId = message.session_id;
          }
          break;
        }

        case "assistant": {
          // Extract text content from the assistant message's content blocks
          const betaMessage = message.message;
          if (betaMessage && betaMessage.content) {
            for (const block of betaMessage.content) {
              if (block.type === "text") {
                const textEvent: TextDeltaEvent = {
                  type: "text_delta",
                  id,
                  content: block.text,
                };
                emit(textEvent);
                finalText = block.text;
              }
            }
          }

          // Check for errors on assistant messages
          if (message.error) {
            const code = mapSdkError(message.error);
            if (code) {
              const errEvent: ErrorEvent = {
                type: "error",
                id,
                message: `SDK error: ${message.error}`,
                code,
              };
              emit(errEvent);
            }
          }

          // Accumulate usage
          if (betaMessage?.usage) {
            totalInputTokens += betaMessage.usage.input_tokens ?? 0;
            totalOutputTokens += betaMessage.usage.output_tokens ?? 0;
          }
          break;
        }

        case "stream_event": {
          // Partial streaming events — extract text deltas
          const evt = message.event;
          if (evt && evt.type === "content_block_delta") {
            const delta = (evt as any).delta;
            if (delta?.type === "text_delta" && delta.text) {
              const textEvent: TextDeltaEvent = {
                type: "text_delta",
                id,
                content: delta.text,
              };
              emit(textEvent);
            }
          }
          break;
        }

        case "result": {
          if (message.session_id) sessionId = message.session_id;
          costUsd = message.total_cost_usd ?? 0;
          if (message.usage) {
            totalInputTokens = message.usage.input_tokens ?? totalInputTokens;
            totalOutputTokens = message.usage.output_tokens ?? totalOutputTokens;
          }
          if (message.subtype === "success") {
            finalText = message.result ?? finalText;
            stopReason = "end_turn";
          } else if (message.subtype === "error_max_turns") {
            stopReason = "max_turns";
          } else if (message.subtype === "error_max_budget_usd") {
            stopReason = "budget";
          } else {
            // error_during_execution or other error subtypes
            const errors = (message as any).errors;
            const errMsg = Array.isArray(errors)
              ? errors.join("; ")
              : "execution error";
            const errEvent: ErrorEvent = {
              type: "error",
              id,
              message: errMsg,
              code: "unknown",
            };
            emit(errEvent);
          }
          break;
        }

        case "rate_limit_event": {
          const info = (message as any).rate_limit_info;
          if (info?.status === "rejected") {
            const errEvent: ErrorEvent = {
              type: "error",
              id,
              message: `Rate limited, resets at ${info.resetsAt ?? "unknown"}`,
              code: "rate_limit",
            };
            emit(errEvent);
          }
          break;
        }

        // Ignore other message types (user replays, system status, hooks, etc.)
        default:
          break;
      }
    }

    // Emit final result
    const resultEvent: ResultEvent = {
      type: "result",
      id,
      text: finalText,
      session_id: sessionId,
      usage: { input_tokens: totalInputTokens, output_tokens: totalOutputTokens },
      cost_usd: costUsd,
    };
    emit(resultEvent);

    // Emit stop
    const stopEvent: StopEvent = {
      type: "stop",
      id,
      reason: stopReason,
      usage: { input_tokens: totalInputTokens, output_tokens: totalOutputTokens },
    };
    emit(stopEvent);
  } catch (err) {
    const errMsg = err instanceof Error ? err.message : String(err);
    const errEvent: ErrorEvent = {
      type: "error",
      id,
      message: errMsg,
      code: classifyError(err),
    };
    emit(errEvent);

    const stopEvent: StopEvent = {
      type: "stop",
      id,
      reason: "end_turn",
      usage: { input_tokens: 0, output_tokens: 0 },
    };
    emit(stopEvent);
  }
}

// ── Read JSON lines from stdin ──────────────────────────────────────────

async function readLines(): Promise<void> {
  const decoder = new TextDecoder();
  let buffer = "";

  const stdin = Bun.stdin.stream();
  const reader = stdin.getReader();

  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });

      // Process complete lines
      let newlineIdx: number;
      while ((newlineIdx = buffer.indexOf("\n")) !== -1) {
        const line = buffer.slice(0, newlineIdx).trim();
        buffer = buffer.slice(newlineIdx + 1);

        if (!line) continue;

        let parsed: unknown;
        try {
          parsed = JSON.parse(line);
        } catch {
          process.stderr.write(`[agent-bridge] invalid JSON: ${line.slice(0, 200)}\n`);
          continue;
        }

        const req = parsed as QueryRequest;
        if (req.type === "query") {
          // Run queries sequentially (the Rust side dispatches one at a time per bridge)
          await handleQuery(req);
        } else {
          process.stderr.write(`[agent-bridge] unknown request type: ${(parsed as any).type}\n`);
        }
      }
    }
  } finally {
    reader.releaseLock();
  }
}

// ── Graceful shutdown ───────────────────────────────────────────────────

let shuttingDown = false;

function shutdown(signal: string): void {
  if (shuttingDown) return;
  shuttingDown = true;
  process.stderr.write(`[agent-bridge] received ${signal}, shutting down\n`);
  process.exit(0);
}

process.on("SIGTERM", () => shutdown("SIGTERM"));
process.on("SIGINT", () => shutdown("SIGINT"));

// ── Main ────────────────────────────────────────────────────────────────

process.stderr.write("[agent-bridge] started, waiting for queries on stdin\n");
readLines().catch((err) => {
  process.stderr.write(`[agent-bridge] fatal: ${err}\n`);
  process.exit(1);
});
