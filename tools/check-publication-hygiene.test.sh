#!/bin/bash
# Planted-index regression tests for check-publication-hygiene.sh. The check reads
# Git's index, so each case uses a throwaway repository and never needs private data.
set -uo pipefail

if [ -n "${TEST_SRCDIR:-}" ]; then
  CHECK="$TEST_SRCDIR/$TEST_WORKSPACE/tools/check-publication-hygiene.sh"
else
  CHECK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-publication-hygiene.sh"
fi

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
export AXON_PUBLICATION_ROOT="$SCRATCH"

git -C "$SCRATCH" init -q
printf '%s\n' 'container example: /home/agent/config' > "$SCRATCH/safe.txt"
git -C "$SCRATCH" add safe.txt

fails=0
expect_pass() {
  if "$CHECK" >/dev/null 2>&1; then :; else
    echo "FAIL: $1 should pass"; fails=$((fails + 1))
  fi
}
expect_fail_with() {
  out="$($CHECK 2>&1)"; status=$?
  if [ "$status" -eq 0 ] || ! printf '%s' "$out" | grep -qF "$2"; then
    echo "FAIL: $1 should fail and name $2"; fails=$((fails + 1))
  fi
}

expect_pass "portable container home"

mkdir -p "$SCRATCH/pkg/__pycache__"
printf '%s\n' 'compiled' > "$SCRATCH/pkg/__pycache__/module.pyc"
git -C "$SCRATCH" add -f pkg/__pycache__/module.pyc
expect_fail_with "tracked bytecode" "pkg/__pycache__/module.pyc"

git -C "$SCRATCH" rm -q --cached pkg/__pycache__/module.pyc
rm -f "$SCRATCH/pkg/__pycache__/module.pyc"
printf '%s\n' '@/''Users/private-user/Developer/project/module.py' > "$SCRATCH/leak.bin"
git -C "$SCRATCH" add leak.bin
expect_fail_with "workstation home in blob metadata" "leak.bin"

printf '%s\n' 'portable metadata' > "$SCRATCH/leak.bin"
git -C "$SCRATCH" add leak.bin
expect_pass "cleaned index"

printf '%s\n' 'private overlay: axon-family' > "$SCRATCH/instance.txt"
git -C "$SCRATCH" add instance.txt
expect_fail_with "named deployment marker" "instance.txt"

printf '%s\n' 'selected deployment overlay' > "$SCRATCH/instance.txt"
git -C "$SCRATCH" add instance.txt
expect_pass "generic deployment language"

if [ "$fails" -gt 0 ]; then
  echo "publication hygiene: $fails check(s) failed"
  exit 1
fi
echo "publication hygiene: all checks passed"
