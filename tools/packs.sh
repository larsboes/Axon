#!/bin/bash
# packs.sh — list and (un)link Axon Packs into the active harness.
#
# A Pack (Packs/<name>/pack.toml + skills/) is a togglable bundle of agent
# skills. Skills live in Axon (public, redacted); linking symlinks them into
# ~/.claude/skills/ so they enrich what the harness already loads. Personal
# values are read from the overlay at runtime, never baked into the link.
#
# Optional, convention-based, no pack.toml field: a pack MAY also carry an
# agents/ directory of Claude-Code-native subagent .md files. If present, it
# is linked as one directory symlink (Claude Code scans agent dirs
# recursively, so a single `~/.claude/agents/<pack>/` link exposes every
# agent inside it -- same pattern as a skill dir, just one link for the
# whole pack instead of one per skill). This is deliberately NOT part of the
# neutral pack.toml spec (see README.md#harness-neutral-packs) -- it is a
# Claude-Code-only convention this deployer happens to also understand;
# other harnesses' deployers simply won't look for an agents/ dir.
#
# bash 3.2-safe. Usage: packs.sh list | link <name> | unlink <name>
set -e

_lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/lib" && pwd)"
source "$_lib/paths.sh"

PACKS_DIR="$AXON_ROOT/Packs"
SKILLS_DEST="${CLAUDE_SKILLS_DIR:-$HOME/.claude/skills}"
AGENTS_DEST="${CLAUDE_AGENTS_DIR:-$HOME/.claude/agents}"

_pack_toml() { echo "$PACKS_DIR/$1/pack.toml"; }

# A pack manifest may name a dedicated `deployer`; that tool owns the pack's
# destinations and this adapter must not link it. Without it, one skill can be
# claimed by a symlink here, a packs-codex copy, and a dedicated ledger at once,
# all pointing at the same path (see schemas/pack.toml.example).
_pack_deployer() { toml_get deployer "$(_pack_toml "$1")"; }

_skill_link_state() {  # <pack> <skill> -> prints state
  local src="$PACKS_DIR/$1/skills/$2" dst="$SKILLS_DEST/$2"
  if [ -L "$dst" ]; then
    [ "$(cd "$(dirname "$dst")" && cd "$(readlink "$dst")" 2>/dev/null && pwd)" = "$src" ] \
      && echo "linked" || echo "linked-elsewhere"
  elif [ -e "$dst" ]; then echo "occupied"    # a real dir/file is already there
  else echo "unlinked"; fi
}

_retired_skill_link_state() {  # <pack> <skill> -> prints state without requiring the retired source to exist
  local src="$PACKS_DIR/$1/skills/$2" dst="$SKILLS_DEST/$2" target
  if [ -L "$dst" ]; then
    target="$(readlink "$dst")"
    [ "$target" = "$src" ] && echo "linked" || echo "linked-elsewhere"
  elif [ -e "$dst" ]; then echo "occupied"
  else echo "unlinked"; fi
}

_agents_link_state() {  # <pack> -> prints state (only meaningful if Packs/<pack>/agents/ exists)
  local src="$PACKS_DIR/$1/agents" dst="$AGENTS_DEST/$1"
  if [ -L "$dst" ]; then
    [ "$(cd "$(dirname "$dst")" && cd "$(readlink "$dst")" 2>/dev/null && pwd)" = "$src" ] \
      && echo "linked" || echo "linked-elsewhere"
  elif [ -e "$dst" ]; then echo "occupied"
  else echo "unlinked"; fi
}

