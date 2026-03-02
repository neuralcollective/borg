#!/usr/bin/env bash
# Tests for run_check in entrypoint.sh
# Runs without the full container environment by substituting a portable
# JSON-escaper for the bun one-liner.

set -euo pipefail

PASS=0
FAIL=0

ok()   { echo "PASS: $1"; PASS=$((PASS + 1)); }
fail() { echo "FAIL: $1"; FAIL=$((FAIL + 1)); }

# Portable replacement for the bun JSON-escape used in run_check.
# Outputs a JSON-encoded string (with surrounding quotes).
_json_escape() {
    python3 -c "import json,sys; print(json.dumps(sys.stdin.read()), end='')"
}

# Reimplementation of run_check that uses _json_escape instead of bun.
# Must stay in sync with the function in entrypoint.sh.
run_check() {
    local phase="$1"
    local cmd="$2"
    if [ -z "$cmd" ]; then return; fi
    local out rc=0
    out=$(bash -c "$cmd" -- 2>&1) || rc=$?
    local passed="false"
    [ "$rc" -eq 0 ] && passed="true"
    local truncated escaped
    truncated=$(printf '%s' "$out" | head -c 8192)
    escaped=$(printf '%s' "$truncated" | _json_escape)
    echo "---BORG_TEST_RESULT---{\"phase\":\"$phase\",\"passed\":$passed,\"exitCode\":$rc,\"output\":$escaped}"
}

# ---------------------------------------------------------------------------
# 1. Empty command is a no-op
result=$(run_check "test" "" 2>&1 || true)
if [ -z "$result" ]; then
    ok "empty cmd is no-op"
else
    fail "empty cmd should produce no output, got: $result"
fi

# ---------------------------------------------------------------------------
# 2. Successful command → passed=true, exitCode=0
result=$(run_check "compileCheck" "echo hello" 2>&1)
if echo "$result" | grep -q '"passed":true' && echo "$result" | grep -q '"exitCode":0'; then
    ok "success → passed=true exitCode=0"
else
    fail "success case wrong: $result"
fi

# ---------------------------------------------------------------------------
# 3. Failing command → passed=false, exitCode non-zero
result=$(run_check "test" "exit 42" 2>&1)
if echo "$result" | grep -q '"passed":false' && echo "$result" | grep -q '"exitCode":42'; then
    ok "failure → passed=false exitCode=42"
else
    fail "failure case wrong: $result"
fi

# ---------------------------------------------------------------------------
# 4. Output is captured and JSON-encoded in the result line
result=$(run_check "lint" "printf 'hello world'" 2>&1)
if echo "$result" | grep -q '"hello world"'; then
    ok "stdout captured in output field"
else
    fail "stdout not captured: $result"
fi

# ---------------------------------------------------------------------------
# 5. stderr is merged into output
result=$(run_check "lint" "echo err >&2" 2>&1)
if echo "$result" | grep -q 'err'; then
    ok "stderr merged into output"
else
    fail "stderr not merged: $result"
fi

# ---------------------------------------------------------------------------
# 6. Multi-word command (no word-splitting issues)
result=$(run_check "test" "printf '%s %s' foo bar" 2>&1)
if echo "$result" | grep -q 'foo bar'; then
    ok "multi-word command works"
else
    fail "multi-word command broken: $result"
fi

# ---------------------------------------------------------------------------
# 7. -- is passed as $0 inside the subshell, not mistaken for a flag
result=$(run_check "test" 'echo "$0"' 2>&1)
if echo "$result" | grep -q '"--"'; then
    ok "\$0 inside subshell is '--'"
else
    fail "\$0 not set to '--': $result"
fi

# ---------------------------------------------------------------------------
# 8. Pipeline in command works (requires shell interpretation)
result=$(run_check "test" "printf 'a\nb\nc\n' | wc -l | tr -d ' '" 2>&1)
if echo "$result" | python3 -c "import sys,json; d=json.loads(sys.stdin.read().split('---BORG_TEST_RESULT---')[1]); exit(0 if '3' in d['output'] else 1)" 2>/dev/null; then
    ok "pipeline in command works"
else
    fail "pipeline broken: $result"
fi

# ---------------------------------------------------------------------------
# 9. Command output is truncated at 8192 bytes
long_cmd="python3 -c \"print('x' * 9000)\""
result=$(run_check "test" "$long_cmd" 2>&1)
# The captured output field should be exactly 8192 x's (plus surrounding quotes)
output_len=$(echo "$result" | python3 -c "
import sys, json
line = sys.stdin.read()
data = json.loads(line.split('---BORG_TEST_RESULT---')[1])
print(len(data['output']))
")
if [ "$output_len" -le 8192 ]; then
    ok "output truncated to 8192 bytes"
else
    fail "output not truncated: len=$output_len"
fi

# ---------------------------------------------------------------------------
echo ""
echo "Results: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
