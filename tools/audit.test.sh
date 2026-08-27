#!/bin/bash
# Proves that image exceptions are selected by capability instead of leaking into every scan.
# Trivy owns expiry parsing; this test owns Axon's routing contract. Bash 3.2-safe.
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
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

AXON_AUDIT_TEST_LOG="$LOG" PATH="$MOCK_BIN:$PATH" \
  "$ROOT/tools/audit" --skip gitleaks --skip semgrep --skip osv >"$SCRATCH/out" 2>&1 || true

# Deliberately NOT asserting exit 0 here. This block owns policy routing, and it runs
# against the tracked trivy-ignore files, whose dates are real -- so once one of them
# lapses this run legitimately exits non-zero for a reason that has nothing to do with
# routing. A routing test that starts failing on a calendar date is the same rot the expiry
# clock exists to prevent. What must hold is narrower and durable: no MOCKED scan produced
# a finding. Exit codes are proved below, over planted policies whose dates are generated
# per run.
if grep -F 'CRITICAL/HIGH finding(s)' "$SCRATCH/out" >/dev/null; then
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

# A capability whose policy file was deleted must still be scanned — bare, with no
# ignorefile. home-assistant and pihole moved to this state when their expired
# policies were removed (da201f9, expiry 2026-08-15 under Axon#184).
expect_bare_scan() {
  cap="$1"
  ref="$2"
  grep -F -- "$ref" "$LOG" >/dev/null || {
    echo "FAIL: $cap was not scanned" >&2
    exit 1
  }
  if grep -F -- '--ignorefile' "$LOG" | grep -F -- "$ref" >/dev/null; then
    echo "FAIL: $cap was scanned with a policy that no longer exists" >&2
    exit 1
  fi
}

