#!/bin/bash
# Synthetic backup ordering/retention matrix. No private overlay, live service, or SSH host.
set -uo pipefail

if [ -n "${TEST_SRCDIR:-}" ]; then
  ROOT="$TEST_SRCDIR/$TEST_WORKSPACE"
else
  ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
fi

SCRATCH="$(mktemp -d /tmp/axon-backup-test.XXXXXX)"
trap 'rm -rf "$SCRATCH"' EXIT
FIXTURE="$SCRATCH/fixture"
OVERLAY="$SCRATCH/overlay"
MOCK_BIN="$SCRATCH/bin"
MOCK_LOG="$SCRATCH/operations.log"
MOCK_RESUME_COUNT="$SCRATCH/resume-count"
mkdir -p "$FIXTURE/tools/lib" "$FIXTURE/capabilities/vaultwarden" \
  "$OVERLAY/config" "$OVERLAY/data/vaultwarden/data" "$OVERLAY/data/vaultwarden/tls" \
  "$MOCK_BIN" "$SCRATCH/remote/vaultwarden"

cp "$ROOT/tools/backup.sh" "$FIXTURE/tools/backup.sh"
cp "$ROOT/tools/lib/toml.sh" "$FIXTURE/tools/lib/toml.sh"

cat > "$FIXTURE/tools/lib/paths.sh" <<PATHS
#!/bin/bash
AXON_ROOT="$FIXTURE"
AXON_PERSONAL_ROOT="$OVERLAY"
AXON_MACHINE_TOML="$OVERLAY/config/machine.toml"
export AXON_ROOT AXON_PERSONAL_ROOT AXON_MACHINE_TOML
source "$FIXTURE/tools/lib/toml.sh"
PATHS
cat > "$FIXTURE/tools/lib/platform.sh" <<'PLATFORM'
#!/bin/bash
AXON_CONTAINER_RUNTIME="docker"
export AXON_CONTAINER_RUNTIME
PLATFORM
cat > "$FIXTURE/tools/lib/bw-agent.sh" <<'AGENT'
#!/bin/bash
:
AGENT

cat > "$FIXTURE/tools/service-runner.sh" <<'RUNNER'
#!/bin/bash
set -u
action="$1"
printf 'service:%s\n' "$action" >> "$MOCK_LOG"
case "$action" in
  stop)
    [ "${MOCK_FAIL_STOP:-0}" -eq 0 ] || exit 1
    ;;
  resume)
    count=0
    [ ! -f "$MOCK_RESUME_COUNT" ] || count="$(cat "$MOCK_RESUME_COUNT")"
    count=$((count + 1))
    printf '%s\n' "$count" > "$MOCK_RESUME_COUNT"
    [ "$count" -gt "${MOCK_RESUME_FAILS:-0}" ] || exit 1
    ;;
esac
RUNNER

cat > "$FIXTURE/capabilities/vaultwarden/service.toml" <<'MANIFEST'
name = "vaultwarden"
image = "vaultwarden/server"
tag = "1.37.0-alpine"
env_file = "config/vaultwarden.env"
backup_paths = ["data/vaultwarden/data", "data/vaultwarden/tls"]
backup_sqlite = "data/vaultwarden/data/db.sqlite3"
backup_target = "synthetic-target"
backup_retain = "2"
MANIFEST
cat > "$OVERLAY/config/machine.toml" <<'MACHINE'
os = "linux"
container_runtime = "docker"
capabilities = ["vaultwarden"]
MACHINE
cat > "$OVERLAY/config/systems.local.toml" <<SYSTEMS
[synthetic-target]
host = "synthetic.invalid"
ssh_user = "backup-test"
backup_root = "$SCRATCH/remote"
SYSTEMS
printf 'VALID DATABASE COPY\n' > "$OVERLAY/data/vaultwarden/data/db.sqlite3"
printf 'attachment\n' > "$OVERLAY/data/vaultwarden/data/attachment.bin"
printf 'certificate\n' > "$OVERLAY/data/vaultwarden/tls/cert.pem"

