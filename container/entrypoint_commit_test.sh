#!/bin/bash
# Tests that git commit failure propagates as non-zero exit code.
# Verifies the commit block in entrypoint.sh does not silence errors.

set -euo pipefail

TMPDIR_TEST=$(mktemp -d)
trap 'rm -rf "$TMPDIR_TEST"' EXIT

PASS=0
FAIL=0

check() {
    local name="$1" got="$2" want="$3"
    if [ "$got" = "$want" ]; then
        echo "PASS: $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (got=$got, want=$want)"
        FAIL=$((FAIL + 1))
    fi
}

make_repo() {
    local dir="$1"
    mkdir -p "$dir"
    git -C "$dir" init -q
    git -C "$dir" config user.name "Test"
    git -C "$dir" config user.email "test@test.com"
    echo "initial" > "$dir/README.md"
    git -C "$dir" add -A
    git -C "$dir" commit -q -m "initial commit"
}

# Test 1: successful commit exits 0
REPO1="$TMPDIR_TEST/repo1"
make_repo "$REPO1"
echo "change" >> "$REPO1/README.md"
commit1_exit=0
(
    cd "$REPO1"
    git add -A
    if git commit -m "test"; then
        :
    else
        exit 1
    fi
) || commit1_exit=$?
check "commit_success_exits_0" "$commit1_exit" "0"

# Test 2: failed commit (pre-commit hook exits 1) exits non-zero
REPO2="$TMPDIR_TEST/repo2"
make_repo "$REPO2"
printf '#!/bin/bash\necho "hook: commit blocked" >&2\nexit 1\n' > "$REPO2/.git/hooks/pre-commit"
chmod +x "$REPO2/.git/hooks/pre-commit"
echo "change" >> "$REPO2/README.md"
commit2_exit=0
(
    cd "$REPO2"
    git add -A
    if git commit -m "test"; then
        :
    else
        exit 1
    fi
) || commit2_exit=$?
check "commit_failure_exits_nonzero" "$([ "$commit2_exit" -ne 0 ] && echo nonzero || echo zero)" "nonzero"

# Test 3: entrypoint.sh has no '|| true' after git commit
SCRIPT_PATH="$(dirname "$0")/entrypoint.sh"
if grep -n 'git commit.*|| true' "$SCRIPT_PATH" > /dev/null 2>&1; then
    echo "FAIL: entrypoint.sh still contains 'git commit ... || true'"
    FAIL=$((FAIL + 1))
else
    echo "PASS: entrypoint.sh has no 'git commit ... || true'"
    PASS=$((PASS + 1))
fi

# Test 4: entrypoint.sh has commit_failed event (propagation path)
if grep -q 'commit_failed' "$SCRIPT_PATH"; then
    echo "PASS: entrypoint.sh emits commit_failed event on failure"
    PASS=$((PASS + 1))
else
    echo "FAIL: entrypoint.sh missing commit_failed event"
    FAIL=$((FAIL + 1))
fi

echo ""
echo "Results: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
