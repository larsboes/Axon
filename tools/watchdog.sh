#!/bin/bash
# Generic watchdog: `watchdog.sh <capability>` keeps calling
# `service-runner.sh start <capability>`. Only meaningful for runtimes with
# no native restart policy (apple-container) — service-runner.sh skips
# installing this on docker/podman since they already have one.
set -euo pipefail
CAP="${1:?usage: watchdog.sh <capability>}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
while true; do
  # stdout is noise (the runtime echoes the container name whenever it actually starts
  # one); stderr is the only thing anyone ever needs, so it lands in the plist's
  # StandardErrorPath instead of /dev/null. `|| true` stays -- the loop has to outlive
  # any single failure -- but the reason no longer disappears along with it. A healthy
  # loop still writes nothing at all: service-runner.sh no-ops on an already-running
  # capability rather than erroring, which is what makes this affordable.
  if ! err="$("$SCRIPT_DIR/service-runner.sh" start "$CAP" 2>&1 >/dev/null)"; then
    echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') watchdog[$CAP]: ${err:-start failed, no output}" >&2
  fi
  sleep 30
done
