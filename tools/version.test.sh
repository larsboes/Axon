#!/bin/bash
# Regression test for tools/lib/version.sh. The case that matters is the one that was
# live and wrong on 2026-07-28: arrow-rs published 58.4.0 after 59.1.0, and the checker
# called the older release "newer".
set -uo pipefail

# Under Bazel the script and its data land in separate spots of the runfiles tree, so the
# lib is addressed the way the other sh_tests here address theirs. Run directly it falls
# back to a path relative to this file, because a test worth having is one you can also
# run without the build system.
if [ -n "${TEST_SRCDIR:-}" ]; then
  _lib="$TEST_SRCDIR/$TEST_WORKSPACE/tools/lib"
else
  _lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/lib"
fi
# shellcheck source=lib/version.sh
. "$_lib/version.sh"

fails=0
check() { # check <description> <expected: yes|no> <command...>
  local desc="$1" expect="$2"; shift 2
  if "$@"; then got=yes; else got=no; fi
  if [ "$got" != "$expect" ]; then
    echo "FAIL: $desc (expected $expect, got $got)"
    fails=$((fails + 1))
  fi
}

# The live regression: a maintenance release published later than a higher minor.
check "59.1.0 > 58.4.0"                  yes ver_gt 59.1.0 58.4.0
check "58.4.0 is NOT > 59.1.0"           no  ver_gt 58.4.0 59.1.0

# Ordinary ordering, including the double-digit trap a lexical sort gets wrong.
check "1.10.0 > 1.9.0 (not lexical)"     yes ver_gt 1.10.0 1.9.0
check "0.71.10 > 0.71.9"                 yes ver_gt 0.71.10 0.71.9
check "equal is not greater"             no  ver_gt 1.2.3 1.2.3
check "patch bump"                       yes ver_gt 1.2.4 1.2.3
check "differing depth: 2.0 vs 2.0.1"    no  ver_gt 2.0 2.0.1

# What must NOT be ordered. Every one of these is a real pin in upstreams.toml, and
# ver_numeric is what stops the checker inventing a comparison for them.
check "plain version is orderable"       yes ver_numeric 59.1.0
check "single component is orderable"    yes ver_numeric 17
check "git sha is not"                   no  ver_numeric f7c4aef
check "cli-v2026.6.0 is not"             no  ver_numeric cli-v2026.6.0
check "ProjectName-v1.1.0 is not"          no  ver_numeric ProjectName-v1.1.0
check "trixie-20260713-slim is not"      no  ver_numeric trixie-20260713-slim
check "17.9-alpine is not"               no  ver_numeric 17.9-alpine
check "empty is not"                     no  ver_numeric ""

# norm_ver strips exactly one leading v, and nothing else.
check "v-prefix stripped"                yes [ "$(norm_ver v8.30.1)" = "8.30.1" ]
check "bare version untouched"           yes [ "$(norm_ver 8.30.1)" = "8.30.1" ]
check "v inside a tag untouched"         yes [ "$(norm_ver cli-v2026.6.0)" = "cli-v2026.6.0" ]

if [ "$fails" -gt 0 ]; then
  echo "version.sh: $fails check(s) failed"
  exit 1
fi
echo "version.sh: all checks passed"
