# tools/lib/publish.sh — one canonical form for a container's published ports, so a declaration
# and a running container can actually be compared.
#
# Why this exists: tools/service-runner.sh builds CONTAINER_ARGS (including every `-p`) and passes
# them to `run -d` only. An existing container is started by name, so a changed `ports` value in
# the manifest or the overlay parses correctly, resolves correctly, and never reaches the thing it
# describes. The two sides could not even be compared before this, because each runtime reports
# its bindings in its own shape and the manifest writes docker's `-p` syntax.
#
# Canonical form, one per line, sorted by the caller:
#
#     HOSTADDR:HOSTPORT:CONTAINERPORT/PROTO
#
# with an unset host address normalising to 0.0.0.0 and an absent protocol to tcp, because that is
# what every runtime here means by leaving them out. A `-p` with no host port at all (`8080`) keeps
# `*` in that field: the runtime picks a port at random, so there is no value to compare against
# and pretending otherwise would report drift on every single start.
#
# bash 3.2-safe (README.md#portable-shell). jq is a declared host requirement (toolchain.toml).

# normalize_publish <-p spec> — canonical form of one declared publish entry.
normalize_publish() {
  local spec="${1:-}" proto=tcp body rest hostaddr hostport cport
  [ -n "$spec" ] || return 0
  case "$spec" in
    */*) proto="${spec##*/}"; body="${spec%/*}" ;;
    *)   body="$spec" ;;
  esac
  case "$body" in
    *:*:*)
      hostaddr="${body%%:*}"; rest="${body#*:}"
      hostport="${rest%%:*}"; cport="${rest##*:}"
      ;;
    *:*)
      hostaddr=""; hostport="${body%%:*}"; cport="${body##*:}"
      ;;
    *)
      hostaddr=""; hostport=""; cport="$body"
      ;;
  esac
  printf '%s:%s:%s/%s\n' "${hostaddr:-0.0.0.0}" "${hostport:-*}" "$cport" "${proto:-tcp}"
}

# publish_from_docker <stdin: `docker inspect --format '{{json .HostConfig.PortBindings}}'`>
# An empty HostIp means every interface, which docker writes as "" and means 0.0.0.0.
publish_from_docker() {
  jq -r '
    if . == null then empty else
      to_entries[]
      | (.key | split("/")) as $cp
      | .value[]?
      | "\(if (.HostIp // "") == "" then "0.0.0.0" else .HostIp end):\(.HostPort):\($cp[0])/\($cp[1] // "tcp")"
    end
  ' 2>/dev/null | sort
}

# publish_from_apple <name> <stdin: `container list -a --format json`>
# apple-container reports the whole inventory, so the container is selected here rather than by
# the caller: filtering on the name is part of reading its answer, not a separate concern.
publish_from_apple() {
  jq -r --arg n "${1:?publish_from_apple needs a container name}" '
    .[]? | select(.configuration.id == $n) | .configuration.publishedPorts[]?
    | "\(.hostAddress // "0.0.0.0"):\(.hostPort):\(.containerPort)/\(.proto // "tcp")"
  ' 2>/dev/null | sort
}

# publish_diff <declared-file> <running-file> — prints one line per difference, exit 1 when they
# differ at all. Both files hold canonical lines, sorted.
publish_diff() {
  local declared="$1" running="$2" only_declared only_running
  only_declared="$(comm -23 "$declared" "$running")"
  only_running="$(comm -13 "$declared" "$running")"
  [ -n "$only_declared$only_running" ] || return 0
  [ -n "$only_declared" ] && printf '%s\n' "$only_declared" | sed 's/^/  declared, not published: /'
  [ -n "$only_running" ] && printf '%s\n' "$only_running" | sed 's/^/  published, not declared: /'
  return 1
}
