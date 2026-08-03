#!/bin/bash
# Tests for tools/lib/publish.sh — the canonical form that lets a declared publish set be compared
# with a running one.
#
# Synthetic on purpose: the runtime JSON fixtures below are the real shapes (captured from
# `container list -a --format json` and `docker inspect --format '{{json .HostConfig.PortBindings}}'`),
# so the cases run without a container runtime and cover the four changes that matter — host
# address, host port, container port, and a publish that was removed.
set -uo pipefail

_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
LIB=""
for _c in "$_dir/lib" "$_dir/tools/lib"; do
  if [ -f "$_c/publish.sh" ]; then LIB="$_c"; break; fi
done
[ -n "$LIB" ] || { echo "publish: cannot find publish.sh next to $_dir" >&2; exit 1; }
# shellcheck source=lib/publish.sh
. "$LIB/publish.sh"

command -v jq >/dev/null 2>&1 || { echo "publish: jq not on PATH — cannot run" >&2; exit 1; }

fails=0
eq() {  # eq <description> <expected> <actual>
  if [ "$2" != "$3" ]; then
    echo "FAIL: $1"
    echo "  expected: $2"
    echo "  actual:   $3"
    fails=$((fails + 1))
  fi
}

# --- the declared side: docker -p syntax as service.toml writes it ---------
eq "host:container"            "0.0.0.0:9090:8080/tcp"   "$(normalize_publish 9090:8080)"
eq "addr:host:container"       "127.0.0.1:9090:8080/tcp" "$(normalize_publish 127.0.0.1:9090:8080)"
eq "explicit proto"            "0.0.0.0:53:53/udp"       "$(normalize_publish 53:53/udp)"
eq "addr + proto"              "127.0.0.1:53:53/udp"     "$(normalize_publish 127.0.0.1:53:53/udp)"
# No host port means the runtime picks one, so there is nothing to compare against. Reporting a
# concrete value here would make every start look like drift.
eq "container port only"       "0.0.0.0:*:8080/tcp"      "$(normalize_publish 8080)"

# --- the running side, in each runtime's own shape ------------------------
docker_json='{"8080/tcp":[{"HostIp":"127.0.0.1","HostPort":"9090"}],"53/udp":[{"HostIp":"","HostPort":"53"}]}'
eq "docker: empty HostIp is every interface" \
  "0.0.0.0:53:53/udp
127.0.0.1:9090:8080/tcp" \
  "$(printf '%s' "$docker_json" | publish_from_docker)"
eq "docker: no bindings at all"  "" "$(printf 'null' | publish_from_docker)"

apple_json='[{"configuration":{"id":"other","publishedPorts":[{"containerPort":1,"hostAddress":"0.0.0.0","hostPort":1,"proto":"tcp"}]},"status":"running"},{"configuration":{"id":"mine","publishedPorts":[{"containerPort":5432,"hostAddress":"0.0.0.0","hostPort":5432,"proto":"tcp"}]},"status":"running"}]'
eq "apple: selects the named container" \
  "0.0.0.0:5432:5432/tcp" \
  "$(printf '%s' "$apple_json" | publish_from_apple mine)"
eq "apple: a name that is not there" "" "$(printf '%s' "$apple_json" | publish_from_apple absent)"

# --- the comparison: the four changes the issue names ---------------------
S="$(mktemp -d)"; trap 'rm -rf "$S"' EXIT
declared() { printf '%s\n' "$@" | sort > "$S/d"; }
running()  { printf '%s\n' "$@" | sort > "$S/r"; }

declared "0.0.0.0:9090:8080/tcp"; running "0.0.0.0:9090:8080/tcp"
publish_diff "$S/d" "$S/r" >/dev/null || { echo "FAIL: identical sets reported as drift"; fails=$((fails + 1)); }

# host address changed — the security-relevant one: a bind narrowed to loopback in the manifest
# while the container still listens on every interface.
declared "127.0.0.1:9090:8080/tcp"; running "0.0.0.0:9090:8080/tcp"
if publish_diff "$S/d" "$S/r" >/dev/null; then
  echo "FAIL: a changed host address was not reported"; fails=$((fails + 1))
fi

declared "0.0.0.0:9091:8080/tcp"; running "0.0.0.0:9090:8080/tcp"
publish_diff "$S/d" "$S/r" >/dev/null && { echo "FAIL: a changed host port was not reported"; fails=$((fails + 1)); }

declared "0.0.0.0:9090:8081/tcp"; running "0.0.0.0:9090:8080/tcp"
publish_diff "$S/d" "$S/r" >/dev/null && { echo "FAIL: a changed container port was not reported"; fails=$((fails + 1)); }

declared "0.0.0.0:9090:8080/tcp"; running "0.0.0.0:9090:8080/tcp" "0.0.0.0:5000:5000/tcp"
publish_diff "$S/d" "$S/r" >/dev/null && { echo "FAIL: a removed publish was not reported"; fails=$((fails + 1)); }

# The message has to say which side each line came from, or the operator cannot act on it.
declared "127.0.0.1:9090:8080/tcp"; running "0.0.0.0:9090:8080/tcp"
out="$(publish_diff "$S/d" "$S/r" 2>&1 || true)"
case "$out" in
  *"declared, not published: 127.0.0.1:9090:8080/tcp"*) ;;
  *) echo "FAIL: the diff did not name the declared side; said: $out"; fails=$((fails + 1)) ;;
esac
case "$out" in
  *"published, not declared: 0.0.0.0:9090:8080/tcp"*) ;;
  *) echo "FAIL: the diff did not name the running side; said: $out"; fails=$((fails + 1)) ;;
esac

if [ "$fails" -gt 0 ]; then
  echo "publish.sh: $fails check(s) failed"
  exit 1
fi
echo "publish.sh: all checks passed"
