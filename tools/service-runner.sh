#!/bin/bash
# The shared "service manifest" interpreter (schemas/service.toml.example).
# Capabilities declare WHAT they need; this is the ONLY place that knows HOW to
# satisfy it on this machine. Two kinds, one interpreter: `kind = "container"`
# (the default) hands it to the container runtime, `kind = "process"` execs a host
# process directly -- a compiled server, or a dev server whose whole point is that
# it reloads while it runs. A new capability of either kind is a new service.toml,
# not a new script.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AXON_ROOT="$(cd "$TOOLS_DIR/.." && pwd)"
source "$TOOLS_DIR/lib/paths.sh"
source "$TOOLS_DIR/lib/platform.sh"
source "$TOOLS_DIR/lib/toml.sh"

usage() {
  echo "usage: service-runner.sh <start|stop|idle-stop|restart|resume|status|install-persistence> <capability>" >&2
  echo "       service-runner.sh up [--all]     # start the autostart set (--all: everything enabled)" >&2
  echo "       service-runner.sh down           # stop everything enabled, dependents first" >&2
  echo "       service-runner.sh status         # one line per enabled service" >&2
  exit 1
}
CMD="${1:-}"; CAP="${2:-}"
[ -n "$CMD" ] || usage

# AXON_CONTAINER_RUNTIME is manifest vocabulary, not always the command name
# (apple-container's CLI is `container`). Resolve the binary lazily, on the first
# container operation, so a machine running only process capabilities never needs a
# container runtime installed at all -- and an unresolvable runtime still says so by
# name. It used to surface as bash's `command not found` from inside start_service --
# which is how the launchd-PATH defect stayed invisible for two weeks (see
# install_persistence).
RUNTIME_BIN=""; RUNTIME_PATH=""
resolve_runtime() {
  [ -z "$RUNTIME_PATH" ] || return 0
  case "$AXON_CONTAINER_RUNTIME" in
    apple-container) RUNTIME_BIN="container" ;;
    docker|podman)   RUNTIME_BIN="$AXON_CONTAINER_RUNTIME" ;;
    *)
      echo "service-runner.sh: unsupported container_runtime '$AXON_CONTAINER_RUNTIME' (axon-overlay/config/machine.toml)" >&2
      exit 1
      ;;
  esac
  RUNTIME_PATH="$(command -v "$RUNTIME_BIN" 2>/dev/null || true)"
  [ -n "$RUNTIME_PATH" ] || {
    echo "service-runner.sh: container_runtime '$AXON_CONTAINER_RUNTIME' needs '$RUNTIME_BIN' on PATH, not found (PATH=$PATH)" >&2
    exit 1
  }
}

# --- fan-out over the enabled set ----------------------------------------
# No capability argument means "this whole machine". The order is capability.sh's
# registry order -- dependency-first, from the one resolver that already exists, never
# a second copy of it here. `down` walks it backwards so nothing is stopped while
# something that requires it is still up.
registry_lines() { "$TOOLS_DIR/capability.sh" registry --lines; }

fan_out() {  # <start|stop|status> [--all]
  local op="$1" all="${2:-}" names="" name kind scope autostart
  while read -r name kind scope autostart _; do
    [ -n "$name" ] || continue
    if [ "$op" = start ] && [ "$all" != "--all" ] && [ "$autostart" != "true" ]; then
      continue
    fi
    names="$names $name"
  done <<EOF
$(registry_lines)
EOF
  [ -n "$names" ] || { echo "nothing to do (no matching services in the registry)"; return 0; }

  if [ "$op" = stop ]; then
    local reversed=""
    for name in $names; do reversed="$name $reversed"; done
    names="$reversed"
  fi
  local rc=0
  for name in $names; do
    "$0" "$op" "$name" || rc=1
  done
  return $rc
}

case "$CMD" in
  up)
    if [ -n "$CAP" ] && [ "$CAP" != "--all" ]; then usage; fi
    fan_out start "$CAP"; exit $?
    ;;
  down)
    if [ -n "$CAP" ]; then usage; fi
    fan_out stop; exit $?
    ;;
  status)
    if [ -z "$CAP" ]; then fan_out status; exit $?; fi
    ;;
esac
[ -n "$CAP" ] || usage

