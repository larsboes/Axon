#!/bin/bash
# Restore and verify one tools/backup.sh archive without writing into live Axon state.
# The extracted tree remains for inspection; PostgreSQL is rehearsed in a disposable
# container with no host mounts or published ports. Bash 3.2-safe.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$TOOLS_DIR/lib/paths.sh"

usage() {
  echo "usage: restore.sh <capability> <archive.tar.gz> [--receipt <receipt.json>] [--destination <empty-dir>] [--runtime <apple-container|docker|podman>] [--allow-legacy]" >&2
  exit 1
}

[ $# -ge 2 ] || usage
CAP="$1"
ARCHIVE="$2"
shift 2

DEST_ARG=""
RECEIPT=""
RUNTIME=""
ALLOW_LEGACY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --receipt)
      [ $# -ge 2 ] || usage
      RECEIPT="$2"; shift 2
      ;;
    --destination)
      [ $# -ge 2 ] || usage
      DEST_ARG="$2"; shift 2
      ;;
    --runtime)
      [ $# -ge 2 ] || usage
      RUNTIME="$2"; shift 2
      ;;
    --allow-legacy)
      ALLOW_LEGACY=1; shift
      ;;
    *) usage ;;
  esac
done

case "$CAP" in
  ''|*[!A-Za-z0-9._-]*) echo "restore.sh: invalid capability name" >&2; exit 1 ;;
esac

MANIFEST="$AXON_ROOT/capabilities/$CAP/service.toml"
[ -f "$MANIFEST" ] || { echo "restore.sh: no $MANIFEST" >&2; exit 1; }
[ -f "$ARCHIVE" ] || { echo "restore.sh: archive not found: $ARCHIVE" >&2; exit 1; }
ARCHIVE="$(cd "$(dirname "$ARCHIVE")" && pwd -P)/$(basename "$ARCHIVE")"

WORK="$(mktemp -d "/tmp/axon-restore-check.XXXXXX")"
RESTORE_CONTAINER=""
RUNTIME_BIN=""
DEST=""
DEST_AUTO=0
RESTORE_COMPLETE=0
cleanup() {
  if [ -n "$RESTORE_CONTAINER" ] && [ -n "$RUNTIME_BIN" ]; then
    case "$RUNTIME" in
      apple-container)
        "$RUNTIME_BIN" stop "$RESTORE_CONTAINER" >/dev/null 2>&1 || true
        "$RUNTIME_BIN" delete "$RESTORE_CONTAINER" >/dev/null 2>&1 || true
        ;;
      docker|podman)
        "$RUNTIME_BIN" rm -f "$RESTORE_CONTAINER" >/dev/null 2>&1 || true
        ;;
    esac
  fi
  rm -rf "$WORK"
  if [ "$DEST_AUTO" -eq 1 ] && [ "$RESTORE_COMPLETE" -ne 1 ] && [ -n "$DEST" ]; then
    rm -rf "$DEST"
  fi
}
trap cleanup EXIT

fail() { echo "restore.sh: $*" >&2; exit 1; }

sha256_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    fail "need shasum or sha256sum to identify the archive"
  fi
}

ARCHIVE_BYTES="$(wc -c < "$ARCHIVE" | tr -d ' ')"
ARCHIVE_SHA256="$(sha256_file "$ARCHIVE")"
if [ -n "$RECEIPT" ]; then
  [ -f "$RECEIPT" ] || fail "receipt not found: $RECEIPT"
  command -v jq >/dev/null 2>&1 || fail "jq is required to verify a receipt"
  RECEIPT_CAP="$(jq -r '.capability // empty' "$RECEIPT")"
  RECEIPT_ARCHIVE="$(jq -r '.tarball // empty' "$RECEIPT")"
  RECEIPT_BYTES="$(jq -r '.bytes // empty' "$RECEIPT")"
  RECEIPT_SHA256="$(jq -r '.sha256 // empty' "$RECEIPT")"
  [ "$RECEIPT_CAP" = "$CAP" ] || fail "receipt belongs to another capability"
  [ "$RECEIPT_ARCHIVE" = "$(basename "$ARCHIVE")" ] || fail "receipt names another archive"
  [ "$RECEIPT_BYTES" = "$ARCHIVE_BYTES" ] || fail "archive byte count does not match the receipt"
  if [ -n "$RECEIPT_SHA256" ]; then
    [ "$RECEIPT_SHA256" = "$ARCHIVE_SHA256" ] || fail "archive SHA-256 does not match the receipt"
  else
    echo "restore.sh: warning: legacy receipt has no SHA-256; size is the only receipt-level check" >&2
  fi
