#!/bin/bash
# Regression test for tools/lib/version.sh. The case that matters is the one that was
# live and wrong on 2026-07-28: arrow-rs published 58.4.0 after 59.1.0, and the checker
# called the older release "newer".
set -uo pipefail

_lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/lib"
# shellcheck source=lib/version.sh
. "$_lib/version.sh"

fails=0
check() { # check <description> <expected: yes|no> <command...>
  local desc="$1" expect="$2"; shift 2
  if "$@"; then got=yes; else got=no; fi
  if [ "$got" != "$expect" ]; then
    echo "FAIL: $desc (expected $expect, got $got)"
    fails=$((fails + 1))
  fi
}

# The live regression: a maintenance release published later than a higher minor.
check "59.1.0 > 58.4.0"                  yes ver_gt 59.1.0 58.4.0
check "58.4.0 is NOT > 59.1.0"           no  ver_gt 58.4.0 59.1.0

# Ordinary ordering, including the double-digit trap a lexical sort gets wrong.
check "1.10.0 > 1.9.0 (not lexical)"     yes ver_gt 1.10.0 1.9.0
check "0.71.10 > 0.71.9"                 yes ver_gt 0.71.10 0.71.9
check "equal is not greater"             no  ver_gt 1.2.3 1.2.3
check "patch bump"                       yes ver_gt 1.2.4 1.2.3
check "differing depth: 2.0 vs 2.0.1"    no  ver_gt 2.0 2.0.1

# What must NOT be ordered. Every one of these is a real pin in upstreams.toml, and
# ver_numeric is what stops the checker inventing a comparison for them.
check "plain version is orderable"       yes ver_numeric 59.1.0
check "single component is orderable"    yes ver_numeric 17
check "git sha is not"                   no  ver_numeric f7c4aef
check "cli-v2026.6.0 is not"             no  ver_numeric cli-v2026.6.0
check "ProjectName-v1.1.0 is not"          no  ver_numeric ProjectName-v1.1.0
check "trixie-20260713-slim is not"      no  ver_numeric trixie-20260713-slim
check "17.9-alpine is not"               no  ver_numeric 17.9-alpine
check "empty is not"                     no  ver_numeric ""

# norm_ver strips exactly one leading v, and nothing else.
check "v-prefix stripped"                yes [ "$(norm_ver v8.30.1)" = "8.30.1" ]
check "bare version untouched"           yes [ "$(norm_ver 8.30.1)" = "8.30.1" ]
check "v inside a tag untouched"         yes [ "$(norm_ver cli-v2026.6.0)" = "cli-v2026.6.0" ]

# --- release-line identity ------------------------------------------------
# The falsifier for the hijack this guards against: a tag that is not on the release line must
# not become the reported version. A synthetic repo rather than this checkout, so the assertion
# holds on a machine that happens to carry no tags, or the wrong ones.
_fixture="$(mktemp -d)"
trap 'rm -rf "$_fixture"' EXIT
_g() { git -C "$_fixture" -c user.email=t@example.invalid -c user.name=t -c commit.gpgsign=false "$@" >/dev/null 2>&1; }

_g init -q
printf '[release]\ntag_glob = "v[0-9]*"\n' > "$_fixture/axon.toml"
_g add axon.toml
_g commit -m "release fixture"
_g tag v0.1.0
printf 'second\n' > "$_fixture/later.txt"
_g add later.txt
_g commit -m "after the release"
# The kind of tag that caused this: a marker parked on the same line, not a release.
_g tag archive/dev-pre-public

AXON_ROOT="$_fixture"; export AXON_ROOT

# `check` runs a command, and `case` is a keyword — so the pattern tests get real predicates.
starts_with() { case "$2" in "$1"*) return 0 ;; *) return 1 ;; esac; }
contains()    { case "$2" in *"$1"*) return 0 ;; *) return 1 ;; esac; }

check "glob comes from axon.toml"        yes [ "$(release_tag_glob)" = "v[0-9]*" ]
check "describe_release finds v0.1.0"    yes starts_with v0.1.0 "$(describe_release)"
check "a non-release tag never wins"     no  contains archive "$(describe_release)"
# The control: an unrestricted describe DOES pick the marker up, which is the whole point.
check "bare describe is hijacked"        yes starts_with archive/ "$(git -C "$_fixture" describe --tags)"
# Explicit revision: --dirty is illegal with a rev, so this exercises the other branch.
check "describe_release takes a rev"     yes [ "$(describe_release v0.1.0)" = "v0.1.0" ]

# --- upstream drift classification ----------------------------------------
# The two inversions this encodes, both observed live on 2026-08-03:
#   * a pin whose cooldown had passed counted as ok, so the summary read green over exactly
#     the entries that needed work;
#   * bun pinned at 1.3.14 against upstream tag bun-v1.3.14 -- the same version -- was reported
#     as 82 days overdue, because an unorderable tag still reached the age arithmetic.
# `yes` here means drift_note exited 0, i.e. nothing is owed.
dn() { drift_note "$@" >/dev/null 2>&1; }
note_of() { drift_note "$@" 2>/dev/null; }

check "pinned to latest owes nothing"    yes dn any 1.2.3 v1.2.3 0 7 14
check "maintenance branch owes nothing"  yes dn arrow-rs 59.1.0 v58.4.0 90 7 14
check "inside cooldown owes nothing"     yes dn any 1.0.0 v1.1.0 1 7 14
check "cooldown window is owed"          no  dn any 1.0.0 v1.1.0 10 7 14
check "past cooldown is owed"            no  dn any 1.0.0 v1.1.0 82 7 14
check "unorderable pair is owed"         no  dn svelte-language-server 1.3.14 bun-v1.3.14 82 7 14
check "unresolvable age is owed"         no  dn any 1.0.0 v1.1.0 "?" 7 14