# --- maintenance hold -----------------------------------------------------
# `stop` takes a capability down AND keeps it down, so a tool can work on its data
# while nothing has it open (tools/backup.sh's cold SQLite copy); `resume` lifts the
# hold and brings it back. Without this the watchdog would race the container back up
# mid-copy 30s later. The lock lives here because this file is the only one that knows
# HOW a capability runs -- no other tool needs to learn the convention.
#
# Fail-safe in the direction that matters: the lock sits in boot-cleared /tmp and
# expires, so a crashed holder can never leave a password manager permanently
# un-restartable. Ignoring a stale hold is reported on stderr, which is exactly where
# the watchdog now writes -- so it surfaces instead of rotting.
#
# Hardcoded /tmp, NOT $TMPDIR: the holder runs in a login shell (per-user
# /var/folders/... tmpdir) while the watchdog runs under launchd, and the two must agree
# on one path or the hold silently protects nothing. Same convention as the watchdog's
# own /tmp/axon-<cap>-watchdog.{log,err}.
MAINT_LOCK="/tmp/axon-$CAP.maintenance"
MAINT_MAX_AGE=1800

maintenance_hold_active() {
  [ -f "$MAINT_LOCK" ] || return 1
  local mtime now age
  mtime="$(stat -f %m "$MAINT_LOCK" 2>/dev/null || stat -c %Y "$MAINT_LOCK" 2>/dev/null || echo 0)"
  now="$(date +%s)"
  age=$(( now - mtime ))
  if [ "$age" -gt "$MAINT_MAX_AGE" ]; then
    echo "service-runner.sh: ignoring stale maintenance hold on '$CAP' (${age}s old, max ${MAINT_MAX_AGE}s): $MAINT_LOCK" >&2
    rm -f "$MAINT_LOCK"
    return 1
  fi
  return 0
}

# A capability's manifest lives with the capability, in public Axon or in the active
# overlay; the spine's own shell (dashboard/, README.md#three-architectural-nouns)
# carries one at the repo root instead, because it is not a capability and never appears
# in machine.toml's enabled set. paths.sh owns the resolution order and the
# declared-twice refusal, so this stays the only place that turns a name into a manifest.
MANIFEST=""
_mf_rc=0
MANIFEST="$(axon_manifest_for "$CAP")" || _mf_rc=$?
if [ "$_mf_rc" -eq 2 ]; then
  exit 2   # axon_manifest_for already named both paths
fi
if [ -z "$MANIFEST" ]; then
  echo "service-runner.sh: no service.toml for '$CAP' (looked in $AXON_CAPS_DIR/$CAP/, $AXON_OVERLAY_CAPS_DIR/$CAP/ and $AXON_ROOT/$CAP/)" >&2
  exit 1
fi
unset _mf_rc

