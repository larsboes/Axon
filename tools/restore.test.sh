#!/bin/bash
# Synthetic restore matrix: no private paths, values, archives, or live runtimes.
set -uo pipefail

if [ -n "${TEST_SRCDIR:-}" ]; then
  ROOT="$TEST_SRCDIR/$TEST_WORKSPACE"
else
  ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
fi
RESTORE="$ROOT/tools/restore.sh"
SCRATCH="$(mktemp -d /tmp/axon-restore-test.XXXXXX)"
trap 'rm -rf "$SCRATCH"' EXIT
MOCK_BIN="$SCRATCH/bin"
mkdir -p "$MOCK_BIN"

cat > "$MOCK_BIN/sqlite3" <<'MOCK'
#!/bin/sh
if grep -q BROKEN "$1"; then
  echo corrupt
  exit 1
fi
echo ok
MOCK

cat > "$MOCK_BIN/docker" <<'MOCK'
#!/bin/sh
printf '%s\n' "$*" >> "$MOCK_RUNTIME_LOG"
cmd="$1"; shift
case "$cmd" in
  run) echo restore-container ;;
  exec)
    [ "${1:-}" = "-i" ] && shift
    shift # disposable container name
    case "${1:-}" in
      pg_isready) exit 0 ;;
      psql)
        # Order matters: the database-list query also mentions pg_database, so it
        # has to be matched before the plain count.
        case "$*" in
          *datistemplate*) printf '%s\n' ${MOCK_PG_DATABASES-axon} ;;
          *pg_database*) echo 3 ;;
          *pg_roles*) echo 5 ;;
          *pg_class*) echo "${MOCK_PG_COUNTS-25|469598}" ;;
          *) cat >/dev/null; exit 0 ;;
        esac
        ;;
    esac
    ;;
  rm) exit 0 ;;
esac
MOCK
chmod +x "$MOCK_BIN/sqlite3" "$MOCK_BIN/docker"
export PATH="$MOCK_BIN:$PATH"
export MOCK_RUNTIME_LOG="$SCRATCH/runtime.log"

fails=0
expect_pass() {
  name="$1"; shift
  if output="$("$@" 2>&1)"; then :; else
    echo "FAIL: $name should pass"
    echo "$output"
    fails=$((fails + 1))
  fi
}
expect_fail_with() {
  name="$1"; expected="$2"; shift 2
  output="$("$@" 2>&1)"; status=$?
  if [ "$status" -eq 0 ] || ! printf '%s' "$output" | grep -qF "$expected"; then
    echo "FAIL: $name should fail with: $expected"
    echo "$output"
    fails=$((fails + 1))
  fi
}

make_vault_tree() { # <stage> <sqlite-content> [omit-tls]
  stage="$1"; content="$2"; omit_tls="${3:-no}"
  mkdir -p "$stage/data/vaultwarden/data"
  printf '%s\n' "$content" > "$stage/data/vaultwarden/data/db.sqlite3"
  if [ "$omit_tls" = no ]; then
    mkdir -p "$stage/data/vaultwarden/tls"
    printf '%s\n' certificate > "$stage/data/vaultwarden/tls/cert.pem"
  fi
  cat > "$stage/axon-backup.toml" <<'META'
format = "1"
capability = "vaultwarden"
created_at = "2026-01-02T030405Z"
image = "vaultwarden/server"
tag = "1.37.0-alpine"
sqlite = "data/vaultwarden/data/db.sqlite3"
pg_dumpall = "false"
backup_paths = ["data/vaultwarden/data", "data/vaultwarden/tls"]
backup_container_paths = []
META
}

vault_stage="$SCRATCH/vault-stage"
mkdir -p "$vault_stage"
make_vault_tree "$vault_stage" VALID
tar czf "$SCRATCH/vaultwarden.tar.gz" -C "$vault_stage" .
vault_bytes="$(wc -c < "$SCRATCH/vaultwarden.tar.gz" | tr -d ' ')"
if command -v shasum >/dev/null 2>&1; then
  vault_sha="$(shasum -a 256 "$SCRATCH/vaultwarden.tar.gz" | awk '{print $1}')"
else
  vault_sha="$(sha256sum "$SCRATCH/vaultwarden.tar.gz" | awk '{print $1}')"
fi
cat > "$SCRATCH/vaultwarden-receipt.json" <<RECEIPT
{"capability":"vaultwarden","tarball":"vaultwarden.tar.gz","bytes":$vault_bytes,"sha256":"$vault_sha"}
RECEIPT
expect_pass "cold path and SQLite restore" "$RESTORE" vaultwarden "$SCRATCH/vaultwarden.tar.gz" \
  --receipt "$SCRATCH/vaultwarden-receipt.json" --destination "$SCRATCH/vault-out"

