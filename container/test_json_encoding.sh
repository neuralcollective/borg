#!/usr/bin/env bash
# Tests that the bun-based JSON encoding used in entrypoint.sh produces valid JSON
# for strings that would break the old sed-based approach.
set -euo pipefail

failures=0

json_encode() {
    printf '%s' "$1" | bun -e "let s='';process.stdin.on('data',c=>s+=c);process.stdin.on('end',()=>process.stdout.write(JSON.stringify(s)));"
}

check_valid_json() {
    local desc="$1"
    local input="$2"
    local encoded
    encoded=$(json_encode "$input")
    # Verify it parses as a JSON string and round-trips correctly
    local decoded
    decoded=$(printf '%s' "$encoded" | bun -e "
let s='';process.stdin.on('data',c=>s+=c);process.stdin.on('end',()=>{
    const v=JSON.parse(s.trim());
    if(typeof v!=='string'){process.stderr.write('not a string\n');process.exit(1);}
    process.stdout.write(v);
});" 2>/dev/null)
    if [ "$decoded" = "$input" ]; then
        echo "PASS: $desc"
    else
        echo "FAIL: $desc"
        echo "  input:   $(printf '%s' "$input" | cat -v)"
        echo "  encoded: $encoded"
        echo "  decoded: $(printf '%s' "$decoded" | cat -v)"
        failures=$((failures + 1))
    fi
}

check_valid_json "simple string" "hello world"
check_valid_json "backslash" 'path\to\file'
check_valid_json "backslash-n sequence" 'error: \n unexpected token'
check_valid_json "backslash-e ANSI escape" $'error: \e[31mred\e[0m'
check_valid_json "double quotes" 'she said "hello"'
check_valid_json "backslash then quote" '\"quoted\"'
check_valid_json "newline characters" "$(printf 'line1\nline2\nline3')"
check_valid_json "tab character" "$(printf 'col1\tcol2')"
check_valid_json "null byte adjacent" "$(printf 'before\x01after')"
check_valid_json "unicode" "error: こんにちは world"
check_valid_json "mixed backslashes and quotes" 'C:\Users\"name"\dir'

if [ "$failures" -gt 0 ]; then
    echo ""
    echo "$failures test(s) FAILED"
    exit 1
else
    echo ""
    echo "All tests passed"
fi
