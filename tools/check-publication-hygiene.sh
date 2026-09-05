#!/bin/bash
# Current-tree publication floor. This deliberately scans the Git index rather than
# the working tree: ignored files may exist locally, but they cannot ride a commit into
# the public repository. It also rejects retired named-overlay and real-device markers.
set -euo pipefail

ROOT="${AXON_PUBLICATION_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

failed=0
mac_home="/""Users/"
linux_home="/""home/"
# An absolute path starts a path. A repo-relative one does not.
#
# `$lib/home/kinds/mail.ts` matched `/home/<name>/` and was reported as a workstation home
# directory: the dashboard's Home ladder lives under `src/lib/home/`, and giving it
# subdirectories was enough to trip this. Requiring the slash to begin a path — at the
# start of a line or after whitespace, a quote, a parenthesis, `=` or `:` — keeps every
# real hit (`/home/lars/...`, `/Users/lars/...` all appear that way) and drops the class of
# false positive that a directory called `home` or `Users` produces anywhere in the tree.
path_start="(^|[^A-Za-z0-9._~-])"

while IFS= read -r -d '' path; do
  case "$path" in
    */__pycache__/*|*.pyc|*.pyo)
      echo "publication hygiene: tracked interpreter artifact: $path" >&2
      failed=1
      ;;
  esac
done < <(git ls-files -z)

# Ask Git for the small candidate set first; opening every indexed blob separately made
# Doctor pay one process launch per file. Inspect the indexed blob for each candidate so
# the verdict still describes exactly what Git would publish. `strings` includes binary
# metadata. Container and CI homes are portable public examples.
while IFS= read -r path; do
  [ -n "$path" ] || continue
  home_hits="$({ git show ":$path" 2>/dev/null || true; } \
    | LC_ALL=C strings \
    | grep -E "${path_start}(${mac_home}|${linux_home})[A-Za-z0-9._-]+/" \
    | grep -Ev "${linux_home}(agent|runner)/" || true)"
  if [ -n "$home_hits" ]; then
    echo "publication hygiene: tracked blob contains a workstation home path: $path" >&2
    failed=1
  fi
done < <(git grep --cached -a -l -E "${path_start}(${mac_home}|${linux_home})[A-Za-z0-9._-]+/" || true)

legacy_tooling_path='~/Developer/'"Tooling"
# Each marker must be followed by a non-identifier character or end of line, so a name that
# merely BEGINS with one does not match. capabilities/finance/src/allocation.rs names a journal
# tag `axon-personal-cents`, which is a field name in a ledger format and exposes nothing — and
# it turned this gate red on every push to main from the commit that introduced it. A marker
# exists to catch a deployment being named, not a string starting with the same letters.
instance_markers="(axon-personal|axon-family|axon-work|lifeos-mono|obsidian-mono|DS220|Open Telekom Cloud|${legacy_tooling_path})([^-A-Za-z0-9]|$)"
while IFS= read -r path; do
  [ -n "$path" ] || continue
  case "$path" in
    # A file whose job is to detect markers has to contain them. That is this script, its
    # test, and since #168 the sibling gate that scans built site bytes for the same list.
    # Nothing else earns an entry here: every other file assembles the string at run time.
    tools/check-publication-hygiene.sh|tools/check-publication-hygiene.test.sh) continue ;;
    tools/check-site-payload.sh) continue ;;
  esac
  echo "publication hygiene: tracked blob contains a deployment-instance marker: $path" >&2
  failed=1
done < <(git grep --cached -a -l -E "$instance_markers" || true)

if [ "$failed" -ne 0 ]; then
  exit 1
fi

echo "publication hygiene passed (tracked artifacts, workstation paths, and deployment markers)"
