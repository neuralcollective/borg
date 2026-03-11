#!/bin/bash
set -e

export NODE_OPTIONS="${NODE_OPTIONS:---max-old-space-size=384}"

INPUT_FILE=$(mktemp /tmp/borg-input.XXXXXX)
chmod 600 "$INPUT_FILE"
VARS_FILE=$(mktemp /tmp/borg-vars.XXXXXX)
chmod 600 "$VARS_FILE"
CLAUDE_OUT=$(mktemp /tmp/borg-claude-out.XXXXXX)
STDERR_FILE=$(mktemp /tmp/borg-stderr.XXXXXX)
trap 'rm -f "$INPUT_FILE" "$VARS_FILE" "$CLAUDE_OUT" "$STDERR_FILE"' EXIT

log_event() {
    echo "---BORG_EVENT---${1}" >&2
}

json_encode() {
    printf '%s' "$1" | bun -e "let s='';process.stdin.on('data',c=>s+=c);process.stdin.on('end',()=>process.stdout.write(JSON.stringify(s)));"
}

run_check() {
    local phase="$1"
    local cmd="$2"
    if [ -z "$cmd" ]; then return; fi
    local out rc=0
    local tmpscript
    tmpscript=$(mktemp /tmp/borg-check.XXXXXX)
    chmod 700 "$tmpscript"
    printf '%s\n' "$cmd" > "$tmpscript"
    out=$(bash "$tmpscript" 2>&1) || rc=$?
    rm -f "$tmpscript"
    local passed="false"
    [ "$rc" -eq 0 ] && passed="true"
    local truncated escaped
    truncated=$(printf '%s' "$out" | head -c 8192)
    escaped=$(printf '%s' "$truncated" | bun -e "
let s='';process.stdin.on('data',c=>s+=c);process.stdin.on('end',()=>process.stdout.write(JSON.stringify(s)));
")
    echo "---BORG_TEST_RESULT---{\"phase\":\"$phase\",\"passed\":$passed,\"exitCode\":$rc,\"output\":$escaped}"
}

cat > "$INPUT_FILE"

INPUT_FILE="$INPUT_FILE" bun -e "
const d=JSON.parse(require('fs').readFileSync(process.env.INPUT_FILE,'utf8'));
const esc = s => s.replace(/'/g, \"'\\\\''\");
process.stdout.write('PROMPT=\'' + esc(d.prompt||'') + \"'\\n\");
process.stdout.write('MODEL=\'' + esc(d.model||'claude-sonnet-4-6') + \"'\\n\");
process.stdout.write('SESSION_ID=\'' + esc(d.resumeSessionId||d.sessionId||'') + \"'\\n\");
process.stdout.write('SYSTEM_PROMPT=\'' + esc(d.systemPrompt||'') + \"'\\n\");
process.stdout.write('ALLOWED_TOOLS=\'' + esc(d.allowedTools||'') + \"'\\n\");
process.stdout.write('MAX_TURNS=\'' + esc(String(d.maxTurns||'200')) + \"'\\n\");
process.stdout.write('COMPILE_CHECK_CMD=\'' + esc(d.compileCheckCmd||'') + \"'\\n\");
process.stdout.write('LINT_CMD=\'' + esc(d.lintCmd||'') + \"'\\n\");
process.stdout.write('TEST_CMD=\'' + esc(d.testCmd||'') + \"'\\n\");
process.stdout.write('PROJECT_ID=\'' + esc(String(d.projectId||'0')) + \"'\\n\");
" > "$VARS_FILE" || { echo "Failed to parse input JSON" >&2; exit 1; }
# shellcheck source=/dev/null
source "$VARS_FILE"
export PROJECT_ID
export BORG_HOST_IP

# Create web_search shim that hits our ZDR proxy
cat <<EOF > /usr/local/bin/web_search
#!/bin/bash
QUERY="\$*"
curl -s -X POST http://\${BORG_HOST_IP}:3132/v1/search \\
     -H "Content-Type: application/json" \\
     -d "{\"query\": \"\$QUERY\", \"project_id\": \${PROJECT_ID:-0}}" | bun -e "
let s=''; process.stdin.on('data', c=>s+=c); process.stdin.on('end', ()=>{
  try {
    const d=JSON.parse(s);
    process.stdout.write(d.results || '');
  } catch(e) {
    process.stdout.write('Search failed: ' + s);
  }
});"
EOF
chmod +x /usr/local/bin/web_search

# The host bind-mounts the task worktree as /workspace
cd /workspace

log_event "{\"type\":\"container_event\",\"event\":\"agent_started\",\"model\":$(json_encode "${MODEL}")}"

if [ -f /workspace/setup.sh ]; then
    log_event "{\"type\":\"container_event\",\"event\":\"setup_started\"}"
    source /workspace/setup.sh
    log_event "{\"type\":\"container_event\",\"event\":\"setup_complete\"}"
fi

CLAUDE_ARGS=(
    --print
    --output-format stream-json
    --model "$MODEL"
    --verbose
    --dangerously-skip-permissions
    --max-turns "$MAX_TURNS"
)

if [ -n "$SESSION_ID" ]; then
    CLAUDE_ARGS+=(--resume "$SESSION_ID")
fi

if [ -n "$ALLOWED_TOOLS" ]; then
    CLAUDE_ARGS+=(--allowedTools "$ALLOWED_TOOLS")
fi

if [ -n "$SYSTEM_PROMPT" ]; then
    CLAUDE_ARGS+=(--append-system-prompt "$SYSTEM_PROMPT")
fi

exitcode=0
printf '%s\n' "$PROMPT" | claude "${CLAUDE_ARGS[@]}" >"$CLAUDE_OUT" 2>"$STDERR_FILE" || exitcode=$?

cat "$CLAUDE_OUT"

if [ ! -s "$CLAUDE_OUT" ] && [ -s "$STDERR_FILE" ]; then
    echo '{"type":"error","message":"Claude CLI produced no output. Stderr:"}'
    cat "$STDERR_FILE" >&2
fi

if [ "$exitcode" -eq 0 ]; then
    log_event "{\"type\":\"container_event\",\"event\":\"agent_complete\"}"
else
    STDERR_TAIL=$(tail -c 2000 "$STDERR_FILE" | bun -e "let s='';process.stdin.on('data',c=>s+=c);process.stdin.on('end',()=>process.stdout.write(JSON.stringify(s)));")
    log_event "{\"type\":\"container_event\",\"event\":\"agent_error\",\"exit_code\":${exitcode},\"stderr_tail\":${STDERR_TAIL}}"
fi

# Run compile/lint/test checks (results sent to stderr for Rust to parse)
if [ -d "/workspace/.git" ]; then
    run_check "compileCheck" "$COMPILE_CHECK_CMD"
    run_check "lint" "$LINT_CMD"
    run_check "test" "$TEST_CMD"
fi

# Signal file for agent → pipeline communication
SIGNAL_FILE="/workspace/.borg/signal.json"
if [ -f "$SIGNAL_FILE" ]; then
    echo "BORG_SIGNAL:$(cat "$SIGNAL_FILE")"
fi

log_event "{\"type\":\"container_event\",\"event\":\"container_exiting\",\"exit_code\":${exitcode}}"

exit "$exitcode"
