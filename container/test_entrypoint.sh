#!/bin/bash
# Tests for entrypoint.sh git option injection prevention
set -e

SCRIPT="$(dirname "$0")/entrypoint.sh"
pass=0
fail=0

check() {
    local desc="$1"
    local result="$2"
    if [ "$result" = "ok" ]; then
        echo "PASS: $desc"
        (( pass++ )) || true
    else
        echo "FAIL: $desc"
        (( fail++ )) || true
    fi
}

# Verify git checkout uses -- before branch name arguments
if grep -qP 'git checkout -b -- "\$BRANCH" "\$BASE"' "$SCRIPT"; then
    check "git checkout uses -- before BRANCH and BASE" "ok"
else
    check "git checkout uses -- before BRANCH and BASE" "fail"
fi

# Verify git push uses -- before branch name argument
if grep -qP 'git push origin -- "\$BRANCH"' "$SCRIPT"; then
    check "git push uses -- before BRANCH" "ok"
else
    check "git push uses -- before BRANCH" "fail"
fi

# Verify the old vulnerable patterns are not present
if grep -qP 'git checkout -b "\$BRANCH" "\$BASE"' "$SCRIPT"; then
    check "no bare git checkout -b without --" "fail"
else
    check "no bare git checkout -b without --" "ok"
fi

if grep -qP 'git push origin "\$BRANCH"' "$SCRIPT"; then
    check "no bare git push without --" "fail"
else
    check "no bare git push without --" "ok"
fi

echo ""
echo "Results: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
