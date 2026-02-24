#!/bin/bash
set -e

# Cap Node heap to prevent OOM kills
export NODE_OPTIONS="${NODE_OPTIONS:---max-old-space-size=384}"

# Read all stdin into a temp file
cat > /tmp/input.json

# Parse input JSON
PROMPT=$(node -e "const d=JSON.parse(require('fs').readFileSync('/tmp/input.json','utf8')); process.stdout.write(d.prompt||'')")
MODEL=$(node -e "const d=JSON.parse(require('fs').readFileSync('/tmp/input.json','utf8')); process.stdout.write(d.model||'claude-opus-4-6')")
SESSION_ID=$(node -e "const d=JSON.parse(require('fs').readFileSync('/tmp/input.json','utf8')); process.stdout.write(d.sessionId||'')")
ASSISTANT_NAME=$(node -e "const d=JSON.parse(require('fs').readFileSync('/tmp/input.json','utf8')); process.stdout.write(d.assistantName||'Borg')")

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

# Allow all tools
CLAUDE_ARGS+=(
    --allowedTools 'Bash,Read,Write,Edit,Glob,Grep,WebSearch,WebFetch,Task,TaskOutput,TaskStop,NotebookEdit,EnterPlanMode,ExitPlanMode,TaskCreate,TaskGet,TaskUpdate,TaskList'
    --permission-mode bypassPermissions
)

# Run Claude Code
echo "$PROMPT" | claude "${CLAUDE_ARGS[@]}" 2>/dev/null || true
