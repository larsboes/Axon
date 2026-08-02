#!/bin/bash
# check-bazel-package-labels.sh — every unit that owns its own BUILD.bazel must appear in
# root BUILD.bazel's _ARCHITECTURE_INPUTS as explicit cross-package labels, or the
# architecture freshness gate silently stops seeing it (Axon#30).
#
# Why this class is dangerous rather than merely untidy: `tools/generate-architecture.sh`
# discovers capabilities by globbing capabilities/*/README.md, and Bazel's glob() cannot
# cross a package boundary. The moment a capability grows its own BUILD.bazel it becomes a
# subpackage, the glob stops reaching it, and //:architecture_up_to_date_test regenerates
# ARCHITECTURE.md in a sandbox that cannot see that capability at all. Today every such
# capability happens to be `kind = "process"` and contributes an em dash to the Service
# column either way, so a missing label produces no diff and the gate passes green while
# describing a repo that is missing a unit. That is a false green, which is the worst kind
# of gate failure: it reports coverage it does not have.
#
# Why this is NOT a Bazel test, against README.md#argue-bazel-per-case's default bias toward Bazel: it cannot be
# one. To enumerate which units own a BUILD.bazel, the check must see those files — and
# they live in exactly the subpackages glob() cannot reach, which is the whole reason the
# hand-maintained label list exists. A Bazel target would therefore need the same
# hand-list it is meant to protect, making the gate circular and worthless. Run from
# tools/doctor instead, which already owns the manifest-vs-reality sweeps and reads the
# real checkout rather than a sandbox. (CI does not yet run doctor — Axon#41.)
#
# Usage: tools/check-bazel-package-labels.sh
# Exit 0 = every required label present, 1 = at least one missing.
set -e

if [ -n "${TEST_SRCDIR:-}" ]; then
  _lib="$TEST_SRCDIR/$TEST_WORKSPACE/tools/lib"
else
  _lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/lib" && pwd)"
fi
source "$_lib/paths.sh"

ROOT_BUILD="${AXON_ROOT_BUILD_OVERRIDE:-$AXON_ROOT/BUILD.bazel}"

if [ ! -f "$ROOT_BUILD" ]; then
  echo "FAIL: root BUILD.bazel not found at $ROOT_BUILD" >&2
  exit 1
fi

# Scope the search to the _ARCHITECTURE_INPUTS assignment, NOT the whole file.
#
# This distinction is the entire check. The first version of this gate grepped all of
# BUILD.bazel and reported green while six capabilities were invisible to the
# architecture gate — because their `//capabilities/<n>:service.toml` labels do exist in
# the file, just in //:service_toml_schema_test's data list instead. A label satisfying a
# different target proves nothing about this one, and accepting it made this gate commit
# the exact false-green it was written to prevent.
#
# The awk range runs from the assignment to the line closing the explicit list (`] + glob([`,
# which starts with `]`). The glob() half that follows is deliberately excluded: glob()
# cannot cross a package boundary, so a subpackage's file can only ever be covered by an
# explicit label above it.
extract_list() {
  awk -v pat="^$1 = \\\\[" '$0 ~ pat, /^\]/' "$ROOT_BUILD"
}

ARCH_INPUTS="$(extract_list _ARCHITECTURE_INPUTS)"
MANIFEST_INPUTS="$(extract_list _MANIFEST_INPUTS)"
for _pair in "_ARCHITECTURE_INPUTS:$ARCH_INPUTS" "_MANIFEST_INPUTS:$MANIFEST_INPUTS"; do
  if [ "${_pair#*:}" = "" ]; then
    echo "FAIL: could not locate the ${_pair%%:*} list in $ROOT_BUILD" >&2
    exit 1
  fi
done

fail=0
checked=0

