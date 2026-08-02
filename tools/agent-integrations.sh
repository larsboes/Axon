#!/bin/bash
# agent-integrations.sh — install an adopted upstream's OWN agent-harness integration,
# from the version pinned in upstreams.toml.
#
# Some upstreams ship an editor/agent integration alongside the tool: a skill, a hook, a
# plugin. graphify is the first, and `graphify install --platform <p>` covers nineteen
# harnesses. This script drives those installers at the pinned version instead of Axon
# keeping a copy of what they emit.
#
# ## Why this exists rather than a checked-in copy
#
# The integration files are upstream's artifact. Committing them here would be a second
# copy of someone else's logic that no `git pull` updates and no cooldown gate governs —
# the same "second copy of the same logic" antipattern Rule 8 forbids at manifest scale,
# and Rule 3's fork rung at file scale. Rule 3's ladder says Adopt first, and graphify is
# already adopted with a verdict and a pin, so the integration comes from that pin.
#
# It is not a pure Adopt, though, and the deviation is worth stating (Rule 3 rung 3,
# "pinned source + local delta"): graphify writes its OpenCode plugin PROJECT-LOCALLY,
# into `.opencode/plugins/` of whatever directory the installer ran from, and registers
# it in that project's opencode.json. That placement is how ~/.opencode came to exist on
# this machine — installed from $HOME, so the plugin only ever loaded when OpenCode
# resolved its project root to $HOME, and never in the one checkout that actually has a
# graph. The hook is self-gating (it no-ops unless the session's directory has a
# graphify-out/graph.json), so hoisting it to the global plugin dir costs nothing in
# projects without a graph and works in every project with one. That hoist is this
# script's only delta, and it is re-derived from the pin on every run — nothing is
# frozen into git.
#
# Skills are unaffected: graphify installs those globally itself.
#
# ## Adding an upstream
#
# Add a row to INTEGRATIONS below. The pin ALWAYS comes from upstreams.toml — never
# restate a version here (Rule 8: one manifest per concern).
#
# Usage:
#   tools/agent-integrations.sh list                 what is available, and its pin
#   tools/agent-integrations.sh status               what is installed on this machine
#   tools/agent-integrations.sh status --json         machine-readable status for tooling
#   tools/agent-integrations.sh status --machine       key/value-ish status, one line per harness
#   tools/agent-integrations.sh install <harness>... e.g. install opencode claude
#   tools/agent-integrations.sh install --all-configured every harness with a config dir
#   graphify detection states: runnable / configured / stale / integrated
#
# Exit 0 = done / nothing to do, 1 = usage or a missing prerequisite.

set -euo pipefail

# Not named _lib: paths.sh ends with `unset _lib`, which would take this one with it and
# leave the next source dereferencing an unset var under `set -u`.
_ai_lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/lib" && pwd)"
source "$_ai_lib/paths.sh"
source "$_ai_lib/toml.sh"

UPSTREAMS="$AXON_ROOT/upstreams.toml"

# Harnesses Axon knows how to detect, and where each keeps its global config.
# graphify's --platform vocabulary is much longer; these are the ones Axon has an
# opinion about. Adding one is a row here plus, if it needs a delta, a case in
# post_install().
harness_command() {
  case "$1" in
    claude)   echo "claude" ;;
    opencode) echo "opencode" ;;
    codex)    echo "codex" ;;
    pi)       echo "pi" ;;
    *)        echo "" ;;
  esac
}

harness_config_dir() {
  case "$1" in
    claude)   echo "$HOME/.claude" ;;
    opencode) echo "${OPENCODE_CONFIG_DIR:-$HOME/.config/opencode}" ;;
    codex)    echo "$HOME/.codex" ;;
    pi)       echo "$HOME/.pi" ;;
    *)        echo "" ;;
  esac
}
HARNESSES="claude opencode codex pi"
ASSISTANT_INTEGRATION_STATE_FILE=".graphify-upstream-pin"

# ── graphify ────────────────────────────────────────────────────────────────
# `uv tool run --from graphifyy==<pin>` and NOT the bare `graphify` on PATH — the
# interactive skill auto-upgrades that one for its own purposes, which would silently
# drag Axon onto whatever version it happens to be. Same reasoning, and the same
# invocation shape, as tools/graphify.sh. See upstreams.toml [graphify].
graphify_pin() { toml_get_in graphify pin "$UPSTREAMS"; }

