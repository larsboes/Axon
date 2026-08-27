#!/bin/bash
# Tests for tools/service-runner.sh's stop/start contract:
#   * `stop` cannot report success while the capability's declared port is still held
#   * `down` followed by `up` brings services back, while an explicit `stop` still keeps one down
#   * machine.toml's `[inference] backend` reaches the started process, and only when declared
#
# The case is real and was hit by hand on 2026-07-30: a panel capability's dev server survived
# `stop`. `bun run dev` supervises its own child; when the supervisor exits first the child is
# reparented and keeps the port, so the recorded pid is gone, running_pid() returns nothing,
# kill_tree never runs, and stop removed the pid file and exited 0 over a live service.
#
# Built on the same throwaway-root idea as manifest-resolution.test.sh: a scratch Axon root with
# its own overlay and capability manifest, so nothing here touches the real machine's state.
# The orphan is a real listener on a real port, because the point of the check is that the port
# is observably held — asserting it against a mock would test the mock.
set -uo pipefail

fails=0
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }

# bun is what holds a real port, so the stop/start half of this file needs it and the
# apple-container half does not. Skipped rather than fatal, matching protection-zones.test.sh:
# on a host with no bun, refusing to run at all would mean the runtime-gate regression below
# never executes (#127). The skip is printed, never silent.
HAVE_BUN=1
command -v bun >/dev/null 2>&1 || HAVE_BUN=0

SCRATCH="$(mktemp -d "/tmp/service-runner.XXXXXX")"
ROOT="$SCRATCH/axon"
OVERLAY="$SCRATCH/overlay"
LISTENER_PID=""
cleanup() {
  if [ -n "$LISTENER_PID" ]; then kill -TERM "$LISTENER_PID" 2>/dev/null; wait "$LISTENER_PID" 2>/dev/null; fi
  rm -rf "$SCRATCH"
  rm -f /tmp/axon-porthog.pid /tmp/axon-porthog.maintenance
  rm -f /tmp/axon-inferhog.pid /tmp/axon-inferhog.maintenance
  rm -f /tmp/axon-inferhog.log /tmp/axon-inferhog.err
}
trap cleanup EXIT

# Locate the real script and libs: this file sits in tools/, so they are siblings.
_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
SRC_TOOLS=""
for _c in "$_dir" "$_dir/tools"; do
  if [ -f "$_c/service-runner.sh" ]; then SRC_TOOLS="$_c"; break; fi
done
[ -n "$SRC_TOOLS" ] || { echo "service-runner: cannot find service-runner.sh next to $_dir" >&2; exit 1; }

# Every case below writes a machine.toml into its own scratch overlay and asserts on what the
# runner reads back. An operator's exported AXON_OVERLAY_ROOT / AXON_MACHINE_TOML wins over the
# scratch axon.toml, so without this the runner reads the REAL machine and the fixture is inert
# (tools/lib/test-support.sh#isolate_axon_env).
source "$SRC_TOOLS/lib/test-support.sh"
isolate_axon_env

