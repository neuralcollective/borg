#!/bin/bash
# Tests for json_str() and the log_event JSON construction in entrypoint.sh
set -euo pipefail

PASS=0
FAIL=0

json_str() {
    printf '%s' "$1" | bun -e "let s='';process.stdin.on('data',c=>s+=c);process.stdin.on('end',()=>process.stdout.write(JSON.stringify(s)));"
}

is_valid_json() {
    printf '%s' "$1" | bun -e "
let s='';
process.stdin.on('data',c=>s+=c);
process.stdin.on('end',()=>{
    try { JSON.parse(s); process.exit(0); } catch(e) { process.stderr.write(e.message+'\n'); process.exit(1); }
});
" 2>/dev/null
}

assert_valid_json() {
    local desc="$1"
    local json="$2"
    if is_valid_json "$json"; then
        echo "PASS: $desc"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $desc"
        echo "  JSON: $json"
        FAIL=$((FAIL + 1))
    fi
}

assert_json_value() {
    local desc="$1"
    local json="$2"
    local expected="$3"
    local got
    got=$(printf '%s' "$json" | bun -e "let s='';process.stdin.on('data',c=>s+=c);process.stdin.on('end',()=>process.stdout.write(JSON.parse(s)))" 2>/dev/null || echo "__PARSE_ERROR__")
    if [ "$got" = "$expected" ]; then
        echo "PASS: $desc"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $desc"
        echo "  Expected: $(printf '%q' "$expected")"
        echo "  Got:      $(printf '%q' "$got")"
        FAIL=$((FAIL + 1))
    fi
}

# --- json_str unit tests ---

VAL='C:\Windows\path\to\file'
assert_valid_json "backslash: json_str output is valid JSON" "$(json_str "$VAL")"
assert_json_value "backslash: value round-trips correctly" "$(json_str "$VAL")" "$VAL"

VAL='say "hello world"'
assert_valid_json "double quote: json_str output is valid JSON" "$(json_str "$VAL")"
assert_json_value "double quote: value round-trips correctly" "$(json_str "$VAL")" "$VAL"

VAL=$'line1\nline2\nline3'
assert_valid_json "newline: json_str output is valid JSON" "$(json_str "$VAL")"
assert_json_value "newline: value round-trips correctly" "$(json_str "$VAL")" "$VAL"

VAL=$'line1\r\nline2'
assert_valid_json "CRLF: json_str output is valid JSON" "$(json_str "$VAL")"
assert_json_value "CRLF: value round-trips correctly" "$(json_str "$VAL")" "$VAL"

VAL=$'error at C:\\path\\file\n"expected" != "actual"\t<tab here>'
assert_valid_json "mixed special chars: json_str output is valid JSON" "$(json_str "$VAL")"
assert_json_value "mixed special chars: value round-trips correctly" "$(json_str "$VAL")" "$VAL"

# --- agent_error event tests (STDERR_TAIL) ---

EXITCODE=1
STDERR='fatal error at C:\Users\runner\work: "unexpected EOF"\r\nstack trace follows'
STDERR_TAIL_JSON=$(printf '%s' "$STDERR" | bun -e "let s='';process.stdin.on('data',c=>s+=c);process.stdin.on('end',()=>process.stdout.write(JSON.stringify(s)));")
EVENT="{\"type\":\"container_event\",\"event\":\"agent_error\",\"exit_code\":${EXITCODE},\"stderr_tail\":${STDERR_TAIL_JSON}}"
assert_valid_json "agent_error: backslash+quote+CRLF in stderr_tail produces valid JSON" "$EVENT"

# --- commit_complete event tests (COMMIT_MSG) ---

COMMIT_MSG='feat: fix path C:\app\main.go "handle errors"'
EVENT="{\"type\":\"container_event\",\"event\":\"commit_complete\",\"message\":$(json_str "$COMMIT_MSG")}"
assert_valid_json "commit_complete: backslash+quote in message produces valid JSON" "$EVENT"

COMMIT_MSG=$'fix: multi\nline\ncommit message'
EVENT="{\"type\":\"container_event\",\"event\":\"commit_complete\",\"message\":$(json_str "$COMMIT_MSG")}"
assert_valid_json "commit_complete: newline in message produces valid JSON" "$EVENT"

# --- push events tests (BRANCH) ---

BRANCH='feature/my-branch'
BRANCH_JSON=$(json_str "$BRANCH")
EVENT="{\"type\":\"container_event\",\"event\":\"push_complete\",\"branch\":${BRANCH_JSON}}"
assert_valid_json "push_complete: normal branch name produces valid JSON" "$EVENT"

EVENT="{\"type\":\"container_event\",\"event\":\"push_failed\",\"branch\":${BRANCH_JSON}}"
assert_valid_json "push_failed: normal branch name produces valid JSON" "$EVENT"

# --- agent_started / clone_started tests ---

MODEL='claude-sonnet-4-6'
REPO_URL='https://github.com/org/repo.git'
MODEL_JSON=$(json_str "$MODEL")
REPO_URL_JSON=$(json_str "$REPO_URL")
EVENT="{\"type\":\"container_event\",\"event\":\"agent_started\",\"model\":${MODEL_JSON},\"repo\":${REPO_URL_JSON}}"
assert_valid_json "agent_started: model and repo produce valid JSON" "$EVENT"

BRANCH_JSON=$(json_str "$BRANCH")
EVENT="{\"type\":\"container_event\",\"event\":\"clone_started\",\"repo\":${REPO_URL_JSON},\"branch\":${BRANCH_JSON}}"
assert_valid_json "clone_started: repo and branch produce valid JSON" "$EVENT"

# --- Summary ---

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ] || exit 1