cat > "$MOCK_BIN/rsync" <<'RSYNC'
#!/bin/bash
printf 'rsync:%s\n' "$2" >> "$MOCK_LOG"
[ "${MOCK_SIGNAL_PARENT:-0}" = "0" ] || {
  kill -"$MOCK_SIGNAL_PARENT" "$PPID"
  sleep 1
  exit 143
}
[ "${MOCK_FAIL_RSYNC:-0}" -eq 0 ] || exit 23
src="${2%/}"; dest="${3%/}"
mkdir -p "$dest"
/bin/cp -R "$src/." "$dest/"
RSYNC
cat > "$MOCK_BIN/sqlite3" <<'SQLITE'
#!/bin/bash
printf 'sqlite:integrity\n' >> "$MOCK_LOG"
if [ "${MOCK_FAIL_SQLITE:-0}" -eq 1 ]; then
  echo corrupt
  exit 1
fi
echo ok
SQLITE
cat > "$MOCK_BIN/cp" <<'COPY'
#!/bin/bash
if [ "${MOCK_FAIL_COLD_COPY:-0}" -eq 1 ] && [ "$1" = "$MOCK_SQLITE_SOURCE" ]; then
  printf 'copy:cold\n' >> "$MOCK_LOG"
  exit 1
fi
/bin/cp "$@"
COPY
cat > "$MOCK_BIN/ssh" <<'SSH'
#!/bin/bash
last=""
for arg in "$@"; do last="$arg"; done
case " $* " in
  *" -O exit "*) printf 'ssh:close\n' >> "$MOCK_LOG"; exit 0 ;;
esac
printf 'ssh:%s\n' "$last" >> "$MOCK_LOG"
/bin/sh -c "$last"
SSH
cat > "$MOCK_BIN/date" <<'DATE'
#!/bin/bash
if [ "$#" -eq 2 ] && [ "$1" = "-u" ] && [ "$2" = "+%Y%m%dT%H%M%SZ" ]; then
  printf '%s\n' "$MOCK_TIMESTAMP"
else
  /bin/date "$@"
fi
DATE
chmod +x "$FIXTURE/tools/backup.sh" "$FIXTURE/tools/service-runner.sh" "$MOCK_BIN/"*

export PATH="$MOCK_BIN:$PATH"
export MOCK_LOG MOCK_RESUME_COUNT
export MOCK_SQLITE_SOURCE="$OVERLAY/data/vaultwarden/data/db.sqlite3"
BACKUP="$FIXTURE/tools/backup.sh"
fails=0

fail() { echo "FAIL: $*"; fails=$((fails + 1)); }
expect_pass() {
  name="$1"; shift
  if output="$("$@" 2>&1)"; then :; else
    fail "$name should pass"
    echo "$output"
  fi
}
expect_fail_with() {
  name="$1"; expected="$2"; shift 2
  output="$("$@" 2>&1)"; status=$?
  if [ "$status" -eq 0 ] || ! printf '%s' "$output" | grep -qF "$expected"; then
    fail "$name should fail with: $expected"
    echo "$output"
  fi
}
expect_fail() {
  name="$1"; shift
  if "$@" >/dev/null 2>&1; then fail "$name should fail"; fi
}
line_of() {
  pattern="$1"
  grep -n -m1 "$pattern" "$MOCK_LOG" | cut -d: -f1
}
assert_order() {
  before="$(line_of "$1")"; after="$(line_of "$2")"
  [ -n "$before" ] && [ -n "$after" ] && [ "$before" -lt "$after" ] \
    || fail "expected '$1' before '$2'"
}
reset_run() {
  : > "$MOCK_LOG"
  rm -f "$MOCK_RESUME_COUNT"
  unset MOCK_FAIL_RSYNC MOCK_FAIL_SQLITE MOCK_FAIL_STOP MOCK_FAIL_COLD_COPY \
    MOCK_RESUME_FAILS MOCK_SIGNAL_PARENT
}
reset_remote() {
  rm -rf "$SCRATCH/remote/vaultwarden"
  mkdir -p "$SCRATCH/remote/vaultwarden"
}
make_old_archive() {
  name="$1"; stamp="$2"
  printf 'old archive\n' > "$SCRATCH/remote/vaultwarden/$name"
  touch -t "$stamp" "$SCRATCH/remote/vaultwarden/$name"
}

file_sha256() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}
source_before="$(file_sha256 "$OVERLAY/data/vaultwarden/data/db.sqlite3")"

