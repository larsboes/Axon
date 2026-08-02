#!/bin/bash
# Proves that image exceptions are selected by capability instead of leaking into every scan.
# Trivy owns expiry parsing; this test owns Axon's routing contract. Bash 3.2-safe.
set -u

if [ -n "${TEST_SRCDIR:-}" ]; then
  ROOT="$TEST_SRCDIR/${TEST_WORKSPACE:-_main}"
else
  ROOT="$(cd "$(dirname "$0")/.." && pwd)"
fi
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
MOCK_BIN="$SCRATCH/bin"
LOG="$SCRATCH/trivy.log"
mkdir -p "$MOCK_BIN"

cat > "$MOCK_BIN/trivy" <<'MOCK'
#!/bin/sh
printf '%s\n' "$*" >> "$AXON_AUDIT_TEST_LOG"
exit 0
MOCK
chmod +x "$MOCK_BIN/trivy"

if ! AXON_AUDIT_TEST_LOG="$LOG" PATH="$MOCK_BIN:$PATH" \
  "$ROOT/tools/audit" --skip gitleaks --skip semgrep --skip osv >"$SCRATCH/out" 2>&1; then
  cat "$SCRATCH/out"
  echo "FAIL: audit rejected clean mocked image scans" >&2
  exit 1
fi

expect_policy() {
  cap="$1"
  ref="$2"
  grep -F -- "--ignorefile $ROOT/trivy-ignore/$cap.txt $ref" "$LOG" >/dev/null || {
    echo "FAIL: $cap did not receive its image-scoped policy" >&2
    exit 1
  }
}

expect_policy home-assistant ghcr.io/home-assistant/home-assistant:2026.7.4
expect_policy pihole pihole/pihole:2026.07.2
expect_policy postgres postgres:17.10-alpine

if grep -F -- '--ignorefile' "$LOG" | grep -F -- 'vaultwarden/server:1.37.0-alpine' >/dev/null; then
  echo "FAIL: clean Vaultwarden scan inherited another capability policy" >&2
  exit 1
fi

grep -F -- 'vaultwarden/server:1.37.0-alpine' "$LOG" >/dev/null || {
  echo "FAIL: Vaultwarden was not scanned" >&2
  exit 1
}

# Repository detection must use Git plumbing: linked worktrees have a .git file,
# not a directory. Run the real audit script against isolated repositories with
# every other scanner skipped and a deterministic gitleaks mock.
PRIMARY="$SCRATCH/primary"
LINKED="$SCRATCH/linked"
NONREPO="$SCRATCH/not-a-repo"
MISSING="$SCRATCH/not-reachable"
AUDIT_FIXTURE="$SCRATCH/audit-fixture"
GITLEAKS_LOG="$SCRATCH/gitleaks.log"
mkdir -p "$NONREPO" "$AUDIT_FIXTURE/tools/lib"

git init -q "$PRIMARY"
git -C "$PRIMARY" -c user.name=Axon -c user.email=axon@example.invalid \
  commit -q --allow-empty -m initial
git -C "$PRIMARY" worktree add -q -b audit-linked "$LINKED"

cp "$ROOT/tools/audit" "$AUDIT_FIXTURE/tools/audit"
cat > "$AUDIT_FIXTURE/tools/lib/paths.sh" <<'PATHS'
AXON_ROOT="$AXON_AUDIT_TEST_ROOT"
AXON_PERSONAL_ROOT="${AXON_AUDIT_TEST_OVERLAY:-}"
export AXON_ROOT AXON_PERSONAL_ROOT
toml_get() { return 0; }
PATHS

cat > "$MOCK_BIN/gitleaks" <<'MOCK'
#!/bin/sh
printf '%s\n' "$*" >> "$AXON_AUDIT_GITLEAKS_LOG"
exit "${AXON_AUDIT_GITLEAKS_RC:-0}"
MOCK
chmod +x "$MOCK_BIN/gitleaks" "$AUDIT_FIXTURE/tools/audit"

run_gitleaks_audit() {
  AXON_AUDIT_GITLEAKS_LOG="$GITLEAKS_LOG" \
  AXON_AUDIT_TEST_ROOT="$PRIMARY" \
  AXON_AUDIT_TEST_OVERLAY="$1" \
  AXON_AUDIT_GITLEAKS_RC="${2:-0}" \
  PATH="$MOCK_BIN:$PATH" \
    "$AUDIT_FIXTURE/tools/audit" --skip semgrep --skip osv --skip trivy
}

: > "$GITLEAKS_LOG"
run_gitleaks_audit "$LINKED" >"$SCRATCH/worktree.out" 2>&1 || {
  cat "$SCRATCH/worktree.out"
  echo "FAIL: linked worktree audit did not complete cleanly" >&2
  exit 1
}
grep -F -- "-s $PRIMARY " "$GITLEAKS_LOG" >/dev/null || {
  echo "FAIL: primary repository was not scanned" >&2
  exit 1
}
grep -F -- "-s $LINKED " "$GITLEAKS_LOG" >/dev/null || {
  echo "FAIL: linked worktree was not scanned" >&2
  exit 1
}
if grep -F "$LINKED" "$SCRATCH/worktree.out" >/dev/null; then
  echo "FAIL: audit output exposed the private overlay coordinate" >&2
  exit 1
fi
grep -F 'private overlay — clean' "$SCRATCH/worktree.out" >/dev/null || {
  echo "FAIL: linked worktree clean status was not explicit" >&2
  exit 1
}

: > "$GITLEAKS_LOG"
if run_gitleaks_audit "$NONREPO" >"$SCRATCH/nonrepo.out" 2>&1; then
  echo "FAIL: non-repository overlay produced a clean audit" >&2
  exit 1
fi
grep -F 'private overlay — not a Git repository' "$SCRATCH/nonrepo.out" >/dev/null || {
  echo "FAIL: non-repository status was not explicit" >&2
  exit 1
}
if grep -F "$NONREPO" "$GITLEAKS_LOG" >/dev/null; then
  echo "FAIL: gitleaks was invoked for a non-repository" >&2
  exit 1
fi

if run_gitleaks_audit "$MISSING" >"$SCRATCH/missing.out" 2>&1; then
  echo "FAIL: unreachable overlay produced a clean audit" >&2
  exit 1
fi
grep -F 'private overlay — not reachable' "$SCRATCH/missing.out" >/dev/null || {
  echo "FAIL: unreachable status was not explicit" >&2
  exit 1
}

run_gitleaks_audit "" >"$SCRATCH/unconfigured.out" 2>&1 || {
  echo "FAIL: an unconfigured optional overlay failed the Axon audit" >&2
  exit 1
}
grep -F 'private overlay — not configured' "$SCRATCH/unconfigured.out" >/dev/null || {
  echo "FAIL: unconfigured status was not explicit" >&2
  exit 1
}

if run_gitleaks_audit "" 1 >"$SCRATCH/leak.out" 2>&1; then
  echo "FAIL: mocked leak produced a clean audit" >&2
  exit 1
fi
grep -F 'Axon — leak(s) found' "$SCRATCH/leak.out" >/dev/null || {
  echo "FAIL: leak status was not explicit" >&2
  exit 1
}

if run_gitleaks_audit "" 2 >"$SCRATCH/error.out" 2>&1; then
  echo "FAIL: mocked scanner error produced a clean audit" >&2
  exit 1
fi
grep -F 'Axon — gitleaks errored (exit 2' "$SCRATCH/error.out" >/dev/null || {
  echo "FAIL: scanner-error status was not explicit" >&2
  exit 1
}

echo "audit policy and repository detection tests: pass"