graphify_has_real_graph() {
  [ -f "$AXON_ROOT/graphify-out/graph.json" ] && [ -s "$AXON_ROOT/graphify-out/graph.json" ]
}

graphify_install_command() {  # graphify_install_command <harness>
  local harness="$1" pin
  pin="$(graphify_pin)"
  [ -n "$pin" ] || return 0
  if command -v graphify >/dev/null 2>&1; then
    printf 'graphify install --platform %s' "$harness"
    return 0
  fi
  printf 'uv tool run --from "graphifyy==%s" graphify install --platform %s' "$pin" "$harness"
}

graphify_command_version() {  # graphify_command_version <command>
  local binary="$1" version=""
  [ -n "$binary" ] || { printf ""; return 0; }
  if "$binary" --version >/dev/null 2>&1; then
    version="$("$binary" --version 2>/dev/null | head -n 1 | tr -d '\r\n')"
  elif "$binary" -V >/dev/null 2>&1; then
    version="$("$binary" -V 2>/dev/null | head -n 1 | tr -d '\r\n')"
  fi
  printf '%s' "$version"
}

graphify_install() {  # graphify_install <harness>
  local harness="$1" pin scratch
  pin="$(graphify_pin)"
  [ -n "$pin" ] || { echo "✗ no pin for [graphify] in $UPSTREAMS" >&2; return 1; }

  # Run from a scratch cwd so the project-local half of the installer lands somewhere
  # disposable instead of polluting whatever directory the operator happens to be in —
  # which is exactly the accident that produced ~/.opencode. The global half (skills)
  # still goes where graphify puts it.
  scratch="$(mktemp -d)"
  # shellcheck disable=SC2064
  trap "rm -rf '$scratch'" RETURN

  if command -v graphify >/dev/null 2>&1; then
    ( cd "$scratch" && graphify install --platform "$harness" ) \
      || { echo "✗ graphify install --platform $harness failed" >&2; return 1; }
  else
    need_uv
    ( cd "$scratch" && uv tool run --from "graphifyy==$pin" graphify install --platform "$harness" ) \
      || { echo "✗ graphify install --platform $harness failed" >&2; return 1; }
  fi

  graphify_post_install "$harness" "$scratch"
}

# The local delta, applied per harness. Everything not named here is pure upstream.
graphify_post_install() {  # graphify_post_install <harness> <scratch>
  local harness="$1" scratch="$2" src dst
  case "$harness" in
    opencode)
      local skills_src skills_dst
      src="$scratch/.opencode/plugins/graphify.js"
      dst="$(harness_config_dir opencode)/plugins/graphify.js"
      skills_src="$scratch/.opencode/skills/graphify"
      skills_dst="$(harness_config_dir opencode)/skills/graphify"
      if [ -d "$skills_src" ]; then
        mkdir -p "$skills_dst"
        if ! cp -a "$skills_src"/. "$skills_dst"/; then
          echo "  x failed to install graphify skill to $skills_dst" >&2
          return 1
        fi
      fi
      if [ -s "$src" ] && grep -qi 'graphify' "$src"; then
        mkdir -p "$(dirname "$dst")"
        local staged="$dst.tmp.$$"
        if ! install -m 0644 "$src" "$staged" || ! mv -f "$staged" "$dst"; then
          rm -f "$staged"
          echo "  x failed to install the verified OpenCode plugin atomically" >&2
          return 1
        fi
        echo "  hoisted plugin  -> $dst (global; upstream writes it project-local)"
      else
        # Upstream stopped emitting it, or moved it. Say so rather than silently
        # leaving a stale copy in place: the delta is only safe while it still applies.
        echo "  ⚠ upstream emitted no .opencode/plugins/graphify.js at this pin —"
        echo "    the global-hoist delta no longer applies; re-read its installer."
      fi
      ;;
  esac
}

json_escape() {
  local text="$1"
  text="${text//\\/\\\\}"
  text="${text//\"/\\\"}"
  text="${text//$'\n'/\\n}"
  text="${text//$'\r'/\\r}"
  printf '%s' "$text"
}

graphify_integration_state_file() {
  local dir="$1"
  echo "$dir/$ASSISTANT_INTEGRATION_STATE_FILE"
}

