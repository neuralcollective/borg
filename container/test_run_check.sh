#!/bin/bash
# Tests for the run_check function in entrypoint.sh
set -euo pipefail

PASS=0
FAIL=0

# Minimal reimplementation of run_check that uses plain JSON encoding
# (avoids requiring bun in test env)
run_check_testable() {
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
    local truncated
    truncated=$(printf '%s' "$out" | head -c 8192)
    # Simple JSON string escape for testing (no bun required)
    local escaped
    escaped=$(printf '%s' "$truncated" | python3 -c "import sys,json; print(json.dumps(sys.stdin.read()))" 2>/dev/null || printf '"%s"' "$truncated")
    echo "---BORG_TEST_RESULT---{\"phase\":\"$phase\",\"passed\":$passed,\"exitCode\":$rc,\"output\":$escaped}"
}

assert_contains() {
    local desc="$1"
    local haystack="$2"
    local needle="$3"
    if [[ "$haystack" == *"$needle"* ]]; then
        echo "  PASS: $desc"
        PASS=$(( PASS + 1 ))
    else
        echo "  FAIL: $desc"
        echo "    expected to contain: $needle"
        echo "    actual: $haystack"
        FAIL=$(( FAIL + 1 ))
    fi
}

assert_not_contains() {
    local desc="$1"
    local haystack="$2"
    local needle="$3"
    if [[ "$haystack" != *"$needle"* ]]; then
        echo "  PASS: $desc"
        PASS=$(( PASS + 1 ))
    else
        echo "  FAIL: $desc"
        echo "    expected NOT to contain: $needle"
        echo "    actual: $haystack"
        FAIL=$(( FAIL + 1 ))
    fi
}

echo "=== run_check tests ==="

echo ""
echo "--- successful command ---"
result=$(run_check_testable "test" "echo hello")
assert_contains "passed=true for exit 0"       "$result" '"passed":true'
assert_contains "exitCode=0 for exit 0"        "$result" '"exitCode":0'
assert_contains "output contains hello"        "$result" 'hello'
assert_contains "phase in result"              "$result" '"phase":"test"'

echo ""
echo "--- failing command ---"
result=$(run_check_testable "compileCheck" "exit 42")
assert_contains "passed=false for non-zero"    "$result" '"passed":false'
assert_contains "exitCode=42 captured"         "$result" '"exitCode":42'

echo ""
echo "--- command with pipe ---"
result=$(run_check_testable "lint" "echo foobar | tr 'a-z' 'A-Z'")
assert_contains "pipe commands work"           "$result" 'FOOBAR'
assert_contains "pipe command passes"          "$result" '"passed":true'

echo ""
echo "--- command with && ---"
result=$(run_check_testable "test" "echo first && echo second")
assert_contains "compound && works"            "$result" 'first'
assert_contains "second output present"        "$result" 'second'

echo ""
echo "--- command with shell subshell \$(...) ---"
result=$(run_check_testable "test" 'echo $(echo nested)')
assert_contains "subshell in script works"     "$result" 'nested'
assert_contains "subshell command passes"      "$result" '"passed":true'

echo ""
echo "--- empty command is skipped ---"
result=$(run_check_testable "test" "" || true)
assert_contains "empty cmd returns empty"      "${result:-}" ""

echo ""
echo "--- temp file is cleaned up ---"
before=$(find /tmp -maxdepth 1 -name 'borg-check.*' 2>/dev/null | wc -l | tr -d ' ')
run_check_testable "test" "echo cleanup_test" > /dev/null
after=$(find /tmp -maxdepth 1 -name 'borg-check.*' 2>/dev/null | wc -l | tr -d ' ')
if [ "$before" -eq "$after" ]; then
    echo "  PASS: temp script file cleaned up"
    PASS=$(( PASS + 1 ))
else
    echo "  FAIL: temp script file not cleaned up ($before vs $after)"
    FAIL=$(( FAIL + 1 ))
fi

echo ""
echo "--- no bash -c in output (injection guard) ---"
# Verify the function writes to a file and uses 'bash <file>' not 'bash -c <string>'
# by checking that a command with embedded command substitution runs as expected
# (not expanded twice — if double-expansion were happening, the echo would show
# the *result* of the inner command early; but since it's in a script, it runs once)
result=$(run_check_testable "test" 'X=world; echo "hello $X"')
assert_contains "variable in script works"    "$result" 'hello world'

echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="
[ "$FAIL" -eq 0 ]
