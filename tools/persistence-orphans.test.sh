#!/bin/bash
# tools/doctor's orphan-unit report, over a planted HOME and a planted overlay.
#
# The check answers one question — is this com.axon.* unit one this machine should have? —
# and getting it wrong is expensive in both directions. Too strict and it tells the operator
# to remove a unit something depends on, which is exactly what it did for the dashboard's
# macmon sidecar (#65). Too loose and a capability's leftover unit keeps starting a service
# the machine no longer enables, silently, which is the whole reason the check exists.
#
# So both directions are asserted here, against the real doctor rather than a reimplementation
# of its rule.
set -uo pipefail

if [ -n "${TEST_SRCDIR:-}" ]; then
  _root="$TEST_SRCDIR/$TEST_WORKSPACE"
else
  _root="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fails=0
fail() { echo "FAIL: $1"; fails=$((fails + 1)); }

FAKE_HOME="$WORK/home"
UNIT_DIR="$FAKE_HOME/Library/LaunchAgents"
OVERLAY="$WORK/overlay"
mkdir -p "$UNIT_DIR" "$OVERLAY/config"

# One enabled capability, so the machine has a non-empty set to compare against.
cat > "$OVERLAY/config/machine.toml" <<'EOF'
os = "macos"
container_runtime = "docker"
capabilities = ["postgres"]
EOF

plant_unit() { printf '<plist/>\n' > "$UNIT_DIR/com.axon.$1.plist"; }

# The three cases the issue names, planted together so one run covers all of them.
plant_unit postgres    # enabled          -> never an orphan
plant_unit dashboard   # spine component  -> exempt via its own root manifest
plant_unit macmon      # declared sidecar -> exempt via dashboard/service.toml
plant_unit trips       # a disabled capability's leftover unit -> ORPHAN
plant_unit nothing-declares-this                                # -> ORPHAN

# Only the persistence section matters here; doctor legitimately fails other checks against a
# planted HOME, and asserting on its exit code would be asserting on those instead.
OUT="$( cd "$_root" && HOME="$FAKE_HOME" AXON_OVERLAY_ROOT="$OVERLAY" tools/doctor 2>&1 )"
PERSIST="$(printf '%s\n' "$OUT" | grep 'persistence is installed for' || true)"

flagged() { printf '%s\n' "$PERSIST" | grep -q "persistence is installed for '$1'"; }

# --- must NOT be flagged ------------------------------------------------------------------
for name in postgres dashboard macmon; do
  if flagged "$name"; then
    fail "'$name' was reported as an orphan. Persistence notes were:
$PERSIST"
  fi
done

# --- must STILL be flagged ----------------------------------------------------------------
# The half that stops the fix from being "exempt everything". Without these the check could be
# deleted entirely and this file would still pass.
for name in trips nothing-declares-this; do
  if ! flagged "$name"; then
    fail "'$name' is a genuine orphan and was not reported. Persistence notes were:
$PERSIST"
  fi
done

# --- the exemption is derived, not spelled ------------------------------------------------
# If a name that no manifest mentions is exempt, the derivation has been replaced by a list.
if ! grep -q 'sidecars' "$_root/dashboard/service.toml"; then
  fail "dashboard/service.toml no longer declares its sidecars — the exemption has moved back into code"
fi
if grep -qE '"macmon"' "$_root/tools/doctor.ts"; then
  fail "tools/doctor.ts names macmon directly — the exemption must come from the manifest"
fi

if [ "$fails" -gt 0 ]; then
  echo "persistence orphans: $fails check(s) failed"
  exit 1
fi
echo "persistence orphans: all checks passed"
