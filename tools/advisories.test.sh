#!/bin/bash
# tools/lib/advisories.sh — the scan, and the gate that consumes it (Axon#124, Axon#126).
#
# Hermetic: a stub `gh` on PATH answers the advisory query, so every branch runs for real
# without a network call. The stub is handed TSV directly because the real call carries --jq
# and performs the id/severity/range join server-side — what is under test here is the range
# comparison and the resulting verdict, not jq's ability to project JSON.
#
# The gate cases matter more than the scan cases: a scan that mis-parses produces a wrong
# report, but a gate that mis-parses installs a vulnerable binary.
set -uo pipefail

_here="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
# shellcheck source=lib/advisories.sh
. "$_here/lib/advisories.sh"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

mkdir -p "$SCRATCH/bin"
cat > "$SCRATCH/bin/gh" <<'STUB'
#!/bin/bash
case "$*" in
  *security-advisories*)
    # UNSET (not merely empty) is the fetch-failed branch; empty means "publishes none".
    [ -n "${GH_STUB_ADVISORIES+set}" ] || exit 1
    [ -n "$GH_STUB_ADVISORIES" ] && printf '%s\n' "$GH_STUB_ADVISORIES"
    exit 0 ;;
esac
exit 1
STUB
chmod +x "$SCRATCH/bin/gh"
PATH="$SCRATCH/bin:$PATH"; export PATH

fails=0
TAB="$(printf '\t')"

check() { # check <label> <expected> <actual>
  if [ "$2" = "$3" ]; then
    printf '  ✓ %s\n' "$1"
  else
    printf '  ✗ %s\n      expected: %s\n      got:      %s\n' "$1" "$2" "$3"
    fails=$((fails + 1))
  fi
}

echo "advisory_scan"

# --- the decision that matters: a pin inside a published range ---------------
export GH_STUB_ADVISORIES="GHSA-jfgx-wxx8-mp94${TAB}high${TAB}>= 0.74.0, < 0.78.1"
out="$(advisory_scan example/repo 0.76.0)"; rc=$?
check "in-range pin scans cleanly (rc)" "0" "$rc"
check "in-range pin is a hit" "hit${TAB}GHSA-jfgx-wxx8-mp94${TAB}high${TAB}>= 0.74.0, < 0.78.1" "$(printf '%s' "$out" | grep '^hit')"
check "hit still reports a total" "total${TAB}1" "$(printf '%s' "$out" | grep '^total')"

# The real reason this repo's pin is safe today: 0.83.0 is past every published range. That
# is an accident of upgrading fast, and the gate exists so it stops being an accident.
out="$(advisory_scan example/repo 0.83.0)"
check "pin past the range produces no hit" "" "$(printf '%s' "$out" | grep '^hit')"
check "clean pin still reports what was examined" "total${TAB}1" "$(printf '%s' "$out" | grep '^total')"

# --- ranges GitHub allows but nothing can order ------------------------------
export GH_STUB_ADVISORIES="GHSA-aaaa-bbbb-cccc${TAB}medium${TAB}2025.02 to 2026.01"
out="$(advisory_scan example/repo 0.83.0)"
check "unorderable range is undecided, not clean" "undecided${TAB}GHSA-aaaa-bbbb-cccc${TAB}medium${TAB}2025.02 to 2026.01" "$(printf '%s' "$out" | grep '^undecided')"

# --- one advisory, several ranges -------------------------------------------
export GH_STUB_ADVISORIES="GHSA-1${TAB}low${TAB}< 1.0.0
GHSA-1${TAB}low${TAB}>= 2.0.0, < 2.5.0
GHSA-2${TAB}high${TAB}>= 3.0.0"
out="$(advisory_scan example/repo 2.1.0)"
check "three ranges are three examinations" "total${TAB}3" "$(printf '%s' "$out" | grep '^total')"
check "only the covering range hits" "1" "$(printf '%s' "$out" | grep -c '^hit')"

