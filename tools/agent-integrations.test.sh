#!/bin/bash
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AXON_ROOT="$(cd "$TOOLS_DIR/.." && pwd)"
SCRIPT="$TOOLS_DIR/agent-integrations.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
export HOME="$tmp/home"
export AXON_TEST_UV_LOG="$tmp/uv.log"
export PATH="$tmp/bin:/usr/bin:/bin:/usr/sbin:/sbin"
mkdir -p "$HOME/.config/opencode" "$tmp/bin"

cat > "$tmp/bin/uv" <<'STUB'
#!/bin/bash
set -euo pipefail
printf '%s|%s\n' "$PWD" "$*" >> "$AXON_TEST_UV_LOG"
if [ "${AXON_TEST_UV_FAIL:-0}" = 1 ]; then
  exit 19
fi
platform="${*: -1}"
if [ "$platform" = opencode ]; then
  mkdir -p .opencode/skills/graphify .opencode/plugins
  printf '%s\n' '// graphify fixture skill' > .opencode/skills/graphify/.keep
  printf '%s\n' '// graphify fixture plugin' > .opencode/plugins/graphify.js
fi
STUB
chmod +x "$tmp/bin/uv"

cat > "$tmp/bin/opencode" <<'STUB'
#!/bin/bash
if [ "$1" = "--version" ] || [ "$1" = "-V" ]; then
  echo "opencode 0.0.0"
else
  exit 0
fi
STUB
chmod +x "$tmp/bin/opencode"

source "$TOOLS_DIR/lib/toml.sh"
pin="$(toml_get_in graphify pin "$AXON_ROOT/upstreams.toml")"
test -n "$pin"

list_output="$($SCRIPT list)"
case "$list_output" in
  *"Configured on this machine:"*) ;;
  *) echo "list did not distinguish configured state" >&2; exit 1 ;;
esac

machine_output="$($SCRIPT status --machine)"
echo "$machine_output" | grep -q '^opencode|runnable|' || {
  echo "status --machine did not report opencode as runnable" >&2
  exit 1
}

$SCRIPT install opencode
plugin="$HOME/.config/opencode/plugins/graphify.js"
test -s "$plugin"
grep -q 'graphify' "$plugin"
grep -Fq "tool run --from graphifyy==$pin graphify install --platform opencode" "$AXON_TEST_UV_LOG"

scratch="$(head -n 1 "$AXON_TEST_UV_LOG" | cut -d '|' -f 1)"
test "$scratch" != "$AXON_ROOT"
test ! -e "$scratch"

first="$(cksum "$plugin")"
$SCRIPT install opencode
test "$(cksum "$plugin")" = "$first"

machine_output="$($SCRIPT status --machine)"
echo "$machine_output" | grep -q '^opencode|integrated|' || {
  echo "status --machine did not report opencode as integrated after install" >&2
  exit 1
}

printf '0.0.0' > "$HOME/.config/opencode/.graphify-upstream-pin"
machine_output="$($SCRIPT status --machine)"
echo "$machine_output" | grep -q '^opencode|stale|' || {
  echo "status --machine did not report stale pin state" >&2
  exit 1
}

printf '%s\n' '// known-good graphify plugin' > "$plugin"
before="$(cksum "$plugin")"
if AXON_TEST_UV_FAIL=1 $SCRIPT install opencode >/dev/null 2>&1; then
  echo "failed upstream install unexpectedly succeeded" >&2
  exit 1
fi
test "$(cksum "$plugin")" = "$before"

if $SCRIPT install unknown-harness >/dev/null 2>&1; then
  echo "unknown harness unexpectedly succeeded" >&2
  exit 1
fi

echo "agent-integrations tests passed"