cmd_list() {
  [ -d "$PACKS_DIR" ] || { echo "no Packs/ yet"; return 0; }
  for toml in "$PACKS_DIR"/*/pack.toml; do
    [ -f "$toml" ] || continue
    local name desc
    name="$(toml_get name "$toml")"
    desc="$(toml_get description "$toml")"
    printf '\n\033[1m%s\033[0m — %s\n' "$name" "$desc"
    local owner; owner="$(_pack_deployer "$name")"
    if [ -n "$owner" ]; then
      # Shown, not hidden: a pack that silently vanished from `list` would read
      # as missing rather than as owned elsewhere.
      printf '  %-24s [%s]\n' "(all skills)" "deployed by $owner"
      continue
    fi
    toml_array skills "$toml" | while IFS= read -r skill; do
      printf '  %-24s [%s]\n' "$skill" "$(_skill_link_state "$name" "$skill")"
    done
    toml_array retired_skills "$toml" | while IFS= read -r skill; do
      [ -n "$skill" ] || continue
      case "$(_retired_skill_link_state "$name" "$skill")" in
        linked) printf '  %-24s [%s]\n' "$skill" "retired-linked";;
      esac
    done
    if [ -d "$PACKS_DIR/$name/agents" ]; then
      printf '  %-24s [%s]\n' "agents/" "$(_agents_link_state "$name")"
    fi
  done
  echo
}

cmd_link() {  # <name>
  local name="$1" toml; toml="$(_pack_toml "$name")"
  [ -f "$toml" ] || { echo "no such pack: $name" >&2; exit 1; }
  local owner; owner="$(_pack_deployer "$name")"
  [ -z "$owner" ] || {
    echo "$name is deployed by $owner, not linked from here — use that tool" >&2
    exit 1
  }
  mkdir -p "$SKILLS_DEST"
  # Process substitution, not `toml_array | while` -- a pipe's right side runs in
  # a subshell in bash, so a fail flag set inside a `cmd | while read` loop is
  # invisible after `done` and cmd_link would exit 0 even on a silent collision.
  # `< <(...)` keeps the loop in this shell so `fail` actually persists.
  local fail=0
  while IFS= read -r skill; do
    local src="$PACKS_DIR/$name/skills/$skill" dst="$SKILLS_DEST/$skill"
    if [ ! -d "$src" ]; then echo "  ✗ $skill: missing at $src" >&2; fail=1; continue; fi
    case "$(_skill_link_state "$name" "$skill")" in
      linked)          echo "  = $skill (already linked)";;
      occupied)        echo "  ✗ $skill: $dst exists and is not our symlink — leaving it" >&2; fail=1;;
      linked-elsewhere)echo "  ✗ $skill: $dst points elsewhere — leaving it" >&2; fail=1;;
      unlinked)        ln -s "$src" "$dst" && echo "  ✓ $skill linked";;
    esac
  done < <(toml_array skills "$toml")
  if [ -d "$PACKS_DIR/$name/agents" ]; then
    mkdir -p "$AGENTS_DEST"
    local asrc="$PACKS_DIR/$name/agents" adst="$AGENTS_DEST/$name"
    case "$(_agents_link_state "$name")" in
      linked)          echo "  = agents/ (already linked)";;
      occupied)        echo "  ✗ agents/: $adst exists and is not our symlink — leaving it" >&2; fail=1;;
      linked-elsewhere)echo "  ✗ agents/: $adst points elsewhere — leaving it" >&2; fail=1;;
      unlinked)        ln -s "$asrc" "$adst" && echo "  ✓ agents/ linked";;
    esac
  fi
  # Remove a retired name only after every current skill linked successfully. The exact
  # symlink target is the ownership proof; a real directory or foreign link is never touched.
  if [ "$fail" = 0 ]; then
    while IFS= read -r skill; do
      [ -n "$skill" ] || continue
      local dst="$SKILLS_DEST/$skill"
      case "$(_retired_skill_link_state "$name" "$skill")" in
        linked) rm "$dst" && echo "  ✓ $skill retired link removed";;
        linked-elsewhere|occupied)
          echo "  ✗ $skill: retired name is not our symlink — leaving it" >&2
          fail=1
          ;;
        unlinked) :;;
      esac
    done < <(toml_array retired_skills "$toml")
  fi
  # Fail-at-end, not fail-fast: every skill (and the agents/ dir, if present)
  # still gets attempted even after a collision, so a partial link is
  # possible on exit 1 -- re-run `list` to see exactly what landed.
  [ "$fail" = 0 ] || { echo "  ✗ $name: one or more skills/agents did not link — see above" >&2; exit 1; }
}

cmd_unlink() {  # <name>
  local name="$1" toml; toml="$(_pack_toml "$name")"
  [ -f "$toml" ] || { echo "no such pack: $name" >&2; exit 1; }
  toml_array skills "$toml" | while IFS= read -r skill; do
    local dst="$SKILLS_DEST/$skill"
    if [ "$(_skill_link_state "$name" "$skill")" = "linked" ]; then
      rm "$dst" && echo "  ✓ $skill unlinked"
    else
      echo "  = $skill (not our link — skipped)"
    fi
  done
  toml_array retired_skills "$toml" | while IFS= read -r skill; do
    [ -n "$skill" ] || continue
    local dst="$SKILLS_DEST/$skill"
    if [ "$(_retired_skill_link_state "$name" "$skill")" = "linked" ]; then
      rm "$dst" && echo "  ✓ $skill retired link unlinked"
    else
      echo "  = $skill retired link (not ours — skipped)"
    fi
  done
  if [ -d "$PACKS_DIR/$name/agents" ]; then
    local adst="$AGENTS_DEST/$name"
    if [ "$(_agents_link_state "$name")" = "linked" ]; then
      rm "$adst" && echo "  ✓ agents/ unlinked"
    else
      echo "  = agents/ (not our link — skipped)"
    fi
  fi
}

case "${1:-list}" in
  list)   cmd_list;;
  link)   [ -n "${2:-}" ] || { echo "usage: packs.sh link <name>" >&2; exit 1; }; cmd_link "$2";;
  unlink) [ -n "${2:-}" ] || { echo "usage: packs.sh unlink <name>" >&2; exit 1; }; cmd_unlink "$2";;
  *)      echo "usage: packs.sh list | link <name> | unlink <name>" >&2; exit 1;;
esac
