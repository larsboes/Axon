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
# HOME so unit files land inside the sandbox.
#
# launchctl, systemctl and loginctl are STUBBED on PATH. The first version of this file did not
# stub them and called the real `install-persistence`, which registered a live `com.axon.always`
# job in the author's launchd pointing at a scratch path that the test then deleted. A test for
# "does this machine's boot persistence match its declaration" must not itself install boot
# persistence on the machine running it. The stubs also make the load state deterministic, which
# a real supervisor cannot be: the same assertion answered `installed` on macOS and
# `installed-not-loaded` on the Linux CI runner.
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

# --- supervisor stubs ------------------------------------------------------
# One state file holds the labels the fake supervisor considers loaded, so load/unload and the
# query that reads them cannot disagree. Deliberately dumb: this stands in for launchd, it does
# not model it.
STUB_BIN="$SCRATCH/bin"; STUB_STATE="$SCRATCH/loaded.txt"
mkdir -p "$STUB_BIN"; : > "$STUB_STATE"
cat > "$STUB_BIN/launchctl" <<'STUB'
#!/bin/bash
case "${1:-}" in
  list)   cat "$AXON_TEST_STUB_STATE" ;;
  load)   printf '%s\t0\tcom.axon.%s\n' "$$" "$(basename "${2:-}" .plist | sed 's/^com\.axon\.//')" >> "$AXON_TEST_STUB_STATE" ;;
  unload) label="com.axon.$(basename "${2:-}" .plist | sed 's/^com\.axon\.//')"
          grep -v "$label\$" "$AXON_TEST_STUB_STATE" > "$AXON_TEST_STUB_STATE.new" || true
          mv "$AXON_TEST_STUB_STATE.new" "$AXON_TEST_STUB_STATE" ;;
esac
exit 0
STUB
cat > "$STUB_BIN/systemctl" <<'STUB'
#!/bin/bash
args="$*"
case "$args" in
  *is-active*)   unit="${args##* }"; grep -q "^${unit}$" "$AXON_TEST_STUB_STATE" && exit 0 || exit 1 ;;
  *enable*)      unit="${args##* }"; echo "$unit" >> "$AXON_TEST_STUB_STATE" ;;
  *disable*)     unit="${args##* }"
                 grep -v "^${unit}$" "$AXON_TEST_STUB_STATE" > "$AXON_TEST_STUB_STATE.new" || true
                 mv "$AXON_TEST_STUB_STATE.new" "$AXON_TEST_STUB_STATE" ;;
esac
exit 0
STUB
printf '#!/bin/bash\necho "Linger=yes"\n' > "$STUB_BIN/loginctl"
chmod +x "$STUB_BIN"/launchctl "$STUB_BIN"/systemctl "$STUB_BIN"/loginctl
export AXON_TEST_STUB_STATE="$STUB_STATE"
PATH="$STUB_BIN:$PATH"; export PATH

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

# The unit file is there and matches, whether or not a supervisor has loaded it. Used for the
# Linux branch, where the load half is genuinely not decidable from a planted tree: that branch's
# persistence_loaded also tests for /run/systemd/system, which no PATH stub can create, so the
# answer is `unknown` on a macOS host and stub-driven on a systemd one. Asserting the file half
# there is asserting the part the planted tree actually determines.
expect_installed_file() {  # expect_installed_file <label> <capability>
  run "$2"
  local got; got="$(state_of)"
  case "$got" in
    installed|installed-not-loaded) ;;
    *) fail "$1: expected the unit to match the declaration, got '$got' (full: $out)" ;;
  esac
}

# The guard that would have caught the first version of this file: if PATH resolution ever puts
# the real supervisor ahead of the stub, every assertion below still passes while the test quietly
# installs launchd jobs on the machine running it. Checked before anything runs, not after.
for _bin in launchctl systemctl loginctl; do
  _resolved="$(command -v "$_bin" 2>/dev/null || true)"
  case "$_resolved" in
    "$STUB_BIN"/*) ;;
    *) echo "FAIL: '$_bin' resolves to '${_resolved:-nothing}', not the stub in $STUB_BIN —"
       echo "      this test would drive the real supervisor on this machine. Refusing to run."
       exit 1 ;;
  esac
done

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
# ...and it actually asked the supervisor to load it. Writing the file without loading it is the
# `installed-not-loaded` state, which install must not leave behind.
grep -q "com\.axon\.always\$" "$STUB_STATE" || fail "install-persistence wrote the unit but never loaded it"

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
expect_installed_file "linux, unit matching the declaration" always

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
