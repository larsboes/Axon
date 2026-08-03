#!/bin/bash
# Tests for tools/lib/runargs.sh — the canonical form that lets a declared run-time argument set be
# compared with a running container's.
#
# Synthetic on purpose: the runtime JSON fixtures below are the real shapes (captured from
# `container list -a --format json` and `docker inspect --format '{{json .}}'`), so the cases run
# without a container runtime and cover every class the issue names — ports, mounts, capabilities,
# network, and the env file whose values must never be rendered.
set -uo pipefail

_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
LIB=""
for _c in "$_dir/lib" "$_dir/tools/lib"; do
  if [ -f "$_c/runargs.sh" ]; then LIB="$_c"; break; fi
done
[ -n "$LIB" ] || { echo "runargs: cannot find runargs.sh next to $_dir" >&2; exit 1; }
# shellcheck source=lib/runargs.sh
. "$LIB/runargs.sh"

command -v jq >/dev/null 2>&1 || { echo "runargs: jq not on PATH — cannot run" >&2; exit 1; }

fails=0
eq() {  # eq <description> <expected> <actual>
  if [ "$2" != "$3" ]; then
    echo "FAIL: $1"
    echo "  expected: $2"
    echo "  actual:   $3"
    fails=$((fails + 1))
  fi
}
contains() {  # contains <description> <needle> <haystack>
  case "$3" in
    *"$2"*) ;;
    *) echo "FAIL: $1"; echo "  expected to contain: $2"; echo "  actual:   $3"; fails=$((fails + 1)) ;;
  esac
}
lacks() {  # lacks <description> <needle> <haystack>
  case "$3" in
    *"$2"*) echo "FAIL: $1"; echo "  must NOT contain: $2"; fails=$((fails + 1)) ;;
  esac
}

# --- canonical forms ------------------------------------------------------
eq "host:container"            "0.0.0.0:9090:8080/tcp"   "$(normalize_publish 9090:8080)"
eq "addr:host:container"       "127.0.0.1:9090:8080/tcp" "$(normalize_publish 127.0.0.1:9090:8080)"
eq "explicit proto"            "0.0.0.0:53:53/udp"       "$(normalize_publish 53:53/udp)"
eq "addr + proto"              "127.0.0.1:53:53/udp"     "$(normalize_publish 127.0.0.1:53:53/udp)"
# No host port means the runtime picks one, so there is nothing to compare against. Reporting a
# concrete value here would make every start look like drift.
eq "container port only"       "0.0.0.0:*:8080/tcp"      "$(normalize_publish 8080)"

# A runtime may echo the kernel spelling of a capability the manifest writes bare. Same bit.
eq "cap: manifest form"        "NET_ADMIN"               "$(normalize_cap NET_ADMIN)"
eq "cap: kernel form"          "NET_ADMIN"               "$(normalize_cap CAP_NET_ADMIN)"
eq "cap: lower case"           "NET_RAW"                 "$(normalize_cap cap_net_raw)"

eq "network: absent is default" "default"                "$(normalize_network "")"
eq "network: bridge is default" "default"                "$(normalize_network bridge)"
eq "network: host is host"      "host"                   "$(normalize_network host)"

# --- the declared side: read from the argv, not re-derived ----------------
# The argv is the point: whatever `run -d` would be given is what gets compared, so the comparison
# cannot drift away from the construction.
eq "declared: a full argv" \
"cap NET_ADMIN
envfile /overlay/env/pihole.env
mount /overlay/data/pihole:/etc/pihole
network default
port 0.0.0.0:53:53/udp" \
  "$(declared_runspec --name pihole --cap-add NET_ADMIN -p 53:53/udp \
       -v /overlay/data/pihole:/etc/pihole --env-file /overlay/env/pihole.env | sort)"

# --network host suppresses every -p in container_init, so the declared stream must carry the
# network and no ports at all. The re-deriving version emitted the manifest's ports regardless,
# which would have been permanent false drift on the first capability declaring both.
eq "declared: host network, no ports" \
"envfile /overlay/env/ha.env
network host" \
  "$(declared_runspec --name ha --network host --env-file /overlay/env/ha.env | sort)"

