#!/bin/bash
# Planted-tree regression tests for check-bun-pin.sh. The real pin homes agree, so on the
# real tree the gate can only ever prove its green path — the exact gap that let #37 ship
# a gate whose red path had never run. Each case here builds a scratch tree via
# AXON_BUN_PIN_ROOT instead.
set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-bun-pin.sh"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

fails=0

plant() { # plant <case> <upstreams-pin> <ci-body> <pages-body> -> echoes tree root
  local root="$SCRATCH/$1"
  rm -rf "$root"
  mkdir -p "$root/.github/workflows"
  printf '[bun]\nurl = "https://github.com/oven-sh/bun"\npin = "%s"\n\n[other]\npin = "9.9.9"\n' "$2" \
    > "$root/upstreams.toml"
  printf 'env:\n%s\n' "$3" > "$root/.github/workflows/ci.yml"
  printf 'env:\n%s\n' "$4" > "$root/.github/workflows/pages.yml"
  printf '%s' "$root"
}

run() { out=$(AXON_BUN_PIN_ROOT="$1" "$CHECK" 2>&1); status=$?; }

expect_pass() {
  run "$2"
  if [ "$status" -ne 0 ]; then
    echo "FAIL: $1 should pass, got exit $status:"; printf '%s\n' "$out" | sed 's/^/    /'
    fails=$((fails + 1))
  fi
}

expect_fail_with() {
  run "$2"
  if [ "$status" -eq 0 ] || ! printf '%s' "$out" | grep -qF "$3"; then
    echo "FAIL: $1 should fail and name '$3', got exit $status:"; printf '%s\n' "$out" | sed 's/^/    /'
    fails=$((fails + 1))
  fi
}

# All three agree — the state the real tree is in, so a gate that fails
# everything cannot pass this file.
expect_pass "three homes in agreement" \
  "$(plant agree 1.3.14 '  BUN_VERSION: "1.3.14"' '  BUN_VERSION: "1.3.14"')"

# The bug the issue reports: bump the record, a workflow stays behind, CI green.
expect_fail_with "ci workflow left behind by a bump" \
  "$(plant ci-behind 1.4.0 '  BUN_VERSION: "1.3.14"' '  BUN_VERSION: "1.4.0"')" \
  ".github/workflows/ci.yml"

# pages.yml was an unchecked home until 2026-08-25: it hardcoded the version while the
# gate compared only ci.yml and the (now retired) .bzl.
expect_fail_with "pages workflow left behind by a bump" \
  "$(plant pages-behind 1.4.0 '  BUN_VERSION: "1.4.0"' '  BUN_VERSION: "1.3.14"')" \
  ".github/workflows/pages.yml"

# Both behind: the message must name both, not stop at the first.
root=$(plant both-behind 1.4.0 '  BUN_VERSION: "1.3.14"' '  BUN_VERSION: "1.3.14"')
run "$root"
if ! printf '%s' "$out" | grep -qF "ci.yml" || ! printf '%s' "$out" | grep -qF "pages.yml"; then
  echo "FAIL: two divergent homes should both be named:"; printf '%s\n' "$out" | sed 's/^/    /'
  fails=$((fails + 1))
fi

# A second declaration sneaking into a workflow is compared too, not just the first.
expect_fail_with "a second BUN_VERSION in the workflow" \
  "$(plant ci-second 1.3.14 '  BUN_VERSION: "1.3.14"
  BUN_VERSION: "1.2.0"' '  BUN_VERSION: "1.3.14"')" \
  "1.2.0"

# Silent-green guards: a renamed key must fail loudly rather than compare nothing.
expect_fail_with "workflow key renamed away" \
  "$(plant ci-renamed 1.3.14 '  BUN_RELEASE: "1.3.14"' '  BUN_VERSION: "1.3.14"')" \
  "declares no bun pin"

# The source of truth itself disappearing must be loud, not a zero-comparison pass.
root=$(plant truth-gone 1.3.14 '  BUN_VERSION: "1.3.14"' '  BUN_VERSION: "1.3.14"')
printf '[bun]\nversion = "1.3.14"\n' > "$root/upstreams.toml"
expect_fail_with "source of truth renamed" "$root" "expected exactly one pin"

# ...and so must a duplicated one, which would make "the" pin ambiguous.
root=$(plant truth-doubled 1.3.14 '  BUN_VERSION: "1.3.14"' '  BUN_VERSION: "1.3.14"')
printf '[bun]\npin = "1.3.14"\npin = "1.4.0"\n' > "$root/upstreams.toml"
expect_fail_with "source of truth declared twice" "$root" "found 2"

# The [bun] section is read section-scoped: another table's pin must not be picked up.
root=$(plant other-section 1.3.14 '  BUN_VERSION: "1.3.14"' '  BUN_VERSION: "1.3.14"')
expect_pass "a different upstreams section pinning something else" "$root"

if [ "$fails" -gt 0 ]; then
  echo "bun pin gate: $fails check(s) failed"
  exit 1
fi
echo "bun pin gate: all checks passed"