fi

# Validate before extraction. Link and special-file members are rejected because a
# syntactically relative filename can still escape through a preceding symlink.
TAR_SEQ=0
LAST_TAR_LIST=""
validate_tar() { # <archive> <description>
  archive="$1"; description="$2"
  TAR_SEQ=$((TAR_SEQ + 1))
  list="$WORK/tar-$TAR_SEQ.list"
  verbose="$WORK/tar-$TAR_SEQ.verbose"
  tar -tzf "$archive" > "$list" 2>/dev/null || fail "$description is truncated or not a readable gzip tar archive"
  [ -s "$list" ] || fail "$description is empty"
  while IFS= read -r member; do
    normalized="${member#./}"
    case "$normalized" in
      ''|.) ;;
      /*|..|../*|*/../*) fail "$description contains an unsafe member path" ;;
    esac
  done < "$list"
  tar -tvzf "$archive" > "$verbose" 2>/dev/null || fail "$description metadata cannot be read"
  if LC_ALL=C grep -Eq '^[lhbcps]' "$verbose"; then
    fail "$description contains a link or special file; refusing unsafe extraction"
  fi
  LAST_TAR_LIST="$list"
}

safe_relative() {
  case "$1" in
    ''|/*|..|../*|*/../*) return 1 ;;
    *) return 0 ;;
  esac
}

path_within() { # <candidate> <root>
  case "$1" in "$2"|"$2"/*) return 0 ;; *) return 1 ;; esac
}

DEST_NEEDS_CREATE=0
if [ -n "$DEST_ARG" ]; then
  if [ -e "$DEST_ARG" ]; then
    [ -d "$DEST_ARG" ] || fail "destination exists and is not a directory: $DEST_ARG"
    [ -z "$(find "$DEST_ARG" -mindepth 1 -maxdepth 1 -print -quit)" ] || fail "destination must be empty: $DEST_ARG"
    DEST="$(cd "$DEST_ARG" && pwd -P)"
  else
    parent="$(dirname "$DEST_ARG")"
    [ -d "$parent" ] || fail "destination parent does not exist: $parent"
    DEST="$(cd "$parent" && pwd -P)/$(basename "$DEST_ARG")"
    DEST_NEEDS_CREATE=1
  fi
else
  DEST="/tmp/axon-restore-$CAP.XXXXXX"
  DEST_NEEDS_CREATE=2
fi

AXON_REAL="$(cd "$AXON_ROOT" && pwd -P)"
OVERLAY_REAL="$(cd "$AXON_PERSONAL_ROOT" 2>/dev/null && pwd -P || true)"
path_within "$DEST" "$AXON_REAL" && fail "destination may not be inside the Axon checkout"
[ -n "$OVERLAY_REAL" ] && path_within "$DEST" "$OVERLAY_REAL" \
  && fail "destination may not be inside the active private overlay"
case "$DEST_NEEDS_CREATE" in
  1) mkdir "$DEST" ;;
  2) DEST="$(mktemp -d "$DEST")"; DEST_AUTO=1 ;;
esac

validate_tar "$ARCHIVE" "backup archive"
OUTER_LIST="$LAST_TAR_LIST"
META_MEMBER="$(grep -E '^(\./)?axon-backup\.toml$' "$OUTER_LIST" || true)"
if [ -n "$META_MEMBER" ]; then
  [ "$(printf '%s\n' "$META_MEMBER" | grep -c .)" -eq 1 ] || fail "archive contains more than one backup manifest"
  tar -xOzf "$ARCHIVE" "$META_MEMBER" > "$WORK/axon-backup.toml"
  FORMAT="$(toml_get format "$WORK/axon-backup.toml")"
  ARCHIVE_CAP="$(toml_get capability "$WORK/axon-backup.toml")"
  [ "$FORMAT" = "1" ] || fail "unsupported backup format '$FORMAT'"
  [ "$ARCHIVE_CAP" = "$CAP" ] || fail "archive belongs to capability '$ARCHIVE_CAP', not '$CAP'"
  CREATED_AT="$(toml_get created_at "$WORK/axon-backup.toml")"
  IMAGE="$(toml_get image "$WORK/axon-backup.toml")"
  TAG="$(toml_get tag "$WORK/axon-backup.toml")"
  SQLITE_REL="$(toml_get sqlite "$WORK/axon-backup.toml")"
  PG_DUMPALL="$(toml_get pg_dumpall "$WORK/axon-backup.toml")"
  # Optional: only archives taken from 2026-08-02 on carry what the dump held.
  EXPECT_TABLES="$(toml_get pg_user_tables "$WORK/axon-backup.toml")"
  EXPECT_ROWS="$(toml_get pg_total_rows "$WORK/axon-backup.toml")"
  PATHS=()
  while IFS= read -r line; do [ -n "$line" ] && PATHS+=("$line"); done < <(toml_array backup_paths "$WORK/axon-backup.toml")
  CPATHS=()
  while IFS= read -r line; do [ -n "$line" ] && CPATHS+=("$line"); done < <(toml_array backup_container_paths "$WORK/axon-backup.toml")

  CURRENT_IMAGE="$(toml_get image "$MANIFEST")"
  CURRENT_TAG="$(toml_get tag "$MANIFEST")"
  CURRENT_SQLITE="$(toml_get backup_sqlite "$MANIFEST")"
  CURRENT_PG="$(toml_get backup_pg_dumpall "$MANIFEST")"; CURRENT_PG="${CURRENT_PG:-false}"
  [ "$IMAGE:$TAG" = "$CURRENT_IMAGE:$CURRENT_TAG" ] \
    || fail "archive image identity differs from the current tracked manifest; use the matching Axon revision"
  [ "$SQLITE_REL:$PG_DUMPALL" = "$CURRENT_SQLITE:$CURRENT_PG" ] \
    || fail "archive database contract differs from the current tracked manifest; use the matching Axon revision"
  [ "$(toml_array backup_paths "$WORK/axon-backup.toml")" = "$(toml_array backup_paths "$MANIFEST")" ] \
    || fail "archive path contract differs from the current tracked manifest; use the matching Axon revision"
  [ "$(toml_array backup_container_paths "$WORK/axon-backup.toml")" = "$(toml_array backup_container_paths "$MANIFEST")" ] \
    || fail "archive container-path contract differs from the current tracked manifest; use the matching Axon revision"
else
  [ "$ALLOW_LEGACY" -eq 1 ] || fail "archive has no axon-backup.toml; pass --allow-legacy only for a known pre-format archive"
  case "$(basename "$ARCHIVE")" in
    "$CAP"-*.tar.gz) ;;
    *) fail "legacy archive name does not identify capability '$CAP'" ;;
  esac
  FORMAT="legacy"
  CREATED_AT="unknown"
  IMAGE="$(toml_get image "$MANIFEST")"
  TAG="$(toml_get tag "$MANIFEST")"
  SQLITE_REL="$(toml_get backup_sqlite "$MANIFEST")"
  PG_DUMPALL="$(toml_get backup_pg_dumpall "$MANIFEST")"
  PATHS=()
  while IFS= read -r line; do [ -n "$line" ] && PATHS+=("$line"); done < <(toml_array backup_paths "$MANIFEST")
  CPATHS=()
  while IFS= read -r line; do [ -n "$line" ] && CPATHS+=("$line"); done < <(toml_array backup_container_paths "$MANIFEST")
  echo "restore.sh: warning: legacy archive identity comes from its filename and the current manifest" >&2
fi
PG_DUMPALL="${PG_DUMPALL:-false}"

[ "${#PATHS[@]}" -gt 0 ] || [ "${#CPATHS[@]}" -gt 0 ] || [ "$PG_DUMPALL" = "true" ] \
  || fail "archive declares no restorable content"

echo "→ archive: $(basename "$ARCHIVE")"
echo "  bytes: $ARCHIVE_BYTES · sha256: $ARCHIVE_SHA256"
echo "  format: $FORMAT · capability: $CAP · created: $CREATED_AT"
echo "→ extract outer archive → $DEST"
tar -xzpf "$ARCHIVE" -C "$DEST"

if [ "${#PATHS[@]}" -gt 0 ]; then
  for path in "${PATHS[@]}"; do
    safe_relative "$path" || fail "backup manifest contains an unsafe path declaration"
    [ -e "$DEST/$path" ] || fail "required backup path is missing from the archive: $path"
  done
  echo "  declared path roots: ${#PATHS[@]} present"
fi

if [ -n "$SQLITE_REL" ]; then
  safe_relative "$SQLITE_REL" || fail "backup manifest contains an unsafe SQLite path"
  [ -f "$DEST/$SQLITE_REL" ] || fail "declared SQLite database is missing from the archive: $SQLITE_REL"
  command -v sqlite3 >/dev/null 2>&1 || fail "sqlite3 is required to verify the restored database"
  sqlite_result="$(sqlite3 "$DEST/$SQLITE_REL" 'pragma integrity_check;' 2>/dev/null || true)"
  [ "$sqlite_result" = "ok" ] || fail "restored SQLite database failed integrity_check"
  echo "  SQLite integrity_check: ok"
fi

if [ "${#CPATHS[@]}" -gt 0 ]; then
  for container_path in "${CPATHS[@]}"; do
    case "$container_path" in /*) ;; *) fail "container backup path must be absolute" ;; esac
    relative="${container_path#/}"
    safe_relative "$relative" || fail "backup manifest contains an unsafe container path"
    safe="$(printf '%s' "$relative" | tr '/' '_')"
    nested="$DEST/container-$safe.tar.gz"
    [ -s "$nested" ] || fail "required container-path archive is missing: container-$safe.tar.gz"
    validate_tar "$nested" "container-path archive for $container_path"
    nested_dest="$DEST/container/$safe"
    mkdir -p "$nested_dest"
    tar -xzpf "$nested" -C "$nested_dest"
    [ -n "$(find "$nested_dest" -mindepth 1 -print -quit)" ] || fail "container-path restore is empty"
    echo "  container path restored with archived modes: $container_path → container/$safe/"
  done
fi

qualified_image() {
  ref="$IMAGE:$TAG"; first="${IMAGE%%/*}"
  case "$RUNTIME" in
    apple-container)
      case "$first" in *.*|*:*|localhost) echo "$ref" ;; *) echo "docker.io/$ref" ;; esac
      ;;
    *) echo "$ref" ;;
  esac
}

verify_postgres() {
  [ -s "$DEST/pg_dumpall.sql" ] || fail "pg_dumpall.sql is missing or empty"
  [ -n "$IMAGE" ] && [ -n "$TAG" ] || fail "PostgreSQL archive has no image identity"
  if [ -z "$RUNTIME" ]; then
    source "$TOOLS_DIR/lib/platform.sh"
    RUNTIME="$AXON_CONTAINER_RUNTIME"
  fi
  case "$RUNTIME" in
    apple-container) RUNTIME_BIN="container" ;;
    docker|podman) RUNTIME_BIN="$RUNTIME" ;;
    *) fail "unsupported container runtime '$RUNTIME'" ;;
  esac
  command -v "$RUNTIME_BIN" >/dev/null 2>&1 || fail "container runtime command not found: $RUNTIME_BIN"
  if [ "$RUNTIME" = "apple-container" ] \
    && ! "$RUNTIME_BIN" system status 2>/dev/null | grep -qE '^status[[:space:]]+running'; then
    "$RUNTIME_BIN" system start >/dev/null
  fi

  RESTORE_CONTAINER="axon-restore-$CAP-$$"
  RESTORE_ADMIN="axon_restore_admin"
  image_ref="$(qualified_image)"
  echo "→ disposable PostgreSQL restore ($RUNTIME, no mounts or published ports)"
  RUN_ARGS=(-d --rm --name "$RESTORE_CONTAINER")
  case "$RUNTIME" in
    apple-container) RUN_ARGS+=(--no-dns) ;;
    docker|podman) RUN_ARGS+=(--network none) ;;
  esac
  "$RUNTIME_BIN" run "${RUN_ARGS[@]}" \
    -e POSTGRES_HOST_AUTH_METHOD=trust \
    -e "POSTGRES_USER=$RESTORE_ADMIN" \
    -e "POSTGRES_DB=$RESTORE_ADMIN" \
    "$image_ref" >/dev/null

  ready=0; attempt=0
  while [ "$attempt" -lt 60 ]; do
    if "$RUNTIME_BIN" exec "$RESTORE_CONTAINER" \
      pg_isready -U "$RESTORE_ADMIN" -d "$RESTORE_ADMIN" >/dev/null 2>&1; then
      ready=1; break
    fi
    sleep 1
    attempt=$((attempt + 1))
  done
  [ "$ready" -eq 1 ] || fail "disposable PostgreSQL instance did not become ready"

  if ! "$RUNTIME_BIN" exec -i "$RESTORE_CONTAINER" \
    psql -U "$RESTORE_ADMIN" -d "$RESTORE_ADMIN" -v ON_ERROR_STOP=1 \
    < "$DEST/pg_dumpall.sql" > "$WORK/postgres-restore.log" 2>&1; then
    fail "pg_dumpall failed to restore into the disposable instance"
  fi
  database_count="$("$RUNTIME_BIN" exec "$RESTORE_CONTAINER" \
    psql -U "$RESTORE_ADMIN" -d "$RESTORE_ADMIN" -Atqc \
    'select count(*) from pg_database where datallowconn;' 2>/dev/null || true)"
  role_count="$("$RUNTIME_BIN" exec "$RESTORE_CONTAINER" \
    psql -U "$RESTORE_ADMIN" -d "$RESTORE_ADMIN" -Atqc \
    'select count(*) from pg_roles;' 2>/dev/null || true)"
  case "$database_count" in ''|*[!0-9]*|0) fail "restored PostgreSQL database query failed" ;; esac
  case "$role_count" in ''|*[!0-9]*|0) fail "restored PostgreSQL role query failed" ;; esac
  echo "  database and role sanity queries: passed"

  # Those two counts are true of a completely empty cluster -- template0,
  # template1, postgres and the default roles always exist -- so on their own
  # they would report success for a dump that replayed into nothing. Count what
  # actually came back instead.
  count_sql="SELECT count(*)::text || '|' || coalesce(sum((xpath('/row/c/text()', query_to_xml(format('select count(*) as c from %I.%I', n.nspname, c.relname), false, true, '')))[1]::text::bigint), 0)::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind = 'r' AND n.nspname NOT IN ('pg_catalog', 'information_schema');"
  restored_tables=0
  restored_rows=0
  for db in $("$RUNTIME_BIN" exec "$RESTORE_CONTAINER" \
                psql -U "$RESTORE_ADMIN" -d "$RESTORE_ADMIN" -Atqc \
                'select datname from pg_database where datallowconn and not datistemplate order by 1;' 2>/dev/null); do
    pair="$("$RUNTIME_BIN" exec "$RESTORE_CONTAINER" \
              psql -U "$RESTORE_ADMIN" -d "$db" -Atqc "$count_sql" 2>/dev/null || true)"
    case "$pair" in
      *'|'*)
        restored_tables=$((restored_tables + ${pair%%|*}))
        restored_rows=$((restored_rows + ${pair##*|}))
        ;;
    esac
  done
  echo "  restored contents: $restored_tables user table(s), $restored_rows row(s)"

  # Archives from before 2026-08-02 carry no expectation. Those still have to
  # contain something, but the exact comparison is skipped and said out loud
  # rather than silently downgraded.
  if [ -n "$EXPECT_TABLES" ] || [ -n "$EXPECT_ROWS" ]; then
    [ "$restored_tables" = "$EXPECT_TABLES" ] || \
      fail "restored $restored_tables user table(s), archive recorded $EXPECT_TABLES"
    [ "$restored_rows" = "$EXPECT_ROWS" ] || \
      fail "restored $restored_rows row(s), archive recorded $EXPECT_ROWS"
    echo "  matches the archive's recorded contents exactly"
  else
    [ "$restored_tables" -gt 0 ] || \
      fail "restored cluster has no user tables — the dump replayed into nothing"
    echo "  archive predates recorded contents; checked non-empty only"
  fi
}

if [ "$PG_DUMPALL" = "true" ]; then
  verify_postgres
fi

echo "✓ restore verified in isolation"
echo "  retained for inspection: $DEST"
echo "  no live capability path was written"
RESTORE_COMPLETE=1