graphify_detect() {  # graphify_detect <harness> -> pipe-delimited row
  local harness="$1"
  local pin dir command skill plugin configured state stale integration_ready command_version
  local marker_file marker_pin dir_present runnable command_present graph_state marker_match install_command

  pin="$(graphify_pin)"
  dir="$(harness_config_dir "$harness")"
  command="$(harness_command "$harness")"
  marker_file="$(graphify_integration_state_file "$dir")"
  graph_state="missing"
  graphify_has_real_graph && graph_state="present"
  install_command="$(graphify_install_command "$harness")"

  dir_present="no"
  runnable="no"
  command_present="no"
  skill="no"
  plugin="n/a"
  configured="no"
  stale="no"
  integration_ready="no"
  command_version="unknown"

  if [ -n "$dir" ] && [ -d "$dir" ]; then
    dir_present="yes"
    if [ -n "$command" ] && command -v "$command" >/dev/null 2>&1; then
      runnable="yes"
      command_present="yes"
      command_version="$(graphify_command_version "$command")"
    fi

    if [ "$harness" = "opencode" ]; then
      [ -d "$dir/skills/graphify" ] && skill="yes"
      [ -f "$dir/plugins/graphify.js" ] && plugin="yes" || plugin="no"
    else
      [ -d "$dir/skills/graphify" ] && skill="yes"
      plugin="n/a"
    fi

    if [ "$skill" = "yes" ] || [ "$plugin" = "yes" ]; then
      configured="yes"
    fi

    if [ "$harness" = "opencode" ]; then
      integration_ready="no"
      [ "$skill" = "yes" ] && [ "$plugin" = "yes" ] && integration_ready="yes"
    else
      integration_ready="$skill"
    fi

    marker_pin=""
    marker_match="no"
    if [ -f "$marker_file" ]; then
      marker_pin="$(tr -d '\n\r' < "$marker_file")"
      [ -n "$pin" ] && [ "$marker_pin" = "$pin" ] && marker_match="yes"
    fi

    if [ "$runnable" = "yes" ] && [ "$configured" = "yes" ] && [ "$integration_ready" = "yes" ] && [ "$marker_match" = "yes" ]; then
      state="integrated"
    elif [ "$configured" = "yes" ] && { [ "$runnable" != "yes" ] || [ "$integration_ready" != "yes" ] || [ "$marker_match" != "yes" ]; }; then
      state="stale"
      stale="yes"
    elif [ "$runnable" = "yes" ]; then
      state="runnable"
    elif [ "$configured" = "yes" ]; then
      state="configured"
    else
      state="missing"
    fi
  else
    state="missing"
  fi

  [ -n "$command_version" ] || command_version="unknown"
  printf '%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s\n' \
    "$harness" "$state" "$dir_present" "$command" "$runnable" "$command_present" \
    "$configured" "$integration_ready" "$stale" "$graph_state" "$dir" "$skill" "$plugin" \
    "$command_version" "$( [ -n "$install_command" ] && echo "$install_command" )"
}

graphify_status() {  # graphify_status <harness>
  local harness="$1" row state dir skill plugin
  row="$(graphify_detect "$harness")"
  IFS='|' read -r harness state _ _ _ _ _ _ _ _ dir skill plugin _ _ <<<"$row"
  printf '  %-9s state=%-11s skill=%-4s plugin=%-4s %s\n' "$harness" "$state" "$skill" "$plugin" "$dir"
}

graphify_write_marker() { # graphify_write_marker <harness>
  local harness="$1" dir file pin
  dir="$(harness_config_dir "$harness")"
  [ -d "$dir" ] || return 0
  pin="$(graphify_pin)"
  file="$(graphify_integration_state_file "$dir")"
  printf '%s' "$pin" > "$file"
}