# The bun case again, on the note itself: an unorderable pair must not be dressed up as drift.
check "unorderable says so"              yes contains "not comparable" "$(note_of svelte-language-server 1.3.14 bun-v1.3.14 82 7 14)"
check "unorderable claims no age"        no  contains "82d old" "$(note_of svelte-language-server 1.3.14 bun-v1.3.14 82 7 14)"
# And the boundaries, because off-by-one here decides whether a bump is allowed.
check "exactly cooldown_min is a window" no  dn any 1.0.0 v1.1.0 7 7 14
check "exactly cooldown_max is a window" no  dn any 1.0.0 v1.1.0 14 7 14
check "one past the window is stale"     no  dn any 1.0.0 v1.1.0 15 7 14
check "window note names the window"     yes contains "cooldown window (7-14d)" "$(note_of any 1.0.0 v1.1.0 10 7 14)"

# --- decoration, and where it must NOT be removed -------------------------
# The two shapes that cannot be told apart by form: a bare pin against a decorated tag, where the
# decoration either names this entry (strip it) or names a different package (never strip it).
check "entry-name prefix is stripped"    yes dn bun 1.3.14 bun-v1.3.14 82 7 14
check "a foreign package stays unread"   no  dn svelte-language-server 0.18.3 svelte2tsx@0.7.59 82 7 14
check "foreign package says unverified"  yes contains "not comparable"   "$(note_of svelte-language-server 0.18.3 svelte2tsx@0.7.59 82 7 14)"

# A decoration both sides carry names the same thing whichever it is.
check "shared prefix compares"           no  dn bitwarden-cli cli-v2026.6.0 cli-v2026.7.0 82 7 14
check "shared prefix, equal versions"    yes dn bitwarden-cli cli-v2026.6.0 cli-v2026.6.0 82 7 14

# A build variant on one side only.
check "variant suffix on the pin"        no  dn vaultwarden 1.37.0-alpine 1.37.1 82 7 14
check "variant suffix, same version"     yes dn vaultwarden 1.37.0-alpine 1.37.0 82 7 14
# Both sides decorated AND suffixed: the date is what is left, and it is orderable.
check "shared prefix plus variant"       no  dn debian trixie-20260713-slim trixie-20260801-slim 82 7 14

# A sha is not a version and never becomes one.
check "a sha stays unverified"           no  dn svelte-ai-tools 6468954 @sveltejs/opencode@0.1.13 82 7 14
# Stripping must not invent an order where the remainder is not a version.
check "unorderable remainder is kept"    no  dn something 1.2.3 name-vNIGHTLY 82 7 14

# The entry-name rule is case-insensitive but never partial: a name that is only a PREFIX of the
# decoration must not match, or ProjectName-v1 would strip for an entry called "Project".
check "partial name does not match"      no  dn proj 1.2.3 project-v9.9.9 82 7 14

# --- GHSA affected ranges (Axon#124) --------------------------------------
#
# Checked against real published data rather than invented shapes. The
# ">= 0.74.0, < 0.78.1" conjunction is earendil-works/pi's advisories, and the unspaced
# "<1.35.8" sitting beside the spaced "<= 1.35.4" is dani-garcia/vaultwarden publishing
# both spellings in the same list. A parser handling only the tidy form passes a synthetic
# test and misses a live advisory.
rc_is() {  # rc_is <description> <expected rc> <version> <range>
  local desc="$1" expect="$2" got
  shift 2
  range_contains "$1" "$2"; got=$?
  if [ "$got" != "$expect" ]; then
    echo "FAIL: $desc (expected rc $expect, got $got)"
    fails=$((fails + 1))
  fi
}

# 0 = affected · 1 = not affected · 2 = undecidable.
rc_is "inside a two-sided range"         0 0.75.0        ">= 0.74.0, < 0.78.1"
rc_is "the lower bound is inclusive"     0 0.74.0        ">= 0.74.0, < 0.78.1"
rc_is "the upper bound is exclusive"     1 0.78.1        ">= 0.74.0, < 0.78.1"
rc_is "above a two-sided range"          1 0.80.10       ">= 0.74.0, < 0.78.1"
rc_is "below a one-sided range"          0 0.78.0        "< 0.79.0"
rc_is "at a one-sided bound"             1 0.79.0        "< 0.79.0"
rc_is "unspaced operator still parses"   1 1.37.0        "<1.35.8"
rc_is "<= includes its bound"            0 1.35.4        "<= 1.35.4"
rc_is "<= excludes just past it"         1 1.35.5        "<= 1.35.4"
rc_is "= matches exactly"                0 1.2.3         "= 1.2.3"
rc_is "= rejects a neighbour"            1 1.2.4         "= 1.2.3"
rc_is "> excludes its own bound"         1 1.0.0         "> 1.0.0"
rc_is "> admits what is above it"        0 2.0.0         "> 1.0.0"
# An image-style pin is the version underneath it: vaultwarden pins 1.37.0-alpine while
# its advisories are written against 1.35.x.
rc_is "a decorated pin is reduced"       1 1.37.0-alpine "<1.35.8"

# Undecidable, and never quietly "not affected". Inventing a verdict where none was
# available is the exact failure the top of this file records.
rc_is "an unorderable version"           2 abc           "< 1.0.0"
rc_is "an empty range"                   2 1.0.0         ""
rc_is "an operator we do not know"       2 1.0.0         "~> 1.0"
rc_is "a range with no operator"         2 1.0.0         "1.0.0"

if [ "$fails" -gt 0 ]; then
  echo "version.sh: $fails check(s) failed"
  exit 1
fi
echo "version.sh: all checks passed"
