#!/bin/bash
# Tests for tools/service-runner.sh's boot-persistence contract (#9).
#
# The defect this covers is an absence: install-persistence existed, install.sh never called it,
# and nothing could report the result — so a capability could declare autostart, be enabled, run
# all day and simply be gone after a reboot. Every state below (installed, missing, stale,
# not-applicable, unsupported) is therefore about what the tool SAYS, which is the part that was
# missing entirely.
#
# Built on the same throwaway-root idea as service-runner.test.sh and manifest-resolution.test.sh:
# a scratch Axon root with its own overlay, machine.toml and capability manifests, plus a scratch
# HOME so unit files land inside the sandbox. Nothing here reads this machine's enabled set,
# touches its LaunchAgents, or runs launchctl/systemctl — `persistence-status` renders and
# compares files, which is exactly why rendering was separated from installing.
set -uo pipefail

fails=0
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }

_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
SRC_TOOLS=""
for _c in "$_dir" "$_dir/tools"; do
  if [ -f "$_c/service-runner.sh" ]; then SRC_TOOLS="$_c"; break; fi
done
[ -n "$SRC_TOOLS" ] || { echo "persistence: cannot find service-runner.sh next to $_dir" >&2; exit 1; }

SCRATCH="$(mktemp -d "${TEST_TMPDIR:-/tmp}/persistence.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT
ROOT="$SCRATCH/axon"
OVERLAY="$SCRATCH/overlay"
FAKE_HOME="$SCRATCH/home"

mkdir -p "$ROOT/tools/lib" "$ROOT/tools/templates" "$OVERLAY/config" \
         "$FAKE_HOME/Library/LaunchAgents" "$FAKE_HOME/.config/systemd/user"