# --- the running side, in each runtime's own shape ------------------------
docker_json='{
  "Config": {"Env": ["PATH=/usr/bin", "POSTGRES_PASSWORD=secret-value", "POSTGRES_DB=axon"]},
  "Mounts": [
    {"Type": "volume", "Name": "axon-postgres-data", "Source": "/var/lib/docker/volumes/axon-postgres-data/_data", "Destination": "/var/lib/postgresql/data"},
    {"Type": "bind", "Source": "/overlay/data/pg", "Destination": "/backup"},
    {"Type": "tmpfs", "Source": "", "Destination": "/run"}
  ],
  "HostConfig": {
    "PortBindings": {"5432/tcp": [{"HostIp": "127.0.0.1", "HostPort": "5432"}], "53/udp": [{"HostIp": "", "HostPort": "53"}]},
    "CapAdd": ["NET_ADMIN"],
    "NetworkMode": "bridge"
  }
}'
eq "docker: every class, canonical" \
"cap NET_ADMIN
mount /overlay/data/pg:/backup
mount axon-postgres-data:/var/lib/postgresql/data
network default
port 0.0.0.0:53:53/udp
port 127.0.0.1:5432:5432/tcp" \
  "$(printf '%s' "$docker_json" | runspec_from_docker)"

# A container with nothing declared still reports its network, or the declared side's `network
# default` would look like an addition on every single container.
eq "docker: bare container still has a network" "network default" \
  "$(printf '{"HostConfig":{}}' | runspec_from_docker)"

apple_json='[
 {"configuration":{"id":"other","publishedPorts":[{"containerPort":1,"hostAddress":"0.0.0.0","hostPort":1,"proto":"tcp"}],"networks":[{"network":"default"}]}},
 {"configuration":{
   "id":"mine",
   "publishedPorts":[{"containerPort":5432,"hostAddress":"0.0.0.0","hostPort":5432,"proto":"tcp"}],
   "capAdd":["CAP_NET_ADMIN"],
   "networks":[{"network":"default"}],
   "mounts":[
     {"destination":"/var/lib/postgresql/data","source":"/Users/x/volumes/axon-postgres-data/volume.img","type":{"volume":{"name":"axon-postgres-data"}}},
     {"destination":"/data","source":"/overlay/data/vaultwarden","type":{"virtiofs":{}}},
     {"destination":"/run","source":"","type":{"tmpfs":{}}}
   ],
   "initProcess":{"environment":["PATH=/usr/bin","POSTGRES_PASSWORD=secret-value","POSTGRES_DB=axon"]}
 }}
]'
eq "apple: selects the named container, every class" \
"cap NET_ADMIN
mount /overlay/data/vaultwarden:/data
mount axon-postgres-data:/var/lib/postgresql/data
network default
port 0.0.0.0:5432:5432/tcp" \
  "$(printf '%s' "$apple_json" | runspec_from_apple mine)"
eq "apple: a name that is not there" "" "$(printf '%s' "$apple_json" | runspec_from_apple absent)"

# --- the comparison, per class --------------------------------------------
S="$(mktemp -d)"; trap 'rm -rf "$S"' EXIT
declared() { printf '%s\n' "$@" | sort > "$S/d"; }
running()  { printf '%s\n' "$@" | sort > "$S/r"; }

declared "port 0.0.0.0:9090:8080/tcp"; running "port 0.0.0.0:9090:8080/tcp"
runspec_diff "$S/d" "$S/r" port >/dev/null || { echo "FAIL: identical sets reported as drift"; fails=$((fails + 1)); }

# host address changed — the security-relevant one: a bind narrowed to loopback in the manifest
# while the container still listens on every interface.
declared "port 127.0.0.1:9090:8080/tcp"; running "port 0.0.0.0:9090:8080/tcp"
runspec_diff "$S/d" "$S/r" port >/dev/null && { echo "FAIL: a changed host address was not reported"; fails=$((fails + 1)); }

declared "port 0.0.0.0:9091:8080/tcp"; running "port 0.0.0.0:9090:8080/tcp"
runspec_diff "$S/d" "$S/r" port >/dev/null && { echo "FAIL: a changed host port was not reported"; fails=$((fails + 1)); }

declared "port 0.0.0.0:9090:8080/tcp"; running "port 0.0.0.0:9090:8080/tcp" "port 0.0.0.0:5000:5000/tcp"
runspec_diff "$S/d" "$S/r" port >/dev/null && { echo "FAIL: a removed publish was not reported"; fails=$((fails + 1)); }

# The three classes #33 adds, each on its own: a class must not be able to go silent while the
# others work.
declared "mount axon-pg-data:/var/lib/postgresql/data"; running "mount postgres_data:/var/lib/postgresql"
runspec_diff "$S/d" "$S/r" mount >/dev/null && { echo "FAIL: a changed mount was not reported"; fails=$((fails + 1)); }

