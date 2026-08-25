#!/bin/bash
# Planted-tree regression tests for the lifecycle-script install policy.
set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-bun-install-policy.sh"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
fails=0

plant() { # plant <name> <ci-install>
  local root="$SCRATCH/$1"
  mkdir -p "$root/.github/workflows" "$root/dashboard" "$root/capabilities/soundscape"
  printf '%s\n' "$2" > "$root/.github/workflows/ci.yml"
  printf 'bun install --frozen-lockfile --ignore-scripts\n' > "$root/.github/workflows/pages.yml"
  printf 'bun install --frozen-lockfile --ignore-scripts\n' > "$root/dashboard/README.md"
  printf 'bun install --frozen-lockfile --ignore-scripts\n' > "$root/capabilities/soundscape/README.md"
  printf '%s' "$root"
}

expect() { # expect <name> <status> <root>
  local out status
  out="$(AXON_BUN_INSTALL_POLICY_ROOT="$3" "$CHECK" 2>&1)"; status=$?
  if [ "$status" -ne "$2" ]; then
    echo "FAIL: $1 expected exit $2, got $status:" >&2
    printf '%s\n' "$out" >&2
    fails=$((fails + 1))
  fi
}

expect "safe installs pass" 0 "$(plant safe 'bun install --frozen-lockfile --ignore-scripts')"
expect "CI install without hook protection fails" 1 "$(plant unsafe 'bun install --frozen-lockfile')"

if [ "$fails" -ne 0 ]; then
  echo "bun install policy test: $fails check(s) failed" >&2
  exit 1
fi
echo "bun install policy test: all checks passed"
