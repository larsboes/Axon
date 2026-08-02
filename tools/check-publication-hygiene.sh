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
    | grep -E "${mac_home}[A-Za-z0-9._-]+/|${linux_home}[A-Za-z0-9._-]+/" \
    | grep -Ev "${linux_home}(agent|runner)/" || true)"
  if [ -n "$home_hits" ]; then
    echo "publication hygiene: tracked blob contains a workstation home path: $path" >&2
    failed=1
  fi
done < <(git grep --cached -a -l -E "${mac_home}[A-Za-z0-9._-]+/|${linux_home}[A-Za-z0-9._-]+/" || true)

legacy_tooling_path='~/Developer/'"Tooling"
instance_markers="axon-personal|axon-family|axon-work|lifeos-mono|obsidian-mono|DS220|Open Telekom Cloud|${legacy_tooling_path}"
while IFS= read -r path; do
  [ -n "$path" ] || continue
  case "$path" in
    tools/check-publication-hygiene.sh|tools/check-publication-hygiene.test.sh) continue ;;
  esac
  echo "publication hygiene: tracked blob contains a deployment-instance marker: $path" >&2
  failed=1
done < <(git grep --cached -a -l -E "$instance_markers" || true)

if [ "$failed" -ne 0 ]; then
  exit 1
fi

echo "publication hygiene passed (tracked artifacts, workstation paths, and deployment markers)"
