# tools/lib/test-support.sh — shared helpers for Axon's shell tests.
#
# Exists for one rule so far, and that rule earned its own file: a test may skip a
# platform-dependent assertion on a developer machine, and may never skip it in CI.
#
# The case that prompted it: tools/backup.test.sh forces its short-write path with /dev/full,
# which Linux has and macOS does not. On macOS the test printed a note and passed, and the note
# claimed "it runs in CI" — an assumption nobody could see failing, because Bazel runs passing
# tests with --test_output=errors and prints nothing at all. A platform-dependent assertion is
# not unusual, so the guard belongs to the convention rather than to that one test.
#
# bash 3.2-safe (README.md#portable-shell).

# in_ci — true when this is an automated run. GitHub Actions sets CI=true; so does essentially
# every other runner, which is the point: the guard should not know which one it is under.
in_ci() { [ "${CI:-}" = "true" ] || [ "${CI:-}" = "1" ]; }

# skippable <reason> — decide whether a platform-dependent assertion may be skipped here.
#
#   if skippable "no /dev/full on this host"; then
#     echo "NOTE: short-write assertion skipped — no /dev/full on this host"
#   else
#     ... run the assertion ...
#   fi
#
# Returns 0 (skip is allowed) outside CI, after printing what is being given up so the reduced
# coverage is visible rather than silent. Inside CI it does not return: a skip there means the
# assertion has quietly stopped running everywhere, which is the failure this prevents.
skippable() {
  local reason="${1:?skippable needs a reason}"
  if in_ci; then
    echo "FAIL: assertion skipped in CI ($reason) — a platform-dependent check that never runs" >&2
    echo "      anywhere is worse than one that fails. Provide the capability on the runner, or" >&2
    echo "      move the assertion to a host that has it." >&2
    exit 1
  fi
  echo "NOTE: reduced coverage on this host — $reason"
  return 0
}
