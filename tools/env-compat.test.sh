#!/bin/bash
# tools/lib/env-compat.sh: a setting exported under one name is visible under the other, the
# name already set is never overwritten, and it behaves the same under bash and zsh.
set -euo pipefail
LIB="$(cd "$(dirname "$0")/lib" && pwd)/env-compat.sh"
fail() { echo "FAIL: $*" >&2; exit 1; }
for sh in bash zsh; do
  command -v "$sh" >/dev/null || { echo "  ⊘ $sh not on PATH, skipped"; continue; }
  out="$(env -i PATH="$PATH" AXON_ONLY_OLD=old SJEL_ONLY_NEW=new AXON_BOTH=old SJEL_BOTH=new \
    "$sh" -c ". '$LIB'; echo \"\$SJEL_ONLY_OLD|\$AXON_ONLY_NEW|\$SJEL_BOTH|\$AXON_BOTH\"")"
  [ "$out" = "old|new|new|old" ] || fail "$sh: got '$out', expected 'old|new|new|old'"
  child="$(env -i PATH="$PATH" AXON_ONLY_OLD=old "$sh" -c ". '$LIB'; env | grep '^SJEL_ONLY_OLD='")"
  [ "$child" = "SJEL_ONLY_OLD=old" ] || fail "$sh: the new name was not exported to children"
done
echo "env-compat: all checks passed"
