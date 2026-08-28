#!/bin/bash
# Synthetic cutover matrix for tools/migrate-pg-to-sqlite. No live Postgres, no overlay, no
# private path: a mock psql answers the catalog queries and emits the INSERT statements a real
# one would, and a real sqlite3 receives them.
#
# WHAT THIS DOES NOT COVER, said out loud rather than implied by a green run: the type
# conversions are SQL that Postgres executes (quote_nullable, to_char for a timestamp, to_json
# for an integer[]), so a mock cannot exercise them. What is covered is everything the shell
# owns — the refusals, the schema/table name mapping, the per-table count verification, the
# receipt, and the dry run's promise to write nothing. The conversions were checked by hand
# against the live instance on 2026-08-28 and the reading is in the cutover procedure.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
TOOL="$ROOT/tools/migrate-pg-to-sqlite"
SCRATCH="$(mktemp -d /tmp/axon-migrate-test.XXXXXX)"
trap 'rm -rf "$SCRATCH"' EXIT

source "$ROOT/tools/lib/test-support.sh"
isolate_axon_env

OVERLAY="$SCRATCH/overlay"
MOCK_BIN="$SCRATCH/bin"
mkdir -p "$OVERLAY/config" "$OVERLAY/data/axon" "$MOCK_BIN"
printf 'os = "linux"\ncontainer_runtime = "docker"\ncapabilities = []\n' > "$OVERLAY/config/machine.toml"
export AXON_OVERLAY_ROOT="$OVERLAY"

fails=0
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }
expect_pass() {
  name="$1"; shift
  if output="$("$@" 2>&1)"; then :; else
    fail "$name should pass"; echo "$output"
  fi
}
expect_fail_with() {
  name="$1"; expected="$2"; shift 2
  output="$("$@" 2>&1)"; status=$?
  if [ "$status" -eq 0 ] || ! printf '%s' "$output" | grep -qF "$expected"; then
    fail "$name should fail with: $expected"; echo "$output"
  fi
}

# The stand-in for `container exec postgres psql`. It answers on the SQL it is handed, which is
# always the LAST -c argument — the first one is the read-only SET the tool opens every session
# with, and a mock that ignored it would let that guarantee rot unnoticed, so it is asserted.
cat > "$MOCK_BIN/mock-psql" <<'MOCK'
#!/bin/bash
sql=""
saw_readonly=0
while [ $# -gt 0 ]; do
  case "$1" in
    -c) sql="$2"
        case "$2" in *"default_transaction_read_only = on"*) saw_readonly=1 ;; esac
        shift 2 ;;
    *) shift ;;
  esac
done
printf '%s\n' "$sql" >> "$MOCK_PSQL_LOG"
[ "$saw_readonly" -eq 1 ] || { echo "mock-psql: session was not set read-only" >&2; exit 1; }
case "$sql" in
  "select 1") echo 1 ;;
  *"from pg_namespace"*)
    printf 'comms\npunctuality\n'
    [ "${MOCK_EXTRA_SCHEMA:-}" = "" ] || printf '%s\n' "$MOCK_EXTRA_SCHEMA"
    ;;
  *"data_type !~"*)
    printf '%s' "${MOCK_BAD_TYPE:-}"
    ;;
  *"from pg_class"*)
    printf 'comms\tfeed_items\t2\n'
    printf 'punctuality\tstop_stats\t%s\n' "${MOCK_STOP_STATS_ROWS:-1}"
    ;;
  *"string_agg('\""*)
    case "$sql" in
      *feed_items*) echo '"id","title"' ;;
      *) echo '"eva","counts"' ;;
    esac
    ;;
  *"case"*"coalesce"*)
    echo "coalesce('''' || replace(a::text, '''', '''''') || '''', 'NULL')"
    ;;
  *"INSERT INTO"*)
    case "$sql" in
      *comms_feed_items*)
        echo "INSERT INTO \"comms_feed_items\" (\"id\",\"title\") VALUES ('a','it''s here');"
        echo "INSERT INTO \"comms_feed_items\" (\"id\",\"title\") VALUES ('b',NULL);"
        ;;
      *)
        echo "INSERT INTO \"punctuality_stop_stats\" (\"eva\",\"counts\") VALUES ('8000105','[1,2,3]');"
        ;;
    esac
    ;;
esac
MOCK
chmod +x "$MOCK_BIN/mock-psql"
export MOCK_PSQL_LOG="$SCRATCH/psql.log"
export AXON_MIGRATE_PSQL="$MOCK_BIN/mock-psql"

