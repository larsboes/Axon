#!/usr/bin/env bash
# Back up every capability that declares a backup contract.
#
# Exists because the schedule that ran backups was a hand-written LaunchAgent naming ONE
# capability — `backup.sh store` — while three declare a contract. tools/doctor called that unit an
# orphan (no manifest owned it), and it was right twice over: nothing versioned it, and nothing
# would have noticed when a fourth capability declared a contract and was never backed up.
#
# The set is DERIVED, never typed. A capability is in scope because its manifest declares
# `backup_target`, which is the same field tools/backup.sh already refuses to run without and the
# same one axon-status reads to decide a row belongs in its registry. One definition, three
# readers.
#
# Runs every contract even when one fails, and exits non-zero if any did. Stopping at the first
# failure would let one broken capability silently cancel the backups of the others, which is the
# shape of outage that ends with two weeks of nothing.
set -uo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$TOOLS_DIR/lib/paths.sh"

# `scope` filters out a capability this machine only consumes: its data lives on the deployment
# that provides it, and so does the authority to back it up (tools/backup.sh says so itself and
# exits 1). Asking anyway would turn a correct refusal into a failed run every night.
# `while read`, not `mapfile`: macOS ships bash 3.2 and this repo stays 3.2-safe, which
# tools/backup.sh states at its own head. mapfile is a bash 4 builtin and fails with
# "command not found" — quietly enough that the loop below would simply have run zero times.
CAPS=()
while IFS= read -r line; do
  [ -n "$line" ] && CAPS+=("$line")
done < <(
  "$TOOLS_DIR/capability.sh" registry 2>/dev/null \
    | "${AXON_BUN:-bun}" -e '
        const rows = JSON.parse(require("fs").readFileSync(0, "utf8"));
        for (const r of rows) {
          if (r.scope === "external") continue;
          if (!r.backup_target) continue;
          console.log(r.name);
        }
      '
)

if [ "${#CAPS[@]}" -eq 0 ]; then
  echo "backup-all.sh: no capability declares a backup contract on this machine — nothing to do."
  exit 0
fi

echo "backup-all.sh: ${#CAPS[@]} contract(s): ${CAPS[*]}"
failed=()
for cap in "${CAPS[@]}"; do
  echo "── $cap"
  if ! "$TOOLS_DIR/backup.sh" "$cap"; then
    failed+=("$cap")
  fi
done

if [ "${#failed[@]}" -gt 0 ]; then
  echo "backup-all.sh: FAILED for ${failed[*]}" >&2
  exit 1
fi
echo "backup-all.sh: every contract backed up."
