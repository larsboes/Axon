#!/bin/bash
# Test for tools/service-runner.sh's stop path — specifically that `stop` cannot report success
# while the capability's declared port is still held.
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

command -v bun >/dev/null 2>&1 || { echo "service-runner: bun not on PATH — cannot run"; exit 1; }

SCRATCH="$(mktemp -d "${TEST_TMPDIR:-/tmp}/service-runner.XXXXXX")"
ROOT="$SCRATCH/axon"
OVERLAY="$SCRATCH/overlay"
LISTENER_PID=""
cleanup() {
  if [ -n "$LISTENER_PID" ]; then kill -TERM "$LISTENER_PID" 2>/dev/null; wait "$LISTENER_PID" 2>/dev/null; fi
  rm -rf "$SCRATCH"
  rm -f /tmp/axon-porthog.pid /tmp/axon-porthog.maintenance
}
trap cleanup EXIT

# Locate the real script and libs. Two layouts: invoked directly this file sits in tools/, so
# they are siblings; under Bazel it sits in the runfiles root and they keep tools/lib.
_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
SRC_TOOLS=""
for _c in "$_dir" "$_dir/tools"; do
  if [ -f "$_c/service-runner.sh" ]; then SRC_TOOLS="$_c"; break; fi
done
[ -n "$SRC_TOOLS" ] || { echo "service-runner: cannot find service-runner.sh next to $_dir" >&2; exit 1; }

mkdir -p "$ROOT/tools/lib" "$OVERLAY/config"
cp "$SRC_TOOLS/service-runner.sh" "$ROOT/tools/"
# The whole lib directory, not a named subset: service-runner sources paths.sh, toml.sh and
# platform.sh today, and a list here would silently rot into a "No such file" the next time it
# picks up a fourth.
cp "$SRC_TOOLS"/lib/*.sh "$ROOT/tools/lib/"
printf 'overlay = "%s"\n' "$OVERLAY" > "$ROOT/axon.toml"
printf 'os = "linux"\ncontainer_runtime = "docker"\ncapabilities = ["porthog"]\n' > "$OVERLAY/config/machine.toml"

# A free port, taken from the kernel rather than guessed: a hardcoded number turns this test
# into a flake on whatever machine already uses it.
PORT="$(bun -e 'const s=Bun.listen({hostname:"127.0.0.1",port:0,socket:{data(){}}});console.log(s.port);s.stop(true)')"
case "$PORT" in ''|*[!0-9]*) echo "service-runner: could not obtain a free port" >&2; exit 1 ;; esac

mkdir -p "$ROOT/capabilities/porthog"
cat > "$ROOT/capabilities/porthog/service.toml" <<TOML
kind = "process"
name = "porthog"
port = "$PORT"
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

if [ "$fails" -gt 0 ]; then
  echo "service-runner: $fails check(s) failed"
  exit 1
fi
echo "service-runner: all checks passed"
