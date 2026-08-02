#!/bin/bash
# lifeos-user-sync — fast-forward sync of the ENTIRE LifeOS identity tree
# (~/.config/LIFEOS/USER, source-install-authoritative) <-> the axon-overlay overlay.
# No symlinks: the source install stays canonical; axon-overlay keeps a versionable copy.
#
#   tools/lifeos-user-sync.sh capture [--dry-run]   # local -> axon-overlay (backup; the default direction)
#   tools/lifeos-user-sync.sh inject  [--dry-run]   # axon-overlay -> local (restore onto a fresh install)
#
# Whole tree, recursively, minus junk (.DS_Store). Fast-forward only: a file copies
# only when the source is strictly newer than the destination (mtime). A newer,
# differing destination is a divergence — SKIPPED and reported, never clobbered.
# Nothing is lost in either direction; you reconcile divergences by hand.
#
# Paths derived via paths.sh (AXON_PERSONAL_ROOT). Source dir overridable via
# LIFEOS_USER_DIR. Zero hardcoded personal paths. bash 3.2-safe.
#
# Exit 0 = clean, 1 = divergence(s) skipped, 2 = usage/setup error.

set -u

_here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/lib/paths.sh
. "$_here/lib/paths.sh"   # exports AXON_ROOT, AXON_PERSONAL_ROOT

USER_SRC="${LIFEOS_USER_DIR:-$HOME/.config/LIFEOS/USER}"
OVERLAY_COPY="$AXON_PERSONAL_ROOT/resources/backups/lifeos/USER"

MODE=""; DRY=0
for a in "$@"; do
  case "$a" in
    capture|inject) MODE="$a" ;;
    --dry-run|-n)   DRY=1 ;;
    -h|--help)      sed -n '2,21p' "$0"; exit 0 ;;
    *) echo "lifeos-user-sync: unknown arg '$a'" >&2; exit 2 ;;
  esac
done
[ -n "$MODE" ] || { echo "lifeos-user-sync: need 'capture' or 'inject'" >&2; exit 2; }

# Direction: capture = local(src) -> overlay ; inject = overlay -> local(src).
# We always WALK the source side, so a brand-new tree populates fully.
if [ "$MODE" = "capture" ]; then FROM="$USER_SRC"; TO="$OVERLAY_COPY"; ARROW="local → axon-overlay"
else                             FROM="$OVERLAY_COPY"; TO="$USER_SRC"; ARROW="axon-overlay → local"; fi
[ -d "$FROM" ] || { echo "lifeos-user-sync: source dir not found: $FROM" >&2; exit 2; }

[ "$DRY" -eq 1 ] && _dl="  [dry-run]" || _dl=""
echo "lifeos-user-sync · $MODE ($ARROW)$_dl"
echo "  from: $FROM"
echo "  to:   $TO"
echo

copied=0; uptodate=0; diverged=0
while IFS= read -r src; do
  rel="${src#$FROM/}"
  dst="$TO/$rel"
  if [ -f "$dst" ] && cmp -s "$src" "$dst"; then
    uptodate=$((uptodate + 1)); continue
  fi
  # fast-forward gate: never clobber a newer, differing destination.
  if [ -f "$dst" ] && [ "$dst" -nt "$src" ]; then
    echo "  ⚠  DIVERGED (dest newer, differs) — skipped: $rel"; diverged=$((diverged + 1)); continue
  fi
  # here: dest absent, OR src newer than dst → fast-forward the copy.
  if [ "$DRY" -eq 1 ]; then
    echo "  →  would copy   $rel"; copied=$((copied + 1)); continue
  fi
  mkdir -p "$(dirname "$dst")"
  cp -p "$src" "$dst" && { echo "  →  copied       $rel"; copied=$((copied + 1)); } \
    || { echo "  ✗  copy FAILED  $rel" >&2; diverged=$((diverged + 1)); }
done < <(find "$FROM" -type f -not -name '.DS_Store')

echo
echo "── ${copied} copied · ${uptodate} up-to-date · ${diverged} diverged ──"
[ "$diverged" -gt 0 ] && exit 1
exit 0