cat > "$SCRATCH/wrong-digest.json" <<RECEIPT
{"capability":"vaultwarden","tarball":"vaultwarden.tar.gz","bytes":$vault_bytes,"sha256":"0000000000000000000000000000000000000000000000000000000000000000"}
RECEIPT
expect_fail_with "receipt digest mismatch" "SHA-256 does not match the receipt" \
  "$RESTORE" vaultwarden "$SCRATCH/vaultwarden.tar.gz" --receipt "$SCRATCH/wrong-digest.json" \
  --destination "$SCRATCH/digest-out"

head -c 24 "$SCRATCH/vaultwarden.tar.gz" > "$SCRATCH/truncated.tar.gz"
expect_fail_with "truncated archive" "truncated or not a readable" \
  "$RESTORE" vaultwarden "$SCRATCH/truncated.tar.gz" --destination "$SCRATCH/truncated-out"

missing_stage="$SCRATCH/missing-stage"
mkdir -p "$missing_stage"
make_vault_tree "$missing_stage" VALID yes
tar czf "$SCRATCH/missing.tar.gz" -C "$missing_stage" .
expect_fail_with "missing declared member" "required backup path is missing" \
  "$RESTORE" vaultwarden "$SCRATCH/missing.tar.gz" --destination "$SCRATCH/missing-out"

expect_fail_with "wrong capability" "archive belongs to capability 'vaultwarden', not 'postgres'" \
  "$RESTORE" postgres "$SCRATCH/vaultwarden.tar.gz" --destination "$SCRATCH/wrong-out"

broken_stage="$SCRATCH/broken-stage"
mkdir -p "$broken_stage"
make_vault_tree "$broken_stage" BROKEN
tar czf "$SCRATCH/broken.tar.gz" -C "$broken_stage" .
expect_fail_with "failed SQLite integrity" "failed integrity_check" \
  "$RESTORE" vaultwarden "$SCRATCH/broken.tar.gz" --destination "$SCRATCH/broken-out"

live_candidate="$ROOT/restore-test-must-not-exist"
expect_fail_with "live checkout destination" "may not be inside the Axon checkout" \
  "$RESTORE" vaultwarden "$SCRATCH/vaultwarden.tar.gz" --destination "$live_candidate"
[ ! -e "$live_candidate" ] || { echo "FAIL: refused live destination was created"; fails=$((fails + 1)); }

container_payload="$SCRATCH/container-payload"
container_stage="$SCRATCH/container-stage"
mkdir -p "$container_payload" "$container_stage"
printf '%s\n' required > "$container_payload/required.txt"
chmod 750 "$container_payload/required.txt"
tar czpf "$container_stage/container-config.tar.gz" -C "$container_payload" .
cat > "$container_stage/axon-backup.toml" <<'META'
format = "1"
capability = "home-assistant"
created_at = "2026-01-02T030405Z"
image = "ghcr.io/home-assistant/home-assistant"
tag = "2026.7.4"
sqlite = ""
pg_dumpall = "false"
backup_paths = []
backup_container_paths = ["/config"]
META
tar czf "$SCRATCH/home-assistant.tar.gz" -C "$container_stage" .
expect_pass "container-path restore" "$RESTORE" home-assistant "$SCRATCH/home-assistant.tar.gz" --destination "$SCRATCH/container-out"
case "$(uname -s)" in
  Darwin) restored_mode="$(stat -f %Lp "$SCRATCH/container-out/container/config/required.txt")" ;;
  *) restored_mode="$(stat -c %a "$SCRATCH/container-out/container/config/required.txt")" ;;
esac
[ "$restored_mode" = 750 ] || { echo "FAIL: container-path mode was $restored_mode, expected 750"; fails=$((fails + 1)); }

postgres_stage="$SCRATCH/postgres-stage"
mkdir -p "$postgres_stage"
printf '%s\n' 'select 1;' > "$postgres_stage/pg_dumpall.sql"
cat > "$postgres_stage/axon-backup.toml" <<'META'
format = "1"
capability = "postgres"
created_at = "2026-01-02T030405Z"
image = "postgres"
tag = "17.10-alpine"
sqlite = ""
pg_dumpall = "true"
backup_paths = []
backup_container_paths = []
META
tar czf "$SCRATCH/postgres.tar.gz" -C "$postgres_stage" .
expect_pass "disposable PostgreSQL restore" "$RESTORE" postgres "$SCRATCH/postgres.tar.gz" \
  --destination "$SCRATCH/postgres-out" --runtime docker
grep -qF -- '--network none' "$MOCK_RUNTIME_LOG" \
  || { echo "FAIL: disposable Docker restore did not disable networking"; fails=$((fails + 1)); }
if grep '^run ' "$MOCK_RUNTIME_LOG" | grep -Eq -- '(^| )(-v|--volume|-p|--publish)( |$)'; then
  echo "FAIL: disposable PostgreSQL restore mounted or published host resources"
  fails=$((fails + 1))
fi

