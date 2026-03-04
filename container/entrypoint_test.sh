#!/bin/bash
# Tests for JSON escaping of dynamic values in entrypoint.sh log_event calls.
set -euo pipefail

PASS=0
FAIL=0

json_encode() {
    printf '%s' "$1" | bun -e "let s='';process.stdin.on('data',c=>s+=c);process.stdin.on('end',()=>process.stdout.write(JSON.stringify(s)));"
}

assert_valid_json() {
    local desc="$1"
    local json="$2"
    if printf '%s' "$json" | bun -e "
let s='';
process.stdin.on('data',c=>s+=c);
process.stdin.on('end',()=>{
  try { JSON.parse(s); process.exit(0); }
  catch(e) { process.stderr.write('INVALID: '+e.message+'\n'); process.exit(1); }
});" 2>/dev/null; then
        echo "PASS: $desc"
        PASS=$((PASS+1))
    else
        echo "FAIL: $desc — not valid JSON: $json"
        FAIL=$((FAIL+1))
    fi
}

assert_field_value() {
    local desc="$1"
    local json="$2"
    local field="$3"
    local expected="$4"
    local actual
    actual=$(printf '%s' "$json" | bun -e "
let s='';
process.stdin.on('data',c=>s+=c);
process.stdin.on('end',()=>{
  const o=JSON.parse(s);
  process.stdout.write(o['$field']);
});")
    if [ "$actual" = "$expected" ]; then
        echo "PASS: $desc"
        PASS=$((PASS+1))
    else
        echo "FAIL: $desc — expected $(printf '%q' "$expected"), got $(printf '%q' "$actual")"
        FAIL=$((FAIL+1))
    fi
}

echo "=== Testing json_encode helper ==="

# AC1: REPO_URL with double quotes produces valid JSON
REPO_URL='https://github.com/user/"evil"repo'
json="{\"type\":\"container_event\",\"event\":\"agent_started\",\"model\":\"test\",\"repo\":$(json_encode "$REPO_URL")}"
assert_valid_json "REPO_URL with double quotes is valid JSON" "$json"
assert_field_value "REPO_URL with double quotes roundtrips correctly" "$json" "repo" "$REPO_URL"

# AC2: REPO_URL with backslash produces valid JSON
REPO_URL='https://host/path\with\backslashes'
json="{\"type\":\"container_event\",\"event\":\"agent_started\",\"model\":\"test\",\"repo\":$(json_encode "$REPO_URL")}"
assert_valid_json "REPO_URL with backslashes is valid JSON" "$json"
assert_field_value "REPO_URL with backslashes roundtrips correctly" "$json" "repo" "$REPO_URL"

# AC3: BRANCH with special characters produces valid JSON
BRANCH='feat/"quoted-branch"'
json="{\"type\":\"container_event\",\"event\":\"clone_started\",\"repo\":\"url\",\"branch\":$(json_encode "$BRANCH")}"
assert_valid_json "BRANCH with double quotes is valid JSON" "$json"
assert_field_value "BRANCH with double quotes roundtrips correctly" "$json" "branch" "$BRANCH"

# AC4: BRANCH with backslash produces valid JSON
BRANCH='feat\backslash'
json="{\"type\":\"container_event\",\"event\":\"clone_started\",\"repo\":\"url\",\"branch\":$(json_encode "$BRANCH")}"
assert_valid_json "BRANCH with backslash is valid JSON" "$json"
assert_field_value "BRANCH with backslash roundtrips correctly" "$json" "branch" "$BRANCH"

# AC5: STDERR_TAIL with newlines and quotes produces valid JSON
STDERR_TAIL=$'Error: "file not found"\nAt line 42\nBackslash: \\'
json="{\"type\":\"container_event\",\"event\":\"agent_error\",\"exit_code\":1,\"stderr_tail\":$(json_encode "$STDERR_TAIL")}"
assert_valid_json "STDERR_TAIL with newlines and quotes is valid JSON" "$json"
assert_field_value "STDERR_TAIL with newlines and quotes roundtrips correctly" "$json" "stderr_tail" "$STDERR_TAIL"

# AC6: STDERR_TAIL with only backslashes
STDERR_TAIL='\\\\'
json="{\"type\":\"container_event\",\"event\":\"agent_error\",\"exit_code\":2,\"stderr_tail\":$(json_encode "$STDERR_TAIL")}"
assert_valid_json "STDERR_TAIL with backslashes is valid JSON" "$json"

# AC7: empty REPO_URL
REPO_URL=''
json="{\"type\":\"container_event\",\"event\":\"agent_started\",\"model\":\"test\",\"repo\":$(json_encode "$REPO_URL")}"
assert_valid_json "empty REPO_URL is valid JSON" "$json"
assert_field_value "empty REPO_URL roundtrips correctly" "$json" "repo" ""

# AC8: empty BRANCH
BRANCH=''
json="{\"type\":\"container_event\",\"event\":\"clone_started\",\"repo\":\"url\",\"branch\":$(json_encode "$BRANCH")}"
assert_valid_json "empty BRANCH is valid JSON" "$json"

# EC1: REPO_URL with control characters (tab)
REPO_URL=$'https://host/path\twith\ttabs'
json="{\"type\":\"container_event\",\"event\":\"agent_started\",\"model\":\"test\",\"repo\":$(json_encode "$REPO_URL")}"
assert_valid_json "REPO_URL with tab characters is valid JSON" "$json"

# EC2: STDERR_TAIL simulating real claude error output with quotes and paths
STDERR_TAIL='Error: ENOENT: no such file or directory, open "/home/user/file \"with\" quotes.txt"
    at Object.openSync (node:fs:583:3)
    at Object.readFileSync (node:fs:451:35)'
json="{\"type\":\"container_event\",\"event\":\"agent_error\",\"exit_code\":1,\"stderr_tail\":$(json_encode "$STDERR_TAIL")}"
assert_valid_json "STDERR_TAIL simulating real error output is valid JSON" "$json"

echo ""
echo "Results: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
