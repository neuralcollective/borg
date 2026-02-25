#!/bin/bash
set -e

# Cap heap to prevent OOM kills (Claude Code runs on bun/node)
export NODE_OPTIONS="${NODE_OPTIONS:---max-old-space-size=384}"

# Read all stdin into a temp file
cat > /tmp/input.json

# Parse input JSON
eval "$(bun -e "
const d=JSON.parse(require('fs').readFileSync('/tmp/input.json','utf8'));
const esc = s => s.replace(/'/g, \"'\\\\''\");
console.log('PROMPT=\'' + esc(d.prompt||'') + '\'');
console.log('MODEL=\'' + esc(d.model||'claude-sonnet-4-6') + '\'');
console.log('SESSION_ID=\'' + esc(d.resumeSessionId||d.sessionId||'') + '\'');
console.log('ASSISTANT_NAME=\'' + esc(d.assistantName||'Borg') + '\'');
console.log('SYSTEM_PROMPT=\'' + esc(d.systemPrompt||'') + '\'');
console.log('ALLOWED_TOOLS=\'' + esc(d.allowedTools||'') + '\'');
console.log('WORKDIR=\'' + esc(d.workdir||'') + '\'');
")"

# Change to workdir if specified
if [ -n "$WORKDIR" ] && [ -d "$WORKDIR" ]; then
    cd "$WORKDIR"
fi

# Build claude args
CLAUDE_ARGS=(
    --print
    --output-format stream-json
    --model "$MODEL"
    --verbose
)

if [ -n "$SESSION_ID" ]; then
    CLAUDE_ARGS+=(--resume "$SESSION_ID")
fi

# Use specified allowed tools, or default to full set
if [ -n "$ALLOWED_TOOLS" ]; then
    CLAUDE_ARGS+=(--allowedTools "$ALLOWED_TOOLS")
else
    CLAUDE_ARGS+=(
        --allowedTools 'Bash,Read,Write,Edit,Glob,Grep,WebSearch,WebFetch,Task,TaskOutput,TaskStop,NotebookEdit,EnterPlanMode,ExitPlanMode,TaskCreate,TaskGet,TaskUpdate,TaskList'
    )
fi

CLAUDE_ARGS+=(--permission-mode bypassPermissions)

# Prepend system prompt to user prompt if provided
if [ -n "$SYSTEM_PROMPT" ]; then
    FULL_PROMPT="$SYSTEM_PROMPT

---

$PROMPT"
else
    FULL_PROMPT="$PROMPT"
fi

# Run Claude Code
echo "$FULL_PROMPT" | claude "${CLAUDE_ARGS[@]}" 2>/dev/null || true
