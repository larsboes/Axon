#!/bin/bash
# Tests for capabilities/agentbox/agentbox — the runtime gate, and the two network modes it
# ships since Q75 (2026-09-02).
#
# The capability had no test file at all until then. It was defended by a table in its README,
# which is a record of one afternoon and not a check: the isolation claim moved from
# apple-container's host-only network to docker's `--network none`, and nothing would have
# reported it if the new mode quietly reached the internet.
#
# Two halves. The first runs anywhere: a scratch overlay declaring a runtime agentbox does not
# accept must be refused BY NAME, before any container call. The second needs a live docker
# daemon and is skipped loudly without one, matching service-runner.test.sh's bun preamble.
set -uo pipefail

fails=0
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }

_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
SRC_ROOT=""
for _c in "$_dir/.." "$_dir"; do
  if [ -f "$_c/capabilities/agentbox/agentbox" ]; then SRC_ROOT="$(cd "$_c" && pwd)"; break; fi
done
[ -n "$SRC_ROOT" ] || { echo "agentbox: cannot find capabilities/agentbox/agentbox from $_dir" >&2; exit 1; }

SCRATCH="$(mktemp -d "/tmp/agentbox.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT
ROOT="$SCRATCH/axon"
OVERLAY="$SCRATCH/overlay"

# An operator's exported AXON_OVERLAY_ROOT / AXON_MACHINE_TOML wins over the scratch axon.toml,
# so without this the launcher reads the REAL machine and the fixture is inert
# (tools/lib/test-support.sh#isolate_axon_env).
source "$SRC_ROOT/tools/lib/test-support.sh"
isolate_axon_env

