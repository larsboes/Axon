# tools/lib/version.sh — comparing upstream version tags.
#
# Extracted from tools/upstream-checker so the ordering can be tested without a network
# call. It exists because getting this wrong is not a cosmetic bug: the checker used to
# treat "differs from the pin" as "newer than the pin", and GitHub's /releases/latest is
# the most recently PUBLISHED non-prerelease, not the highest version. A project with
# maintenance branches publishes 58.4.0 after 59.1.0, and the checker reported that as a
# cooldown-violating release not to be adopted -- advice to downgrade, wearing the
# formatting of a supply-chain warning (README.md#pins-and-cooldown).
#
# Portable shell, bash 3.2 compatible (README.md#portable-shell). `sort -V` is present on BSD/macOS and
# GNU alike, so nothing here needs coreutils.

# norm_ver <tag> — strip a leading v.
norm_ver() { printf '%s' "$1" | sed -E 's/^v//'; }

# ver_numeric <v> — true when the tag is a dotted number that can actually be ordered.
# cli-v2026.6.0, ProjectName-v1.1.0, trixie-20260713-slim and git shas are not, and
# inventing an order for them would be worse than admitting there isn't one.
ver_numeric() { printf '%s' "$1" | grep -Eq '^[0-9]+(\.[0-9]+)*$'; }

# ver_gt <a> <b> — true when a sorts strictly above b.
ver_gt() {
  [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n1)" = "$1" ]
}

# --- upstream drift classification ----------------------------------------
# drift_note <pin> <latest_tag> <age_days> <cooldown_min> <cooldown_max>
#   Prints the one status note for a pinned upstream against the newest upstream release.
#   Exit 0 = nothing is owed. Exit 1 = the entry needs attention (the caller's `warn`).
#
# Extracted from tools/upstream-checker so the classification can be tested without a network
# call, and because the rule was demonstrably wrong in two directions at once:
#
#   * `warn` tracked "bumped too early" and never tracked "stale". A pin whose cooldown had
#     passed a month ago counted toward `ok`, so the summary read green over exactly the entries
#     that needed work. That is inverted here: being inside cooldown is `ok` — the pin is correct
#     and the operator is deliberately meant to do nothing — while a passed cooldown, and the
#     audit window before it, are what earn a warn. It also stops --strict failing a build for the
#     one situation where the right move is to wait.
#
#   * A tag that cannot be ordered was compared as a string and then handed a drift verdict
#     anyway. bun pinned at `1.3.14` against upstream tag `bun-v1.3.14` — the same version — was
#     reported as 82 days overdue. `ver_numeric` already knew the tag was unorderable; nothing
#     asked it before deciding. An unorderable pair is now reported as unverified, not as drift,
#     because a gate that cannot answer must say so rather than guess.
drift_note() {
  local pin="$1" latest_tag="$2" age_days="$3" cd_min="$4" cd_max="$5"
  local latest_norm pin_norm
  latest_norm="$(norm_ver "$latest_tag")"; pin_norm="$(norm_ver "$pin")"

  if [ "$latest_norm" = "$pin_norm" ]; then
    printf '✓ pinned to latest release (%s)\n' "$latest_tag"; return 0
  fi

  # Neither side orderable => no verdict is available. Checked before the maintenance-branch
  # case and before any age arithmetic, so an unorderable tag can never reach them.
  if ! ver_numeric "$latest_norm" || ! ver_numeric "$pin_norm"; then
    printf '⚠ pin (%s) and upstream tag (%s) are not comparable as versions — drift unverified\n' \
      "$pin" "$latest_tag"; return 1
  fi

  # GitHub's /releases/latest is the most recently PUBLISHED non-prerelease, not the highest
  # version. A project with maintenance branches publishes 58.4.0 after 59.1.0, and comparing
  # only for inequality reported that as a cooldown-violating release that must not be adopted
  # -- advice to downgrade, dressed as a security warning. arrow-rs did exactly this on
  # 2026-07-28.
  if ! ver_gt "$latest_norm" "$pin_norm"; then
    printf "✓ pin (%s) is ahead of upstream's newest release (%s) — maintenance branch, nothing to bump\n" \
      "$pin" "$latest_tag"; return 0
  fi

  if ! ver_numeric "$age_days"; then
    printf '⚠ newer: %s, release date unresolvable — cooldown unverified\n' "$latest_tag"; return 1
  fi

  if [ "$age_days" -lt "$cd_min" ]; then
    printf '🔴 newer: %s (%sd old) — IN COOLDOWN, do NOT bump yet (LiteLLM class)\n' "$latest_tag" "$age_days"
    return 0
  fi
  if [ "$age_days" -le "$cd_max" ]; then
    printf '🟡 newer: %s (%sd old) — cooldown window (%s-%sd), bump only after audit\n' \
      "$latest_tag" "$age_days" "$cd_min" "$cd_max"
    return 1
  fi
  printf '🟢 newer: %s (%sd old) — cooldown passed, audit the delta then bump pin + tag together\n' \
    "$latest_tag" "$age_days"
  return 1
}

# --- release-line identity ------------------------------------------------
# Defensive source: normally toml.sh arrives via paths.sh, but version.sh is also sourced
# directly by tests that never went through it. Same idiom as delta.sh.
command -v toml_get_in >/dev/null 2>&1 || . "$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/toml.sh"

# release_tag_glob — the pattern that decides which tags are release tags, from axon.toml
# [release] tag_glob. Read rather than hardcoded because six call sites in two languages ask
# the same question; a literal in each is how a non-release tag takes over version identity.
# Fails loudly rather than guessing: a missing key means the manifest is wrong, and a silent
# default would restore exactly the one-home problem this exists to remove.
release_tag_glob() {
  local g
  g="$(toml_get_in release tag_glob "${AXON_ROOT:?AXON_ROOT unset}/axon.toml")"
  [ -n "$g" ] || { echo "version.sh: axon.toml has no [release] tag_glob" >&2; return 1; }
  printf '%s' "$g"
}

# describe_release [<rev>] — `git describe` restricted to the release line. Degrades to a short
# sha when no release tag is reachable, which is the honest answer for an untagged checkout and
# the reason --always is kept. Marks a dirty tree only when describing the working copy: git
# rejects --dirty together with an explicit revision.
describe_release() {
  local rev="${1:-}" glob
  glob="$(release_tag_glob)" || return 1
  if [ -n "$rev" ]; then
    git -C "$AXON_ROOT" describe --tags --always --match "$glob" "$rev" 2>/dev/null
  else
    git -C "$AXON_ROOT" describe --tags --always --dirty --match "$glob" 2>/dev/null
  fi
}
