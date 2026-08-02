#!/bin/bash
# check-architecture-fresh.sh — sh_test body for //:architecture_up_to_date_test.
# Fails if ARCHITECTURE.md is stale relative to the manifests it's generated from.
# Only meant to run under `bazel test` (sandboxed AXON_ROOT -- see paths.sh's
# self-location and generate-architecture.sh's BUILD_WORKSPACE_DIRECTORY override,
# which is never set here): it regenerates in place, but "in place" is the
# ephemeral sandbox copy, never the real checkout. See README.md, "Generated architecture".
set -e

# sh_test relocates its entrypoint to <package>/<target-name> in the runfiles tree,
# losing the tools/ subdirectory context a plain invocation would have -- so this
# file's own dirname no longer finds tools/lib next to it. TEST_SRCDIR/TEST_WORKSPACE
# are the standard Bazel test env vars for the runfiles root; use those instead of a
# dirname guess. Never falls back to plain dirname-based lookup: this script only
# ever runs as this sh_test's body.
_lib="$TEST_SRCDIR/$TEST_WORKSPACE/tools/lib"
source "$_lib/paths.sh"

before="$AXON_ROOT/ARCHITECTURE.md"
after="$(mktemp)"
trap 'rm -f "$after"' EXIT

# ARCHITECTURE_OUT: generate into a scratch file, never overwrite the sandboxed
# (read-only) ARCHITECTURE.md in place -- a test must never mutate anything.
ARCHITECTURE_OUT="$after" bash "$AXON_ROOT/tools/generate-architecture.sh" >/dev/null

# The "Generated: <timestamp>" line always differs -- that's not staleness.
if diff -q <(grep -v '^> Generated:' "$before") <(grep -v '^> Generated:' "$after") >/dev/null; then
  echo "ARCHITECTURE.md is up to date."
  exit 0
fi

echo "ARCHITECTURE.md is stale. Run: bazel run //:generate_architecture" >&2
diff <(grep -v '^> Generated:' "$before") <(grep -v '^> Generated:' "$after") >&2 || true
exit 1
