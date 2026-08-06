#!/bin/bash
# tools/axon.test.sh — public CLI contract: discoverable without the Axon skill.
set -euo pipefail

# Under Bazel, sh_test relocates its entrypoint to <package>/<target-name> in the
# runfiles tree, so this file's own dirname is no longer tools/ and `..` lands
# outside the sources -- the same trap tools/check-architecture-fresh.sh documents.
# TEST_SRCDIR/TEST_WORKSPACE are the standard runfiles-root env vars and are the
# authority whenever they are set.
#
# The dirname fallback stays, unlike in that script, because this one is genuinely
# useful to run by hand: it is the contract a person checks after touching the CLI,
# and requiring `bazel test` to answer "is the entrypoint still discoverable" would
# put the check behind the build it is meant to be independent of.
if [ -n "${TEST_SRCDIR:-}" ] && [ -n "${TEST_WORKSPACE:-}" ]; then
  ROOT="$TEST_SRCDIR/$TEST_WORKSPACE"
else
  ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fi
AXON="$ROOT/axon"

fail() { echo "FAIL: $*" >&2; exit 1; }
contains() { case "$1" in *"$2"*) ;; *) fail "expected '$2' in: $1" ;; esac; }

[ -x "$AXON" ] || fail "axon is not executable"
[ ! -e "$ROOT/scripts/axapi" ] || fail "legacy scripts/axapi remains"
[ ! -e "$ROOT/Packs/axon/pack.toml" ] || fail "retired Axon Pack manifest remains"

out="$("$AXON" help)"
contains "$out" "capability list"
contains "$out" "pack deploy"
contains "$out" "search <words...>"

out="$("$AXON" help capability)"
contains "$out" "ingest <url>"

out="$("$AXON" help pack)"
contains "$out" "opencode"

echo "axon CLI contract passed"