cp "$SRC_TOOLS/service-runner.sh" "$SRC_TOOLS/capability.sh" "$SRC_TOOLS/watchdog.sh" "$ROOT/tools/"
# The whole lib and template directories rather than a named subset: a list here would rot into a
# "No such file" the next time service-runner picks up another one.
cp "$SRC_TOOLS"/lib/*.sh "$ROOT/tools/lib/"
cp -R "$SRC_TOOLS"/templates/. "$ROOT/tools/templates/"   # -R: templates/ has subdirectories
printf 'overlay = "%s"\n' "$OVERLAY" > "$ROOT/axon.toml"

# Four capabilities, one per state the issue names. `always` is the interesting one; the rest fix
# the shape of an answer that must NOT be "install something".
mkcap() {  # mkcap <name> <autostart> [extra manifest lines]
  mkdir -p "$ROOT/capabilities/$1"
  { printf 'kind = "process"\nname = "%s"\nautostart = "%s"\ncommand = ["/bin/sh", "-c", "sleep 1"]\n' "$1" "$2"
    shift 2; [ $# -gt 0 ] && printf '%s\n' "$@"
  } > "$ROOT/capabilities/$1/service.toml"
}
mkcap always true
mkcap ondemand false
mkcap gone true

machine() {  # machine <os> <capabilities-toml-array>
  printf 'os = "%s"\ncontainer_runtime = "docker"\ncapabilities = %s\n' "$1" "$2" \
    > "$OVERLAY/config/machine.toml"
}

run() {  # run <capability> [command] — defaults to persistence-status
  out="$(HOME="$FAKE_HOME" XDG_CONFIG_HOME="$FAKE_HOME/.config" \
         "$ROOT/tools/service-runner.sh" "${2:-persistence-status}" "$1" 2>&1)"
  status=$?
}
state_of() { printf '%s' "$out" | cut -f2 | head -1; }

expect_state() {  # expect_state <label> <capability> <expected state>
  run "$2"
  local got; got="$(state_of)"
  [ "$got" = "$3" ] || fail "$1: expected state '$3', got '$got' (full: $out)"
}

# --- macOS -----------------------------------------------------------------
machine macos '["always", "ondemand", "gone"]'
PLIST="$FAKE_HOME/Library/LaunchAgents/com.axon.always.plist"

# missing — the whole point of the issue: enabled, autostart, nothing installed.
rm -f "$PLIST"
expect_state "macos, nothing installed" always missing

# installed — render the unit the way install would, then confirm it is recognised.
run always install-persistence
[ -f "$PLIST" ] || fail "install-persistence did not write $PLIST (said: $out)"
expect_state "macos, freshly installed" always installed

# stale — the unit exists but no longer matches what the declaration renders to. This is the case
# a plain existence check cannot see, and it is how a moved runtime binary silently keeps a
# watchdog pointing at a PATH that no longer resolves.
printf '<!-- hand-edited, no longer what the template renders -->\n' >> "$PLIST"
expect_state "macos, unit edited out from under the declaration" always stale

# ...and re-installing brings it back into agreement, so `stale` is actionable, not terminal.
run always install-persistence
expect_state "macos, re-installed after drift" always installed

# not applicable — a capability the manifest declares on-demand. A watchdog would call `start`
# every 30s and contradict the manifest, so the answer is n/a, never "missing".
expect_state "macos, on-demand capability" ondemand "n/a"
run ondemand
case "$out" in
  *"on-demand"*) ;;
  *) fail "the n/a answer must say why, said: $out" ;;
esac

# a natively-restarting container runtime owes no watchdog either
mkdir -p "$ROOT/capabilities/dockercap"
printf 'kind = "container"\nname = "dockercap"\nautostart = "true"\nimage = "x"\ntag = "1"\n' \
  > "$ROOT/capabilities/dockercap/service.toml"
machine macos '["dockercap"]'
expect_state "macos, docker restarts it natively" dockercap "n/a"
machine macos '["always", "ondemand", "gone"]'

# --- disabled: the unit outlives the enabled set ---------------------------
# watchdog.sh calls `service-runner.sh start <cap>` every 30s and consults nothing about the
# enabled set, so a unit left behind by `capability.sh disable` walks the capability back up.
# The tool has to keep answering for a capability that is no longer enabled, or nothing can
# report the leftover.
run gone install-persistence
GONE_PLIST="$FAKE_HOME/Library/LaunchAgents/com.axon.gone.plist"
[ -f "$GONE_PLIST" ] || fail "could not install persistence for the to-be-disabled capability"
machine macos '["always", "ondemand"]'
expect_state "macos, unit left behind by a disable" gone installed
[ -f "$GONE_PLIST" ] || fail "the leftover unit disappeared on its own — the disposition must be explicit"

# remove-persistence is that explicit disposition, and it is idempotent.
run gone remove-persistence
[ ! -f "$GONE_PLIST" ] || fail "remove-persistence left $GONE_PLIST behind"
expect_state "macos, after removal" gone missing
run gone remove-persistence
[ "$status" -eq 0 ] || fail "remove-persistence on an already-removed unit should be a no-op, got exit $status"

# --- Linux -----------------------------------------------------------------
# The same capability contract, the other backend. Rendering is pure, so this runs on either host:
# what is asserted is the unit the declaration implies, not that systemd accepted it.
machine linux '["always", "ondemand", "gone"]'
UNIT="$FAKE_HOME/.config/systemd/user/axon-always.service"
rm -f "$UNIT"
expect_state "linux, nothing installed" always missing

# install-persistence would call systemctl, which this host may not have — so the file half is
# rendered directly and only the reporting is exercised, which is the half that was missing.
HOME="$FAKE_HOME" XDG_CONFIG_HOME="$FAKE_HOME/.config" \
  "$ROOT/tools/service-runner.sh" persistence-status always >/dev/null 2>&1
sed -e "s|__WATCHDOG_PATH__|$ROOT/tools/watchdog.sh|" \
    -e "s|__PATH__|/bin:/usr/local/bin:/usr/bin:/bin|" \
    -e "s|__CAPABILITY__|always|" \
    -e "s|__LOG_OUT__|/tmp/axon-always-watchdog.log|" \
    -e "s|__LOG_ERR__|/tmp/axon-always-watchdog.err|" \
    "$ROOT/tools/templates/systemd-watchdog.service.tmpl" > "$UNIT"
expect_state "linux, unit matching the declaration" always installed

printf '# hand-edited\n' >> "$UNIT"
expect_state "linux, unit edited" always stale

# --- unsupported OS --------------------------------------------------------
# An OS with no backend must say so rather than report `missing`, which would send an operator
# after an install that cannot work here.
machine windows '["always"]'
expect_state "windows has no backend" always unsupported
machine freebsd '["always"]'
expect_state "an unknown os" always unsupported

# --- the report never leaks this machine's own state -----------------------
# The issue asks for exactly this: test evidence must not carry a machine-specific enabled set or
# service history. Everything above ran against $SCRATCH, so no path in any answer may point at
# the real home.
machine macos '["always"]'
run always
case "$out" in
  "$SCRATCH"*|*"$SCRATCH"*) ;;
  *) fail "the answer should reference the scratch root only, said: $out" ;;
esac
case "$out" in
  *"$HOME/Library"*) fail "the answer referenced the real HOME: $out" ;;
esac

if [ "$fails" -gt 0 ]; then
  echo "persistence: $fails check(s) failed"
  exit 1
fi
echo "persistence: all checks passed"
