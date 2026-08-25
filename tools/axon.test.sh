#!/bin/bash
# tools/axon.test.sh — public CLI contract: discoverable without the Axon skill.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AXON="$ROOT/axon"

fail() { echo "FAIL: $*" >&2; exit 1; }
contains() { case "$1" in *"$2"*) ;; *) fail "expected '$2' in: $1" ;; esac; }

[ -x "$AXON" ] || fail "axon is not executable"
[ ! -e "$ROOT/scripts/axapi" ] || fail "legacy scripts/axapi remains"

# `Packs/axon/pack.toml` was asserted absent here from #102 (the CLI port retired the
# Pack) until 2026-08-25. #137 gave the Axon Pack a dedicated deployer and put the
# manifest back, and this line should have gone with it. It did not, and the assertion
# stayed green for two months because the sh_test declared only `axon` and
# `tools/axon-context` as data: the sandbox never materialized the file the test was
# looking for, so `[ ! -e ... ]` was true about the sandbox and false about the repo.
# PRD Q44 (2026-08-25) retired Bazel, this test started reading the real checkout, and
# it went red on the first run. Dropped rather than inverted — the CLI contract below
# is what this file is for; where the Pack lives is Packs/axon/README.md's fact.

out="$("$AXON" help)"
contains "$out" "capability list"
contains "$out" "pack deploy"
contains "$out" "search <words...>"

out="$("$AXON" help capability)"
contains "$out" "ingest <url>"

out="$("$AXON" help pack)"
contains "$out" "opencode"

echo "axon CLI contract passed"
