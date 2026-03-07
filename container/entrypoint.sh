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
process.stdout.write('REPO_URL=\'' + esc(d.repoUrl||'') + \"'\\n\");
process.stdout.write('MIRROR_PATH=\'' + esc(d.mirrorPath||'') + \"'\\n\");
process.stdout.write('BRANCH=\'' + esc(d.branch||'') + \"'\\n\");
process.stdout.write('BASE=\'' + esc(d.base||'origin/main') + \"'\\n\");
process.stdout.write('COMMIT_MSG=\'' + esc(d.commitMessage||'feat: borg agent changes') + \"'\\n\");
process.stdout.write('GIT_AUTHOR_NAME=\'' + esc(d.gitAuthorName||'Borg') + \"'\\n\");
process.stdout.write('GIT_AUTHOR_EMAIL=\'' + esc(d.gitAuthorEmail||'borg@localhost') + \"'\\n\");
process.stdout.write('PUSH_AFTER_COMMIT=\'' + esc(d.pushAfterCommit ? '1' : '') + \"'\\n\");
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
     -d "\$(jq -n --arg q "\$QUERY" --argjson pid "\${PROJECT_ID:-0}" '{query: \$q, project_id: \$pid}')" | bun -e "
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

REPO_DIR=/workspace/repo

log_event "{\"type\":\"container_event\",\"event\":\"agent_started\",\"model\":$(json_encode "${MODEL}"),\"repo\":$(json_encode "${REPO_URL}")}"

if [ -n "$REPO_URL" ]; then
    CLONE_START=$(date +%s%3N)
    log_event "{\"type\":\"container_event\",\"event\":\"clone_started\",\"repo\":$(json_encode "${REPO_URL}"),\"branch\":$(json_encode "${BRANCH}")}"

    # Clone to temp dir first, then move into repo dir (which may have mounted volumes)
    CLONE_TMP=$(mktemp -d /workspace/clone.XXXXXX)
    CLONE_ARGS=(--depth 50)
    if [ -n "$MIRROR_PATH" ] && [ -d "$MIRROR_PATH" ]; then
        CLONE_ARGS+=(--reference "$MIRROR_PATH")
    fi
    git clone "${CLONE_ARGS[@]}" "$REPO_URL" "$CLONE_TMP/src"
    # Move cloned contents into repo dir (preserves mounted volumes like target/)
    find "$REPO_DIR" -mindepth 1 -maxdepth 1 ! -name target ! -name node_modules -exec rm -rf {} + 2>/dev/null || true
    shopt -s dotglob
    mv "$CLONE_TMP/src"/* "$CLONE_TMP/src"/.* "$REPO_DIR/" 2>/dev/null || true
    shopt -u dotglob
    rm -rf "$CLONE_TMP"
    cd "$REPO_DIR"
    if [ -n "$BRANCH" ]; then
        # Fetch the task branch if it exists on remote
        git fetch --depth 50 origin "+refs/heads/$BRANCH:refs/remotes/origin/$BRANCH" 2>/dev/null || true
        if git rev-parse --verify "origin/$BRANCH" >/dev/null 2>&1; then
            git checkout -b -- "$BRANCH" "origin/$BRANCH"
        else
            git checkout -b -- "$BRANCH" "$BASE"
        fi
    fi

    CLONE_END=$(date +%s%3N)
    log_event "{\"type\":\"container_event\",\"event\":\"clone_complete\",\"duration_ms\":$(( CLONE_END - CLONE_START ))}"
else
    cd /workspace
fi

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

# Run test/lint/compile checks before committing (only when a repo was cloned)
if [ -n "$REPO_URL" ] && [ -d "$REPO_DIR" ]; then
    cd "$REPO_DIR"
    run_check "compileCheck" "$COMPILE_CHECK_CMD"
    run_check "lint" "$LINT_CMD"
    run_check "test" "$TEST_CMD"
fi

if [ -n "$REPO_URL" ] && [ -d "$REPO_DIR/.git" ]; then
    cd "$REPO_DIR"
    git config user.name "$GIT_AUTHOR_NAME"
    git config user.email "$GIT_AUTHOR_EMAIL"

    if ! git diff --quiet HEAD 2>/dev/null || [ -n "$(git ls-files --others --exclude-standard)" ]; then
        # Exclude secrets from commits
        printf '.env\n.env.*\ncredentials*\n*.key\n*.pem\n' >> .gitignore 2>/dev/null || true
        git add -A
        git commit -m "$COMMIT_MSG" || true
        log_event "{\"type\":\"container_event\",\"event\":\"commit_complete\",\"message\":\"${COMMIT_MSG}\"}"
    else
        log_event "{\"type\":\"container_event\",\"event\":\"commit_skipped\"}"
    fi

    if [ -n "$PUSH_AFTER_COMMIT" ] && [ -n "$BRANCH" ]; then
        if git push origin -- "$BRANCH"; then
            log_event "{\"type\":\"container_event\",\"event\":\"push_complete\",\"branch\":\"${BRANCH}\"}"
        else
            log_event "{\"type\":\"container_event\",\"event\":\"push_failed\",\"branch\":\"${BRANCH}\"}"
        fi
    fi

    SIGNAL_FILE="$REPO_DIR/.borg/signal.json"
    if [ -f "$SIGNAL_FILE" ]; then
        echo "BORG_SIGNAL:$(cat "$SIGNAL_FILE")"
    fi
fi

log_event "{\"type\":\"container_event\",\"event\":\"container_exiting\",\"exit_code\":${exitcode}}"

exit "$exitcode"