# Each list is checked against what ITS consumers actually read, not against a uniform
# rule — over-requiring is its own kind of wrong, because a redundant label reads as a
# real requirement to whoever maintains the list next.
#
#   _ARCHITECTURE_INPUTS  README.md (the What column) + service.toml (Service, Port,
#                         Panel). Both genuinely read by tools/generate-architecture.sh.
#   _MANIFEST_INPUTS      service.toml only. Both consumers — check-service-tomls.sh and
#                         check-manifest-integrity.sh — parse nothing else; the README
#                         labels five of the seven capabilities happen to carry there are
#                         inert. Requiring them would manufacture work with no failure
#                         behind it.
#
# A lib needs README.md in the architecture list and nothing in the manifest list: a lib
# has no service.toml, because it is compiled into a binary rather than run (README.md#three-architectural-nouns).
check_in_list() {
  _list_name="$1"
  _list="$2"
  _dir="$3"
  _noun="$4"
  shift 4
  _name="$(basename "$_dir")"
  for _file in "$@"; do
    # Only require a label for a file that actually exists — a capability with no
    # service.toml on disk is scaffolding, and demanding a label for a nonexistent file
    # would make this gate fail on a state the rest of the tooling already tolerates.
    [ -f "$_dir/$_file" ] || continue
    _label="//$_noun/$_name:$_file"
    if ! printf '%s\n' "$_list" | grep -qF "\"$_label\""; then
      echo "FAIL [$_name]: $_dir/$_file is in a Bazel subpackage but '$_label' is absent from $_list_name — the gates reading that list cannot see it (Axon#30)" >&2
      fail=1
    fi
  done
}

for dir in "$AXON_ROOT"/capabilities/*/; do
  dir="${dir%/}"
  [ -f "$dir/BUILD.bazel" ] || continue
  checked=$((checked + 1))
  check_in_list _ARCHITECTURE_INPUTS "$ARCH_INPUTS" "$dir" capabilities README.md service.toml
  check_in_list _MANIFEST_INPUTS "$MANIFEST_INPUTS" "$dir" capabilities service.toml
done

for dir in "$AXON_ROOT"/libs/*/; do
  dir="${dir%/}"
  [ -f "$dir/BUILD.bazel" ] || continue
  checked=$((checked + 1))
  check_in_list _ARCHITECTURE_INPUTS "$ARCH_INPUTS" "$dir" libs README.md
done

# Packs are swept too even though none owns a BUILD.bazel today (verified 2026-07-30).
# The generator reads Packs/*/pack.toml through the same glob() that could not reach the
# six capability service.toml files, so a Pack growing a BUILD.bazel would silently drop
# its row from the Packs table by exactly the mechanism just fixed. Checking the empty
# case now costs one loop and means the class is closed rather than half-closed.
for dir in "$AXON_ROOT"/Packs/*/; do
  dir="${dir%/}"
  [ -f "$dir/BUILD.bazel" ] || continue
  checked=$((checked + 1))
  check_in_list _ARCHITECTURE_INPUTS "$ARCH_INPUTS" "$dir" Packs pack.toml README.md
done

if [ "$checked" -eq 0 ]; then
  echo "FAIL: no unit owning a BUILD.bazel found — expected at least one" >&2
  exit 1
fi

# Second mechanism, same question: can everyone else's build see what mine sees?
#
# A Bazel label makes a file visible across a package boundary; being tracked by git makes
# it visible on another machine at all. The generator globs capabilities/*/README.md and
# capabilities/*/service.toml off disk, so an UNTRACKED manifest feeds real data into
# ARCHITECTURE.md — and then nobody else can reproduce it. `bazel test` passes locally
# because it globs the same working tree; CI clones and gets a different answer. That is
# the local-green/CI-red shape, and it has bitten this repo before (see the 2026-07-16
# note about untracked capabilities appearing in a committed ARCHITECTURE.md).
#
# Skipped when git is unavailable (a Bazel sandbox has no .git) rather than failing: this
# check is about the commit, and doctor is where it runs.
if git -C "$AXON_ROOT" rev-parse --git-dir >/dev/null 2>&1; then
  for f in "$AXON_ROOT"/capabilities/*/README.md "$AXON_ROOT"/capabilities/*/service.toml \
           "$AXON_ROOT"/Packs/*/pack.toml "$AXON_ROOT"/libs/*/README.md; do
    [ -f "$f" ] || continue
    _rel="${f#"$AXON_ROOT"/}"
    if [ -n "$(git -C "$AXON_ROOT" ls-files --others --exclude-standard -- "$_rel")" ]; then
      echo "FAIL: $_rel is untracked but the architecture generator reads it — ARCHITECTURE.md would carry data nobody else can reproduce (local green, CI red)" >&2
      fail=1
    fi
  done
fi

if [ "$fail" -ne 0 ]; then
  echo "Bazel-package label check FAILED." >&2
  exit 1
fi

echo "Bazel-package label check passed ($checked units own a BUILD.bazel; every label their consumers read is present in _ARCHITECTURE_INPUTS and _MANIFEST_INPUTS, and no generator input is untracked)."
