#!/bin/bash
# check-bun-install-policy.sh — the frontend install boundary must never execute lifecycle hooks.
#
# ChainDrop used an npm preinstall hook, before the dependency tree had finished installing. Axon's
# frontend packages are fully resolved by committed Bun lockfiles, and neither CI type checks nor
# the Bazel external repository need lifecycle hooks. Keep that invariant in the three places that
# perform or teach dependency installation.
set -eu

ROOT="${AXON_BUN_INSTALL_POLICY_ROOT:-.}"
fail=0

require() { # require <path> <literal>
  local path="$1" literal="$2"
  if ! grep -Fq -- "$literal" "$ROOT/$path"; then
    echo "FAIL: $path must contain $literal" >&2
    fail=1
  fi
}

require ".github/workflows/ci.yml" "bun install --frozen-lockfile --ignore-scripts"
require "tools/bazel/bun/deps.bzl" '"--ignore-scripts"'
require "dashboard/README.md" "bun install --frozen-lockfile --ignore-scripts"

if [ "$fail" -ne 0 ]; then
  echo "bun install policy FAILED — dependency lifecycle hooks must stay disabled." >&2
  exit 1
fi

echo "bun install policy passed (CI, Bazel, and operator documentation disable lifecycle hooks)."