# Relative paths in a manifest resolve against the root that manifest came from, so an
# overlay capability's workdir and build output stay inside the overlay. A manifest that
# genuinely needs an Axon path — an overlay capability driven by a shared tool, say —
# writes ${AXON_ROOT} and gets it expanded below.
case "$MANIFEST" in
  "$AXON_OVERLAY_CAPS_DIR"/*) CAP_ROOT="$AXON_OVERLAY_ROOT" ;;
  *)                          CAP_ROOT="$AXON_ROOT" ;;
esac

NAME="$(toml_get name "$MANIFEST")"
KIND="$(toml_get kind "$MANIFEST")"
[ -n "$KIND" ] || KIND="container"
AUTOSTART="$(toml_get autostart "$MANIFEST")"

container_init() {  # every container-only manifest field, read only when it applies
IMAGE="$(toml_get image "$MANIFEST")"
TAG="$(toml_get tag "$MANIFEST")"
ENV_FILE_REL="$(toml_get env_file "$MANIFEST")"
ENV_FILE="$AXON_PERSONAL_ROOT/$ENV_FILE_REL"
MANAGED_VOLUME="$(toml_get managed_volume "$MANIFEST")"
NETWORK_MODE="$(toml_get network_mode "$MANIFEST")"

PORTS=()
while IFS= read -r line; do [ -n "$line" ] && PORTS+=("$line"); done < <(toml_array ports "$MANIFEST")
VOLUMES=()
while IFS= read -r line; do [ -n "$line" ] && VOLUMES+=("$line"); done < <(toml_array volumes "$MANIFEST")
CAP_ADD=()
while IFS= read -r line; do [ -n "$line" ] && CAP_ADD+=("$line"); done < <(toml_array cap_add "$MANIFEST")

# `ports` is the one manifest field that describes the HOST rather than the capability:
# the shipped vaultwarden manifest publishes 0.0.0.0 for the family Pi, where
# `tailscale serve` terminates TLS in front of it and needs the container reachable on
# the host's tailnet address. The Mac has no such front, so it overrides to loopback
# (2026-07-30) — a vault on the house LAN was the bug that motivated this. service.toml is
# tracked and shared, so that difference cannot live there — same reasoning and the same
# home as every other machine-local field: section [capability.<name>] in
# <overlay>/config/machine.toml (schemas/machine.toml.example).
# Absent file, section or key leaves the manifest value standing, so a machine that
# overrides nothing behaves exactly as before.
if [ -f "$AXON_MACHINE_TOML" ]; then
  PORT_OVERRIDE=()
  while IFS= read -r line; do [ -n "$line" ] && PORT_OVERRIDE+=("$line"); done \
    < <(toml_array_in "capability.$CAP" ports "$AXON_MACHINE_TOML")
  if [ ${#PORT_OVERRIDE[@]} -gt 0 ]; then
    PORTS=("${PORT_OVERRIDE[@]}")
  fi
fi

CONTAINER_ARGS=(--name "$NAME")
# Every optional array below expands as ${A[@]+"${A[@]}"}, never as "${A[@]}": bash 3.2
# under `set -u` treats an empty array's plain expansion as an unbound variable and dies.
# The red path is the machine that does NOT declare the field, so the naive form works
# everywhere it is tested and breaks on the first capability that omits one.
#
# cap_add: Linux capabilities the image needs beyond the runtime's default set — pihole's
# NET_ADMIN, and nothing else so far. Bare names, no CAP_ prefix: docker's canonical form,
# and apple-container accepts it too (verified 2026-07-28 by watching the CapEff bit move,
# not by reading --help).
for c in ${CAP_ADD[@]+"${CAP_ADD[@]}"}; do CONTAINER_ARGS+=(--cap-add "$c"); done
if [ -n "$NETWORK_MODE" ]; then
  # host (or other) networking: join the host network namespace. `ports` are NOT
  # published — host mode binds the container's own ports directly. See
  # schemas/service.toml.example (required by home-assistant for LAN mDNS discovery).
  CONTAINER_ARGS+=(--network "$NETWORK_MODE")
else
  for p in ${PORTS[@]+"${PORTS[@]}"}; do CONTAINER_ARGS+=(-p "$p"); done
fi
# One managed volume per capability, named "axon-$CAP-data" — matches today's only
# consumer (postgres, one volume entry). Extend with a per-entry name if a second
# capability ever needs more than one managed volume.
for v in ${VOLUMES[@]+"${VOLUMES[@]}"}; do
  host_path="${v%%:*}"
  container_path="${v#*:}"
  if [ "$MANAGED_VOLUME" = "true" ] && [ "$AXON_CONTAINER_RUNTIME" = "apple-container" ]; then
    # See schemas/service.toml.example — works around virtiofs bind mounts not
    # supporting guest-side chown/chmod (confirmed on capabilities/postgres).
    vol_name="axon-$CAP-data"
    container volume inspect "$vol_name" >/dev/null 2>&1 || container volume create "$vol_name" >/dev/null
    CONTAINER_ARGS+=(-v "$vol_name:$container_path")
  elif [ "${host_path#/}" != "$host_path" ]; then
    # An absolute host path is a system path the container needs from the machine
    # itself, not data the overlay owns: /run/dbus so home-assistant can reach the
    # host's message bus for Bluetooth, /etc/localtime so its clock matches. Used
    # verbatim, and deliberately NOT created — a missing one means the host is not
    # what the manifest assumed, and mkdir would paper over that with an empty
    # directory the container then mounts and silently does without.
    [ -e "$host_path" ] || {
      echo "service-runner.sh: $CAP declares the system path $host_path, which does not exist on this host" >&2
      exit 1
    }
    CONTAINER_ARGS+=(-v "$host_path:$container_path")
  else
    resolved="$AXON_PERSONAL_ROOT/$host_path"
    mkdir -p "$resolved"
    CONTAINER_ARGS+=(-v "$resolved:$container_path")
  fi
done
CONTAINER_ARGS+=(--env-file "$ENV_FILE")
}

# --- process kind ---------------------------------------------------------
# Everything a host process needs, read the same lazy way. The pid file, log and
# error paths follow the watchdog's /tmp convention for the same reason it does: the
# holder may be a login shell (per-user /var/folders tmpdir) while the supervisor runs
# under launchd, and the two have to agree on one path or a "stop" silently stops
# nothing.
PID_FILE="/tmp/axon-$CAP.pid"
PROC_LOG="/tmp/axon-$CAP.log"
PROC_ERR="/tmp/axon-$CAP.err"

COMMAND=(); BUILD_CMD=(); WORKDIR=""; PORT=""; HEALTH_PATH=""; BUILD_OUTPUT=""

process_init() {
  while IFS= read -r line; do [ -n "$line" ] && COMMAND+=("$line"); done < <(toml_array command "$MANIFEST")
  while IFS= read -r line; do [ -n "$line" ] && BUILD_CMD+=("$line"); done < <(toml_array build "$MANIFEST")
  BUILD_OUTPUT="$(toml_get build_output "$MANIFEST")"
  WORKDIR="$(toml_get workdir "$MANIFEST")"
  PORT="$(toml_get port "$MANIFEST")"
  HEALTH_PATH="$(toml_get health_path "$MANIFEST")"

  [ ${#COMMAND[@]} -gt 0 ] || {
    echo "service-runner.sh: '$CAP' is kind=process but declares no command = [...]" >&2
    exit 1
  }

  # Bazel-less fallback for a Rust capability. Bazel is the build spine
  # (README.md#bazel-as-the-build-spine), and service.toml declares the bazel build unconditionally because that is
  # the primary, tracked-and-shared path. But a machine can be a pure dev box with cargo
  # and no bazel (WSL, a fresh laptop), and the same crate builds identically from its
  # own Cargo.toml + Cargo.lock — same source, same pins, a different build frontend, not
  # a second build system with logic of its own. So this is the argued-per-case exception
  # README.md#argue-bazel-per-case allows, not a hole in README.md#bazel-as-the-build-spine: it fires ONLY when the declared build tool is
  # bazel AND bazel is absent AND a Cargo.toml sits beside the capability. When bazel is
  # present, nothing here changes. Runs the crate's default binary (its package name);
  # a capability that renames its [[bin]] away from the package name would teach it here.
  # Anchored on the manifest's own directory, so this works identically for a capability
  # the overlay owns. --manifest-path is absolute because the build runs from workdir,
  # which a manifest may point anywhere it likes.
  local _cap_dir="${MANIFEST%/service.toml}"
  if [ "${BUILD_CMD[0]:-}" = "bazel" ] && ! command -v bazel >/dev/null 2>&1 \
     && [ -f "$_cap_dir/Cargo.toml" ]; then
    local _pkg
    _pkg="$(toml_get_in package name "$_cap_dir/Cargo.toml")"
    _pkg="${_pkg:-$CAP}"
    echo "service-runner.sh: bazel not on PATH — building '$CAP' with cargo (README.md#argue-bazel-per-case fallback)" >&2
    BUILD_CMD=(cargo build --release --manifest-path "$_cap_dir/Cargo.toml")
    COMMAND=("$_cap_dir/target/release/$_pkg")
  fi

  # ${AXON_ROOT} and ${AXON_OVERLAY_ROOT} in any argument expand first, and they are the
  # only interpolations this manifest format performs. They exist so an overlay
  # capability can name a shared Axon tool, and hand that tool a path back into the
  # overlay, without either side hardcoding one machine's checkout location.
  local _i
  for _i in "${!COMMAND[@]}"; do
    COMMAND[$_i]="${COMMAND[$_i]//\$\{AXON_ROOT\}/$AXON_ROOT}"
    COMMAND[$_i]="${COMMAND[$_i]//\$\{AXON_OVERLAY_ROOT\}/$AXON_OVERLAY_ROOT}"
  done

  # command[0] is resolved against the capability's own root, not against workdir: a
  # manifest naming bazel-bin/... means that path in the checkout it came from, wherever
  # the process then runs. A bare name (bun, uv) is left alone -- that one is PATH's job.
  case "${COMMAND[0]}" in
    /*) ;;
    */*) COMMAND[0]="$CAP_ROOT/${COMMAND[0]}" ;;
  esac

  # Same per-machine override seam the container `ports` field has: a port is a fact
  # about the host, and service.toml is tracked and shared.
  if [ -f "$AXON_MACHINE_TOML" ]; then
    local override
    override="$(toml_get_in "capability.$CAP" port "$AXON_MACHINE_TOML")"
    if [ -n "$override" ]; then PORT="$override"; fi
  fi
}

