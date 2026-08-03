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

# strip_decoration <entry name> <tag> — the orderable version inside a decorated tag, or the tag
# unchanged when removing the decoration would mean guessing.
#
# The entry name is a parameter because it is the only thing that separates two identical shapes.
# bun pins `1.3.14` against tag `bun-v1.3.14` -- the same version. svelte-language-server pins
# `0.18.3` against tag `svelte2tsx@0.7.59` -- a different package in the same monorepo. Both are a
# bare pin against a decorated tag; nothing about their form tells them apart. What does is whether
# the decoration names the entry, which makes the rule derived from the manifest rather than from a
# list of upstreams somebody has to maintain.
#
# Every rule here removes decoration only when the result is demonstrably orderable. Where it is
# not, the tag comes back whole and the caller reports it unverified -- deliberately, because
# inventing an order is how this file's other comment came to describe advising a downgrade as a
# security warning.
strip_decoration() {
  local name="$1" core="$2" lname ltag rest
  lname="$(printf '%s' "$name" | tr 'A-Z' 'a-z')"
  ltag="$(printf '%s' "$core" | tr 'A-Z' 'a-z')"

  # 1. A prefix that names this entry, in the separators upstreams actually use.
  if [ -n "$lname" ]; then
    case "$ltag" in
      "$lname"[-_@/]*)
        core="${core:${#lname}}"
        core="${core#[-_@/]}"
        ;;
    esac
  fi

  # 2. The leading v, same as norm_ver.
  core="${core#v}"

  # 3. A build or distribution variant on the end: -alpine, -slim, -bookworm. Removed only when
  #    what remains can be ordered, so trixie-20260713-slim stays whole rather than turning into a
  #    date that would then be compared against a version.
  case "$core" in
    *-*)
      rest="${core%-*}"
      if ver_numeric "$rest"; then core="$rest"; fi
      ;;
  esac

  printf '%s' "$core"
}

# shared_decoration <pin> <tag> — the digit-free leading decoration both sides carry, or empty.
#
# A decoration present on BOTH sides names the same thing whichever it is, so it cannot mislead:
# bitwarden-cli pins `cli-v2026.6.0` against tag `cli-v2026.7.0`, and the entry name matches
# neither. Digit-free on purpose -- splitting at the first digit would cut `svelte2tsx` in half and
# make two unrelated packages look like they share a prefix.
# Tested with grep before extracting with sed, rather than sed's `t` branch: BSD sed does not
# accept a label terminated by a semicolon, so the one-expression form works on GNU and errors on
# macOS (README.md#portable-shell).
shared_decoration() {
  local a="" b=""
  if printf '%s' "$1" | grep -Eq '^[^0-9]+[0-9]'; then
    a="$(printf '%s' "$1" | sed -E 's/^([^0-9]+)[0-9].*$/\1/')"
  fi
  if printf '%s' "$2" | grep -Eq '^[^0-9]+[0-9]'; then
    b="$(printf '%s' "$2" | sed -E 's/^([^0-9]+)[0-9].*$/\1/')"
  fi
  [ -n "$a" ] && [ "$a" = "$b" ] && printf '%s' "$a"
  return 0
}

# --- installed-version probes ---------------------------------------------
#
# Several pins in upstreams.toml mean "what is INSTALLED on this host", not "what the
# registry lists" — apple-container's comment says so outright. That intent was documented
# and unenforced, and on 2026-08-03 an audit found two entries where the machine had quietly
# moved PAST the pin: brew had upgraded typst to 0.15.1 and bitwarden-cli to 2026.7.0 while
# the manifest still claimed 0.15.0 and cli-v2026.6.0. Nothing noticed, because the drift
# check compares the pin against releases/latest — both sides remote. Nothing ever asked the
# host.
#
# That matters more than a stale number. The cooldown exists so a release waits out the
# window in which a compromised one gets yanked, and that protection is worth exactly as much
# as the machine's willingness to stay on the pinned version. `brew upgrade` does not read
# upstreams.toml. bitwarden-cli is how tools/setup-secret.sh reads and writes the vault, and
# it ran an unaudited version for some number of days.

