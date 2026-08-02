#!/bin/bash
# tools/capability.sh — manage which capabilities are enabled on this machine.
# The `capabilities = [...]` line in <overlay>/config/machine.toml is the single
# source of truth, and this is the one tool that writes it (enable/disable). It
# lives in the overlay rather than in axon.toml because the enabled set is a fact
# about one machine, and axon.toml is tracked and shared — see
# schemas/machine.toml.example. Every read goes
# through tools/lib/toml.sh; requires= dependencies are resolved transitively and
# cycle-safely on enable, and guarded on disable so a still-needed capability
# can't be stranded.
#
# No service is ever started here — after enable, the suggested
# `tools/service-runner.sh start <name>` is printed for you to run, never run
# automatically.
#
#   tools/capability.sh list             # every capability + enabled/disabled + requires
#   tools/capability.sh enable <name>    # enable <name> and everything it requires
#   tools/capability.sh disable <name>   # disable <name> (blocked if a dependent needs it)
#   tools/capability.sh registry         # the enabled set as JSON, in dependency order
#   tools/capability.sh -h               # this help
#
# bash 3.2-safe.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AXON_ROOT="$(cd "$TOOLS_DIR/.." && pwd)"

case "${1:-}" in
  -h|--help) sed -n '2,19p' "$0"; exit 0 ;;
esac

# paths.sh (not toml.sh directly) — it resolves this machine's overlay and exports
# AXON_MACHINE_TOML, the file the enabled set lives in.
source "$TOOLS_DIR/lib/paths.sh"

CAPS_DIR="$AXON_ROOT/capabilities"

if [ ! -f "$AXON_MACHINE_TOML" ]; then
  echo "capability.sh: no machine.toml at $AXON_MACHINE_TOML — run tools/install.sh first." >&2
  exit 1
fi

# --- shared reads (all via toml.sh) --------------------------------------

_enabled_names() {  # space-separated, in machine.toml order
  toml_array capabilities "$AXON_MACHINE_TOML" | tr '\n' ' '
}

_cap_requires() {  # <name> -> space-separated direct requires (empty if none/no manifest)
  local mf
  mf="$(axon_manifest_for "$1")" || return 0
  [ -n "$mf" ] || return 0
  toml_array requires "$mf" | tr '\n' ' '
}

_has_service() {  # <name> -> exit 0 if either root declares it
  axon_manifest_for "$1" >/dev/null 2>&1
}

# A capability directory, from whichever root holds it. Separate from _manifest_for
# because a capability is allowed to exist before it declares a service: `enable` and
# dependency resolution ask "is this a real capability", not "does it run".
_cap_dir_for() {  # <name> -> its directory, empty if neither root has it
  if [ -d "$CAPS_DIR/$1" ]; then
    echo "$CAPS_DIR/$1"
  elif [ -d "$AXON_OVERLAY_CAPS_DIR/$1" ]; then
    echo "$AXON_OVERLAY_CAPS_DIR/$1"
  fi
}

# Both capability roots, deduplicated. An overlay-owned capability is enabled, started
# and reported exactly like a public one; only the tracked generators ignore it.
_cap_dirs() {  # names of every <root>/capabilities/<name>/ directory, one per line
  local d root
  for root in "$CAPS_DIR" "$AXON_OVERLAY_CAPS_DIR"; do
    [ -d "$root" ] || continue
    for d in "$root"/*/; do
      [ -d "$d" ] || continue
      basename "$d"
    done
  done | sort -u
}

# A spine component may carry a service.toml at the repo root (today: dashboard/,
# README.md#three-architectural-nouns). Discovered by glob rather than listed, so the list can never go
# stale: a top-level service.toml IS the declaration. Spine services are always in the
# registry and never in machine.toml's `capabilities` — the spine exists on every
# machine by definition, which is exactly README.md#three-architectural-nouns's membership test.
_spine_names() {  # names of every <root>/<name>/service.toml, one per line
  local f
  for f in "$AXON_ROOT"/*/service.toml; do
    [ -f "$f" ] || continue
    basename "$(dirname "$f")"
  done
}

