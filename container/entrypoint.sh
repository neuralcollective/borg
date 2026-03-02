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
" > "$VARS_FILE" || { echo "Failed to parse input JSON" >&2; exit 1; }
# shellcheck source=/dev/null
source "$VARS_FILE"

REPO_DIR=/workspace/repo

if [ -n "$REPO_URL" ]; then
    CLONE_ARGS=(--depth 50)
    if [ -n "$MIRROR_PATH" ] && [ -d "$MIRROR_PATH" ]; then
        CLONE_ARGS+=(--reference "$MIRROR_PATH")
    fi
    git clone "${CLONE_ARGS[@]}" "$REPO_URL" "$REPO_DIR"
    cd "$REPO_DIR"
    if [ -n "$BRANCH" ]; then
        git checkout -b "$BRANCH" "$BASE"
    fi
else
    cd /workspace
fi

if [ -f /workspace/setup.sh ]; then
    source /workspace/setup.sh
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

if [ -n "$REPO_URL" ] && [ -d "$REPO_DIR/.git" ]; then
    cd "$REPO_DIR"
    git config user.name "$GIT_AUTHOR_NAME"
    git config user.email "$GIT_AUTHOR_EMAIL"

    if ! git diff --quiet HEAD 2>/dev/null || [ -n "$(git ls-files --others --exclude-standard)" ]; then
        git add -A
        git commit -m "$COMMIT_MSG" || true
    fi

    if [ -n "$PUSH_AFTER_COMMIT" ] && [ -n "$BRANCH" ]; then
        git push origin "$BRANCH"
    fi

    SIGNAL_FILE="$REPO_DIR/.borg/signal.json"
    if [ -f "$SIGNAL_FILE" ]; then
        echo "---BORG_SIGNAL---$(cat "$SIGNAL_FILE")"
    fi
fi

exit "$exitcode"
