#!/bin/bash
# The pin-vs-installed check, over planted manifests and planted binaries.
#
# The tracked manifest can only ever demonstrate the state this machine happens to be in —
# on the day this was written, four entries agreed and one did not — so every red path here
# gets a fixture. The states that matter are the four the issue named: agreeing, disagreeing,
# absent, unprobeable. Absent is the one worth the most: reporting a tool that is not
# installed as "matches the pin" would be the exact failure the check exists to prevent,
# one level up.
set -uo pipefail

if [ -n "${TEST_SRCDIR:-}" ]; then
  _root="$TEST_SRCDIR/$TEST_WORKSPACE"
else
  _root="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
fi
# shellcheck source=lib/version.sh
. "$_root/tools/lib/version.sh"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fails=0
fail() { echo "FAIL: $1"; fails=$((fails + 1)); }
check() { # check <description> <expected: yes|no> <command...>
  local desc="$1" expect="$2"; shift 2
  if "$@" >/dev/null 2>&1; then got=yes; else got=no; fi
  [ "$got" = "$expect" ] || fail "$desc (expected $expect, got $got)"
}

# --- probe_argv_safe: the manifest is data, and data must not reach a shell --------------
check "a plain probe is safe"            yes probe_argv_safe "container --version"
check "a path probe is safe"             yes probe_argv_safe "/usr/local/bin/btm --version"
check "flags with = are safe"            yes probe_argv_safe "tool --format=json"
check "empty is not a probe"             no  probe_argv_safe ""
check "command substitution rejected"    no  probe_argv_safe 'echo $(id)'
check "backtick rejected"                no  probe_argv_safe 'echo `id`'
check "semicolon chain rejected"         no  probe_argv_safe "btm --version; rm -rf /"
check "pipe rejected"                    no  probe_argv_safe "btm --version | sh"
check "ampersand rejected"               no  probe_argv_safe "btm --version && id"
check "redirect rejected"                no  probe_argv_safe "btm --version > /etc/passwd"
check "variable expansion rejected"      no  probe_argv_safe 'btm --version $HOME'
check "glob rejected"                    no  probe_argv_safe "btm --version *"
check "newline rejected"                 no  probe_argv_safe "$(printf 'btm\nid')"
check "quote rejected"                   no  probe_argv_safe "btm --version 'x'"
check "leading dash rejected"            no  probe_argv_safe "--version"
check "parent-dir escape rejected"       no  probe_argv_safe "../../bin/btm --version"

# --- probe_extract_version: every tool answers differently -------------------------------
eq() { # eq <description> <expected> <actual>
  [ "$2" = "$3" ] || fail "$1 (expected '$2', got '$3')"
}
eq "bottom's answer"      "0.14.7"   "$(probe_extract_version 'bottom 0.14.7')"
eq "a bare version"       "2026.7.0" "$(probe_extract_version '2026.7.0')"
eq "typst's parenthetes"  "0.15.1"   "$(probe_extract_version 'typst 0.15.1 (unknown commit)')"
eq "apple-container's"    "1.0.0"    "$(probe_extract_version 'container CLI version 1.0.0 (build: release, commit: ee848e3)')"
eq "a leading v"          "1.2.3"    "$(probe_extract_version 'tool v1.2.3')"
# A sha is not a version, and a tool that prints only one is unprobeable rather than matched.
eq "a bare sha yields nothing"  "" "$(probe_extract_version 'ee848e3')"
eq "empty yields nothing"       "" "$(probe_extract_version '')"
eq "prose with no version"      "" "$(probe_extract_version 'unknown option --version')"

# --- probe_agrees: the decorations on these two sides name neither the entry nor each other
check "bare vs cli-v decorated"   yes probe_agrees bitwarden-cli "cli-v2026.7.0" "2026.7.0"
check "v-prefixed pin"            yes probe_agrees gitleaks "v8.30.1" "8.30.1"
check "image variant suffix"      yes probe_agrees vaultwarden "1.37.0-alpine" "1.37.0"
check "identical"                 yes probe_agrees typst "0.15.1" "0.15.1"
check "a patch apart disagrees"   no  probe_agrees typst "0.15.0" "0.15.1"
check "a minor apart disagrees"   no  probe_agrees bottom "0.14.3" "0.14.7"
check "empty installed disagrees" no  probe_agrees bottom "0.14.3" ""
check "empty pin disagrees"       no  probe_agrees bottom "" "0.14.3"