# Pull mode produces a raw archive on stdout, keeps diagnostics on stderr, and does
# not resolve or contact a push target. Removing systems.local.toml proves destination
# coordinates are not a hidden stream-mode precondition.
reset_remote
reset_run
export MOCK_TIMESTAMP=20251231T010101Z
mv "$OVERLAY/config/systems.local.toml" "$OVERLAY/config/systems.local.toml.saved"
stream_archive="$SCRATCH/vaultwarden-stream.tar.gz"
stream_log="$SCRATCH/vaultwarden-stream.log"
if "$BACKUP" --stream vaultwarden > "$stream_archive" 2> "$stream_log"; then :; else
  fail "stream backup should pass without target coordinates"
fi
mv "$OVERLAY/config/systems.local.toml.saved" "$OVERLAY/config/systems.local.toml"
tar -tzf "$stream_archive" >/dev/null 2>&1 || fail "stream stdout was not a valid gzip archive"
tar -tzf "$stream_archive" | grep -q './axon-backup.toml' \
  || fail "stream archive omitted its backup contract"
grep -q 'stream vaultwarden archive' "$stream_log" \
  || fail "stream diagnostics were not written to stderr"
if grep -q '^ssh:' "$MOCK_LOG"; then fail "stream mode contacted a push target"; fi
[ ! -e "$OVERLAY/backup/receipts/vaultwarden.json" ] \
  || fail "stream mode fabricated a push receipt"
assert_order 'service:stop' 'sqlite:integrity'
assert_order 'sqlite:integrity' 'service:resume'
[ "$(grep -c '^service:resume$' "$MOCK_LOG")" = 1 ] \
  || fail "stream mode did not resume exactly once"

# A write that cannot complete must make the producer fail, not exit 0 over an archive the
# consumer only partly received. /dev/full accepts opens and fails every write with ENOSPC, which
# is the short-write path forced deterministically -- the case that reached this suite once as a
# flake, where the archive arrived without its manifest member and the producer still exited 0.
#
# A closing pipe (`| head -c 32`) was tried first and does not work: the fixture archive fits in
# the pipe buffer, so cat completes before the reader goes away and there is no error to see. That
# it looked like a valid test is exactly why it is written down here rather than left out.
if [ -c /dev/full ]; then
  reset_remote
  reset_run
  export MOCK_TIMESTAMP=20251231T020202Z
  short_log="$SCRATCH/vaultwarden-short.log"
  if "$BACKUP" --stream vaultwarden > /dev/full 2> "$short_log"; then
    fail "stream exited 0 although every write to the consumer failed"
  fi
  grep -q 'failed part-way' "$short_log" \
    || fail "an unwritable stream did not say so; log: $(cat "$short_log")"
else
  echo "NOTE: short-write assertion skipped — no /dev/full on this host (it runs in CI, which is Linux)"
fi

# Recovery mode is additive and the stopped-state order spans every host path plus
# SQLite verification. Resume precedes all SSH work.
reset_remote
make_old_archive vaultwarden-20240101T010101Z.tar.gz 202401010101
make_old_archive vaultwarden-20250101T010101Z.tar.gz 202501010101
reset_run
export MOCK_TIMESTAMP=20260101T010101Z
expect_pass "no-prune coherent backup" "$BACKUP" --no-prune vaultwarden
count="$(find "$SCRATCH/remote/vaultwarden" -maxdepth 1 -name 'vaultwarden-*.tar.gz' | wc -l | tr -d ' ')"
[ "$count" = 3 ] || fail "no-prune kept $count archives, expected 3"
[ -f "$SCRATCH/remote/vaultwarden/vaultwarden-20240101T010101Z.tar.gz" ] \
  || fail "no-prune removed the oldest archive"
grep -q '"retention_applied": false' "$OVERLAY/backup/receipts/vaultwarden.json" \
  || fail "no-prune receipt did not record retention_applied=false"
if grep -q 'tail -n +' "$MOCK_LOG"; then fail "no-prune issued the remote prune command"; fi
assert_order 'service:stop' 'rsync:.*data/vaultwarden/data'
assert_order 'rsync:.*data/vaultwarden/data' 'sqlite:integrity'
assert_order 'sqlite:integrity' 'service:resume'
assert_order 'service:resume' 'ssh:mkdir'
[ "$(grep -c '^service:resume$' "$MOCK_LOG")" = 1 ] || fail "successful run did not resume exactly once"