mkdir -p "$ROOT/tools/lib" "$ROOT/capabilities/agentbox" "$OVERLAY/config"
cp "$SRC_ROOT"/tools/lib/*.sh "$ROOT/tools/lib/"
cp "$SRC_ROOT/capabilities/agentbox/agentbox" "$ROOT/capabilities/agentbox/"
cp -R "$SRC_ROOT/capabilities/agentbox/profiles" "$ROOT/capabilities/agentbox/"
printf 'overlay = "%s"\n' "$OVERLAY" > "$ROOT/axon.toml"
printf 'agent = "pi"\nmodel_port = "8000"\n' > "$OVERLAY/config/agentbox.toml"

BOX="$ROOT/capabilities/agentbox/agentbox"

# --- 1. the runtime gate ---------------------------------------------------
# podman is the interesting rejection: its verbs match docker's closely enough that the box
# would half-work, and it spells the host `host.containers.internal`, so model_base_url would
# render an endpoint that never resolves. Refused by name beats accepted untested.
for runtime in podman apple-container ""; do
  printf 'os = "macos"\ncontainer_runtime = "%s"\ncapabilities = []\n' "$runtime" \
    > "$OVERLAY/config/machine.toml"
  out="$("$BOX" doctor 2>&1)"; rc=$?
  [ "$rc" -eq 0 ] && fail "container_runtime = '$runtime' was accepted (rc=0)"
  case "$out" in
    *"agentbox needs docker"*) ;;
    *"missing 'os' or 'container_runtime'"*) ;;   # the empty case dies one layer earlier, in platform.sh
    *) fail "container_runtime = '$runtime' was refused without naming what agentbox needs: $out" ;;
  esac
  # A doctor that cannot use the declared runtime must not then answer runtime questions from
  # whatever daemon happens to be installed. Every such line reads "not checked".
  case "$runtime" in
    podman|apple-container)
      case "$out" in
        *"image: not checked"*) ;;
        *) fail "container_runtime = '$runtime' still had doctor query a runtime it does not declare: $out" ;;
      esac ;;
  esac
done

# --- 2. the two network modes ----------------------------------------------
HAVE_DOCKER=1
command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1 || HAVE_DOCKER=0

if [ "$HAVE_DOCKER" -eq 0 ]; then
  echo "  ⊘ no answering docker daemon — the network-mode checks are skipped"
else
  # A stand-in for the agentbox image: this asserts what the FLAGS do, and building the real
  # image needs a 100 MB pinned tarball off the network. The base image is READ from the
  # Containerfile rather than named here — upstreams.toml [debian] pins a dated tag and says
  # a rebuild is a decision, so writing `debian:trixie-slim` in a test would be a second,
  # rolling reference to the same dependency. It also has to be glibc: resolver behaviour on
  # `--network none` differs (glibc fails in ~2 ms, musl burns a 10 s timeout).
  PROBE_IMAGE="$(sed -n 's/^ARG BASE_IMAGE=//p' "$SRC_ROOT/capabilities/agentbox/Containerfile" | head -n1)"
  [ -n "$PROBE_IMAGE" ] || fail "could not read ARG BASE_IMAGE from the agentbox Containerfile"
  if ! docker image inspect "$PROBE_IMAGE" >/dev/null 2>&1; then
    docker pull -q "$PROBE_IMAGE" >/dev/null 2>&1 \
      || { echo "  ⊘ cannot obtain $PROBE_IMAGE — the network-mode checks are skipped"; PROBE_IMAGE=""; }
  fi

  # The flags come out of the launcher, not out of this file. Both modes are single
  # assignments there for exactly this reason, so an edit to either one is an edit to what
  # these checks prove; a copy here would go on passing after the box changed modes.
  # Extraction failing is a failure, never a silent skip.
  MODEL_HOST="$(sed -n 's/^MODEL_HOST="\(.*\)"$/\1/p' "$BOX" | head -n1)"
  OFFLINE_NET_ARGS="$(sed -n 's/^OFFLINE_NET_ARGS="\(.*\)"$/\1/p' "$BOX" | head -n1)"
  ONLINE_NET_ARGS="$(sed -n 's/^ONLINE_NET_ARGS="\(.*\)"$/\1/p' "$BOX" | head -n1)"
  ONLINE_NET_ARGS="${ONLINE_NET_ARGS//\$MODEL_HOST/$MODEL_HOST}"
  if [ -z "$MODEL_HOST" ] || [ -z "$OFFLINE_NET_ARGS" ] || [ -z "$ONLINE_NET_ARGS" ]; then
    fail "could not read MODEL_HOST / OFFLINE_NET_ARGS / ONLINE_NET_ARGS from $BOX — the network checks below would assert a copy"
    PROBE_IMAGE=""
  fi

  if [ -n "$PROBE_IMAGE" ]; then
    # Every run is named, so the leftover check below can filter on containers THIS file
    # created. An `ancestor=` filter would also catch a container the operator happens to have
    # built from the same pinned base image.
    NAME_PREFIX="agentbox-test-$$"
    run_probe() {  # <suffix> <net args> <bash -c script>
      # shellcheck disable=SC2086  # $2 is a flag list and has to split
      docker run --rm --name "$NAME_PREFIX-$1" $2 --entrypoint /bin/bash "$PROBE_IMAGE" -c "$3"
    }

    # The offline mode is the whole security claim. Each of these three is a way it could be
    # false while the box still looked closed.
    out="$(run_probe route "$OFFLINE_NET_ARGS" 'cat /proc/net/route' 2>&1 | tail -n +2)"
    [ -z "$out" ] || fail "'$OFFLINE_NET_ARGS' left a route in the box: $out"

    run_probe tcp "$OFFLINE_NET_ARGS" 'exec 3<>/dev/tcp/1.1.1.1/443' >/dev/null 2>&1 \
      && fail "'$OFFLINE_NET_ARGS' reached 1.1.1.1:443"

    run_probe dns "$OFFLINE_NET_ARGS" "getent hosts $MODEL_HOST" >/dev/null 2>&1 \
      && fail "'$OFFLINE_NET_ARGS' resolved $MODEL_HOST — the host is supposed to be gone too"

    # The control. Without it, an image too broken to open any socket would pass every check
    # above and the block would read as isolation when it is a bug.
    run_probe control "$ONLINE_NET_ARGS" "getent hosts $MODEL_HOST" >/dev/null 2>&1 \
      || fail "the --online control could not resolve $MODEL_HOST — the probe proves nothing"

    # `--rm` on every run is what keeps the box disposable. A leftover would be a writable
    # layer surviving a session, which is the property `--rm` is there for.
    leftover="$(docker ps -a --filter "name=^$NAME_PREFIX-" --format '{{.Names}}')"
    [ -z "$leftover" ] || fail "a probe container outlived its run: $leftover"
  fi
fi

if [ "$fails" -eq 0 ]; then
  echo "agentbox: all checks passed"
else
  echo "agentbox: $fails check(s) failed"
  exit 1
fi
