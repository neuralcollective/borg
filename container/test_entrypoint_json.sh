#!/bin/bash
# Tests that agent_error log event produces valid JSON for tricky stderr content.
# Uses the same bun JSON.stringify pattern as the fix in entrypoint.sh.

PASS=0
FAIL=0

check_valid_json() {
    local desc="$1"
    local input="$2"

    local escaped
    escaped=$(printf '%s' "$input" | bun -e "
let s='';process.stdin.on('data',c=>s+=c);process.stdin.on('end',()=>process.stdout.write(JSON.stringify(s)));
")

    local json="{\"type\":\"container_event\",\"event\":\"agent_error\",\"exit_code\":1,\"stderr_tail\":${escaped}}"

    if printf '%s' "$json" | bun -e "
let s='';process.stdin.on('data',c=>s+=c);process.stdin.on('end',()=>{
  try{JSON.parse(s);process.exit(0);}
  catch(e){process.stderr.write(e.message+'\n');process.exit(1);}
});
" 2>/dev/null; then
        echo "PASS: $desc"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $desc"
        printf '  input:  %s\n' "$input"
        printf '  json:   %s\n' "$json"
        FAIL=$((FAIL + 1))
    fi
}

check_valid_json "plain text" "some error message"
check_valid_json "backslash in path" 'error at C:\Users\foo\bar.exe'
check_valid_json "backslash-n sequence" $'line1\nline2\nline3'
check_valid_json "double quotes" 'error: "unexpected" token'
check_valid_json "backslash then quote" 'msg: \"escaped\"'
check_valid_json "mixed special chars" $'error at "file\\path"\n  caused by: \\n extra'
check_valid_json "tab character" $'col1\tcol2\tcol3'
check_valid_json "control characters" $'before\x01\x02after'
check_valid_json "empty string" ""

echo ""
echo "Results: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
