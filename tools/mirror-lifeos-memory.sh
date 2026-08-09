#!/usr/bin/env bash
# tools/mirror-lifeos-memory.sh — refresh the tracked mirror of the LifeOS MEMORY archive.
#
# Sibling of mirror-lifeos-user.sh, deliberately a second script rather than a `--zone` flag
# on that one. Three things argued for the split: the two zones select content by opposite
# rules (USER mirrors the whole tree minus two known-regenerable paths, MEMORY mirrors an
# explicit allow-list and rejects everything else), the USER tool is already wired into
# doctor, a test and an installed git hook by that name, and a tool called
# "mirror-lifeos-user" that also copies MEMORY is a name that lies. The rsync and diff calls
# they share are a dozen lines; the coupling would have cost more than the duplication.
#
#   tools/mirror-lifeos-memory.sh            # report what diverged, write nothing
#   tools/mirror-lifeos-memory.sh --apply    # refresh the mirror
#
# Copies INTO the overlay only, never back, so a bad mirror can lose a backup but never the
# original. Committing stays a human step.
#
# bash 3.2-safe.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$TOOLS_DIR/lib/paths.sh"   # AXON_PERSONAL_ROOT, axon_lifeos_memory_dir

# WHAT IS WORTH KEEPING, stated as an allow-list rather than an ignore-list. MEMORY is the
# one tree where the majority of the bytes are worth nothing on restore, so the default has
# to be "exclude" — an ignore-list silently adopts every new subdirectory LifeOS invents,
# and the ones it has invented so far have mostly been logs.
#
#   KNOWLEDGE     the typed archive (People, Companies, Ideas, Research). Hand-curated, and
#                 the single most expensive thing here to reconstruct.
#   WORK          ISAs and work history: what was built, against which articulated ideal.
#   RELATIONSHIP  who the system has learned to model, and how.
#   SECURITY      findings and their disposition.
#
# Deliberately absent, each for a reason that was checked rather than assumed:
#   STATE         hot runtime cache. Sampled 2026-08-09: weather-cache, model-cache,
#                 location-cache, delta-surface-heartbeat, last-response, session scratch —
#                 several rewritten per turn. Mirroring it would leave the overlay working
#                 tree permanently dirty, which is how a drift signal gets trained into
#                 noise (the same argument mirror-lifeos-user.sh makes for CACHE/).
#   VOICE, SKILLS append-only event logs (voice-events.jsonl, execution.jsonl).
#   PULSE_DATA    derived from the sources above; regenerates.
#   LEARNING      55 MB of logs.
#   OBSERVABILITY 55 MB of logs.
# Together those are ~115 of the tree's ~118 MB and ~3 MB of it is the part you would miss.
ZONES="KNOWLEDGE WORK RELATIONSHIP SECURITY"

if [ -z "${LIFEOS_MEMORY_DIR:-}" ]; then
  LIFEOS_MEMORY_DIR="$(axon_lifeos_memory_dir)" || {
    echo "mirror-lifeos-memory: declare a [[state_mount]] with tool = \"lifeos\" in $AXON_MACHINE_TOML, or set LIFEOS_MEMORY_DIR" >&2
    exit 1
  }
fi
MIRROR="$AXON_PERSONAL_ROOT/resources/backups/lifeos/MEMORY"

APPLY=0
[ "${1:-}" = "--apply" ] && APPLY=1

[ -d "$LIFEOS_MEMORY_DIR" ] || { echo "mirror-lifeos-memory: no $LIFEOS_MEMORY_DIR" >&2; exit 1; }

# Same credential refusal as the USER mirror, and for the same reason: a secret sitting
# untracked in the working tree is still a secret in the wrong place, and a quiet skip is how
# a backup ends up missing the file you needed. Scoped to the zones actually copied, so a key
# parked in OBSERVABILITY cannot block a mirror that would never have touched it.
for z in $ZONES; do
  [ -d "$LIFEOS_MEMORY_DIR/$z" ] || continue
  if find "$LIFEOS_MEMORY_DIR/$z" \( -name '*.env' -o -name '.env*' -o -name 'id_rsa*' \
       -o -name 'id_ed25519*' -o -name '*.pem' -o -name '*.key' \) -print -quit | grep -q .; then
    echo "mirror-lifeos-memory: credential-shaped files found in $z — refusing to mirror." >&2
    echo "  move them out (secrets belong in Keychain/Vaultwarden), then re-run." >&2
    exit 1
  fi
done

# `|| true`: diff exits 1 when files differ, which is the normal case, and under
# `set -euo pipefail` that would kill the script before it could report anything.
#
# The missing-mirror case is reported explicitly rather than left to diff. `diff -rq a b` with
# no `b` writes only to stderr, so the 2>/dev/null that suppresses the noisy-but-harmless
# cases was also swallowing the loudest one: the first dry-run on this machine printed
# "up to date ✅" for a mirror that did not exist at all. A backup tool whose green means
# "I could not find the backup" is worse than no tool.
REPORT=""
for z in $ZONES; do
  [ -d "$LIFEOS_MEMORY_DIR/$z" ] || continue
  if [ ! -d "$MIRROR/$z" ]; then
    REPORT="${REPORT}Zone $z has no mirror yet at $MIRROR/$z
"
    continue
  fi
  z_report="$(diff -rq -x '.DS_Store' "$LIFEOS_MEMORY_DIR/$z" "$MIRROR/$z" 2>/dev/null || true)"
  [ -n "$z_report" ] && REPORT="${REPORT}${z_report}
"
done
DIVERGED="$(printf '%s' "$REPORT" | grep -c . || true)"

if [ "$APPLY" -eq 0 ]; then
  echo "── LifeOS MEMORY mirror (dry-run) ──"
  echo "  source: $LIFEOS_MEMORY_DIR"
  echo "  mirror: $MIRROR"
  echo "  zones:  $ZONES"
  if [ "$DIVERGED" -eq 0 ]; then
    echo "  up to date ✅"
  else
    printf '%s' "$REPORT" | sed 's/^/  /'
    echo "  $DIVERGED path(s) diverged — run with --apply"
  fi
  exit 0
fi

mkdir -p "$MIRROR"
# --delete so an upstream deletion propagates; without it the mirror becomes a union of every
# state the tree has ever held, which is a backup of nothing.
for z in $ZONES; do
  [ -d "$LIFEOS_MEMORY_DIR/$z" ] || continue
  mkdir -p "$MIRROR/$z"
  rsync -a --delete --delete-excluded --exclude '.DS_Store' "$LIFEOS_MEMORY_DIR/$z/" "$MIRROR/$z/"
done

echo "✓ mirror refreshed: $DIVERGED path(s) updated → resources/backups/lifeos/MEMORY"
echo "  not committed — review and commit in the overlay when you're ready."