# Normal mode preserves the declared retention behavior.
reset_remote
make_old_archive vaultwarden-20230101T010101Z.tar.gz 202301010101
make_old_archive vaultwarden-20240101T010101Z.tar.gz 202401010101
make_old_archive vaultwarden-20250101T010101Z.tar.gz 202501010101
reset_run
export MOCK_TIMESTAMP=20260102T010101Z
expect_pass "normal retention backup" "$BACKUP" vaultwarden
count="$(find "$SCRATCH/remote/vaultwarden" -maxdepth 1 -name 'vaultwarden-*.tar.gz' | wc -l | tr -d ' ')"
[ "$count" = 2 ] || fail "normal retention kept $count archives, expected 2"
grep -q '"retention_applied": true' "$OVERLAY/backup/receipts/vaultwarden.json" \
  || fail "normal receipt did not record retention_applied=true"
grep -q 'tail -n +' "$MOCK_LOG" || fail "normal mode did not issue the remote prune command"

# A path-copy failure happens under the hold and the EXIT trap resumes before returning.
reset_remote
reset_run
export MOCK_TIMESTAMP=20260103T010101Z MOCK_FAIL_RSYNC=1
expect_fail "rsync failure resumes" "$BACKUP" --no-prune vaultwarden
assert_order 'service:stop' 'rsync:'
assert_order 'rsync:' 'service:resume'
[ "$(grep -c '^service:resume$' "$MOCK_LOG")" = 1 ] || fail "rsync failure did not attempt one resume"

# Signals take the same loud, deterministic exit path as command failures. A TERM while
# the first path is being copied must resume and must not ship a partial snapshot.
reset_remote
reset_run
export MOCK_TIMESTAMP=20260103T020202Z MOCK_SIGNAL_PARENT=TERM
expect_fail "interruption resumes" "$BACKUP" --no-prune vaultwarden
assert_order 'service:stop' 'rsync:'
assert_order 'rsync:' 'service:resume'
[ "$(grep -c '^service:resume$' "$MOCK_LOG")" = 1 ] || fail "interruption did not attempt one resume"
[ -z "$(find "$SCRATCH/remote/vaultwarden" -mindepth 1 -print -quit)" ] \
  || fail "interruption shipped an archive"

# Integrity failure also resumes, and nothing reaches the remote target.
reset_remote
reset_run
export MOCK_TIMESTAMP=20260104T010101Z MOCK_FAIL_SQLITE=1
if "$BACKUP" --no-prune vaultwarden >/dev/null 2>&1; then fail "SQLite integrity failure should fail"; fi
assert_order 'sqlite:integrity' 'service:resume'
[ -z "$(find "$SCRATCH/remote/vaultwarden" -mindepth 1 -print -quit)" ] \
  || fail "SQLite failure shipped an archive"

# The cold database copy has its own failure boundary after path staging.
reset_remote
reset_run
export MOCK_TIMESTAMP=20260104T020202Z MOCK_FAIL_COLD_COPY=1
expect_fail "cold-copy failure resumes" "$BACKUP" --no-prune vaultwarden
assert_order 'rsync:.*data/vaultwarden/tls' 'copy:cold'
assert_order 'copy:cold' 'service:resume'
[ "$(grep -c '^service:resume$' "$MOCK_LOG")" = 1 ] || fail "cold-copy failure did not attempt one resume"
[ -z "$(find "$SCRATCH/remote/vaultwarden" -mindepth 1 -print -quit)" ] \
  || fail "cold-copy failure shipped an archive"

# A resume failure is loud and gets one second attempt from the EXIT trap. The backup
# remains failed even if a later operator action can recover the service.
reset_remote
reset_run
export MOCK_TIMESTAMP=20260105T010101Z MOCK_RESUME_FAILS=2
expect_fail_with "resume failure is explicit" "CRITICAL: failed to resume" "$BACKUP" --no-prune vaultwarden
[ "$(grep -c '^service:resume$' "$MOCK_LOG")" = 2 ] || fail "resume failure did not receive two explicit attempts"
[ -z "$(find "$SCRATCH/remote/vaultwarden" -mindepth 1 -print -quit)" ] \
  || fail "resume failure shipped an archive"

source_after="$(file_sha256 "$OVERLAY/data/vaultwarden/data/db.sqlite3")"
[ "$source_before" = "$source_after" ] || fail "synthetic live database was modified"

if [ "$fails" -gt 0 ]; then
  echo "backup tests: $fails failure(s)"
  exit 1
fi
echo "backup tests: stream, coherent hold, no-prune, retention, and resume failures passed"
