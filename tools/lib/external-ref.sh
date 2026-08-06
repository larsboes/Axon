#!/bin/bash
# tools/lib/external-ref.sh — the one resolver for a capability this machine CONSUMES but
# does not RUN (retired-tracker#169).
#
# Axon's default is loopback: a capability is a process on this host, and every consumer
# computes http://127.0.0.1:<port>. That default is right often enough that it was never
# written down as an assumption — until a vault moved to another host owned by another
# overlay, and the tools that needed its address each grew their own lookup.
#
# The shape those lookups converged on, generalized here exactly once:
#
#   <overlay>/config/machine.toml     [capability.<name>] provided_by = "<systems id>"
#   <overlay>/config/systems.local.toml   [<systems id>]  url = "https://..."
#
# Two files rather than one because they answer two different questions. machine.toml is
# already where "what does THIS machine run" lives, so "and this one it doesn't, someone
# else does" belongs beside it. systems.local.toml is already the private half of Axon's
# systems map, and the address has a home there — putting a URL in machine.toml too would
# be two homes for one fact. The indirection also buys provenance: `provided_by` names WHO
# provides it, and the id is free to differ from the capability name.
#
# What this deliberately does NOT do: resolve loopback. A locally managed capability is
# unchanged by every line in this file — axon-status still builds its own 127.0.0.1 URL
# from the manifest port, and it should, because that is not a reference to anything.
# Absent `provided_by`, nothing here has an opinion.
#
# bash 3.2-safe, single-line TOML only (tools/lib/toml.sh), because the Bazel sh_test
# sandbox that gates this has no bun.
#
# Requires: tools/lib/paths.sh sourced first (AXON_MACHINE_TOML, AXON_OVERLAY_ROOT), which
# already sources toml.sh.

# Where the private half of the systems map lives for the active overlay.
axon_systems_local() {
  echo "$AXON_OVERLAY_ROOT/config/systems.local.toml"
}

# capability_provider <name> — the systems id declared as this capability's external
# provider on this machine, empty when the machine manages it itself.
capability_provider() {
  [ -f "$AXON_MACHINE_TOML" ] || return 0
  toml_get_in "capability.$1" provided_by "$AXON_MACHINE_TOML"
}

# external_capabilities — every capability name this machine declares an external provider
# for, one per line. The declaration IS the list: an `external = [...]` array beside it
# could disagree with the sections, and the failure mode of two lists is that the shorter
# one silently wins.
external_capabilities() {
  [ -f "$AXON_MACHINE_TOML" ] || return 0
  local sec name
  for sec in $(toml_sections "$AXON_MACHINE_TOML"); do
    case "$sec" in
      capability.*) name="${sec#capability.}" ;;
      *) continue ;;
    esac
    [ -n "$(toml_get_in "$sec" provided_by "$AXON_MACHINE_TOML")" ] || continue
    echo "$name"
  done
}

# capability_endpoint <name> [ENV_KEY] — the base URL a CLIENT on this machine should dial
# for <name>, on stdout.
#
#   1. the declared provider's url, when this machine consumes the capability
#   2. ENV_KEY out of the capability's own env_file, when named — this machine HOSTS it,
#      and a server's own config is the only place that knows its public address. The
#      caller passes the key because it is the caller that knows the convention
#      (vaultwarden calls it DOMAIN); a generic reader guessing at env var names would be
#      inventing a contract no capability agreed to.
#   3. nothing.
#
# Exit codes are the point of this function as much as the output: 0 resolved, 1 nothing
# declared (the caller writes the message — it knows what the address is FOR), 2 a
# dangling reference, already reported here.
#
# 2 never degrades to 3. A `provided_by` naming an id with no url means the operator said
# something specific and got it wrong, and quietly falling through to a local address
# would point the caller at whatever answers on this host — which for a password vault is
# either nothing or, far worse, the wrong vault.
capability_endpoint() {  # <name> [ENV_KEY]
  local name="$1" env_key="${2:-}" provider url env_file systems
  provider="$(capability_provider "$name")"
  if [ -n "$provider" ]; then
    systems="$(axon_systems_local)"
    if [ -f "$systems" ]; then
      url="$(toml_get_in "$provider" url "$systems")"
    else
      url=""
    fi
    if [ -z "$url" ]; then
      echo "external-ref: $AXON_MACHINE_TOML declares [capability.$name] provided_by = \"$provider\"," >&2
      echo "  but $systems has no [$provider] url = \"...\" to resolve it to." >&2
      echo "  Add that entry, or drop the provided_by line if this machine runs $name itself." >&2
      return 2
    fi
    echo "${url%/}"
    return 0
  fi

  if [ -n "$env_key" ]; then
    env_file="$AXON_OVERLAY_ROOT/$(_capability_env_file "$name")"
    if [ -f "$env_file" ]; then
      url="$(grep -m1 "^$env_key=" "$env_file" | cut -d= -f2-)"
      if [ -n "$url" ]; then
        echo "${url%/}"
        return 0
      fi
    fi
  fi

  return 1
}

# The env_file the capability's manifest declares, relative to the overlay. Empty when the
# capability has no manifest here at all, which is the normal state for one this machine
# only consumes — its manifest lives in whichever repo owns the host.
_capability_env_file() {  # <name>
  local mf="$AXON_ROOT/capabilities/$1/service.toml"
  [ -f "$mf" ] || return 0
  toml_get env_file "$mf"
}
