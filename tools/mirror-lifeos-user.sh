#!/bin/bash
# tools/mirror-lifeos-user.sh — refresh the tracked mirror of the LifeOS USER tree.
#
# LifeOS keeps the principal's personal data (TELOS, identity, config, work state) at
# $LIFEOS_USER_DIR, which is NOT a git repository and therefore has no undo. axon-overlay
# already reserves a home for it: `.gitignore` un-ignores `resources/backups/**` and calls
# that path the "identity mirror". This tool is the refresh step that was missing — the
# mirror existed and went stale, because nothing ever re-ran the copy.
#
#   tools/mirror-lifeos-user.sh            # report what diverged, write nothing
#   tools/mirror-lifeos-user.sh --apply    # refresh the mirror
#
# It only ever copies INTO the overlay. It never writes back to the live tree, so a bad
# mirror can lose a backup but never the original. Committing stays a human step: the
# tool leaves the working tree dirty and says so.
#
# bash 3.2-safe.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$TOOLS_DIR/lib/paths.sh"   # AXON_PERSONAL_ROOT

# install-hook — write the pre-commit trigger into the active overlay, with this checkout's
# own path substituted in. The tool that owns the refresh owns its trigger's installation:
# it is the one thing that already knows where Axon lives, so nothing has to guess a sibling
# path. Same template-plus-sed shape as the launchd and systemd units.
if [ "${1:-}" = "install-hook" ]; then
  TMPL="$TOOLS_DIR/templates/hooks/pre-commit-lifeos-mirror.tmpl"
  DEST="$AXON_PERSONAL_ROOT/.git/hooks/pre-commit"
  [ -f "$TMPL" ] || { echo "mirror-lifeos-user: missing template $TMPL" >&2; exit 1; }
  [ -d "$AXON_PERSONAL_ROOT/.git" ] || { echo "mirror-lifeos-user: $AXON_PERSONAL_ROOT is not a git repository" >&2; exit 1; }
  mkdir -p "$(dirname "$DEST")"
  if [ -f "$DEST" ] && ! grep -q '__MIRROR_TOOL__\|mirror-lifeos-user' "$DEST"; then
    echo "mirror-lifeos-user: $DEST exists and is not this hook — refusing to overwrite it." >&2
    exit 1
  fi
  sed -e "s|__MIRROR_TOOL__|$TOOLS_DIR/mirror-lifeos-user.sh|" "$TMPL" > "$DEST"
  chmod +x "$DEST"
  echo "mirror-lifeos-user: installed $DEST"
  exit 0
fi

# Source of truth for the live tree: the USER zone of this machine's declared `lifeos`
# state mount. Overridable via LIFEOS_USER_DIR for tests and one-off targets. Resolved
# rather than defaulted, so this and lifeos-user-sync.sh cannot drift apart.
if [ -z "${LIFEOS_USER_DIR:-}" ]; then
  LIFEOS_USER_DIR="$(axon_lifeos_user_dir)" || {
    echo "mirror-lifeos-user: declare a [[state_mount]] with tool = \"lifeos\" in $AXON_MACHINE_TOML, or set LIFEOS_USER_DIR" >&2
    exit 1
  }
fi
MIRROR="$AXON_PERSONAL_ROOT/resources/backups/lifeos/USER"

APPLY=0
[ "${1:-}" = "--apply" ] && APPLY=1

[ -d "$LIFEOS_USER_DIR" ] || { echo "mirror-lifeos-user: no $LIFEOS_USER_DIR" >&2; exit 1; }

# Refuse to mirror anything that looks like a live credential. axon-overlay's section-5
# hard blocks would stop most of these at commit time, but a secret sitting untracked in
# the working tree is still a secret in the wrong place. Fail loud instead of filtering
# quietly: a silent skip is how a backup ends up missing the file you needed.
if find "$LIFEOS_USER_DIR" \( -name '*.env' -o -name '.env*' -o -name 'id_rsa*' \
     -o -name 'id_ed25519*' -o -name '*.pem' -o -name '*.key' \) -print -quit | grep -q .; then
  echo "mirror-lifeos-user: credential-shaped files found in the source tree — refusing to mirror." >&2
  echo "  move them out of $LIFEOS_USER_DIR (secrets belong in Keychain/Vaultwarden), then re-run." >&2
  exit 1
fi

# `|| true` on both: `diff` exits 1 when files differ, which is the NORMAL case here, and
# under `set -euo pipefail` that non-zero propagates out of the command substitution and
# kills the script before it can report anything. Captured once rather than re-run, so the
# count and the listing can never disagree.
REPORT="$(diff -rq -x '.DS_Store' -x 'CACHE' "$LIFEOS_USER_DIR" "$MIRROR" 2>/dev/null || true)"
DIVERGED="$(printf '%s' "$REPORT" | grep -c . || true)"

if [ "$APPLY" -eq 0 ]; then
  echo "── LifeOS USER mirror (dry-run) ──"
  echo "  source: $LIFEOS_USER_DIR"
  echo "  mirror: $MIRROR"
  if [ "$DIVERGED" -eq 0 ]; then
    echo "  up to date ✅"
  else
    printf '%s\n' "$REPORT" | sed 's/^/  /'
    echo "  $DIVERGED path(s) diverged — run with --apply"
  fi
  exit 0
fi

mkdir -p "$MIRROR"
# --delete so a file deleted upstream disappears from the mirror too; without it the
# mirror slowly becomes a union of every state the tree has ever had, which is not a
# backup of anything. .DS_Store excluded: noise, and axon-overlay blocks it anyway.
#
# CACHE/ excluded for two reasons that happen to agree. It is regenerable by
# definition, so a recovery copy of it recovers nothing. And it is rewritten
# constantly — freshness.json changed twice during a single session — which made
# doctor's mirror-drift check report divergence permanently. A signal that is
# always on is not a signal, and it would have trained us to ignore the one case
# that matters: real identity content diverging from its only backup.
#
# --delete-excluded, not just --delete: rsync PROTECTS excluded paths in the
# destination by default, so without it the CACHE files already mirrored would
# sit there forever, stale and never updated again — the worst of both.
rsync -a --delete --delete-excluded --exclude '.DS_Store' --exclude 'CACHE/' "$LIFEOS_USER_DIR/" "$MIRROR/"

echo "✓ mirror refreshed: $DIVERGED path(s) updated → resources/backups/lifeos/USER"
echo "  not committed — review and commit in axon-overlay when you're ready."
