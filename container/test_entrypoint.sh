#!/bin/bash
# Tests for cmd_is_safe() in entrypoint.sh

# Mirror of cmd_is_safe() from entrypoint.sh — kept in sync manually.
cmd_is_safe() {
    local cmd="$1"
    [[ "$cmd" != *$'\n'* ]] &&
    [[ "$cmd" != *$'\r'* ]] &&
    [[ "$cmd" != *\\* ]] &&
    ! printf '%s' "$cmd" | grep -q '[;$`(){}!<>]'
}

pass=0
fail=0

assert_safe() {
    local cmd="$1"
    if cmd_is_safe "$cmd"; then
        echo "PASS safe:     $cmd"
        (( pass++ )) || true
    else
        echo "FAIL expected safe, got rejected: $cmd"
        (( fail++ )) || true
    fi
}

assert_unsafe() {
    local cmd="$1" desc="${2:-$1}"
    if ! cmd_is_safe "$cmd"; then
        echo "PASS rejected: $desc"
        (( pass++ )) || true
    else
        echo "FAIL expected rejection, got allowed: $desc"
        (( fail++ )) || true
    fi
}

# --- Safe commands (common build/lint/test invocations) ---
assert_safe "cargo build"
assert_safe "cargo test"
assert_safe "cargo clippy -- -D warnings"
assert_safe "cargo build --release"
assert_safe "npm test"
assert_safe "npm run build && npm run test"
assert_safe "bun test"
assert_safe "pytest -v"
assert_safe "pytest -v --tb=short"
assert_safe "pytest tests/ -k test_foo"
assert_safe "just t"
assert_safe "make all"
assert_safe "./gradlew test"
assert_safe "python -m pytest tests/"
assert_safe "cargo build && cargo test"
assert_safe "go build ./..."
assert_safe "go test ./..."
assert_safe "eslint src/"
assert_safe "tsc --noEmit"

# --- Injection via semicolon ---
assert_unsafe "cargo build; rm -rf /workspace" "semicolon chaining"
assert_unsafe "; rm -rf /" "bare semicolon"

# --- Command substitution via $() ---
assert_unsafe 'cargo build && $(rm -rf /)' 'dollar-paren substitution'
assert_unsafe 'echo ${PATH}' 'dollar-brace variable'
assert_unsafe 'echo $HOME' 'dollar variable'

# --- Backtick command substitution ---
assert_unsafe 'cargo build && `rm -rf /`' 'backtick substitution'

# --- Subshell ---
assert_unsafe 'cargo build && (rm -rf /)' 'subshell via parens'

# --- Brace grouping ---
assert_unsafe 'cargo build && { rm -rf /; }' 'brace grouping'

# --- Redirects ---
assert_unsafe 'cargo build > /dev/null' 'stdout redirect'
assert_unsafe 'cargo build < /dev/null' 'stdin redirect'

# --- Exclamation (history expansion) ---
assert_unsafe '!cargo' 'history expansion'

# --- Backslash ---
assert_unsafe 'cargo build \\ thing' 'backslash escape'
assert_unsafe $'cargo build\\\nrm -rf /' 'backslash before newline'

# --- Newline injection ---
assert_unsafe $'cargo build\nrm -rf /' 'embedded newline'
assert_unsafe $'cargo build\rrm -rf /' 'embedded carriage return'

echo ""
echo "Results: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