legacy_stage="$SCRATCH/legacy-stage"
mkdir -p "$legacy_stage/data/vaultwarden/data" "$legacy_stage/data/vaultwarden/tls"
printf '%s\n' VALID > "$legacy_stage/data/vaultwarden/data/db.sqlite3"
printf '%s\n' certificate > "$legacy_stage/data/vaultwarden/tls/cert.pem"
tar czf "$SCRATCH/vaultwarden-20260102T030405Z.tar.gz" -C "$legacy_stage" .
expect_fail_with "legacy archive is explicit" "pass --allow-legacy" \
  "$RESTORE" vaultwarden "$SCRATCH/vaultwarden-20260102T030405Z.tar.gz" --destination "$SCRATCH/legacy-refused"
expect_pass "known legacy archive" "$RESTORE" vaultwarden "$SCRATCH/vaultwarden-20260102T030405Z.tar.gz" \
  --destination "$SCRATCH/legacy-out" --allow-legacy

# A restored cluster always has template0/template1/postgres and the default
# roles, so the database and role counts alone report success for a dump that
# replayed into nothing. These pin the contents check that closes that.
make_pg_archive() { # <name> [extra-manifest-lines]
  stage="$SCRATCH/pg-$1-stage"
  mkdir -p "$stage"
  printf '%s\n' 'select 1;' > "$stage/pg_dumpall.sql"
  {
    printf 'format = "1"\ncapability = "postgres"\ncreated_at = "2026-01-02T030405Z"\n'
    printf 'image = "postgres"\ntag = "17.10-alpine"\nsqlite = ""\npg_dumpall = "true"\n'
    [ $# -ge 2 ] && printf '%s\n' "$2"
    printf 'backup_paths = []\nbackup_container_paths = []\n'
  } > "$stage/axon-backup.toml"
  tar czf "$SCRATCH/pg-$1.tar.gz" -C "$stage" .
}

make_pg_archive recorded 'pg_user_tables = "25"
pg_total_rows = "469598"'
MOCK_PG_COUNTS='25|469598' expect_pass "recorded contents restored exactly" \
  "$RESTORE" postgres "$SCRATCH/pg-recorded.tar.gz" --destination "$SCRATCH/pg-recorded-out" --runtime docker
MOCK_PG_COUNTS='25|469598' output="$("$RESTORE" postgres "$SCRATCH/pg-recorded.tar.gz" \
  --destination "$SCRATCH/pg-recorded-out2" --runtime docker 2>&1)"
printf '%s' "$output" | grep -qF "25 user table(s), 469598 row(s)" || {
  echo "FAIL: restore should report what it counted"; echo "$output"; fails=$((fails + 1)); }

MOCK_PG_COUNTS='25|469000' expect_fail_with "short row count is caught" \
  "restored 469000 row(s), archive recorded 469598" \
  "$RESTORE" postgres "$SCRATCH/pg-recorded.tar.gz" --destination "$SCRATCH/pg-short-out" --runtime docker

MOCK_PG_COUNTS='24|469598' expect_fail_with "missing table is caught" \
  "restored 24 user table(s), archive recorded 25" \
  "$RESTORE" postgres "$SCRATCH/pg-recorded.tar.gz" --destination "$SCRATCH/pg-missing-out" --runtime docker

# The exact failure the old check could not see: everything replayed into an
# empty cluster, and pg_database/pg_roles still answer non-zero.
make_pg_archive legacy
MOCK_PG_COUNTS='0|0' expect_fail_with "empty restore is caught without a recorded expectation" \
  "the dump replayed into nothing" \
  "$RESTORE" postgres "$SCRATCH/pg-legacy.tar.gz" --destination "$SCRATCH/pg-empty-out" --runtime docker

MOCK_PG_COUNTS='25|469598' expect_pass "archive without recorded contents still restores" \
  "$RESTORE" postgres "$SCRATCH/pg-legacy.tar.gz" --destination "$SCRATCH/pg-legacy-out" --runtime docker

# --- container-runtime preflight (#163) ------------------------------------------------------
# Verifying a PostgreSQL dump runs a disposable instance. The runtime check used to live inside
# verify_postgres, reached only after a destination was created and the archive extracted into it,
# so the one operation whose whole purpose is proving a backup is recoverable announced its
# missing dependency at the very end. `podman` stands in for an uninstalled runtime: this suite
# stubs `docker` on PATH and nothing stubs podman.
if command -v podman >/dev/null 2>&1; then
  echo "NOTE: reduced coverage on this host — podman is installed, so it cannot stand in for an absent runtime"
else
  PREFLIGHT_DEST="$SCRATCH/pg-preflight-out"
  expect_fail_with "an absent container runtime is caught before anything is created" \
    "which is not on PATH" \
    "$RESTORE" postgres "$SCRATCH/pg-legacy.tar.gz" --destination "$PREFLIGHT_DEST" --runtime podman
  # The half that makes it a preflight rather than merely an earlier error message.
  [ ! -e "$PREFLIGHT_DEST" ] || {
    echo "FAIL: the destination was created before the dependency check ran"; fails=$((fails + 1)); }
fi

if [ "$fails" -gt 0 ]; then
  echo "restore tests: $fails failure(s)"
  exit 1
fi
echo "restore tests: all synthetic backup forms and failure cases passed"