mkdir -p "$ROOT/tools/lib" "$OVERLAY/config"
cp "$SRC_TOOLS/service-runner.sh" "$SRC_TOOLS/capability.sh" "$ROOT/tools/"
# The whole lib directory, not a named subset: service-runner sources paths.sh, toml.sh and
# platform.sh today, and a list here would silently rot into a "No such file" the next time it
# picks up a fourth.
cp "$SRC_TOOLS"/lib/*.sh "$ROOT/tools/lib/"
printf 'overlay = "%s"\n' "$OVERLAY" > "$ROOT/axon.toml"
printf 'os = "linux"\ncontainer_runtime = "docker"\ncapabilities = ["porthog"]\n' > "$OVERLAY/config/machine.toml"

if [ "$HAVE_BUN" -eq 0 ]; then
  echo "  ⊘ bun not on PATH — the stop/start port checks are skipped"
else

# A free port, taken from the kernel rather than guessed: a hardcoded number turns this test
# into a flake on whatever machine already uses it.
PORT="$(bun -e 'const s=Bun.listen({hostname:"127.0.0.1",port:0,socket:{data(){}}});console.log(s.port);s.stop(true)')"
case "$PORT" in ''|*[!0-9]*) echo "service-runner: could not obtain a free port" >&2; exit 1 ;; esac

mkdir -p "$ROOT/capabilities/porthog"
cat > "$ROOT/capabilities/porthog/service.toml" <<TOML
kind = "process"
name = "porthog"
port = "$PORT"
autostart = true
command = ["true"]
TOML

# The orphan: a listener this capability's pid file does not know about, which is exactly the
# state a reparented dev-server child leaves behind.
bun -e "Bun.serve({port:$PORT,fetch:()=>new Response('held')})" >/dev/null 2>&1 &
LISTENER_PID=$!

ready=0
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
  if (: < "/dev/tcp/127.0.0.1/$PORT") 2>/dev/null; then ready=1; break; fi
  sleep 0.25
done
[ "$ready" -eq 1 ] || { echo "service-runner: the orphan listener never came up" >&2; exit 1; }

# A pid file pointing at a pid that is already gone — the exact shape running_pid() returns
# nothing for. 2^22 is above every default pid_max, so it cannot collide with a live process.
echo 4194304 > /tmp/axon-porthog.pid

out="$("$ROOT/tools/service-runner.sh" stop porthog 2>&1)"; rc=$?

# The falsifier. Before the fix this exited 0 with the port still held.
[ "$rc" -ne 0 ] || fail "stop exited 0 while port $PORT was still held"
case "$out" in
  *"still answers on port $PORT"*) ;;
  *) fail "stop did not name the held port; said: $out" ;;
esac
if ! (: < "/dev/tcp/127.0.0.1/$PORT") 2>/dev/null; then
  fail "the orphan died on its own — this run proved nothing"
fi

# The control: with the port free, the same call must succeed. Without this the assertion above
# would also pass on a stop that always fails.
kill -TERM "$LISTENER_PID" 2>/dev/null; wait "$LISTENER_PID" 2>/dev/null; LISTENER_PID=""
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
  (: < "/dev/tcp/127.0.0.1/$PORT") 2>/dev/null || break
  sleep 0.25
done
echo 4194304 > /tmp/axon-porthog.pid
out2="$("$ROOT/tools/service-runner.sh" stop porthog 2>&1)"; rc2=$?
[ "$rc2" -eq 0 ] || fail "stop failed on a free port; rc=$rc2, said: $out2"

# --- down then up must not be a dead end ----------------------------------
# `stop <cap>` takes a maintenance hold on purpose: it exists so a tool can work on a capability's
# data while nothing has it open, and tools/backup.sh depends on it. `down` used to fan out that
# same holding stop, so every service answered the following `up` with "is held for maintenance,
# not starting" and the advertised pair no-opped on its second half.
SR="$ROOT/tools/service-runner.sh"
rm -f /tmp/axon-porthog.pid /tmp/axon-porthog.maintenance

"$SR" stop porthog >/dev/null 2>&1
[ -f /tmp/axon-porthog.maintenance ] \
  || fail "stop did not take the maintenance hold — the backup path depends on it"
out_held="$("$SR" start porthog 2>&1)"
case "$out_held" in
  *"held for maintenance"*) ;;
  *) fail "start ignored an explicit maintenance hold; said: $out_held" ;;
esac

# The hold survives a --no-hold stop: only `resume` lifts a window someone deliberately opened.
"$SR" stop porthog --no-hold >/dev/null 2>&1
[ -f /tmp/axon-porthog.maintenance ] \
  || fail "--no-hold cleared a hold it did not take"

rm -f /tmp/axon-porthog.maintenance
"$SR" stop porthog --no-hold >/dev/null 2>&1
[ ! -f /tmp/axon-porthog.maintenance ] \
  || fail "stop --no-hold took a hold anyway — this is what down fans out"
out_free="$("$SR" start porthog 2>&1)"
case "$out_free" in
  *"held for maintenance"*) fail "start refused although no hold was set; said: $out_free" ;;
esac

# The real fan-out, not just the per-capability mechanism: `down` must leave nothing held, and
# the `up` that follows must not answer "held for maintenance".
rm -f /tmp/axon-porthog.pid /tmp/axon-porthog.maintenance
"$SR" down >/dev/null 2>&1
[ ! -f /tmp/axon-porthog.maintenance ] \
  || fail "down took a maintenance hold — the following up is a no-op"
out_up="$("$SR" up 2>&1)"
case "$out_up" in
  *"held for maintenance"*) fail "up refused after down — the pair is still a dead end: $out_up" ;;
esac

# An unknown flag is rejected rather than silently ignored: `stop cap --nohold` must not read as
# a hold-taking stop because of a missing dash.
if "$SR" stop porthog --nohold >/dev/null 2>&1; then
  fail "an unknown flag was accepted"
fi

fi  # HAVE_BUN

# --- the machine's local model runtime reaches the process it starts --------
# `[inference] backend` in machine.toml was documented in three places as the way a host
# without oMLX names the runtime it does have, and libs/inference has read
# AXON_INFERENCE_BACKEND since it existed — but nothing ever exported it. The documented
# mechanism was fiction, so a host whose only runtime is Ollama had no way to say so, and
# every role stayed pointed at a port with nothing behind it.
#
# Its own scratch root: the machine.toml above describes the porthog host, and this case is
# about what a DIFFERENT machine declares.
#
# Ambient value cleared first — an operator who exports one for debugging would otherwise
# make the control below pass for the wrong reason.
unset AXON_INFERENCE_BACKEND

INF_ROOT="$SCRATCH/inference"
INF_OVERLAY="$SCRATCH/inference-overlay"
ENV_DUMP="$SCRATCH/inference-backend-seen"
INF_SR="$INF_ROOT/tools/service-runner.sh"

mkdir -p "$INF_ROOT/tools/lib" "$INF_OVERLAY/config" "$INF_ROOT/capabilities/inferhog"
cp "$SRC_TOOLS/service-runner.sh" "$SRC_TOOLS/capability.sh" "$INF_ROOT/tools/"
cp "$SRC_TOOLS"/lib/*.sh "$INF_ROOT/tools/lib/"
printf 'overlay = "%s"\n' "$INF_OVERLAY" > "$INF_ROOT/axon.toml"

# No port and no health_path, so `start` execs the command and returns without waiting on
# anything — which is why the assertion polls for the file rather than reading it once.
cat > "$INF_ROOT/capabilities/inferhog/service.toml" <<'TOML'
kind = "process"
name = "inferhog"
command = ["capabilities/inferhog/dump-backend"]
TOML
cat > "$INF_ROOT/capabilities/inferhog/dump-backend" <<SH
#!/bin/bash
printf '%s' "\${AXON_INFERENCE_BACKEND:-<unset>}" > "$ENV_DUMP"
SH
chmod +x "$INF_ROOT/capabilities/inferhog/dump-backend"

inference_backend_seen() {  # start inferhog and echo what it was handed
  rm -f "$ENV_DUMP" /tmp/axon-inferhog.pid
  "$INF_SR" start inferhog >/dev/null 2>&1
  local waited=0
  while [ "$waited" -lt 40 ] && [ ! -s "$ENV_DUMP" ]; do sleep 0.25; waited=$((waited + 1)); done
  cat "$ENV_DUMP" 2>/dev/null || true
}

cat > "$INF_OVERLAY/config/machine.toml" <<'TOML'
os = "linux"
container_runtime = "docker"
capabilities = ["inferhog"]

[inference]
backend = "ollama"
TOML
seen="$(inference_backend_seen)"
[ "$seen" = "ollama" ] \
  || fail "[inference] backend never reached the started process (saw '$seen')"

# The control. A machine that declares no runtime must not have one invented for it: an
# exported value here would move every loopback role onto a backend nobody chose.
printf 'os = "linux"\ncontainer_runtime = "docker"\ncapabilities = ["inferhog"]\n' \
  > "$INF_OVERLAY/config/machine.toml"
seen="$(inference_backend_seen)"
[ "$seen" = "<unset>" ] \
  || fail "a machine declaring no [inference] backend still exported one (saw '$seen')"

# --- the runtime gate must precede every runtime call (#125) ----------------
# apple-container's apiserver is a separate launchd XPC service. When it is down, every
# `container` call fails with "XPC connection error", and ensure_runtime exists to revive it.
# It used to run inside start_service, after container_prepare had already reached
# container_init -- which issues `container volume create` for a managed_volume capability.
# Under `set -e` that call aborted the run before the recovery it was supposed to trigger, so a
# Mac whose apiserver went down never came back on its own. Observed 2026-08-07: postgres stayed
# down for 90 watchdog cycles, and punctuality crash-looped against it the whole time.
#
# Asserted as an ordering over the runtime's own call log rather than as an exit code, because
# `start` can exit 0 for reasons that have nothing to do with the fix.
AC_ROOT="$SCRATCH/ac"
AC_OVERLAY="$SCRATCH/ac-overlay"
AC_BIN="$SCRATCH/ac-bin"
CALL_LOG="$SCRATCH/container-calls.log"
RUNTIME_STATE="$SCRATCH/apiserver-state"

mkdir -p "$AC_ROOT/tools/lib" "$AC_OVERLAY/config" "$AC_BIN" "$AC_ROOT/capabilities/volhog"
cp "$SRC_TOOLS/service-runner.sh" "$SRC_TOOLS/capability.sh" "$AC_ROOT/tools/"
cp "$SRC_TOOLS"/lib/*.sh "$AC_ROOT/tools/lib/"
printf 'overlay = "%s"\n' "$AC_OVERLAY" > "$AC_ROOT/axon.toml"
printf 'os = "darwin"\ncontainer_runtime = "apple-container"\ncapabilities = ["volhog"]\n' \
  > "$AC_OVERLAY/config/machine.toml"

# managed_volume is what makes container_init call the runtime at all. postgres was the only
# capability that declared it, and the only one that got stuck; it was retired on 2026-08-27,
# so this fixture is the whole of that coverage now — the image name below is the one the
# behaviour was observed against, not a manifest that still exists.
cat > "$AC_ROOT/capabilities/volhog/service.toml" <<'TOML'
name = "volhog"
image = "postgres"
tag = "16-alpine"
volumes = ["data/volhog/data:/var/lib/postgresql/data"]
managed_volume = "true"
TOML

# The stand-in apiserver. It starts down, records every call, and refuses everything except
# `system start`/`system status` until it is revived — which is exactly what the real one does.
echo stopped > "$RUNTIME_STATE"
cat > "$AC_BIN/container" <<SHIM
#!/bin/bash
echo "\$*" >> "$CALL_LOG"
case "\$1 \${2:-}" in
  "system status")
    if [ "\$(cat "$RUNTIME_STATE")" = running ]; then echo "status             running"
    else echo "apiserver is not running and not registered with launchd"; fi
    exit 0 ;;                       # exits 0 in both states, like the real CLI
  "system start")
    echo running > "$RUNTIME_STATE"; exit 0 ;;
esac
if [ "\$(cat "$RUNTIME_STATE")" != running ]; then
  echo 'Error: interrupted: "XPC connection error: Connection invalid"' >&2
  echo "Ensure container system service has been started with \\\`container system start\\\`." >&2
  exit 1
fi
case "\$1 \${2:-}" in
  "volume inspect") exit 1 ;;       # absent, so the create path runs
  "volume create")  exit 0 ;;
  "list "*|list)    echo '[]'; exit 0 ;;
  "run "*|run)      exit 0 ;;
esac
exit 0
SHIM
chmod +x "$AC_BIN/container"

PATH="$AC_BIN:$PATH" "$AC_ROOT/tools/service-runner.sh" start volhog >/dev/null 2>&1
ac_rc=$?

[ -f "$CALL_LOG" ] || fail "the runtime was never called at all — this run proved nothing"
first_volume="$(grep -n '^volume ' "$CALL_LOG" | head -1 | cut -d: -f1)"
first_start="$(grep -n '^system start' "$CALL_LOG" | head -1 | cut -d: -f1)"

# The falsifier. Before the fix, `volume create` ran first and `set -e` killed the run there,
# so `system start` never appeared in this log at all.
if [ -z "$first_start" ]; then
  fail "the apiserver was never started; calls were: $(tr '\n' '|' < "$CALL_LOG")"
elif [ -n "$first_volume" ] && [ "$first_volume" -lt "$first_start" ]; then
  fail "a volume call preceded the runtime gate (volume at line $first_volume, start at $first_start)"
fi
[ "$ac_rc" -eq 0 ] || fail "start failed against a recoverable apiserver; rc=$ac_rc"

# The control: with the apiserver already up, the gate must not restart it.
: > "$CALL_LOG"
echo running > "$RUNTIME_STATE"
PATH="$AC_BIN:$PATH" "$AC_ROOT/tools/service-runner.sh" start volhog >/dev/null 2>&1
if grep -q '^system start' "$CALL_LOG"; then
  fail "the gate restarted an apiserver that was already running"
fi

# --- kind = "data": a file, not a process (PRD Q45) -----------------------------------------
# capabilities/store declares the shared SQLite database so ONE manifest owns its backup
# contract. Nothing starts it, and the two ways that could go wrong are opposite: a lifecycle
# verb falling through to the container path and reporting a missing image as a broken
# capability, or the whole-machine fan-out calling `stop` on it and turning `down`'s exit
# status into noise. Both are asserted here, on a root where the data unit is the only thing
# enabled — so a fan-out that did not skip it could not hide behind another capability.
DATA_ROOT="$SCRATCH/data-axon"
DATA_OVERLAY="$SCRATCH/data-overlay"
mkdir -p "$DATA_ROOT/tools/lib" "$DATA_OVERLAY/config" "$DATA_ROOT/capabilities/filehog"
cp "$SRC_TOOLS/service-runner.sh" "$SRC_TOOLS/capability.sh" "$DATA_ROOT/tools/"
cp "$SRC_TOOLS"/lib/*.sh "$DATA_ROOT/tools/lib/"
printf 'overlay = "%s"\n' "$DATA_OVERLAY" > "$DATA_ROOT/axon.toml"
printf 'os = "linux"\ncontainer_runtime = "docker"\ncapabilities = ["filehog"]\n' > "$DATA_OVERLAY/config/machine.toml"
cat > "$DATA_ROOT/capabilities/filehog/service.toml" <<'TOML'
kind = "data"
name = "filehog"
backup_sqlite_online = "data/filehog/filehog.db"
backup_target = "backup-target"
TOML

out="$("$DATA_ROOT/tools/service-runner.sh" start filehog 2>&1)"; rc=$?
[ "$rc" -ne 0 ] || fail "start on a kind=data manifest exited 0"
case "$out" in
  *"kind=data"*) ;;
  *) fail "the refusal did not name the kind; said: $out" ;;
esac

# The reporting verb answers instead of refusing: doctor reads a row per enabled capability,
# and a data unit legitimately owes no supervisor unit.
out="$("$DATA_ROOT/tools/service-runner.sh" persistence-status filehog 2>&1)"; rc=$?
[ "$rc" -eq 0 ] || fail "persistence-status on a kind=data manifest failed; said: $out"
case "$out" in
  "filehog	n/a	"*) ;;
  *) fail "persistence-status did not report n/a; said: $out" ;;
esac

# The fan-out. With the data unit the only enabled capability, a runner that walked it would
# print its refusal and exit non-zero.
out="$("$DATA_ROOT/tools/service-runner.sh" down 2>&1)"; rc=$?
[ "$rc" -eq 0 ] || fail "down exited $rc with only a kind=data unit enabled; said: $out"
case "$out" in
  *"nothing to do"*) ;;
  *) fail "down did not skip the data unit; said: $out" ;;
esac

if [ "$fails" -gt 0 ]; then
  echo "service-runner: $fails check(s) failed"
  exit 1
fi
echo "service-runner: all checks passed"