# probe_argv_safe <probe> — is this manifest value safe to execute without a shell?
#
# A manifest is data, and data that reaches a shell is an injection vector. The probe is
# therefore never passed to `sh -c`, `eval`, or a backtick: the caller word-splits it and
# execs argv directly. This function is the second lock — it rejects every character that
# would matter IF someone later reintroduced a shell, so the safety does not rest on one
# call site staying correct. Allowed: letters, digits, and - _ . / = : plus the spaces that
# separate arguments. That admits `container --version` and `bw --version`, and rejects
# `$(...)`, backticks, ; | & > < newline, quotes and globs.
probe_argv_safe() {
  case "$1" in
    "") return 1 ;;
    *[!A-Za-z0-9\ ./_=:-]*) return 1 ;;
  esac
  # A leading dash would make the first word look like a flag rather than a command, and a
  # path is only allowed to be absolute or bare — never a ../ escape assembled in a manifest.
  case "$1" in
    -*|*..*) return 1 ;;
  esac
  return 0
}

# probe_extract_version <output> — the first version-shaped token in a tool's --version text.
#
# Every tool answers differently: `bottom 0.14.7`, `typst 0.15.1 (a1b2c3)`, `container CLI
# version 1.0.0 (build: release, commit: ee848e3)`, or a bare `2026.7.0`. Rather than a
# per-tool parser, take the first token that is a dotted numeric version. That is the whole
# contract, and an entry whose tool does not answer that way is reported as unprobeable
# rather than quietly compared against something else.
probe_extract_version() {
  printf '%s' "$1" \
    | tr ' \t\n' '\n\n\n' \
    | sed -E 's/^v//; s/[(),]//g' \
    | grep -E '^[0-9]+(\.[0-9]+)+$' \
    | head -1
}

# probe_core <string> — reduce a pin or a probe answer to its bare dotted version, or empty.
#
# strip_decoration is not the right tool here: it removes a decoration that names the ENTRY
# (`bun-v1.3.14` for `bun`), and the decorations on this side name neither the entry nor each
# other. bitwarden-cli pins `cli-v2026.7.0` while `bw --version` answers a bare `2026.7.0`;
# vaultwarden pins `1.37.0-alpine` while the binary would say `1.37.0`. Both sides are reduced
# to the version itself instead, which is the only part a host can actually confirm.
probe_core() {
  printf '%s' "$1" | sed -E 's/^[^0-9]*//; s/[^0-9.].*$//; s/\.$//'
}

# probe_agrees <entry name> <pin> <installed> — does the host match the pin?
probe_agrees() {
  local pin_core installed_core
  pin_core="$(probe_core "$2")"
  installed_core="$(probe_core "$3")"
  [ -n "$pin_core" ] && [ "$pin_core" = "$installed_core" ]
}

# --- upstream drift classification ----------------------------------------
# drift_note <entry name> <pin> <latest_tag> <age_days> <cooldown_min> <cooldown_max>
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
  local name="$1" pin="$2" latest_tag="$3" age_days="$4" cd_min="$5" cd_max="$6"
  local latest_norm pin_norm deco
  pin_norm="$(strip_decoration "$name" "$pin")"
  latest_norm="$(strip_decoration "$name" "$latest_tag")"

  # Second chance before giving up: a decoration both sides carry, which the entry-name rule above
  # cannot see because it does not name the entry.
  if ! ver_numeric "$pin_norm" || ! ver_numeric "$latest_norm"; then
    deco="$(shared_decoration "$pin" "$latest_tag")"
    if [ -n "$deco" ]; then
      pin_norm="$(strip_decoration "" "${pin#"$deco"}")"
      latest_norm="$(strip_decoration "" "${latest_tag#"$deco"}")"
    fi
  fi

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