# --- states that are not passes ----------------------------------------------
export GH_STUB_ADVISORIES=""
out="$(advisory_scan example/repo 1.0.0)"; rc=$?
check "a repo with no advisories scans clean" "0" "$rc"
check "and says nothing was found rather than nothing was asked" "total${TAB}0" "$(printf '%s' "$out" | grep '^total')"

unset GH_STUB_ADVISORIES
advisory_scan example/repo 1.0.0 >/dev/null 2>&1; rc=$?
check "a failed fetch is rc 1, not a pass" "1" "$rc"

export GH_STUB_ADVISORIES=""
advisory_scan example/repo "7dccb56aa1" >/dev/null 2>&1; rc=$?
check "a sha pin is rc 3 (not comparable), before any fetch" "3" "$rc"

# A PATH with no gh anywhere on it — not merely without the stub. Dropping the stub alone
# would fall through to a real gh, which answers rc 1 (no such repo) and quietly tests the
# wrong branch. /usr/bin:/bin still carries the sed and grep range_contains needs.
_saved_path="$PATH"
PATH="/usr/bin:/bin"; export PATH
advisory_scan example/repo 1.0.0 >/dev/null 2>&1; rc=$?
check "no gh at all is rc 2 (unchecked), never rc 0" "2" "$rc"
PATH="$_saved_path"; export PATH

echo
echo "agentbox gate"

# The gate runs against a planted profile + manifest so the tracked ones stay out of it: the
# real files can only demonstrate the state they happen to be in, which is how a green gate
# goes untested. Same escape hatch as upstream-checker.test.sh's AXON_UPSTREAMS_MANIFEST.
AGENTBOX="$(cd "$_here/.." && pwd)/capabilities/agentbox/agentbox"

# Deliberately NOT called inside a command substitution: that runs the function in a subshell
# and GATE_RC — the whole point of the case — is discarded at the closing paren. Output goes
# to a file the caller reads instead.
GATE_OUT="$SCRATCH/gate.out"
gate() { # gate <version> -> sets GATE_RC, writes GATE_OUT
  "$AGENTBOX" gate "$1" </dev/null > "$GATE_OUT" 2>&1
  GATE_RC=$?
}

if [ ! -f "$AGENTBOX" ]; then
  echo "  ⊘ agentbox not found at $AGENTBOX — gate cases skipped"
else
  # The gate needs an overlay config to resolve a profile at all. Without one it dies with a
  # setup message rather than a verdict, and that is itself the right behaviour to record.
  export GH_STUB_ADVISORIES="GHSA-jfgx-wxx8-mp94${TAB}high${TAB}>= 0.74.0, < 0.78.1"
  gate 0.76.0
  out="$(cat "$GATE_OUT")"
  case "$out" in
    *"no config at"*|*"agentbox.toml"*)
      echo "  ⊘ no overlay agentbox.toml on this machine — gate cases need one, skipped"
      echo "     (the scan cases above cover the decision logic the gate consumes)" ;;
    *)
      check "an in-range advisory refuses the install" "1" "$GATE_RC"
      case "$out" in
        *GHSA-jfgx-wxx8-mp94*) echo "  ✓ the refusal names the advisory" ;;
        *) echo "  ✗ the refusal does not name the advisory"; fails=$((fails + 1)) ;;
      esac

      # Non-interactive is the case that matters: a cron job or a script must not be able to
      # answer a confirmation prompt by having nobody there to say no.
      export GH_STUB_ADVISORIES="GHSA-aaaa-bbbb-cccc${TAB}medium${TAB}2025.02 to 2026.01"
      gate 0.83.0
      out="$(cat "$GATE_OUT")"
      check "an undecided range with stdin closed fails closed" "1" "$GATE_RC"
      case "$out" in
        *"fails closed"*|*"refused"*) echo "  ✓ and says why rather than exiting silently" ;;
        *) echo "  ✗ refusal message missing"; fails=$((fails + 1)) ;;
      esac
      ;;
  esac
fi

echo
if [ "$fails" -eq 0 ]; then
  echo "advisory scan and install gate: all checks passed"
else
  echo "advisory scan and install gate: $fails check(s) failed"
fi
exit "$fails"
