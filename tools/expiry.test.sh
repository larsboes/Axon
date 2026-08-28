#!/bin/bash
# Unit tests for tools/lib/expiry.sh — the classification behind tools/audit's expiry clock.
#
# tools/audit.test.sh already drives this end to end, and keeps doing so: that is what proves
# the wiring, the exit codes and the summary lines. What it cannot do cheaply is the matrix —
# every case there costs a fixture root, a stub paths.sh and a scanner mocked onto PATH. The
# boundaries below are one function call each, which is the whole reason the helpers were
# lifted out of the script (same argument lib/version.sh's header makes about network calls).
#
# Every date is generated relative to `date` at run time. A literal would pass today and start
# failing on its own expiry — the bug under test wearing a different hat.
set -uo pipefail

_lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/lib"
# shellcheck source=lib/expiry.sh
. "$_lib/expiry.sh"

fails=0
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

day_offset() {  # day_offset <+N|-N> — an ISO date N days from today, UTC. GNU then BSD.
  if date --version >/dev/null 2>&1; then
    date -u -d "$1 days" +%Y-%m-%d
  else
    date -u -v"$1"d +%Y-%m-%d
  fi
}

# note <expected rc> <expected substring> <label> <total> <warn> <dates…>
note() {
  local want_rc="$1" needle="$2"; shift 2
  local out got
  out="$(expiry_note "$@")"; got=$?
  if [ "$got" != "$want_rc" ]; then
    echo "FAIL: [$needle] expected rc $want_rc, got $got"
    echo "      $out"
    fails=$((fails + 1))
    return
  fi
  case "$out" in
    *"$needle"*) ;;
    *) echo "FAIL: output did not contain '$needle'"
       echo "      $out"
       fails=$((fails + 1)) ;;
  esac
}

# --- the three states, at a 14-day window ---------------------------------------------
note 0 "expires $(day_offset +90) (90d left)"          policy 1 14 "$(day_offset +90)"
note 1 "inside the 14d re-decision window"             policy 1 14 "$(day_offset +14)"
note 2 "EXPIRED $(day_offset -1) (1d ago"              policy 1 14 "$(day_offset -1)"

# --- the boundary, which is where an off-by-one would live ----------------------------
# Inclusive: a policy expiring exactly `warn` days out is already inside the window.
note 1 "inside the 14d re-decision window"             policy 1 14 "$(day_offset +14)"
note 0 "(15d left)"                                    policy 1 14 "$(day_offset +15)"
# Today is 0 days, not -1 and not expired. The reason day_epoch pins 00:00:00 on both sides:
# without it this case flips depending on the hour the audit runs.
note 1 "(0d left"                                      policy 1 14 "$(day_offset +0)"
note 2 "EXPIRED"                                       policy 1 14 "$(day_offset -1)"

# --- the nearest date governs, not the first, the last, or the majority ---------------
note 2 "1 of 3 dated entries"    policy 3 14 "$(day_offset +90)" "$(day_offset -2)" "$(day_offset +45)"
note 1 "expires $(day_offset +2)" policy 3 14 "$(day_offset +90)" "$(day_offset +2)" "$(day_offset +45)"
note 0 "expires $(day_offset +45)" policy 2 14 "$(day_offset +90)" "$(day_offset +45)"

# --- undated entries ------------------------------------------------------------------
# The count is derived from the total, so an entry carrying no date is visible rather than
# silently absent. It outranks a merely-near date: nothing will ever re-decide it.
note 1 "2 undated and therefore permanent"             policy 3 14 "$(day_offset +90)"
note 1 "no entry carries a date"                       policy 4 14
# ...but a lapsed date still outranks an undated entry: one is dead config, the other is a
# finding the scanner has already started reporting again.
note 2 "EXPIRED"                                       policy 3 14 "$(day_offset -5)"

# --- the window is a parameter, so the same policy changes verdict with it ------------
note 0 "(20d left)"                                    policy 1 14 "$(day_offset +20)"
note 1 "inside the 30d re-decision window"             policy 1 30 "$(day_offset +20)"
note 0 "(20d left)"                                    policy 1 0  "$(day_offset +20)"

# --- a malformed date is undated, never a number --------------------------------------
# grep matches `exp:2026-13-45` structurally; day_epoch is what refuses it. Reported as
# undated rather than turned into arithmetic on a date that does not exist.
note 1 "no entry carries a date"                       policy 1 14 "2026-13-45"
note 1 "1 undated and therefore permanent"             policy 2 14 "$(day_offset +90)" "2026-13-45"

# --- the readers, over planted files ---------------------------------------------------
# expiry_dates_trivy was tested here too, over a planted trivy-ignore file. Both the
# function and the format it read are gone (52aa8c5 emptied trivy-ignore/ on 2026-08-28),
# so only the osv reader remains.

cat > "$SCRATCH/osv.toml" <<OSV
# header comment
[[IgnoredVulns]]
id = "RUSTSEC-0000-0001"
ignoreUntil = 2026-11-01
reason = "not a date: 2020-01-01 appears in prose here"

[[IgnoredVulns]]
id = "RUSTSEC-0000-0002"
ignoreUntil = 2026-12-01
OSV
got="$(expiry_dates_osv "$SCRATCH/osv.toml" | tr '\n' ' ')"
# The prose date in `reason` must NOT be collected: only ignoreUntil lines are dates.
[ "$got" = "2026-11-01 2026-12-01 " ] || {
  echo "FAIL: expiry_dates_osv returned '$got' (a prose date may have leaked in)"
  fails=$((fails + 1)); }
got="$(osv_ignored_count "$SCRATCH/osv.toml")"
[ "$got" = "2" ] || { echo "FAIL: osv_ignored_count returned '$got'"; fails=$((fails + 1)); }

# A policy file with no exceptions at all counts zero rather than erroring — grep -c exits 1
# on no match, and the caller guards on the count.
printf '# nothing accepted here\n' > "$SCRATCH/empty.toml"
got="$(osv_ignored_count "$SCRATCH/empty.toml")"
[ "$got" = "0" ] || { echo "FAIL: empty policy counted '$got'"; fails=$((fails + 1)); }

# --- days_until ------------------------------------------------------------------------
[ "$(days_until "$(day_offset +7)")" = "7" ]   || { echo "FAIL: days_until +7"; fails=$((fails + 1)); }
[ "$(days_until "$(day_offset -7)")" = "-7" ]  || { echo "FAIL: days_until -7"; fails=$((fails + 1)); }
[ "$(days_until "$(day_offset +0)")" = "0" ]   || { echo "FAIL: days_until today"; fails=$((fails + 1)); }
days_until "not-a-date" >/dev/null 2>&1
[ $? -ne 0 ] || { echo "FAIL: days_until accepted a non-date"; fails=$((fails + 1)); }

# --- nothing leaks into the caller's scope ---------------------------------------------
# The inline version declared none of these local and clobbered names tools/audit uses
# elsewhere (`d`, `label`, `total`). Nothing broke, purely because of the order the calls
# happened to run in. This is the assertion that keeps it that way.
d="sentinel-d"; label="sentinel-label"; total="sentinel-total"; days="sentinel-days"
expiry_note policy 1 14 "$(day_offset +90)" >/dev/null
for v in d label total days; do
  eval "cur=\$$v"
  [ "$cur" = "sentinel-$v" ] || {
    echo "FAIL: expiry_note clobbered \$$v (now '$cur')"; fails=$((fails + 1)); }
done

if [ "$fails" -gt 0 ]; then
  echo "expiry.sh: $fails check(s) failed"
  exit 1
fi
echo "expiry.sh: all checks passed"