health_url() {
  [ -n "$PORT" ] && [ -n "$HEALTH_PATH" ] || return 1
  echo "http://127.0.0.1:$PORT$HEALTH_PATH"
}

process_healthy() {
  local url
  url="$(health_url)" || return 1
  curl -sf -o /dev/null --max-time 2 "$url"
}

running_pid() {  # echo the live pid, or nothing
  [ -f "$PID_FILE" ] || return 0
  local pid
  pid="$(cat "$PID_FILE" 2>/dev/null || true)"
  case "$pid" in ''|*[!0-9]*) return 0 ;; esac
  kill -0 "$pid" 2>/dev/null && echo "$pid"
  return 0
}

# Kill the tree, leaves first. `bun run dev` is a supervisor of its own -- vite is its
# child, and killing only the parent leaves the port held by an orphan. pgrep -P is on
# both macOS and Linux; recursion depth here is 2-3 in practice.
kill_tree() {
  local pid="$1" child
  for child in $(pgrep -P "$pid" 2>/dev/null || true); do kill_tree "$child"; done
  kill -TERM "$pid" 2>/dev/null || true
}

maybe_build() {  # [force] — build when the artifact is missing, or always on restart
  [ ${#BUILD_CMD[@]} -gt 0 ] || return 0
  if [ "${1:-}" != "force" ]; then
    # What "already built" means, in the order the manifest can say it:
    #
    #   build_output — an explicit path the build produces. Needed by any capability
    #     whose command[0] is a bare interpreter (bun, uv): the old rule below returned
    #     early on those, so a manifest could declare a build that silently never ran.
    #     A panel serving a `dist/` that does not exist yet is exactly that case.
    #   command[0] — the compiled binary, for the bazel capabilities. Unchanged.
    if [ -n "$BUILD_OUTPUT" ]; then
      case "$BUILD_OUTPUT" in
        /*) if [ -e "$BUILD_OUTPUT" ]; then return 0; fi ;;
        *)  if [ -e "$CAP_ROOT/$BUILD_OUTPUT" ]; then return 0; fi ;;
      esac
    else
      case "${COMMAND[0]}" in
        /*) if [ -e "${COMMAND[0]}" ]; then return 0; fi ;;
        *)  return 0 ;;   # a bare name on PATH, and nothing declared to look for
      esac
    fi
  fi
  echo "building $CAP: ${BUILD_CMD[*]}"
  # In the capability's own workdir, not the repo root: `bun run build` has to run where
  # the package.json is. A capability without a workdir (every bazel one) still builds
  # at the root, which is where those commands already expected to be.
  ( cd "$CAP_ROOT/${WORKDIR:-.}" && "${BUILD_CMD[@]}" )
}

wait_healthy() {  # returns 0 as soon as the service answers; 1 after the deadline
  health_url >/dev/null || return 0   # nothing declared to wait on
  local i=0
  while [ "$i" -lt 60 ]; do           # 60 * 0.5s = 30s, enough for a cold Next/Vite boot
    if process_healthy; then return 0; fi
    if [ -z "$(running_pid)" ]; then
      echo "service-runner.sh: '$CAP' exited during startup — tail $PROC_ERR" >&2
      return 1
    fi
    sleep 0.5
    i=$((i + 1))
  done
  echo "service-runner.sh: '$CAP' started but never answered $(health_url) — tail $PROC_ERR" >&2
  return 1
}

start_process() {
  if maintenance_hold_active; then
    echo "service-runner.sh: '$CAP' is held for maintenance, not starting ($MAINT_LOCK)"
    return 0
  fi
  if [ -n "$(running_pid)" ]; then return 0; fi   # already ours, already up
  # Something else is on the port -- a hand-started dev server, or the survivor of a
  # lost pid file. Adopting it would be a lie (this script cannot stop what it did not
  # start) and starting a second one just fails to bind. Say which it is instead.
  if process_healthy; then
    echo "service-runner.sh: '$CAP' already answers on port $PORT but is not managed here — stop it yourself first if you want this to own it" >&2
    return 0
  fi
  maybe_build
  (
    cd "$CAP_ROOT/${WORKDIR:-.}"
    # One number, one declaration. The manifest's `port` (after any machine-local
    # override) is what the dashboard proxies to and what axon-status polls, so it has
    # to be what the process binds -- otherwise the registry describes a service that
    # is listening somewhere else. A capability honours AXON_PORT above its own config;
    # one that ignores it is free to, and simply has to keep its config in step.
    if [ -n "$PORT" ]; then export AXON_PORT="$PORT"; fi
    # Where the shell lives, for a capability that serves its own page and needs a way
    # back to it. Read from dashboard/service.toml — the port keeps exactly one home,
    # and a panel never learns a number. Only the port: the HOST has to come from the
    # browser's own `location`, or the link breaks the moment the dashboard is opened
    # over Tailscale rather than as localhost (same reasoning as api.ts's panelUrl).
    AXON_SHELL_PORT="$(toml_get port "$AXON_ROOT/dashboard/service.toml")"
    export AXON_SHELL_PORT
    nohup "${COMMAND[@]}" >>"$PROC_LOG" 2>>"$PROC_ERR" &
    echo $! > "$PID_FILE"
  )
  wait_healthy
}

stop_process() {  # [hold|nohold]
  if [ "${1:-hold}" = hold ]; then : > "$MAINT_LOCK"; fi
  local pid; pid="$(running_pid)"
  if [ -n "$pid" ]; then
    kill_tree "$pid"
    local i=0
    while [ "$i" -lt 20 ] && kill -0 "$pid" 2>/dev/null; do sleep 0.25; i=$((i + 1)); done
    kill -KILL "$pid" 2>/dev/null || true
  fi
  rm -f "$PID_FILE"
}

status_process() {
  local pid state health
  pid="$(running_pid)"
  if [ -n "$pid" ]; then state="running (pid $pid)"; else state="stopped"; fi
  if health_url >/dev/null; then
    if process_healthy; then health="healthy"; else health="no answer"; fi
  else
    health="no health_path"
  fi
  if maintenance_hold_active 2>/dev/null; then state="$state, held"; fi
  printf '  %-14s %-9s %-22s %s\n' "$CAP" "process" "$state" "$health"
}

# apple-container's apiserver is its own launchd XPC service, separate from the
# containers it hosts. After a reboot it is neither running nor registered, and every
# `container` call dies with "XPC connection error: Connection invalid" -- nothing else
# brings it back, so starting it is part of starting a capability. docker/podman daemons
# own their lifecycle (Docker Desktop, systemd) and are deliberately not managed here.
ensure_runtime() {
  [ "$AXON_CONTAINER_RUNTIME" = "apple-container" ] || return 0
  # Match on the status field, not the exit code: `container system status` exits 0 in
  # both states and only differs in what it prints.
  "$RUNTIME_BIN" system status 2>/dev/null | grep -qE '^status[[:space:]]+running' && return 0
  "$RUNTIME_BIN" system start
}

# Fully-qualified image reference for whichever runtime we're on.
#
# apple-container needs an explicit registry, docker/podman default to Docker Hub. Until
# 2026-07-26 the apple-container branch just prefixed "docker.io/" unconditionally, which
# silently made the `image` field unable to name any other registry: a capability pinned to
# ghcr.io/... became docker.io/ghcr.io/... on macOS. That is why the home-assistant
# capability pointed at the Docker Hub mirror rather than the ghcr image the family node
# actually runs — a pin to a different artifact than the live one.
#
# The registry test is Docker's own: a reference has a registry when its first path segment
# contains a dot or a colon, or is exactly "localhost". Anything else is a Docker Hub short
# name and keeps the exact docker.io/<name> form this script already used — postgres and
# vaultwarden run on macOS through that form today, so the working path is left alone and
# only the broken one (an already-qualified reference getting prefixed anyway) changes.
qualified_image() {
  local ref="$IMAGE:$TAG" first="${IMAGE%%/*}"
  case "$AXON_CONTAINER_RUNTIME" in
    apple-container)
      case "$first" in
        *.*|*:*|localhost) echo "$ref" ;;          # already registry-qualified — pass through
        *)                 echo "docker.io/$ref" ;; # Docker Hub short name — unchanged behaviour
      esac
      ;;
    *) echo "$ref" ;;                              # docker/podman resolve short names themselves
  esac
}

# Three states, not two. The old existence-only check re-issued `start` against an
# already-running container on every 30s watchdog cycle, and the resulting error is why
# watchdog.sh discarded its own output. Branch on running/stopped/absent and the healthy
# path is genuinely silent, which is what makes honest logging affordable.
# Idempotent: resume_service reaches start_service, and container_init appends to
# arrays, so calling it twice would build a doubled argv.
CONTAINER_READY=0
container_prepare() {
  if [ "$CONTAINER_READY" -eq 1 ]; then return 0; fi
  resolve_runtime
  container_init
  CONTAINER_READY=1
}

start_service() {
  container_prepare
  if maintenance_hold_active; then
    echo "service-runner.sh: '$CAP' is held for maintenance, not starting ($MAINT_LOCK)"
    return 0
  fi
  ensure_runtime
  case "$AXON_CONTAINER_RUNTIME" in
    apple-container)
      if "$RUNTIME_BIN" list --format json 2>/dev/null | grep -q "\"id\":\"$NAME\""; then
        :  # already running
      elif "$RUNTIME_BIN" list -a --format json 2>/dev/null | grep -q "\"id\":\"$NAME\""; then
        "$RUNTIME_BIN" start "$NAME"
      else
        "$RUNTIME_BIN" run -d "${CONTAINER_ARGS[@]}" "$(qualified_image)"
      fi
      ;;
    docker|podman)
      if "$RUNTIME_BIN" ps --format '{{.Names}}' 2>/dev/null | grep -qx "$NAME"; then
        :  # already running
      elif "$RUNTIME_BIN" ps -a --format '{{.Names}}' 2>/dev/null | grep -qx "$NAME"; then
        "$RUNTIME_BIN" start "$NAME"
      else
        "$RUNTIME_BIN" run -d --restart unless-stopped "${CONTAINER_ARGS[@]}" "$(qualified_image)"
      fi
      ;;
  esac
}

# Take it down and KEEP it down. Idempotent: an already-stopped capability is fine,
# the hold is what the caller is really after.
stop_service() {  # [hold|nohold]
  container_prepare
  if [ "${1:-hold}" = hold ]; then : > "$MAINT_LOCK"; fi
  ensure_runtime
  "$RUNTIME_BIN" stop "$NAME" >/dev/null 2>&1 || true
}

resume_service() {
  rm -f "$MAINT_LOCK"
  start_service
}

status_service() {
  container_prepare
  local state="stopped"
  if "$RUNTIME_BIN" ps --format '{{.Names}}' 2>/dev/null | grep -qx "$NAME" \
     || "$RUNTIME_BIN" list --format json 2>/dev/null | grep -q "\"id\":\"$NAME\""; then
    state="running"
  fi
  if maintenance_hold_active 2>/dev/null; then state="$state, held"; fi
  printf '  %-14s %-9s %-22s %s\n' "$CAP" "container" "$state" "$IMAGE:$TAG"
}

install_persistence() {
  # A watchdog and an on-demand capability are opposite claims about the same process.
  # The watchdog calls `start` every 30s and knows nothing about `autostart`, so
  # installing one on a capability the manifest declares on-demand keeps it up forever,
  # while the Projects page tells the operator the opposite. The manifest is the
  # authority; persistence is only meaningful for what is supposed to be running.
  if [ "$AUTOSTART" != "true" ]; then
    echo "service-runner.sh: '$CAP' declares no autostart — it is on-demand, and a watchdog would defeat that. Not installing persistence." >&2
    echo "  (set autostart = \"true\" in $MANIFEST if this capability is meant to always run)" >&2
    return 1
  fi
  if [ "$KIND" = container ]; then
    case "$AXON_CONTAINER_RUNTIME" in
      docker|podman)
        echo "service-runner.sh: $AXON_CONTAINER_RUNTIME already runs with --restart unless-stopped — no watchdog needed, skipping"
        return 0
        ;;
    esac
  fi
  # Both launchd and systemd --user hand a supervised process a MINIMAL environment that
  # does NOT inherit the login shell's PATH. Every container runtime installs outside the
  # bare set (/usr/local/bin, /opt/homebrew/bin on macOS; the runtime can sit outside
  # /usr/bin on Linux too), so without an injected PATH the watchdog loops on exit 127
  # forever — which is exactly what launchd did from 2026-07-09 until 2026-07-25. Resolved
  # from the real binary rather than a hardcoded candidate list: this machine's real answer
  # lands in this machine's own unit, and Axon stays free of per-machine paths. Moving the
  # runtime binary means re-running install-persistence.
  #
  # Same defect, same fix, for a process capability: what it needs on PATH is its own
  # interpreter (bun, uv) and its builder (bazel), resolved here rather than hardcoded.
  # Shared by both OS branches below (README.md#documentation-stays-owned-and-current — resolve once, render per-OS).
  if [ "$KIND" = container ]; then
    resolve_runtime
    runtime_dir="$(dirname "$RUNTIME_PATH")"
  else
    runtime_dir=""
    case "${COMMAND[0]}" in
      /*) cmd_bin="${COMMAND[0]}" ;;
      *)  cmd_bin="$(command -v "${COMMAND[0]}" 2>/dev/null || true)" ;;
    esac
    if [ -n "$cmd_bin" ]; then runtime_dir="$(dirname "$cmd_bin")"; fi
    if [ ${#BUILD_CMD[@]} -gt 0 ]; then
      build_bin="$(command -v "${BUILD_CMD[0]}" 2>/dev/null || true)"
      if [ -n "$build_bin" ]; then runtime_dir="$runtime_dir:$(dirname "$build_bin")"; fi
    fi
  fi
  WATCHDOG_PATH="$TOOLS_DIR/watchdog.sh"
  LOG_OUT="/tmp/axon-$CAP-watchdog.log"
  LOG_ERR="/tmp/axon-$CAP-watchdog.err"

  case "$AXON_OS" in
    macos)
      plist_dst="$HOME/Library/LaunchAgents/com.axon.$CAP.plist"
      sed -e "s|__LABEL__|com.axon.$CAP|" \
          -e "s|__WATCHDOG_PATH__|$WATCHDOG_PATH|" \
          -e "s|__PATH__|$runtime_dir:/usr/bin:/bin:/usr/sbin:/sbin|" \
          -e "s|__CAPABILITY__|$CAP|" \
          -e "s|__LOG_OUT__|$LOG_OUT|" \
          -e "s|__LOG_ERR__|$LOG_ERR|" \
          "$TOOLS_DIR/templates/launchd-watchdog.plist.tmpl" > "$plist_dst"
      launchctl unload "$plist_dst" 2>/dev/null || true
      launchctl load "$plist_dst"
      echo "installed $plist_dst"
      ;;
    linux)
      # systemd --user is the per-user analogue of a LaunchAgent. It needs a running
      # systemd (PID 1, or WSL2 with `systemd=true` in /etc/wsl.conf) and the user bus;
      # fail with the fix rather than a cryptic systemctl error where it's absent.
      if ! command -v systemctl >/dev/null 2>&1 || [ ! -d /run/systemd/system ]; then
        echo "service-runner.sh: systemd not available (no systemctl, or systemd isn't PID 1)." >&2
        echo "  On WSL, add 'systemd=true' under [boot] in /etc/wsl.conf, then 'wsl --shutdown' and reopen." >&2
        echo "  Until then run '$WATCHDOG_PATH $CAP' manually (e.g. inside tmux/screen)." >&2
        exit 1
      fi
      unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
      mkdir -p "$unit_dir"
      unit_dst="$unit_dir/axon-$CAP.service"
      sed -e "s|__WATCHDOG_PATH__|$WATCHDOG_PATH|" \
          -e "s|__PATH__|$runtime_dir:/usr/local/bin:/usr/bin:/bin|" \
          -e "s|__CAPABILITY__|$CAP|" \
          -e "s|__LOG_OUT__|$LOG_OUT|" \
          -e "s|__LOG_ERR__|$LOG_ERR|" \
          "$TOOLS_DIR/templates/systemd-watchdog.service.tmpl" > "$unit_dst"
      systemctl --user daemon-reload
      systemctl --user enable --now "axon-$CAP.service"
      echo "installed $unit_dst (systemctl --user)"
      # A --user unit only runs while the user has a session unless lingering is enabled.
      # For a capability meant to survive logout / start at boot, enable it once.
      if ! loginctl show-user "$(id -un)" 2>/dev/null | grep -q '^Linger=yes'; then
        echo "  note: run 'loginctl enable-linger $(id -un)' so this survives logout / reboot." >&2
      fi
      ;;
    windows)
      echo "service-runner.sh: no windows persistence backend yet." >&2
      echo "For now: run '$WATCHDOG_PATH $CAP' manually, or add a Windows Task Scheduler entry here." >&2
      exit 1
      ;;
    *)
      echo "service-runner.sh: unknown os '$AXON_OS' (axon-overlay/config/machine.toml)" >&2
      exit 1
      ;;
  esac
}

if [ "$KIND" = process ]; then process_init; fi

case "$CMD" in
  start)
    if [ "$KIND" = process ]; then start_process; else start_service; fi
    ;;
  stop)
    if [ "$KIND" = process ]; then stop_process hold; else stop_service hold; fi
    ;;
  restart)
    # nohold on the way down: a restart is not a maintenance window, and leaving the
    # hold set would make the start half a no-op.
    if [ "$KIND" = process ]; then
      stop_process nohold; maybe_build force; start_process
    else
      stop_service nohold; start_service
    fi
    ;;
  idle-stop)
    # Stop WITHOUT holding it down. `stop` exists so a tool can work on a capability's
    # data while nothing has it open, and it sets a hold to keep the watchdog from
    # racing that window. An unread page is not a maintenance window: holding it would
    # mean the next thing to ask for the panel gets a silent no-op until the lock aged
    # out. Used by axon-status's idle reaper; a capability with no idle_timeout in its
    # manifest is never a target.
    if [ "$KIND" = process ]; then stop_process nohold; else stop_service nohold; fi
    ;;
  resume)
    if [ "$KIND" = process ]; then rm -f "$MAINT_LOCK"; start_process; else resume_service; fi
    ;;
  status)
    if [ "$KIND" = process ]; then status_process; else status_service; fi
    ;;
  install-persistence) install_persistence ;;
  *) usage ;;
esac