declared "cap NET_ADMIN" "cap NET_RAW"; running "cap NET_ADMIN"
runspec_diff "$S/d" "$S/r" cap >/dev/null && { echo "FAIL: a dropped capability was not reported"; fails=$((fails + 1)); }

declared "network host"; running "network default"
runspec_diff "$S/d" "$S/r" network >/dev/null && { echo "FAIL: a changed network was not reported"; fails=$((fails + 1)); }

# One class differing must not drag another into the report — the operator acts per class.
declared "port 0.0.0.0:1:1/tcp" "cap NET_ADMIN"; running "port 0.0.0.0:2:2/tcp" "cap NET_ADMIN"
runspec_diff "$S/d" "$S/r" cap >/dev/null || { echo "FAIL: an unchanged class reported drift"; fails=$((fails + 1)); }

# The message has to say which side each line came from, or the operator cannot act on it.
declared "port 127.0.0.1:9090:8080/tcp"; running "port 0.0.0.0:9090:8080/tcp"
out="$(runspec_diff "$S/d" "$S/r" port 2>&1 || true)"
contains "the diff names the declared side" "declared, not in container: 127.0.0.1:9090:8080/tcp" "$out"
contains "the diff names the running side" "in container, not declared: 0.0.0.0:9090:8080/tcp" "$out"

# --- the env file: drift reported, values never rendered ------------------
SECRET="hunter2-do-not-print"
printf 'POSTGRES_DB=axon\nPOSTGRES_PASSWORD=%s\n\n# a comment\n' "$SECRET" > "$S/env"

# Same values on both sides: no drift, nothing printed.
out="$(printf 'PATH=/usr/bin\nPOSTGRES_DB=axon\nPOSTGRES_PASSWORD=%s\n' "$SECRET" | env_diff "$S/env")"
rc=$?
eq "env: identical is silent" "" "$out"
[ "$rc" -eq 0 ] || { echo "FAIL: identical env reported as drift"; fails=$((fails + 1)); }

# A rotated credential — the case the issue calls the one that bites quietly.
out="$(printf 'POSTGRES_DB=axon\nPOSTGRES_PASSWORD=old-value\n' | env_diff "$S/env" || true)"
contains "env: a rotated value is reported by key" "value differs: POSTGRES_PASSWORD" "$out"
lacks    "env: the new value never appears"        "$SECRET"                           "$out"
lacks    "env: the old value never appears"        "old-value"                         "$out"

# A key added to the file but never applied, because the container predates it.
out="$(printf 'POSTGRES_DB=axon\n' | env_diff "$S/env" || true)"
contains "env: a missing key is reported" "declared, not in container: POSTGRES_PASSWORD" "$out"
lacks    "env: still no value"            "$SECRET"                                       "$out"

# The image's own environment is not drift: a container carries far more than the file declares.
out="$(printf 'PATH=/x\nLANG=C\nPOSTGRES_DB=axon\nPOSTGRES_PASSWORD=%s\n' "$SECRET" | env_diff "$S/env")"
eq "env: image defaults are not drift" "" "$out"

# `KEY` with no `=` takes its value from the host environment at creation time. There is nothing
# to compare, and saying nothing would report an unchecked key as checked.
printf 'FROM_HOST\n' > "$S/env-bare"
out="$(printf 'FROM_HOST=whatever\n' | env_diff "$S/env-bare" || true)"
contains "env: a host-sourced key is named as uncheckable" "cannot check" "$out"

# A declared env file that is not there at all is a fail, not a match.
out="$(printf 'X=1\n' | env_diff "$S/does-not-exist" || true)"
contains "env: a missing file is reported" "env file is missing" "$out"

# The guard that matters most: no path through this lib may put the values on disk. bash 3.2 backs
# `<<<` and `<<EOF` with a real temp file, so a here-string in the comparison would spill every
# credential into $TMPDIR to answer whether one changed.
# Comment lines are skipped, or this would fire on the paragraph in runargs.sh that explains it.
offenders="$(awk '!/^[[:space:]]*#/ && (/<<</ || /<<[A-Za-z_]/) { print NR": "$0 }' "$LIB/runargs.sh")"
if [ -n "$offenders" ]; then
  echo "FAIL: runargs.sh uses a here-string or here-doc — bash 3.2 backs both with a temp file,"
  echo "      which would write container credentials to disk:"
  printf '%s\n' "$offenders" | sed 's/^/  /'
  fails=$((fails + 1))
fi

if [ "$fails" -gt 0 ]; then
  echo "runargs.sh: $fails check(s) failed"
  exit 1
fi
echo "runargs.sh: all checks passed"