# Every tracked policy, discovered rather than named. postgres was the one this line
# asserted until PRD Q45 retired the capability on 2026-08-27, taking its 15 gosu findings
# with it; a hand-written name would have made that deletion look like a broken test
# instead of a policy that no longer has an image. The loop reads the same two manifest
# fields tools/audit does, so a policy added tomorrow is routed-checked without an edit —
# and a repository with no policy at all says so rather than passing over nothing.
policies=0
for policy in "$ROOT"/trivy-ignore/*.txt; do
  [ -f "$policy" ] || continue
  cap="$(basename "$policy" .txt)"
  manifest="$ROOT/capabilities/$cap/service.toml"
  if [ ! -f "$manifest" ]; then
    echo "FAIL: trivy-ignore/$cap.txt has no capabilities/$cap/service.toml to apply to" >&2
    exit 1
  fi
  image="$(sed -n 's/^image *= *"\([^"]*\)".*/\1/p' "$manifest" | head -1)"
  tag="$(sed -n 's/^tag *= *"\([^"]*\)".*/\1/p' "$manifest" | head -1)"
  expect_policy "$cap" "$image:$tag"
  policies=$((policies + 1))
done
[ "$policies" -gt 0 ] || echo "  ⊘ no trivy-ignore/*.txt tracked — policy routing is unexercised this run"

expect_bare_scan home-assistant ghcr.io/home-assistant/home-assistant:2026.7.4
expect_bare_scan pihole pihole/pihole:2026.07.2
expect_bare_scan vaultwarden vaultwarden/server:1.37.0-alpine

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
# tools/audit sources lib/expiry.sh relative to itself, so the fixture root needs it even
# on the path that skips trivy and osv entirely.
cp "$ROOT/tools/lib/expiry.sh" "$AUDIT_FIXTURE/tools/lib/expiry.sh"
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

# --- the expiry clock -----------------------------------------------------
#
# Trivy and osv-scanner enforce their own dates; what is tested here is that Axon SAYS a
# date is coming, and refuses to treat a lapsed policy as a live one. That needs planted
# policies: the tracked files can only ever show the states they happen to be in today,
# and the three interesting ones (near, past, undated) are none of them.
#
# Every fixture date is generated relative to `date` at run time. A literal would pass
# today and start failing on its own expiry, which is the bug under test wearing a
# different hat.

day_offset() {  # day_offset <+N|-N> — an ISO date N days from today, UTC. GNU then BSD.
  if date --version >/dev/null 2>&1; then
    date -u -d "$1 days" +%Y-%m-%d
  else
    date -u -v"$1"d +%Y-%m-%d
  fi
}

EXPIRY_ROOT="$SCRATCH/expiry-root"
EXPIRY_LOG="$SCRATCH/expiry-trivy.log"
mkdir -p "$EXPIRY_ROOT/tools/lib" "$EXPIRY_ROOT/trivy-ignore" "$EXPIRY_ROOT/capabilities/demo"
cp "$ROOT/tools/audit" "$EXPIRY_ROOT/tools/audit"
cp "$ROOT/tools/lib/toml.sh" "$EXPIRY_ROOT/tools/lib/toml.sh"
cp "$ROOT/tools/lib/expiry.sh" "$EXPIRY_ROOT/tools/lib/expiry.sh"
chmod +x "$EXPIRY_ROOT/tools/audit"

# Unlike the gitleaks fixture above, this stub sources the real toml.sh: the expiry clock
# reads its window through toml_get_in, and stubbing that away would test a script that
# cannot read its own configuration.
cat > "$EXPIRY_ROOT/tools/lib/paths.sh" <<'PATHS'
AXON_ROOT="$AXON_AUDIT_TEST_ROOT"
AXON_PERSONAL_ROOT=""
export AXON_ROOT AXON_PERSONAL_ROOT
. "$AXON_ROOT/tools/lib/toml.sh"
PATHS

cat > "$EXPIRY_ROOT/capabilities/demo/service.toml" <<'SVC'
image        = "example.invalid/demo"
tag          = "1.0.0"
SVC

write_window() {  # write_window <days> — the fixture root's [audit] expiry window
  cat > "$EXPIRY_ROOT/axon.toml" <<TOML
[audit]
expiry_warn_days = "$1"
TOML
}

write_policy() {  # write_policy <offset|none>… — one trivy entry per argument
  printf '# planted policy\n' > "$EXPIRY_ROOT/trivy-ignore/demo.txt"
  _n=0
  for _off in "$@"; do
    _n=$((_n + 1))
    if [ "$_off" = "none" ]; then
      printf 'CVE-2026-9000%s\n' "$_n" >> "$EXPIRY_ROOT/trivy-ignore/demo.txt"
    else
      printf 'CVE-2026-9000%s exp:%s\n' "$_n" "$(day_offset "$_off")" >> "$EXPIRY_ROOT/trivy-ignore/demo.txt"
    fi
  done
}

run_expiry() {  # everything but trivy skipped; the mock keeps the image scan clean
  AXON_AUDIT_TEST_ROOT="$EXPIRY_ROOT" \
  AXON_AUDIT_TEST_LOG="$EXPIRY_LOG" \
  PATH="$MOCK_BIN:$PATH" \
    "$EXPIRY_ROOT/tools/audit" --skip gitleaks --skip semgrep --skip osv
}

expiry_case() {  # expiry_case <label> <expected rc> <substring> — run and assert
  _label="$1"; _want_rc="$2"; _needle="$3"
  run_expiry >"$SCRATCH/expiry.out" 2>&1
  _rc=$?
  if [ "$_rc" -ne "$_want_rc" ]; then
    cat "$SCRATCH/expiry.out"
    echo "FAIL: $_label — expected exit $_want_rc, got $_rc" >&2
    exit 1
  fi
  grep -F -- "$_needle" "$SCRATCH/expiry.out" >/dev/null || {
    cat "$SCRATCH/expiry.out"
    echo "FAIL: $_label — output did not contain '$_needle'" >&2
    exit 1
  }
}

write_window 14

# Comfortably ahead: reported with its date, no warning, and the audit stays green.
write_policy +90
expiry_case "distant expiry" 0 "expires $(day_offset +90) (90d left)"
if grep -F 'inside the' "$SCRATCH/expiry.out" >/dev/null; then
  echo "FAIL: a policy 90 days out was reported as needing a re-decision" >&2
  exit 1
fi

# Inside the window: warns, and still exits 0 — near is notice, not failure. A gate that
# fails on "soon" is one that gets skipped rather than heeded.
write_policy +3
expiry_case "near expiry warns" 0 "inside the 14d re-decision window"
grep -F 'exception policies needing a re-decision inside 14d' "$SCRATCH/expiry.out" >/dev/null || {
  echo "FAIL: the near-expiry summary line was missing" >&2
  exit 1
}

# Past its date: fails by name, even though the scanner itself returned clean. This is the
# whole point — without it the only signal is trivy going red for reasons nobody connects
# back to a policy that quietly stopped applying.
write_policy -2
expiry_case "lapsed policy fails" 1 "EXPIRED $(day_offset -2) (2d ago"
grep -F 'exception policies past their date: trivy-ignore/demo.txt' "$SCRATCH/expiry.out" >/dev/null || {
  echo "FAIL: the lapsed summary line did not name the policy" >&2
  exit 1
}

# One lapsed entry among healthy ones still fails: the nearest date is what governs, not
# the average or the majority.
write_policy +90 -2 +45
expiry_case "one lapsed entry among many fails" 1 "1 of 3 dated entries"

# An entry with no date at all never expires, so nothing will ever re-decide it. Reported
# as needing attention rather than passing silently.
write_policy +90 none
expiry_case "undated entry is surfaced" 0 "1 undated and therefore permanent"

# The window is configuration, not a literal in the script: the same policy changes verdict
# when only axon.toml moves.
write_policy +20
expiry_case "20 days out is quiet at a 14d window" 0 "(20d left)"
if grep -F 'inside the' "$SCRATCH/expiry.out" >/dev/null; then
  echo "FAIL: 20 days out warned at a 14-day window" >&2
  exit 1
fi
write_window 30
expiry_case "the same policy warns at a 30d window" 0 "inside the 30d re-decision window"
write_window 14

if grep -qE '(^|[^0-9])14([^0-9]|$)' "$EXPIRY_ROOT/tools/audit"; then
  echo "FAIL: tools/audit contains a literal 14 — the window must come from axon.toml" >&2
  exit 1
fi

# The advisory exceptions in osv-scanner.toml get the same clock. Its entries are TOML
# blocks with a bare `ignoreUntil` date rather than trailing `exp:`, so this is a second
# reader over the same reporting path, and it needs its own proof.
run_osv_expiry() {
  AXON_AUDIT_TEST_ROOT="$EXPIRY_ROOT" \
  PATH="$MOCK_BIN:$PATH" \
    "$EXPIRY_ROOT/tools/audit" --skip gitleaks --skip semgrep --skip trivy
}
cat > "$MOCK_BIN/osv-scanner" <<'MOCK'
#!/bin/sh
exit 0
MOCK
chmod +x "$MOCK_BIN/osv-scanner"

cat > "$EXPIRY_ROOT/osv-scanner.toml" <<TOML
[[IgnoredVulns]]
id = "RUSTSEC-0000-0001"
ignoreUntil = $(day_offset +60)
TOML
if ! run_osv_expiry >"$SCRATCH/osv.out" 2>&1; then
  cat "$SCRATCH/osv.out"
  echo "FAIL: a distant osv exception failed the audit" >&2
  exit 1
fi
grep -F "policy: osv-scanner.toml — 1 ID(s), expires $(day_offset +60) (60d left)" "$SCRATCH/osv.out" >/dev/null || {
  cat "$SCRATCH/osv.out"
  echo "FAIL: the osv exception was not reported with its date" >&2
  exit 1
}

cat > "$EXPIRY_ROOT/osv-scanner.toml" <<TOML
[[IgnoredVulns]]
id = "RUSTSEC-0000-0001"
ignoreUntil = $(day_offset -5)
TOML
if run_osv_expiry >"$SCRATCH/osv-expired.out" 2>&1; then
  cat "$SCRATCH/osv-expired.out"
  echo "FAIL: a lapsed osv exception produced a clean audit" >&2
  exit 1
fi
grep -F 'exception policies past their date: osv-scanner.toml' "$SCRATCH/osv-expired.out" >/dev/null || {
  cat "$SCRATCH/osv-expired.out"
  echo "FAIL: the lapsed osv policy was not named" >&2
  exit 1
}

echo "audit policy, repository detection and expiry-clock tests: pass"