cmd_status() { # cmd_status [json|machine]
  local output="${1:-human}"
  local h row state dir_present command runnable command_present configured integration_ready stale graph_state dir skill plugin command_version install_command
  local pin first="1"

  pin="$(graphify_pin)"
  if [ "$output" = "json" ]; then
    printf '{ "upstream": "graphify", "pin": "%s", "harnesses": [' "$(json_escape "$pin")"
  elif [ "$output" = "machine" ]; then
    :
  else
    echo "graphify (pin $(graphify_pin)):"
  fi

  for h in $HARNESSES; do
    row="$(graphify_detect "$h")"
    IFS='|' read -r harness state dir_present command runnable command_present configured integration_ready stale graph_state dir skill plugin command_version install_command <<<"$row"
    if [ "$output" = "json" ]; then
      if [ "$first" = "0" ]; then
        printf ','
      fi
      first="0"
      [ -n "$install_command" ] || install_command="$(graphify_install_command "$h")"
      printf '{'
      printf '"name":"%s",' "$(json_escape "$h")"
      printf '"state":"%s",' "$(json_escape "$state")"
      printf '"runnable":%s,' "$( [ "$runnable" = "yes" ] && printf true || printf false )"
      printf '"configured":%s,' "$( [ "$configured" = "yes" ] && printf true || printf false )"
      printf '"integrated":%s,' "$( [ "$state" = "integrated" ] && printf true || printf false )"
      printf '"stale":%s,' "$( [ "$stale" = "yes" ] && printf true || printf false )"
      printf '"command":"%s",' "$( [ -n "$command" ] && json_escape "$command" || echo "" )"
      printf '"config_dir":"%s",' "$(json_escape "$dir")"
      printf '"skill":"%s",' "$(json_escape "$skill")"
      printf '"plugin":"%s",' "$(json_escape "$plugin")"
      printf '"graph_state":"%s",' "$(json_escape "$graph_state")"
      printf '"command_version":"%s",' "$(json_escape "$command_version")"
      printf '"command_present":%s' "$( [ "$command_present" = "yes" ] && printf true || printf false )"
      printf ',"install_command":"%s"' "$(json_escape "$install_command")"
      printf '}'
    elif [ "$output" = "machine" ]; then
      printf '%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s|%s\n' \
        "$h" "$state" "$dir_present" "$command" "$runnable" "$command_present" \
        "$configured" "$integration_ready" "$stale" "$graph_state" "$dir" \
        "$(json_escape "$skill")" "$(json_escape "$plugin")" "$(json_escape "$command_version")" \
        "$(json_escape "$install_command")"
    else
      printf '  %-9s state=%-11s skill=%-4s plugin=%-4s %s\n' \
        "$h" "$state" "$skill" "$plugin" "$dir"
    fi
  done

  if [ "$output" = "json" ]; then
    printf ' ] }'
  fi
}

# ── dispatch ────────────────────────────────────────────────────────────────

need_uv() {
  command -v uv >/dev/null 2>&1 || {
    echo "✗ uv is required (Rule 17) — see toolchain.toml [uv]" >&2
    exit 1
  }
}

configured_harnesses() {
  local h dir
  for h in $HARNESSES; do
    dir="$(harness_config_dir "$h")"
    [ -n "$dir" ] && [ -d "$dir" ] && echo "$h"
  done
}

cmd_list() {
  echo "Agent-harness integrations shipped by adopted upstreams:"
  echo
  printf '  %-10s pin %-10s harnesses: %s\n' "graphify" "$(graphify_pin)" "$HARNESSES"
  echo
  echo "Pins come from upstreams.toml. Configured on this machine: $(configured_harnesses | tr '\n' ' ')"
}

cmd_install() {
  local targets=""
  if [ "${1:-}" = "--all-configured" ] || [ "${1:-}" = "--all-detected" ]; then
    targets="$(configured_harnesses)"
    [ -n "$targets" ] || { echo "No known harness config dirs found — nothing to do."; return 0; }
  else
    targets="$*"
  fi
  [ -n "$targets" ] || { echo "usage: agent-integrations.sh install <harness>... | --all-configured" >&2; exit 1; }

  local h
  for h in $targets; do
    if [ -z "$(harness_config_dir "$h")" ]; then
      echo "✗ unknown harness '$h' (known: $HARNESSES)" >&2
      exit 1
    fi
    echo "graphify -> $h"
    graphify_install "$h"
    graphify_write_marker "$h"
  done
  echo
  echo "Restart the harness to pick up new plugins (most load them once at startup)."
}

case "${1:-list}" in
  list|--list)     cmd_list ;;
  status|--status) shift; cmd_status "${1#--}" ;;
  install)         shift; cmd_install "$@" ;;
  -h|--help|help)  sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//' ;;
  *) echo "agent-integrations.sh: unknown command '$1' (try: list|status|install|-h)" >&2; exit 1 ;;
esac
