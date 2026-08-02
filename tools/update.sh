#!/bin/bash
# tools/update.sh — the maintainer half of tools/install.sh. Fast-forward
# this checkout to origin/main and re-run tools/doctor as a post-update
# sanity check. Never force-pushes/rebases/resets: a diverged checkout
# (local commits ahead AND behind) is left alone with instructions, not
# silently discarded. Every Axon deployment, current or future, runs this
# the same way — there is no fleet/central-registry concept; each checkout
# maintains itself, per
# `README.md#documentation-stays-owned-and-current`'s bring-your-
# own-check philosophy.
#
#   tools/update.sh            # fetch + ff-only pull + doctor
#   tools/update.sh --check    # fetch + version summary + incoming preview, never pulls
#   tools/update.sh --no-pull  # fetch + report only, don't pull
#   tools/update.sh -h         # this help
#
# Interactive: when stdin is a TTY, the pull path shows the incoming preview
# and asks before fast-forwarding. Non-TTY (CI, cron, pipes) never prompts
# and pulls straight through, exactly as before.
#
# bash 3.2-safe.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AXON_ROOT="$(cd "$TOOLS_DIR/.." && pwd)"

# paths.sh (which sources toml.sh) — it resolves this machine's overlay and exports
# AXON_MACHINE_TOML, where the enabled capability set moved on 2026-07-26. See
# schemas/machine.toml.example.
source "$TOOLS_DIR/lib/paths.sh"
# The categorized version-to-version delta (capabilities/upstreams/toolchain/commits), shared
# with tools/release so the "what changed" view and the release notes never drift (README.md#documentation-stays-owned-and-current).
source "$TOOLS_DIR/lib/delta.sh"

case "${1:-}" in
  -h|--help)
    sed -n '2,21p' "$0"
    exit 0
    ;;
esac
NO_PULL=0
[ "${1:-}" = "--no-pull" ] && NO_PULL=1
CHECK=0
[ "${1:-}" = "--check" ] && CHECK=1

# Incoming preview: the categorized delta the pull would bring (capabilities/upstreams/
# toolchain/commits, via tools/lib/delta.sh) plus which ENABLED capabilities (machine.toml, via
# toml.sh) it touches — capabilities are the churny surface, so that's the "does this update
# concern me" signal. The delta targets origin/main because that is what update.sh actually
# fast-forwards to; the newest RELEASE tag is shown separately as a version-identity line (a tag
# is a label to stand next to, not what the pull merges). Shown by --check and the interactive
# confirm.
print_incoming() {
  echo
  echo "What the update brings (HEAD..origin/main):"
  echo
  print_manifest_delta HEAD origin/main
  echo
  echo "Enabled capabilities with incoming changes:"
  CAP_INCOMING=0
  while IFS= read -r cap; do
    [ -n "$cap" ] || continue
    # Three-dot on purpose: merge-base→origin/main = what origin actually
    # brings in. Two-dot (endpoint diff) would list OUR local commits'
    # files as "incoming" on any checkout that's ahead.
    if [ -n "$(git diff --name-only HEAD...origin/main -- "capabilities/$cap/")" ]; then
      echo "  $cap"
      CAP_INCOMING=1
    fi
  done <<EOF
$(toml_array capabilities "$AXON_MACHINE_TOML")
EOF
  if [ "$CAP_INCOMING" -eq 0 ]; then
    echo "  (no enabled capability affected)"
  fi
}

echo "Axon update · $AXON_ROOT"
echo

cd "$AXON_ROOT"

# --check never pulls, so the dirty-tree guard (which only protects the
# pull) would just block a read-only status question — skip it there.
if [ "$CHECK" -eq 0 ] && [ -n "$(git status --porcelain)" ]; then
  echo "update.sh: working tree not clean — commit or stash before updating." >&2
  git status --short >&2
  exit 1
fi

echo "Fetching origin/main..."
git fetch --quiet origin main

read -r AHEAD BEHIND <<<"$(git rev-list --left-right --count HEAD...origin/main)"
echo "  $AHEAD ahead, $BEHIND behind origin/main"

if [ "$CHECK" -eq 1 ]; then
  echo
  echo "  installed: $(git describe --tags --always --dirty) ($(git log -1 --format=%cs))"
  echo "  latest:    $(git describe --tags --always origin/main) ($(git log -1 --format=%cs origin/main)) — origin/main"
  # Release-aware identity: once tags exist, say where this checkout sits relative to the newest
  # release, not only relative to the moving main branch. Silent when no release has been cut yet.
  LATEST_TAG="$(latest_release_ref)"
  [ -n "$LATEST_TAG" ] && echo "  release:   $LATEST_TAG ($(git log -1 --format=%cs "$LATEST_TAG")) — newest release tag"
  print_incoming
  echo
  echo "Check only (--check) — not pulling."
  exit 0
fi

if [ "$BEHIND" -eq 0 ]; then
  echo
  echo "Already up to date."
  exit 0
fi

if [ "$AHEAD" -gt 0 ]; then
  echo
  echo "update.sh: diverged (local commits ahead AND behind) — not auto-merging." >&2
  echo "Resolve by hand: git log HEAD..origin/main / git merge origin/main" >&2
  exit 1
fi

if [ "$NO_PULL" -eq 1 ]; then
  echo
  echo "$BEHIND commit(s) available (--no-pull given, not pulling)."
  exit 0
fi

# Interactive confirm — TTY only. Non-TTY (CI, cron, `echo | update.sh`)
# skips both preview and prompt and pulls straight through, byte-compatible
# with the pre-interactive behavior (same guard idiom as install.sh's
# capability prompt: `read` on a closed stdin would EOF non-zero under
# `set -e`).
if [ -t 0 ]; then
  print_incoming
  echo
  read -r -p "Fast-forward now? [y/N]: " CONFIRM
  case "$CONFIRM" in
    y|Y) ;;
    *)
      echo "not pulling."
      exit 0
      ;;
  esac
fi

echo
echo "Fast-forwarding..."
OLD_HEAD="$(git rev-parse HEAD)"   # before the pull, for the changed-capabilities report
git merge --ff-only origin/main

# Report which ENABLED capabilities moved in this update — capabilities are the
# churny surface, so a compact "what changed" summary is more useful than the
# raw log. Only runs on the pull path (BEHIND==0 and --no-pull both exit above),
# so --no-pull naturally skips it. Read via toml.sh; heredoc (not a pipe) keeps
# the loop in this shell so CHANGED survives.
echo
echo "Enabled capabilities changed in this update:"
CHANGED=0
while IFS= read -r cap; do
  [ -n "$cap" ] || continue
  changed_files="$(git diff --name-only "$OLD_HEAD"..HEAD -- "capabilities/$cap/")"
  if [ -n "$changed_files" ]; then
    echo "  $cap:"
    echo "$changed_files" | head | sed 's/^/    /'
    CHANGED=1
  fi
done <<EOF
$(toml_array capabilities "$AXON_MACHINE_TOML")
EOF
if [ "$CHANGED" -eq 0 ]; then
  echo "  (no enabled capability changed)"
fi

echo
echo "Re-running tools/doctor to confirm the update didn't break anything:"
echo
exec "$TOOLS_DIR/doctor"
