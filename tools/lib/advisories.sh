# tools/lib/advisories.sh — published GitHub Security Advisories against a pinned version.
#
# Extracted from tools/upstream-checker (Axon#124) once a second consumer appeared: the
# agentbox self-update gate (Axon#126) has to answer the same question — "does a published
# advisory cover the version about to be installed?" — at a completely different moment, on
# the machine, seconds before a binary is placed. Two consumers is what makes this a lib
# rather than a helper; the expiry clock moved for the same reason (41d66e8).
#
# Why this exists at all: osv-scanner is Axon's adopted vulnerability gate and OSV does not
# carry GHSA for every ecosystem. Queried for `@earendil-works/pi-coding-agent` it answers
# `{}` while GitHub publishes four advisories against that repo, one of them high severity.
# The scanner adopted for exactly this job is structurally blind to them, so this is the
# only path that can see them.
#
# Portable shell, bash 3.2 compatible (README.md#portable-shell).

# range_contains / pin_comparable live in version.sh. Defensive source, same idiom as
# version.sh uses for toml.sh: a caller that already sourced it pays nothing, and a test
# that sources this file alone still works.
command -v range_contains >/dev/null 2>&1 \
  || . "$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/version.sh"

# gh_advisories <owner/repo> — published advisories as `GHSA<TAB>severity<TAB>range` lines,
# one line per affected range (an advisory naming three ranges yields three).
#   Exit 0 = fetched (output may be empty) · 1 = fetch failed · 2 = gh unavailable.
#
# gh rather than curl: pairing an advisory's id and severity with each of its nested ranges
# is a join, and doing that with grep over raw JSON is how a parser silently attaches the
# wrong severity to the wrong range. gh bundles jq, so the join is declarative — and its
# absence is reported as unchecked rather than passed.
gh_advisories() {
  local repo="$1" out rc
  command -v gh >/dev/null 2>&1 || return 2
  out="$(gh api "repos/$repo/security-advisories" --jq '
    .[]
    | select(.state == "published")
    | .ghsa_id as $id
    | .severity as $sev
    | (.vulnerabilities // [])[]
    | select(.vulnerable_version_range != null)
    | [$id, $sev, .vulnerable_version_range]
    | @tsv' 2>/dev/null)"
  rc=$?
  [ "$rc" -eq 0 ] || return 1
  printf '%s' "$out"
  return 0
}

# advisory_scan <owner/repo> <pin> — fetch, then decide each advisory against the pin.
#
# Prints one TSV line per advisory that is NOT clean, plus one summary line, always last:
#
#   hit<TAB><ghsa><TAB><severity><TAB><range>        the pin is inside this range
#   undecided<TAB><ghsa><TAB><severity><TAB><range>  the range is not orderable
#   total<TAB><n>                                    advisories examined
#
# A clean advisory produces no line on purpose: the caller counts `hit` lines to decide, and
# a green list that scrolls is how a gate becomes wallpaper. `total` is printed regardless so
# "this repo publishes advisories and none reaches this pin" stays distinguishable from
# "nobody looked" — a distinction the caller cannot reconstruct from an empty stream.
#
# Exit: 0 = scanned · 1 = fetch failed · 2 = gh unavailable · 3 = pin not comparable.
#
# Undecided is not a pass. GitHub lets an advisory carry a free-text vulnerable_version_range,
# and real ones behind Axon's manifest include '2025.02 to 2026.01', '≤ 2026.5.2', '8+' and a
# bare '1.35.4'. Refusing to order those is correct — inventing a verdict is the failure
# lib/version.sh exists to prevent — but the caller has to be told, so it is a status, not a
# silence.
advisory_scan() {
  local repo="$1" pin="$2" out rc id sev range total=0

  pin_comparable "$pin" || return 3

  out="$(gh_advisories "$repo")"
  rc=$?
  [ "$rc" -eq 0 ] || return "$rc"

  # Heredoc rather than a pipe: `while read` on the right of a pipe runs in a subshell under
  # bash 3.2, and every counter incremented in here would be discarded at the loop's end —
  # silently reporting zero hits over a list that had some.
  while IFS="$(printf '\t')" read -r id sev range; do
    [ -n "$id" ] || continue
    total=$((total + 1))
    range_contains "$pin" "$range"
    case $? in
      0) printf 'hit\t%s\t%s\t%s\n' "$id" "$sev" "$range" ;;
      2) printf 'undecided\t%s\t%s\t%s\n' "$id" "$sev" "$range" ;;
    esac
  done <<ADVISORIES
$out
ADVISORIES

  printf 'total\t%s\n' "$total"
  return 0
}
