#!/bin/bash
# check-bun-pin.sh — one bun pin, three homes, and they have to agree (CI: repo gates).
#
# upstreams.toml [bun] `pin` is the source of truth, because README.md#one-manifest-per-concern
# already makes upstreams.toml the owner of every external project's pin. It held that role
# jointly with tools/bazel/bun/repositories.bzl until 2026-08-25, when PRD Q44 retired Bazel
# and took the hermetic bun toolchain with it; the .bzl was the declared truth then, so this
# gate moved rather than disappeared.
#
# The two workflows cannot read a TOML file, so each repeats the version as a BUN_VERSION
# env value. That is the divergence this gate exists for (#40): bumping one home left the
# others silently behind with CI green.
#
# Every declaration is compared, not just the first one per file, so a second BUN_VERSION
# appearing in a workflow is caught too. A home that yields no value at all fails loudly:
# a renamed key must not read as "nothing to compare".
set -e

ROOT="${AXON_BUN_PIN_ROOT:-.}"

TRUTH_FILE="$ROOT/upstreams.toml"
CI_FILE="$ROOT/.github/workflows/ci.yml"
PAGES_FILE="$ROOT/.github/workflows/pages.yml"

for f in "$TRUTH_FILE" "$CI_FILE" "$PAGES_FILE"; do
  [ -r "$f" ] || { echo "FAIL: $f is missing or unreadable" >&2; exit 1; }
done

# Source of truth: exactly one `pin` in the [bun] section, or the contract is gone.
# Section-scoped, so another table's pin can never stand in for it.
truth=$(awk '/^\[/ { in_s = ($0 == "[bun]"); next } in_s' "$TRUTH_FILE" \
          | grep -E '^pin[[:space:]]*=' | sed -E 's/.*"([^"]*)".*/\1/')
truth_count=$(printf '%s\n' "$truth" | grep -c . || true)
if [ "$truth_count" -ne 1 ]; then
  echo "FAIL: expected exactly one pin in upstreams.toml [bun], found $truth_count" >&2
  echo "      that entry is the declared source of truth; nothing else can be compared without it" >&2
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
    [ "$v" = "$truth" ] || echo "FAIL [$label]: pins $v, but upstreams.toml [bun] says $truth"
  done
}

workflow_pins() { # workflow_pins <file>
  grep -E '^[[:space:]]*BUN_VERSION[[:space:]]*:' "$1" \
    | sed -E 's/.*:[[:space:]]*"?([^"[:space:]]*)"?.*/\1/'
}

divergences=$(
  check_home ".github/workflows/ci.yml" "$(workflow_pins "$CI_FILE")"
  check_home ".github/workflows/pages.yml" "$(workflow_pins "$PAGES_FILE")"
)

if [ -n "$divergences" ]; then
  printf '%s\n' "$divergences" >&2
  echo "bun pin check FAILED — one pin, three homes, and they disagree." >&2
  exit 1
fi

echo "bun pin check passed (upstreams.toml, ci.yml and pages.yml all pin $truth)."
