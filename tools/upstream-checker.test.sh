#!/bin/bash
# Planted-manifest regression tests for tools/upstream-checker's release-drift declaration (#38).
#
# The tracked upstreams.toml can only ever demonstrate the states it happens to be in, which is
# how a permanently-warning gate survived: nothing exercised the paths where a declaration is
# malformed, or where an undeclared entry must still warn. Each case here builds its own manifest
# via AXON_UPSTREAMS_MANIFEST instead — the same escape hatch check-bun-pin.sh uses.
#
# Hermetic: a stub `gh` on PATH stands in for the release API, so the drift block runs for real
# without a network call. gh_latest() prefers `gh` when it is present, so curl is never reached.
set -uo pipefail

if [ -n "${TEST_SRCDIR:-}" ]; then
  CHECKER="$TEST_SRCDIR/$TEST_WORKSPACE/tools/upstream-checker"
else
  CHECKER="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/upstream-checker"
fi

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

CALLS="$SCRATCH/gh-calls.txt"
: > "$CALLS"
mkdir -p "$SCRATCH/bin"
# Records what it was asked for, then answers from GH_STUB_TAG. Empty output makes gh_latest
# fail, which is the "no releases / rate-limited" branch.
cat > "$SCRATCH/bin/gh" <<'STUB'
#!/bin/bash
echo "$*" >> "$AXON_TEST_GH_CALLS"
[ -n "${GH_STUB_TAG:-}" ] || exit 1
printf '{"tag_name": "%s", "published_at": "%s"}\n' "$GH_STUB_TAG" "${GH_STUB_PUBLISHED:-2000-01-01T00:00:00Z}"
STUB
chmod +x "$SCRATCH/bin/gh"
export AXON_TEST_GH_CALLS="$CALLS"
PATH="$SCRATCH/bin:$PATH"; export PATH

fails=0

# The header comment is the vocabulary's only home, so a planted manifest carries it verbatim.
HEADER='# verdict = "adopt" | "contribute" | "overlay" | "fork" | "build" | "inspiration" | "quarry" | "reject"
# pin_kind = "commit" | "image" | "monorepo" | "hosted" | "dataset"'

plant() { # plant <case> <entry body...> -> echoes manifest path
  local case_name="$1"; shift
  local mf="$SCRATCH/$case_name.toml"
  printf '%s\n\n%s\n' "$HEADER" "$*" > "$mf"
  printf '%s' "$mf"
}

# One complete entry, so a case can vary a single field without re-stating the rest. The pin is
# a parameter rather than a default an extra line overrides: toml_get_in reads every matching
# line in the section, so two `pin =` lines are two values, not a replacement.
entry() { # entry <name> <pin> [extra lines...]
  local n="$1" p="$2"; shift 2
  printf '[%s]\nurl = "https://github.com/example/%s"\nverdict = "adopt"\nlicense = "MIT"\nwhy = "fixture"\npin = "%s"\n%s\n' \
    "$n" "$n" "$p" "$*"
}

run() { # run <manifest> [flags...]
  local mf="$1"; shift
  : > "$CALLS"
  out="$(AXON_UPSTREAMS_MANIFEST="$mf" "$CHECKER" "$@" 2>&1)"; status=$?
}

check() { # check <label> <condition-description> <0|1 result>
  if [ "$3" -ne 0 ]; then
    echo "FAIL: $1 — $2"; printf '%s\n' "$out" | sed 's/^/    /'
    fails=$((fails + 1))
  fi
}

expect_out() { # expect_out <label> <substring>
  printf '%s' "$out" | grep -qF "$2"; check "$1" "output should contain '$2'" $?
}
expect_no_out() { # expect_no_out <label> <substring>
  ! printf '%s' "$out" | grep -qF "$2"; check "$1" "output should NOT contain '$2'" $?
}
expect_status() { # expect_status <label> <code>
  [ "$status" -eq "$2" ]; check "$1" "expected exit $2, got $status" $?
}

