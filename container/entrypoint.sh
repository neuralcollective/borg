#!/bin/bash
set -e

# Cap heap to prevent OOM kills (Claude Code runs on bun/node)
export NODE_OPTIONS="${NODE_OPTIONS:---max-old-space-size=384}"

# Run setup script if bind-mounted (sourced so PATH exports persist)
if [ -f /workspace/setup.sh ]; then
    source /workspace/setup.sh
fi

# Read all stdin into a private temp file
INPUT_FILE=$(mktemp /tmp/borg-input.XXXXXX)
chmod 600 "$INPUT_FILE"
trap 'rm -f "$INPUT_FILE"' EXIT

cat > "$INPUT_FILE"

# Parse input JSON
eval "$(bun -e "
const d=JSON.parse(require('fs').readFileSync('$INPUT_FILE','utf8'));
const esc = s => s.replace(/'/g, \"'\\\\''\");
console.log('PROMPT=\'' + esc(d.prompt||'') + '\'');
console.log('MODEL=\'' + esc(d.model||'claude-sonnet-4-6') + '\'');
console.log('SESSION_ID=\'' + esc(d.resumeSessionId||d.sessionId||'') + '\'');
console.log('ASSISTANT_NAME=\'' + esc(d.assistantName||'Borg') + '\'');
console.log('SYSTEM_PROMPT=\'' + esc(d.systemPrompt||'') + '\'');
console.log('ALLOWED_TOOLS=\'' + esc(d.allowedTools||'') + '\'');
console.log('WORKDIR=\'' + esc(d.workdir||'') + '\'');
")"

# Change to workdir if specified (must be under /workspace)
if [ -n "$WORKDIR" ]; then
    case "$WORKDIR" in
        /workspace|/workspace/*)
            if [ -d "$WORKDIR" ]; then
                cd "$WORKDIR"
            else
                echo "Warning: WORKDIR $WORKDIR does not exist, staying in $(pwd)" >&2
            fi
            ;;
        *)
            echo "Warning: WORKDIR $WORKDIR is not under /workspace, ignoring" >&2
            ;;
    esac
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

# Run Claude Code — capture exit code and stderr for diagnostics
exitcode=0
echo "$FULL_PROMPT" | claude "${CLAUDE_ARGS[@]}" 2>/tmp/claude_stderr.log || exitcode=$?

# If no stdout was produced, dump stderr so the pipeline can see what went wrong
if [ ! -s /dev/stdout ] && [ -s /tmp/claude_stderr.log ]; then
    echo '{"type":"error","message":"Claude CLI produced no output. Stderr:"}'
    cat /tmp/claude_stderr.log >&2
fi

exit "$exitcode"
