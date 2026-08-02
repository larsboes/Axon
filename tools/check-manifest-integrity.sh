#!/bin/bash
# check-manifest-integrity.sh — sh_test body for //:manifest_integrity_test.
# Referential integrity over the capability manifests, so a dangling name can never ship:
#   every entry in any capabilities/*/service.toml `requires = [...]` maps to a real
#   capabilities/<name>/ dir.
# Pure file-based (dir-existence) check: operates only on files/dirs materialized from the
# sh_test's `data`, so it runs identically from the repo root or inside `bazel test`'s
# runfiles sandbox where git and the wider checkout are absent.
#
# What this gate deliberately no longer checks, and where those checks went:
# until 2026-07-26 it also validated the ENABLED capability set — that each enabled name
# had a directory, and that the set was dependency-closed. Both read `capabilities = [...]`
# from the tracked axon.toml. That field now lives in <overlay>/config/machine.toml, which
# is per-machine and outside the hermetic sandbox by construction, so those two checks moved
# to tools/doctor, which runs on a real machine and can see its overlay. The split is the
# honest one: a hermetic test checks what is true of the repo, doctor checks what is true of
# the machine. See schemas/machine.toml.example.
set -e

# Runfiles-relocation: identical to check-service-tomls.sh / check-architecture-fresh.sh —
# resolve the lib dir from TEST_SRCDIR/TEST_WORKSPACE under `bazel test`, else self-locate
# for a direct repo-root run. paths.sh sources toml.sh and exports AXON_ROOT.
if [ -n "${TEST_SRCDIR:-}" ]; then
  _lib="$TEST_SRCDIR/$TEST_WORKSPACE/tools/lib"
else
  _lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/lib" && pwd)"
fi
source "$_lib/paths.sh"

fail=0

# Every service.toml `requires = [...]` entry → each must be a real capabilities/<name>/ dir.
# Word-splitting on toml_array's newline-per-element output is safe: capability names carry
# no whitespace (bash 3.2-safe, no mapfile).
for svc in "$AXON_ROOT"/capabilities/*/service.toml "$AXON_ROOT"/*/service.toml; do
  [ -f "$svc" ] || continue          # empty glob → literal path, skip it
  owner="$(basename "$(dirname "$svc")")"
  for dep in $(toml_array requires "$svc"); do
    if [ ! -d "$AXON_ROOT/capabilities/$dep" ]; then
      echo "FAIL [$owner]: requires '$dep' but capabilities/$dep/ does not exist" >&2
      fail=1
    fi
  done
done

if [ "$fail" -ne 0 ]; then
  echo "manifest integrity check FAILED." >&2
  exit 1
fi

echo "manifest integrity check passed (every service.toml requires= resolves to a real capability)."
echo "Enabled-set checks (dirs exist, set is dependency-closed) run in tools/doctor — machine-level, not hermetic."
