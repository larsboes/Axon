#!/bin/bash
# check-bun-pin.sh — sh_test body for //:bun_pin_equality_test.
# The bun version is written in three files. tools/bazel/bun/repositories.bzl is
# the declared source of truth (both the .bzl and .github/workflows/ci.yml say so
# in comments), and until 2026-08-02 nothing enforced it: bumping one left the
# others silently divergent and CI stayed green.
#
# The workflow lives outside every package glob, so it is named explicitly in the
# sh_test's `data` rather than globbed -- that is the whole reason this gate did
# not already exist.
#
# Every declaration is compared, not just the first one per file, so a second
# BUN_VERSION appearing in ci.yml is caught too. A home that yields no value at
# all fails loudly: a renamed key must not read as "nothing to compare".
set -e

ROOT="${AXON_BUN_PIN_ROOT:-.}"

TRUTH_FILE="$ROOT/tools/bazel/bun/repositories.bzl"
CI_FILE="$ROOT/.github/workflows/ci.yml"
UPSTREAMS_FILE="$ROOT/upstreams.toml"

for f in "$TRUTH_FILE" "$CI_FILE" "$UPSTREAMS_FILE"; do
  [ -r "$f" ] || { echo "FAIL: $f is missing or unreadable" >&2; exit 1; }
done

# Source of truth: exactly one BUN_VERSION assignment, or the contract is gone.
truth=$(grep -E '^BUN_VERSION[[:space:]]*=' "$TRUTH_FILE" | sed -E 's/.*"([^"]*)".*/\1/')
truth_count=$(printf '%s\n' "$truth" | grep -c . || true)
if [ "$truth_count" -ne 1 ]; then
  echo "FAIL: expected exactly one BUN_VERSION in tools/bazel/bun/repositories.bzl, found $truth_count" >&2
  echo "      that file is the declared source of truth; nothing else can be compared without it" >&2
  exit 1
fi

# Each other home. Every finding goes to stdout and is collected by the caller:
# this runs inside a command substitution, so a `fail=1` assigned here would be
# set in a subshell and lost — the same silent-green shape this gate exists to
# catch, and how the first draft of it passed its own red-path test.
check_home() { # check_home <label> <extractor-output>
  local label="$1" values="$2" n
  n=$(printf '%s\n' "$values" | grep -c . || true)
  if [ "$n" -eq 0 ]; then
    echo "FAIL [$label]: declares no bun pin — the key was renamed or removed, so nothing was compared"
    return
  fi
  printf '%s\n' "$values" | while IFS= read -r v; do
    [ -n "$v" ] || continue
    [ "$v" = "$truth" ] || echo "FAIL [$label]: pins $v, but repositories.bzl says $truth"
  done
}

divergences=$(
  check_home ".github/workflows/ci.yml" \
    "$(grep -E '^[[:space:]]*BUN_VERSION[[:space:]]*:' "$CI_FILE" | sed -E 's/.*:[[:space:]]*"?([^"[:space:]]*)"?.*/\1/')"
  check_home "upstreams.toml [bun]" \
    "$(awk '/^\[/ { in_s = ($0 == "[bun]"); next } in_s' "$UPSTREAMS_FILE" \
        | grep -E '^pin[[:space:]]*=' | sed -E 's/.*"([^"]*)".*/\1/')"
)

if [ -n "$divergences" ]; then
  printf '%s\n' "$divergences" >&2
  echo "bun pin check FAILED — one pin, three homes, and they disagree." >&2
  exit 1
fi

echo "bun pin check passed (repositories.bzl, ci.yml and upstreams.toml all pin $truth)."