TARGET="$OVERLAY/data/axon/axon.db"
make_schema() {
  rm -f "$TARGET" "$TARGET-wal" "$TARGET-shm"
  sqlite3 "$TARGET" "
    CREATE TABLE comms_feed_items (id TEXT PRIMARY KEY, title TEXT);
    CREATE TABLE punctuality_stop_stats (eva TEXT, counts TEXT);
  "
}

# --- refusals, before anything is written ---------------------------------------------------
rm -f "$TARGET"
expect_fail_with "a missing target is refused with the sequence that creates it" \
  "tools/service-runner.sh up --all" "$TOOL" --target "$TARGET"

make_schema
sqlite3 "$TARGET" "INSERT INTO comms_feed_items VALUES ('x', 'already here');"
expect_fail_with "a target holding rows is refused" "will not merge" "$TOOL" --target "$TARGET"

make_schema
sqlite3 "$TARGET" "DROP TABLE punctuality_stop_stats;"
expect_fail_with "a target missing a table is refused before any row moves" \
  "missing tables the source has: punctuality_stop_stats" "$TOOL" --target "$TARGET"
[ "$(sqlite3 "$TARGET" 'select count(*) from comms_feed_items;')" = 0 ] \
  || fail "rows were copied into a target that was missing a table"

make_schema
MOCK_EXTRA_SCHEMA=analytics expect_fail_with "an unrecognised schema stops the run" \
  "does not know about: analytics" "$TOOL" --target "$TARGET"

make_schema
MOCK_BAD_TYPE=inet expect_fail_with "a column type with no rule stops the run" \
  "no conversion rule: inet" "$TOOL" --target "$TARGET"

# A test schema is skipped by name, and says so. Silence here would mean a per-pid leftover
# either becoming a table or vanishing without a word.
make_schema
output="$(MOCK_EXTRA_SCHEMA=tasks_test_completion_12701 "$TOOL" --target "$TARGET" --dry-run 2>&1)"
printf '%s' "$output" | grep -qF "skipping test schema: tasks_test_completion_12701" \
  || { fail "a test schema was not reported as skipped"; echo "$output"; }

# --- dry run --------------------------------------------------------------------------------
make_schema
output="$("$TOOL" --target "$TARGET" --dry-run 2>&1)"
printf '%s' "$output" | grep -qF "comms.feed_items" || { fail "the plan does not name the source table"; echo "$output"; }
printf '%s' "$output" | grep -qF "comms_feed_items" || fail "the plan does not name the target table"
printf '%s' "$output" | grep -qF "nothing was written" || fail "the dry run did not say it wrote nothing"
[ "$(sqlite3 "$TARGET" 'select count(*) from comms_feed_items;')" = 0 ] \
  || fail "the dry run wrote rows"

# --- the copy -------------------------------------------------------------------------------
make_schema
expect_pass "cutover" "$TOOL" --target "$TARGET"
[ "$(sqlite3 "$TARGET" 'select count(*) from comms_feed_items;')" = 2 ] || fail "comms rows did not arrive"
[ "$(sqlite3 "$TARGET" 'select count(*) from punctuality_stop_stats;')" = 1 ] || fail "punctuality rows did not arrive"
[ "$(sqlite3 "$TARGET" "select title from comms_feed_items where id = 'a';")" = "it's here" ] \
  || fail "an embedded quote did not survive the copy"
[ "$(sqlite3 "$TARGET" "select count(*) from comms_feed_items where title is null;")" = 1 ] \
  || fail "a NULL arrived as something else"
[ -f "$OVERLAY/backup/receipts/migrate-pg-to-sqlite.txt" ] || fail "no receipt was written"
grep -qF "integrity_check: ok" "$OVERLAY/backup/receipts/migrate-pg-to-sqlite.txt" \
  || fail "the receipt does not record the file's own verdict"

# The count comparison is the whole verification, so it has to be able to fail. The source
# claims two rows for a table the mock then emits one row for.
make_schema
MOCK_STOP_STATS_ROWS=99 expect_fail_with "a short copy is caught by the count check" \
  "row counts do not match" "$TOOL" --target "$TARGET"

if [ "$fails" -gt 0 ]; then
  echo "migrate-pg-to-sqlite tests: $fails failure(s)"
  exit 1
fi
echo "migrate-pg-to-sqlite tests: refusals, dry run, copy, and count verification passed"
