#!/bin/bash
# Planted-tree regression tests for check-bun-pin.sh. The gate's `data` is the
# three real pin homes, which agree — so on its own it can only ever prove the
# green path, the exact gap that let #37 ship a gate whose red path had never
# run. Each case here builds a scratch tree via AXON_BUN_PIN_ROOT instead.
set -uo pipefail

if [ -n "${TEST_SRCDIR:-}" ]; then
  CHECK="$TEST_SRCDIR/$TEST_WORKSPACE/tools/check-bun-pin.sh"
else
  CHECK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-bun-pin.sh"
fi

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

fails=0

plant() { # plant <case> <bzl-version> <ci-body> <upstreams-pin> -> echoes tree root
  local root="$SCRATCH/$1"
  rm -rf "$root"
  mkdir -p "$root/tools/bazel/bun" "$root/.github/workflows"
  printf 'BUN_VERSION = "%s"\n' "$2" > "$root/tools/bazel/bun/repositories.bzl"
  printf 'env:\n%s\n' "$3" > "$root/.github/workflows/ci.yml"
  printf '[bun]\nurl = "https://github.com/oven-sh/bun"\npin = "%s"\n\n[other]\npin = "9.9.9"\n' "$4" \
    > "$root/upstreams.toml"
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
  "$(plant agree 1.3.14 '  BUN_VERSION: "1.3.14"' 1.3.14)"

# The bug the issue reports: bump one home, the others stay behind, CI green.
expect_fail_with "workflow left behind by a bump" \
  "$(plant ci-behind 1.4.0 '  BUN_VERSION: "1.3.14"' 1.4.0)" \
  ".github/workflows/ci.yml"

expect_fail_with "upstreams.toml left behind by a bump" \
  "$(plant upstreams-behind 1.4.0 '  BUN_VERSION: "1.4.0"' 1.3.14)" \
  "upstreams.toml [bun]"

# Both behind: the message must name both, not stop at the first.
root=$(plant both-behind 1.4.0 '  BUN_VERSION: "1.3.14"' 1.3.14)
run "$root"
if ! printf '%s' "$out" | grep -qF "ci.yml" || ! printf '%s' "$out" | grep -qF "upstreams.toml"; then
  echo "FAIL: two divergent homes should both be named:"; printf '%s\n' "$out" | sed 's/^/    /'
  fails=$((fails + 1))
fi

# A second declaration sneaking into the workflow is compared too, not just the first.
expect_fail_with "a second BUN_VERSION in the workflow" \
  "$(plant ci-second 1.3.14 '  BUN_VERSION: "1.3.14"
  BUN_VERSION: "1.2.0"' 1.3.14)" \
  "1.2.0"

# Silent-green guards: a renamed key must fail loudly rather than compare nothing.
expect_fail_with "workflow key renamed away" \
  "$(plant ci-renamed 1.3.14 '  BUN_RELEASE: "1.3.14"' 1.3.14)" \
  "declares no bun pin"

root=$(plant upstreams-renamed 1.3.14 '  BUN_VERSION: "1.3.14"' 1.3.14)
printf '[bun]\nversion = "1.3.14"\n' > "$root/upstreams.toml"
expect_fail_with "upstreams pin renamed away" "$root" "declares no bun pin"

# The source of truth itself disappearing must be loud, not a zero-comparison pass.
root=$(plant truth-gone 1.3.14 '  BUN_VERSION: "1.3.14"' 1.3.14)
printf 'BUN_RELEASE = "1.3.14"\n' > "$root/tools/bazel/bun/repositories.bzl"
expect_fail_with "source of truth renamed" "$root" "expected exactly one BUN_VERSION"

# ...and so must a duplicated one, which would make "the" pin ambiguous.
root=$(plant truth-doubled 1.3.14 '  BUN_VERSION: "1.3.14"' 1.3.14)
printf 'BUN_VERSION = "1.3.14"\nBUN_VERSION = "1.4.0"\n' > "$root/tools/bazel/bun/repositories.bzl"
expect_fail_with "source of truth declared twice" "$root" "found 2"

# The [bun] section is read section-scoped: another table's pin must not be picked up.
root=$(plant other-section 1.3.14 '  BUN_VERSION: "1.3.14"' 1.3.14)
expect_pass "a different upstreams section pinning something else" "$root"

if [ "$fails" -gt 0 ]; then
  echo "bun pin gate: $fails check(s) failed"
  exit 1
fi
echo "bun pin gate: all checks passed"