# Empty output means "no manifest", which is a normal answer here. A duplicate
# declaration is not: axon_manifest_for has already named both paths on stderr, and
# continuing would mean picking one, so this exits instead. `|| rc=$?` keeps `set -e`
# from taking the decision away first.
_manifest_for() {  # <name> -> path of its service.toml, empty if it has none
  local mf="" rc=0
  mf="$(axon_manifest_for "$1")" || rc=$?
  if [ "$rc" -eq 2 ]; then exit 2; fi
  if [ -n "$mf" ]; then echo "$mf"; fi
  return 0
}

# --- the single machine.toml write point ---------------------------------

_write_capabilities() {  # <space-separated ordered names> -> rewrite the one line
  local count
  count="$(grep -cE '^capabilities[[:space:]]*=' "$AXON_MACHINE_TOML" || true)"
  # More than one line is a corrupted file: a best-effort write would rewrite both
  # and quietly lose whichever the reader was not using. Refuse.
  if [ "$count" -gt 1 ]; then
    echo "capability.sh: found $count 'capabilities = [...]' lines in $AXON_MACHINE_TOML — fix that file by hand first." >&2
    exit 1
  fi
  local names="$1" joined="" n
  for n in $names; do
    if [ -z "$joined" ]; then joined="\"$n\""; else joined="$joined, \"$n\""; fi
  done
  local newline="capabilities = [$joined]"
  if [ "$count" -eq 0 ]; then
    # A machine.toml written before this field existed, or by an installer run that
    # had nothing to enable yet. Seed it with its explanatory comment rather than
    # demanding a hand-edit first.
    {
      echo
      echo "# Capabilities enabled on THIS machine — written by tools/capability.sh"
      echo "# (enable/disable resolve service.toml \`requires =\` transitively); hand-editing"
      echo "# is legal, and tools/doctor re-checks that the set stays dependency-closed."
      echo "# Single-line array per tools/lib/toml.sh's contract."
      echo "$newline"
    } >> "$AXON_MACHINE_TOML"
    return 0
  fi
  # `-i.bak` + rm is the portable form that behaves identically under BSD sed
  # (macOS) and GNU sed (Linux); same idiom tools/lib/toml.sh:toml_set uses.
  sed -i.bak -E "s|^capabilities[[:space:]]*=.*|$newline|" "$AXON_MACHINE_TOML"
  rm -f "$AXON_MACHINE_TOML.bak"
}

# --- transitive requires resolution (bash-3.2, cycle-safe) ---------------

# No associative arrays here on purpose (bash 3.2 / macOS stock bash has none):
# RESOLVED is a space-separated, dependency-first list; VISITING is the current
# DFS stack, which doubles as the cycle guard.
RESOLVED=""
VISITING=""

_resolve() {  # <name> -> append <name> + its transitive requires to RESOLVED, deps first
  local name="$1"
  case " $RESOLVED " in *" $name "*) return 0 ;; esac  # already settled
  case " $VISITING " in *" $name "*) return 0 ;; esac  # on the stack -> cycle, stop
  if [ -z "$(_cap_dir_for "$name")" ]; then
    echo "capability.sh: required capability '$name' has no capabilities/$name/ directory in Axon or the overlay" >&2
    exit 1
  fi
  VISITING="$VISITING $name"
  local dep
  for dep in $(_cap_requires "$name"); do
    _resolve "$dep"
  done
  case " $RESOLVED " in *" $name "*) ;; *) RESOLVED="$RESOLVED $name" ;; esac
}

# --- subcommands ----------------------------------------------------------

cmd_list() {
  local enabled; enabled=" $(_enabled_names) "  # space-padded for membership tests
  local name status reqs
  while IFS= read -r name; do
    [ -n "$name" ] || continue
    status="disabled"
    case "$enabled" in *" $name "*) status="enabled" ;; esac
    reqs="$(_cap_requires "$name")"
    reqs="${reqs% }"  # trim the single trailing space tr left behind
    if [ -n "$reqs" ]; then
      printf '  %-18s [%s]  requires: %s\n' "$name" "$status" "$reqs"
    else
      printf '  %-18s [%s]\n' "$name" "$status"
    fi
  done <<EOF
