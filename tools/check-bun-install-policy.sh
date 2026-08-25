#!/bin/bash
# check-bun-install-policy.sh — the frontend install boundary must never execute lifecycle hooks.
#
# ChainDrop used an npm preinstall hook, before the dependency tree had finished installing. Axon's
# frontend packages are fully resolved by committed Bun lockfiles, and nothing that installs them
# needs lifecycle hooks. Keep that invariant in every place that performs or teaches a frontend
# dependency install.
#
# tools/bazel/bun/deps.bzl was the fourth such place until 2026-08-25, when PRD Q44 retired the
# hermetic bun toolchain along with the rest of Bazel. pages.yml and the soundscape README took
# its place in this list because they install and teach the same thing — pages.yml had been
# doing so unchecked.
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
require ".github/workflows/pages.yml" "bun install --frozen-lockfile --ignore-scripts"
require "dashboard/README.md" "bun install --frozen-lockfile --ignore-scripts"
require "capabilities/soundscape/README.md" "bun install --frozen-lockfile --ignore-scripts"

if [ "$fail" -ne 0 ]; then
  echo "bun install policy FAILED — dependency lifecycle hooks must stay disabled." >&2
  exit 1
fi

echo "bun install policy passed (both workflows and both operator READMEs disable lifecycle hooks)."