# --- the green path: a declared entry is its own group, never a warning ------------------
mf="$(plant declared "$(entry pinned-to-a-sha abc1234 'pin_kind = "commit"
tracked_by = "diffing upstream default branch against this sha"')")"
run "$mf"
expect_status "declared entry" 0
expect_out    "declared entry" "○ release drift n/a (commit)"
expect_out    "declared entry" "tracked by: diffing upstream default branch against this sha"
expect_out    "declared entry" "1 n/a"
expect_no_out "declared entry" "1 warn"

# ...and the release API is never asked about it. This is the whole point: the declaration says
# the check does not apply, so spending a call to confirm that would contradict it.
[ ! -s "$CALLS" ]; check "declared entry" "gh should not be called at all, got: $(cat "$CALLS")" $?

# A declaration is a manifest fact, not a network one, so it must hold offline too — which is
# the mode tools/doctor spawns.
run "$mf" --offline
expect_out    "declared entry offline" "○ release drift n/a (commit)"
expect_no_out "declared entry offline" "1 warn"

# --strict promotes warnings to failures. A declared entry is not a warning, so a manifest whose
# only non-ok entries are declared must still pass — otherwise the gate stays unusable.
run "$mf" --strict
expect_status "declared entry under --strict" 0

# --- the acceptance criterion that keeps this honest -------------------------------------
# An UNDECLARED entry the lookup cannot resolve still warns. Rate limiting and a transient API
# failure land on this branch too, so it must not look like a deliberate opt-out.
mf="$(plant undeclared "$(entry unresolvable abc1234)")"
run "$mf"
expect_status "undeclared unresolvable entry" 0
expect_out    "undeclared unresolvable entry" "⚠ latest release not resolvable"
expect_out    "undeclared unresolvable entry" "1 warn"
expect_no_out "undeclared unresolvable entry" "1 n/a"
[ -s "$CALLS" ]; check "undeclared unresolvable entry" "gh should have been called" $?

# Real drift on an undeclared entry keeps working — the declaration path must not have taken
# over the ordinary case.
mf="$(plant drifting "$(entry drifting 1.0.0)")"
GH_STUB_TAG="v2.0.0" GH_STUB_PUBLISHED="2000-01-01T00:00:00Z" run "$mf"
expect_out "undeclared drifting entry" "cooldown passed"
expect_out "undeclared drifting entry" "1 warn"

# --- malformed declarations fail loudly rather than becoming a quiet opt-out --------------
mf="$(plant kind-only "$(entry kind-only abc1234 'pin_kind = "commit"')")"
run "$mf"
expect_status "pin_kind without tracked_by" 1
expect_out    "pin_kind without tracked_by" "without tracked_by"

mf="$(plant tracked-only "$(entry tracked-only abc1234 'tracked_by = "vibes"')")"
run "$mf"
expect_status "tracked_by without pin_kind" 1
expect_out    "tracked_by without pin_kind" "tracked_by without pin_kind"

mf="$(plant bad-kind "$(entry bad-kind abc1234 'pin_kind = "whatever"
tracked_by = "something"')")"
run "$mf"
expect_status "invalid pin_kind" 1
expect_out    "invalid pin_kind" "invalid pin_kind"

# The vocabulary has exactly one home. Losing it must stop the run, not silently allow or reject
# every declaration — the same silent-green guard the verdict vocabulary already has.
mf="$SCRATCH/no-vocab.toml"
printf '# verdict = "adopt" | "reject"\n\n%s\n' "$(entry plain abc1234)" > "$mf"
run "$mf"
expect_status "manifest without a pin_kind vocabulary" 2
expect_out    "manifest without a pin_kind vocabulary" "could not derive pin_kind vocabulary"

# --- precedence: a declaration does not absolve an entry of anything else ------------------
# `adopt` without a pin is a warning on its own. Declaring the release check inapplicable must
# not launder that into the n/a group.
mf="$(plant declared-but-unpinned "$(entry declared-but-unpinned "" 'pin_kind = "hosted"
tracked_by = "the hosted API surface"')")"
run "$mf"
expect_out    "declared but unpinned" "without pin"
expect_out    "declared but unpinned" "1 warn"
expect_no_out "declared but unpinned" "1 n/a"

# A missing required field is a fail, and fail outranks the declaration.
mf="$(plant declared-but-incomplete "[broken]
url = \"https://github.com/example/broken\"
verdict = \"adopt\"
pin = \"abc1234\"
pin_kind = \"commit\"
tracked_by = \"a sha diff\"")"
run "$mf"
expect_status "declared but incomplete" 1
expect_out    "declared but incomplete" "missing required field"
expect_out    "declared but incomplete" "1 fail"

# --- the machine view carries the same group ----------------------------------------------
# axon-status and the dashboard read this payload; upstream-watch.yml selects `status == "warn"`
# on it, so a declared entry landing in that selection would page someone weekly for nothing.
mf="$(plant json "$(entry declared-json abc1234 'pin_kind = "image"
tracked_by = "Docker Hub tags"')")"
run "$mf" --json
expect_status "json payload" 0
expect_out    "json payload" '"na":1'
expect_out    "json payload" '"status":"na"'
expect_out    "json payload" '"pin_kind":"image"'
expect_out    "json payload" '"warn":0'

if [ "$fails" -gt 0 ]; then
  echo "upstream-checker declaration gate: $fails check(s) failed"
  exit 1
fi
echo "upstream-checker declaration gate: all checks passed"