$(_cap_dirs)
EOF
}

cmd_enable() {  # <name>
  local name="$1"
  if [ -z "$(_cap_dir_for "$name")" ]; then
    echo "capability.sh: no such capability '$name'" >&2
    echo "valid options:" >&2
    _cap_dirs | sed 's/^/  /' >&2
    exit 1
  fi

  RESOLVED=""; VISITING=""
  _resolve "$name"
  echo "Resolution chain (dependencies first): ${RESOLVED# }"

  local enabled; enabled=" $(_enabled_names) "
  local newly="" n
  for n in $RESOLVED; do
    case "$enabled" in
      *" $n "*) echo "  = $n (already enabled)" ;;
      *)        newly="$newly $n"; echo "  + $n (enabling)" ;;
    esac
  done

  if [ -z "$newly" ]; then
    echo "Nothing to do — already enabled."
    exit 0
  fi

  # Append the newly-enabled names to the existing order; the existing order is
  # preserved untouched so an enable/disable round-trip is byte-identical.
  _write_capabilities "$(_enabled_names)$newly"
  echo "machine.toml: capabilities updated."

  # v0 never starts services — just surface the next step for each newly-enabled
  # capability that actually has a container manifest.
  local suggested=""
  for n in $newly; do
    _has_service "$n" && suggested="$suggested $n"
  done
  if [ -n "$suggested" ]; then
    echo
    echo "Next step (not run automatically — start each when ready):"
    for n in $suggested; do
      echo "  tools/service-runner.sh start $n"
    done
  fi
}

# --- registry (the one place manifest facts leave the shell) --------------
#
# Emits the enabled set as JSON so non-shell consumers never learn to parse TOML:
# tools/lib/toml.sh stays the only parser (README.md#one-manifest-per-concern), and everyone else reads
# this. Three consumers today — service-runner.sh's up/down fan-out, axon-status
# (which dials each health URL and starts capabilities on demand), and
# dashboard/vite.config.ts (which builds its proxy table from it). That third reader
# is the trigger axon-status's own source comment named: the port literals it used to
# duplicate now live in one manifest each.
#
# Order is dependency-first, straight out of the same _resolve the enable path uses —
# so `up` starts postgres before the capabilities that require it, and `down` walks
# the list backwards.

