#!/bin/bash
# Source AFTER paths.sh. Exports AXON_OS and AXON_CONTAINER_RUNTIME from
# axon-overlay/config/machine.toml — the one place per-machine platform
# facts are declared. Capability scripts branch on these instead of
# assuming macOS and one container runtime the way the first vaultwarden cut did.

if [ -z "${AXON_PERSONAL_ROOT:-}" ]; then
  echo "platform.sh: source tools/lib/paths.sh first" >&2
  return 1 2>/dev/null || exit 1
fi
source "$(dirname "${BASH_SOURCE[0]}")/toml.sh"

# paths.sh resolves which machine this is — single-file layout, config/machines/<host>.toml,
# or an explicit name in axon.local.toml. Rebuilding the path here would mean a second,
# quietly diverging answer to the same question.
MACHINE_TOML="${AXON_MACHINE_TOML:-}"
if [ -z "$MACHINE_TOML" ] || [ ! -f "$MACHINE_TOML" ]; then
  echo "platform.sh: missing ${MACHINE_TOML:-<unresolved>} — copy schemas/machine.toml.example into the overlay, or name this machine in axon.local.toml" >&2
  return 1 2>/dev/null || exit 1
fi

AXON_OS="$(toml_get os "$MACHINE_TOML")"
AXON_CONTAINER_RUNTIME="$(toml_get container_runtime "$MACHINE_TOML")"
export AXON_OS AXON_CONTAINER_RUNTIME

if [ -z "$AXON_OS" ] || [ -z "$AXON_CONTAINER_RUNTIME" ]; then
  echo "platform.sh: $MACHINE_TOML missing 'os' or 'container_runtime'" >&2
  return 1 2>/dev/null || exit 1
fi
