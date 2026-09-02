#!/bin/bash
# tools/host-patch.sh — take the patch, every day, on this machine.
#
# The job capabilities/host-patch runs (kind = "process", schedule = "24h"). Q74 deleted the
# adoption cooldown; this is what stands in its place. Nothing here decides WHETHER to upgrade —
# the package manager owns that, and one binary has one owner.
#
# Contract, in this order and no other:
#   1. every step is `command -v` guarded: a machine without rustup is not a failed patch run
#   2. every failure is recorded and non-fatal to the next: a job that stops on the first
#      broken formula patches nothing after it
#   3. tools/audit runs LAST, over a machine that was just patched, so its verdict describes
#      today rather than yesterday
#   4. a receipt is written whatever happened, because tools/doctor reads it — a scheduled job
#      that silently stopped firing is the failure mode a scheduled job actually has, and a
#      launchd StartInterval unit does not fire while the Mac sleeps
#
# NOT run here: `bun upgrade` and `uv self update`. Both are Homebrew formulae on this host
# (toolchain.toml [bun], [uv]) and `brew upgrade --formula` below already moves them. Two owners
# of one binary is a shape this deployment has already paid for — a ~/.local/bin yt-dlp shadowed
# brew's copy on PATH and returned HTTP 403 on every media URL while --dump-json kept working
# (PRD §13). Nothing here compares the host's bun against a workflow's either: both CI workflows
# install `latest` (README.md#patch-first), so there is no second number to agree with.
#
# NOT run here either: any image or container scan. grype lives in
# .github/workflows/security.yml, which installs it itself and runs whether or not this machine
# is awake. Nothing on this host needs grype for Axon to be patched.
#
# Exit 0 = patched and the audit was clean · 1 = the audit found something · 2 = a step failed.
# bash 3.2-safe.
set -u

_here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=tools/lib/paths.sh
. "$_here/lib/paths.sh"   # AXON_ROOT, AXON_PERSONAL_ROOT

RAN=""; SKIPPED=""; FAILED=""

step() {  # step <binary> <label> <argv...>
  bin="$1"; label="$2"; shift 2
  if ! command -v "$bin" >/dev/null 2>&1; then
    echo "· $label — $bin not installed, skipped"
    SKIPPED="$SKIPPED $label"
    return 0
  fi
  echo "▸ $label"
  if "$@"; then
    RAN="$RAN $label"
  else
    rc=$?
    echo "  ✗ $label failed (exit $rc) — continuing" >&2
    FAILED="$FAILED $label"
  fi
  return 0
}

echo "Axon host-patch · $(date -u +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u)"

step brew   "brew update"           brew update
step brew   "brew upgrade formula"  brew upgrade --formula
step brew   "brew upgrade cask"     brew upgrade --cask
step brew   "brew cleanup"          brew cleanup -s
step brew   "brew autoremove"       brew autoremove
step uv     "uv tool upgrade"       uv tool upgrade --all
step rustup "rustup update"         rustup update

echo
echo "▸ tools/audit"
"$AXON_ROOT/tools/audit"
AUDIT_RC=$?
case "$AUDIT_RC" in
  0) AUDIT="clean" ;;
  1) AUDIT="finding" ;;
  *) AUDIT="scanner-missing" ;;
esac

# The receipt. tools/doctor reads it and says how old it is, which is the only way a launchd
# job that stopped firing becomes visible without somebody going to look.
if [ -n "${AXON_PERSONAL_ROOT:-}" ] && mkdir -p "$AXON_PERSONAL_ROOT/data/host-patch" 2>/dev/null; then
  printf '{"at":"%s","ran":"%s","skipped":"%s","failed":"%s","audit":"%s"}\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$(echo $RAN)" "$(echo $SKIPPED)" \
    "$(echo $FAILED)" "$AUDIT" \
    > "$AXON_PERSONAL_ROOT/data/host-patch/last.json"
else
  echo "host-patch: no overlay configured — no receipt written, so tools/doctor cannot report this run" >&2
fi

echo
echo "── host-patch: ran$RAN ──"
[ -n "$SKIPPED" ] && echo "── skipped (not installed):$SKIPPED ──"
[ -n "$FAILED" ]  && echo "── FAILED:$FAILED ──"
echo "── audit: $AUDIT ──"

[ -n "$FAILED" ] && exit 2
[ "$AUDIT_RC" -ne 0 ] && exit 1
exit 0
