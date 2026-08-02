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
