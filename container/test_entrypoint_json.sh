#!/bin/bash
# Tests for json_str() in entrypoint.sh — verifies values with JSON-special chars
# produce valid, correctly-escaped JSON strings.
set -e

json_str() {
    printf '%s' "$1" | bun -e "let s='';process.stdin.on('data',c=>s+=c);process.stdin.on('end',()=>process.stdout.write(JSON.stringify(s)));"
}

pass=0
fail=0

check() {
    local desc="$1" input="$2" expected="$3"
    local got
    got=$(json_str "$input")
    if [ "$got" = "$expected" ]; then
        echo "  PASS: $desc"
        (( pass++ )) || true
    else
        echo "  FAIL: $desc"
        echo "        input:    $(printf '%s' "$input" | cat -v)"
        echo "        expected: $expected"
        echo "        got:      $got"
        (( fail++ )) || true
    fi
}

check_valid_json() {
    local desc="$1" input="$2"
    local got
    got=$(json_str "$input")
    if printf '%s' "$got" | bun -e "process.stdin.on('data',d=>JSON.parse(String(d)));process.stdin.on('end',()=>process.exit(0))" 2>/dev/null; then
        echo "  PASS: $desc (valid JSON)"
        (( pass++ )) || true
    else
        echo "  FAIL: $desc (invalid JSON output: $got)"
        (( fail++ )) || true
    fi
}

echo "=== json_str tests ==="

# Plain strings
check "plain string" "hello" '"hello"'
check "empty string" "" '""'

# Double quotes — the primary injection vector
check "double quote" '"' '"\""'
check "commit msg with quotes" 'fix: handle "edge case"' '"fix: handle \"edge case\""'

# Backslash
check "backslash" '\' '"\\"'
check "backslash+quote" '\"' '"\\\""'

# Newlines and tabs — common in stderr/commit messages
check_valid_json "newline in value" "line1
line2"
check_valid_json "tab in value" "col1	col2"
check_valid_json "carriage return" $'foo\rbar'

# Control characters
check_valid_json "null byte replaced" $'foo\x00bar'

# Unicode
check "unicode" "café ñoño" '"café ñoño"'
check_valid_json "emoji" "fix: add 🚀 feature"

# Repo URL with special chars
check_valid_json "repo url" "https://github.com/org/repo-\"special\".git"

# Branch name with slash and quotes
check_valid_json "branch with slash" 'feat/fix-"thing"'

# Stderr tail with multiple special chars (simulating real error output)
check_valid_json "stderr snippet" 'error: "file.rs": cannot find function `foo` in this scope
   --> src/main.rs:10:5
note: use "bar" instead'

echo ""
echo "Results: ${pass} passed, ${fail} failed"
[ "$fail" -eq 0 ]
