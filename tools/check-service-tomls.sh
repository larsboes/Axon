#!/bin/bash
# check-service-tomls.sh — sh_test body for //:service_toml_schema_test.
# Asserts every service.toml declares the load-bearing fields its kind needs, and that a
# container's tag is a real pin: present, non-empty, and never "latest" (an unpinned tag
# makes a deploy non-reproducible; see README.md#pins-and-cooldown). Pure file-based check:
# operates only on
# files declared in the sh_test's `data`, so it runs identically from the repo root or
# inside `bazel test`'s runfiles sandbox where git and the wider checkout are absent.
#
# Two kinds, two field sets (schemas/service.toml.example):
#   container (the default when `kind` is absent) -> name, image, tag; tag pinned
#   process                                       -> name, command, port; no image/tag
# A manifest carrying fields from both kinds is rejected rather than resolved by
# precedence: "which half is real" must never be a guess at start time.
#
# Spine manifests (a service.toml at the repo root, today only dashboard/) are checked
# by the same rules — the runner reads them through the same interpreter.
set -e

# Runfiles-relocation: same rationale as check-architecture-fresh.sh — under `bazel test`
# the entrypoint is relocated and loses its tools/ context, so resolve the lib dir from
# TEST_SRCDIR/TEST_WORKSPACE (Bazel's runfiles-root env vars). For a direct repo-root
# invocation those are unset, so self-locate from this file's own dirname instead.
# paths.sh sources toml.sh and exports AXON_ROOT — both are all we need here.
if [ -n "${TEST_SRCDIR:-}" ]; then
  _lib="$TEST_SRCDIR/$TEST_WORKSPACE/tools/lib"
else
  _lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/lib" && pwd)"
fi
source "$_lib/paths.sh"

fail=0
found=0
for svc in "$AXON_ROOT"/capabilities/*/service.toml "$AXON_ROOT"/*/service.toml; do
  [ -f "$svc" ] || continue          # empty glob → literal path, skip it
  found=1
  cap="$(basename "$(dirname "$svc")")"

  kind="$(toml_get kind "$svc")"
  [ -n "$kind" ] || kind="container"

  case "$kind" in
    container)
      for key in name image tag; do
        if [ -z "$(toml_get "$key" "$svc")" ]; then
          echo "FAIL [$cap]: missing or empty required field '$key' — $svc" >&2
          fail=1
        fi
      done

      tag="$(toml_get tag "$svc")"
      if [ "$tag" = "latest" ]; then
        echo "FAIL [$cap]: tag must be a pinned version, never 'latest' — $svc" >&2
        fail=1
      fi
      ;;
    process)
      if [ -z "$(toml_get name "$svc")" ]; then
        echo "FAIL [$cap]: missing or empty required field 'name' — $svc" >&2
        fail=1
      fi
      if [ -z "$(toml_array command "$svc")" ]; then
        echo "FAIL [$cap]: kind=process needs a non-empty command = [...] — $svc" >&2
        fail=1
      fi
      if [ -z "$(toml_get port "$svc")" ]; then
        echo "FAIL [$cap]: kind=process needs a port — the registry, the proxy and the health poll all read it — $svc" >&2
        fail=1
      fi
      for key in image tag volumes env_file; do
        if [ -n "$(toml_get "$key" "$svc")" ] || [ -n "$(toml_array "$key" "$svc")" ]; then
          echo "FAIL [$cap]: kind=process must not declare container field '$key' — $svc" >&2
          fail=1
        fi
      done
      ;;
    *)
      echo "FAIL [$cap]: unknown kind '$kind' (expected container or process) — $svc" >&2
      fail=1
      ;;
  esac
done

if [ "$found" -eq 0 ]; then
  echo "FAIL: no capabilities/*/service.toml found — expected at least one" >&2
  exit 1
fi

# One host port, one owner. A scalar `port` (process kind) and the host side of every
# container `ports = ["[ip:]host:container"]` mapping share one namespace on the machine;
# two manifests shipping the same default is the scouting-vs-vaultwarden 8080 collision,
# which was found by starting the full enabled set instead of by a gate. Now it's a gate.
port_lines=""
for svc in "$AXON_ROOT"/capabilities/*/service.toml "$AXON_ROOT"/*/service.toml; do
  [ -f "$svc" ] || continue
  cap="$(basename "$(dirname "$svc")")"
  hostports="$(toml_get port "$svc")"
  for m in $(toml_array ports "$svc"); do
    hp="${m%:*}"                      # drop the container side
    hp="${hp##*:}"                    # drop an optional bind address
    hostports="$hostports $hp"
  done
  for hp in $hostports; do
    [ -n "$hp" ] || continue
    port_lines="${port_lines}${hp} ${cap}
"
  done
done
dups="$(printf '%s' "$port_lines" | sort -n | awk '{ if ($1 == prev) print $1 " (" prevcap ", " $2 ")"; prev = $1; prevcap = $2 }')"
if [ -n "$dups" ]; then
  echo "FAIL: host port declared by more than one manifest: $dups" >&2
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  echo "service.toml schema check FAILED." >&2
  exit 1
fi

echo "service.toml schema check passed ($(basename "$AXON_ROOT"): containers pinned with name/image/tag, processes with name/command/port, no container fields, host ports unique)."