# --- end to end, through the real checker, over planted manifests and binaries -----------
BIN="$WORK/bin"; mkdir -p "$BIN"
plant() { # plant <name> <stdout>
  printf '#!/bin/sh\necho "%s"\n' "$2" > "$BIN/$1"
  chmod +x "$BIN/$1"
}
plant agreeing   "planted 1.2.3"
plant disagreeing "planted 9.9.9"
plant mute       "no version here at all"

manifest() { # manifest <entry-body-lines...> -> path
  local path="$WORK/upstreams.toml"
  {
    # The checker derives its vocabulary from these header lines, so a fixture needs them.
    echo '# verdict = "adopt" | "contribute" | "overlay" | "fork" | "build" | "inspiration" | "quarry" | "reject"'
    echo '# pin_kind = "commit" | "image" | "monorepo" | "hosted" | "dataset"'
    echo
    cat
  } > "$path"
  printf '%s' "$path"
}

run_checker() { # run_checker <manifest path> -> combined output
  AXON_UPSTREAMS_MANIFEST="$1" PATH="$BIN:$PATH" \
    "$_root/tools/upstream-checker" --offline 2>&1
}

expect_note() { # expect_note <description> <manifest path> <substring>
  local out; out="$(run_checker "$2")"
  case "$out" in
    *"$3"*) ;;
    *) fail "$1 — no note matching '$3'. Got:
$out" ;;
  esac
}

MF="$(manifest <<'EOF'
[demo]
url = "https://github.com/example/demo"
verdict = "adopt"
license = "MIT"
pin = "1.2.3"
installed_probe = "agreeing --version"
why = "fixture"
EOF
)"
expect_note "agreeing" "$MF" "✓ installed matches the pin (1.2.3"

MF="$(manifest <<'EOF'
[demo]
url = "https://github.com/example/demo"
verdict = "adopt"
license = "MIT"
pin = "1.2.3"
installed_probe = "disagreeing --version"
why = "fixture"
EOF
)"
expect_note "disagreeing" "$MF" "✗ pin says 1.2.3, machine has 9.9.9"
run_checker "$MF" >/dev/null 2>&1 && fail "a disagreement did not fail the checker"

MF="$(manifest <<'EOF'
[demo]
url = "https://github.com/example/demo"
verdict = "adopt"
license = "MIT"
pin = "1.2.3"
installed_probe = "definitely-not-installed --version"
why = "fixture"
EOF
)"
expect_note "absent" "$MF" "is not installed on this machine — absent, not agreeing"
# The load-bearing half: absent must never be reported as agreement.
case "$(run_checker "$MF")" in
  *"installed matches the pin"*) fail "an absent tool was reported as matching the pin" ;;
esac

MF="$(manifest <<'EOF'
[demo]
url = "https://github.com/example/demo"
verdict = "adopt"
license = "MIT"
pin = "1.2.3"
installed_probe = "mute --version"
why = "fixture"
EOF
)"
expect_note "unprobeable" "$MF" "unprobeable, so the pin is unverified"

MF="$(manifest <<'EOF'
[demo]
url = "https://github.com/example/demo"
verdict = "adopt"
license = "MIT"
pin = "1.2.3"
installed_probe = "agreeing --version; touch /tmp/axon-probe-injection"
why = "fixture"
EOF
)"
expect_note "an unsafe probe is refused" "$MF" "✗ installed_probe is not a safe argv"
[ -e /tmp/axon-probe-injection ] && fail "the injected command RAN — the probe reached a shell"

MF="$(manifest <<'EOF'
[demo]
url = "https://github.com/example/demo"
verdict = "adopt"
license = "MIT"
pin = ""
installed_probe = "agreeing --version"
why = "fixture"
EOF
)"
expect_note "probe without a pin" "$MF" "✗ installed_probe without pin"

# An entry with no probe is untouched — this check is opt-in, not a new obligation on 59 entries.
MF="$(manifest <<'EOF'
[demo]
url = "https://github.com/example/demo"
verdict = "adopt"
license = "MIT"
pin = "1.2.3"
why = "fixture"
EOF
)"
case "$(run_checker "$MF")" in
  *installed*) fail "an entry without installed_probe was probed anyway" ;;
esac

if [ "$fails" -gt 0 ]; then
  echo "installed-probe: $fails check(s) failed"
  exit 1
fi
echo "installed-probe: all checks passed"