_json_str() {  # <value> -> a JSON string. Refuses anything that would need escaping.
  case "$1" in
    *[\"\\]*)
      echo "capability.sh: manifest value needs JSON escaping, which this emitter deliberately does not do: $1" >&2
      exit 1
      ;;
  esac
  printf '"%s"' "$1"
}

_json_array_from() {  # stdin: one element per line -> ["a", "b"]
  local out="" v
  while IFS= read -r v; do
    [ -n "$v" ] || continue
    if [ -z "$out" ]; then out="$(_json_str "$v")"; else out="$out, $(_json_str "$v")"; fi
  done
  printf '[%s]' "$out"
}

REGISTRY_FIRST=1

_emit_service() {  # <name> <manifest> <scope>
  local name="$1" mf="$2" scope="$3" kind
  kind="$(toml_get kind "$mf")"
  [ -n "$kind" ] || kind="container"
  [ "$REGISTRY_FIRST" -eq 1 ] || printf ',\n  '
  REGISTRY_FIRST=0
  printf '{"name": %s, "kind": %s, "scope": %s' \
    "$(_json_str "$name")" "$(_json_str "$kind")" "$(_json_str "$scope")"
  local key
  for key in port health_path panel_port panel_path autostart proxy_api_only idle_timeout; do
    printf ', "%s": %s' "$key" "$(_json_str "$(toml_get "$key" "$mf")")"
  done
  printf ', "proxy_extra": %s' "$(toml_array proxy_extra "$mf" | _json_array_from)"
  printf ', "requires": %s}' "$(toml_array requires "$mf" | _json_array_from)"
}

# The shell's own view of the same registry: whitespace-separated fields, because
# bash reading its own JSON with sed would be a regex parser nobody asked for. Same
# loop, same order, same source — only the rendering differs.
_emit_line() {  # <name> <manifest> <scope>
  local name="$1" mf="$2" scope="$3" kind autostart
  kind="$(toml_get kind "$mf")";           [ -n "$kind" ] || kind="container"
  autostart="$(toml_get autostart "$mf")"; [ -n "$autostart" ] || autostart="false"
  printf '%s %s %s %s\n' "$name" "$kind" "$scope" "$autostart"
}

_emit() {  # <json|lines> <name> <manifest> <scope>
  local fmt="$1"; shift
  if [ "$fmt" = lines ]; then _emit_line "$@"; else _emit_service "$@"; fi
}

cmd_registry() {  # [--lines]
  local fmt="json"
  [ "${1:-}" = "--lines" ] && fmt="lines"

  RESOLVED=""; VISITING=""
  local n
  for n in $(_enabled_names); do _resolve "$n"; done

  REGISTRY_FIRST=1
  if [ "$fmt" = json ]; then printf '[\n  '; fi
  # scope names the root a capability came from, so consumers do not have to re-derive
  # it from a path. tools/self.ts uses it to keep overlay capabilities out of self.json:
  # that file is tracked and public, and a capability name is a fact about a private
  # deployment. An implicit "is it in the repo" test would work today and rot silently.
  local _mf _scope
  for n in $RESOLVED; do
    _has_service "$n" || continue
    _mf="$(_manifest_for "$n")"
    case "$_mf" in
      "$AXON_OVERLAY_CAPS_DIR"/*) _scope="overlay-capability" ;;
      *)                          _scope="capability" ;;
    esac
    _emit "$fmt" "$n" "$_mf" "$_scope"
  done
  # Spine last: it is the shell that consumes the capabilities, so bringing it up
  # after them means its first discovery call already sees the truth.
  while IFS= read -r n; do
    [ -n "$n" ] || continue
    _emit "$fmt" "$n" "$AXON_ROOT/$n/service.toml" spine
  done <<EOF
$(_spine_names)
EOF
  if [ "$fmt" = json ]; then printf '\n]\n'; fi
}

cmd_disable() {  # <name>
  local name="$1"
  local enabled; enabled=" $(_enabled_names) "

  case "$enabled" in
    *" $name "*) ;;
    *) echo "capability.sh: '$name' is not enabled — nothing to do."; exit 0 ;;
  esac

  # Block if any OTHER enabled capability directly requires this one — removing
  # it would strand that dependent. Direct requires suffices: a transitive
  # dependent reaches <name> only through an enabled intermediate that itself
  # directly requires it, and that intermediate is caught here.
  local dependents="" e dep
  for e in $(_enabled_names); do
    [ "$e" = "$name" ] && continue
    for dep in $(_cap_requires "$e"); do
      [ "$dep" = "$name" ] && dependents="$dependents $e"
    done
  done
  if [ -n "$dependents" ]; then
    echo "capability.sh: cannot disable '$name' — still required by:${dependents}" >&2
    echo "disable the dependent(s) first." >&2
    exit 1
  fi

  local kept="" n
  for n in $(_enabled_names); do
    [ "$n" = "$name" ] || kept="$kept $n"
  done
  _write_capabilities "$kept"
  echo "Disabled '$name'. machine.toml: capabilities updated."
}

case "${1:-list}" in
  list)     cmd_list ;;
  enable)   [ -n "${2:-}" ] || { echo "usage: capability.sh enable <name>" >&2; exit 1; }; cmd_enable "$2" ;;
  disable)  [ -n "${2:-}" ] || { echo "usage: capability.sh disable <name>" >&2; exit 1; }; cmd_disable "$2" ;;
  registry) cmd_registry "${2:-}" ;;
  *)        echo "usage: capability.sh list | enable <name> | disable <name> | registry [--lines]" >&2; exit 1 ;;
esac
