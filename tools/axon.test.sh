#!/bin/bash
# tools/axon.test.sh — public CLI contract: discoverable without the Axon skill.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
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
