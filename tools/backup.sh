#!/bin/bash
# tools/backup.sh — capability data backup, driven by the capability's own
# service.toml [backup] declaration (same "commands as data" split as
# service-runner.sh). Takes a COLD SQLite copy with the capability held down (never
# reaches into a live DB from the host — see the backup_sqlite block), tars the declared
# paths, and ships a timestamped tarball to a remote host with retention. The remote is
# a systems.toml id; its private coordinates (host/ssh_user/backup_root) live in the
# overlay's systems.local.toml.
#
#   tools/backup.sh <capability>              # take + push a backup now
#   tools/backup.sh --no-prune <capability>   # additive recovery-rehearsal backup
#   tools/backup.sh --stream <capability>     # archive bytes on stdout; diagnostics on stderr
#
# Reached over ssh. With vault-only SSH keys (Bitwarden agent) the vault must be
# unlocked — approve the agent prompt when it appears. bash 3.2-safe.
set -euo pipefail

# Stream mode is a machine-initiated pull contract: stdout must contain archive bytes
# and nothing else, including output from sourced helpers. Preserve the caller's stdout
# on fd 3 before loading any helper, and route every normal message to stderr.
case "$#:${1:-}" in
  2:--stream) exec 3>&1 1>&2 ;;
esac

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$TOOLS_DIR/lib/paths.sh"                 # AXON_ROOT, AXON_PERSONAL_ROOT, toml_*
source "$TOOLS_DIR/lib/platform.sh"              # AXON_CONTAINER_RUNTIME (pg_dumpall exec)
# Best-effort: make the vault SSH agent available even if not launched from an
# interactive shell (shared with init.zsh). No-op if the app isn't running.
source "$TOOLS_DIR/lib/bw-agent.sh" 2>/dev/null || true

usage() { echo "usage: backup.sh [--no-prune|--stream] <capability>" >&2; exit 1; }
NO_PRUNE=0
STREAM=0
case "$#:${1:-}" in
  1:--no-prune|1:--stream) usage ;;
  1:*) CAP="$1" ;;
  2:--no-prune) NO_PRUNE=1; CAP="$2" ;;
  2:--stream) STREAM=1; CAP="$2" ;;
  *) usage ;;
esac

MANIFEST="$AXON_ROOT/capabilities/$CAP/service.toml"
[ -f "$MANIFEST" ] || { echo "backup.sh: no $MANIFEST" >&2; exit 1; }

TARGET_ID="$(toml_get backup_target "$MANIFEST")"
SQLITE_REL="$(toml_get backup_sqlite "$MANIFEST")"
PG_DUMPALL="$(toml_get backup_pg_dumpall "$MANIFEST")"
PG_DUMPALL="${PG_DUMPALL:-false}"
# The container name, read with the other manifest fields rather than inside the
# pg_dumpall block: two staging paths now exec into the container, and a variable
# defined in the branch that happens to run first is a trap for the second.
NAME="$(toml_get name "$MANIFEST")"; NAME="${NAME:-$CAP}"
RETAIN="$(toml_get backup_retain "$MANIFEST")"; RETAIN="${RETAIN:-14}"
PATHS=()
while IFS= read -r line; do [ -n "$line" ] && PATHS+=("$line"); done < <(toml_array backup_paths "$MANIFEST")
CPATHS=()
while IFS= read -r line; do [ -n "$line" ] && CPATHS+=("$line"); done < <(toml_array backup_container_paths "$MANIFEST")

# These strings become source paths and tar member names. The manifest gate catches
# bad tracked declarations; the runtime check is still required because backup must not
# depend on CI having run before it touches private state.
safe_relative() {
  case "$1" in ''|/*|..|../*|*/../*) return 1 ;; *) return 0 ;; esac
}
for p in ${PATHS[@]+"${PATHS[@]}"}; do
  safe_relative "$p" || { echo "backup.sh: unsafe backup path declaration: $p" >&2; exit 1; }
done
if [ -n "$SQLITE_REL" ]; then
  safe_relative "$SQLITE_REL" || { echo "backup.sh: unsafe SQLite path declaration: $SQLITE_REL" >&2; exit 1; }
  [ "${#CPATHS[@]}" -eq 0 ] && [ "$PG_DUMPALL" != "true" ] || {
    echo "backup.sh: backup_sqlite cannot be combined with container paths or pg_dumpall in one coherent snapshot" >&2
    exit 1
  }
fi
for cp in ${CPATHS[@]+"${CPATHS[@]}"}; do
  case "$cp" in /*) ;; *) echo "backup.sh: container backup path must be absolute: $cp" >&2; exit 1 ;; esac
  safe_relative "${cp#/}" || { echo "backup.sh: unsafe container backup path declaration: $cp" >&2; exit 1; }
done

# The container runtime, resolved once — pg_dumpall and backup_container_paths both need it.
runtime_exec() {
  case "$AXON_CONTAINER_RUNTIME" in
    apple-container) echo "container exec $NAME" ;;
    docker|podman)   echo "$AXON_CONTAINER_RUNTIME exec $NAME" ;;
    *) echo "backup.sh: unsupported container_runtime '$AXON_CONTAINER_RUNTIME'" >&2; exit 1 ;;
  esac
}

# Where a capability's backups land is a fact about the machine, not about the
# capability: the Mac ships vaultwarden's tarballs to the Pi, and the Pi cannot ship
# them to itself (ISA anti-claim A3). Same override home as service-runner.sh's `ports`.
if [ -f "$AXON_MACHINE_TOML" ]; then
  TARGET_OVERRIDE="$(toml_get_in "capability.$CAP" backup_target "$AXON_MACHINE_TOML")"
  if [ -n "$TARGET_OVERRIDE" ]; then TARGET_ID="$TARGET_OVERRIDE"; fi
fi

if [ "$STREAM" -eq 0 ]; then
  [ -n "$TARGET_ID" ] || { echo "backup.sh: $CAP has no backup_target in service.toml" >&2; exit 1; }
fi
# A capability declares its data as host paths, paths read from inside the container, an
# in-container pg dump, or a mix. Postgres has no host path at all under apple-container
# (managed volume), so requiring backup_paths would make it permanently un-backupable.
[ "${#PATHS[@]}" -gt 0 ] || [ "${#CPATHS[@]}" -gt 0 ] || [ "$PG_DUMPALL" = "true" ] || {
  echo "backup.sh: $CAP declares no backup_paths, backup_container_paths or backup_pg_dumpall in service.toml" >&2; exit 1; }

# Resolve every local precondition before a SQLite capability is held. A missing source
# or verifier discovered after stop would create avoidable downtime, and shipping an
# unverified credential database is not an acceptable degraded mode.
for p in ${PATHS[@]+"${PATHS[@]}"}; do
  [ -e "$AXON_PERSONAL_ROOT/$p" ] || {
    echo "backup.sh: declared backup path is missing: $p" >&2; exit 1; }
done
if [ "${#PATHS[@]}" -gt 0 ]; then
  command -v rsync >/dev/null 2>&1 || {
    echo "backup.sh: rsync is required to stage declared backup paths" >&2; exit 1; }
fi
if [ -n "$SQLITE_REL" ]; then
  [ -f "$AXON_PERSONAL_ROOT/$SQLITE_REL" ] || {
    echo "backup.sh: declared SQLite database is missing: $SQLITE_REL" >&2; exit 1; }
  command -v sqlite3 >/dev/null 2>&1 || {
    echo "backup.sh: sqlite3 is required to verify a cold SQLite backup" >&2; exit 1; }
fi

# Remote coordinates are required only by push mode. Stream mode deliberately has no
# destination knowledge: the authenticated caller owns transport and encrypted storage.
if [ "$STREAM" -eq 0 ]; then
  SYS_LOCAL="$AXON_PERSONAL_ROOT/config/systems.local.toml"
  [ -f "$SYS_LOCAL" ] || { echo "backup.sh: no $SYS_LOCAL (target coordinates)" >&2; exit 1; }
  HOST="$(toml_get_in "$TARGET_ID" host "$SYS_LOCAL")"
  SSH_USER="$(toml_get_in "$TARGET_ID" ssh_user "$SYS_LOCAL")"
  REMOTE_ROOT="$(toml_get_in "$TARGET_ID" backup_root "$SYS_LOCAL")"
  { [ -n "$HOST" ] && [ -n "$SSH_USER" ] && [ -n "$REMOTE_ROOT" ]; } || {
    echo "backup.sh: target '$TARGET_ID' needs host + ssh_user + backup_root in $SYS_LOCAL" >&2; exit 1; }
fi

# Finish the toolchain preflight before creating staging state or acquiring a service
# hold. Conditional commands are checked only for contracts that use them.
require_command() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "backup.sh: $1 is required for this backup contract" >&2
    exit 1
  }
}
for command_name in awk basename cat date dirname mkdir rm tar tr wc; do
  require_command "$command_name"
done
if [ "$STREAM" -eq 0 ]; then require_command ssh; fi
if ! command -v shasum >/dev/null 2>&1 && ! command -v sha256sum >/dev/null 2>&1; then
  echo "backup.sh: shasum or sha256sum is required to record archive identity" >&2
  exit 1
fi
if [ -n "$SQLITE_REL" ]; then
  require_command cp
  require_command head
  [ -x "$TOOLS_DIR/service-runner.sh" ] || {
    echo "backup.sh: executable service-runner.sh is required for a coherent SQLite snapshot" >&2
    exit 1
  }
fi
if [ "${#CPATHS[@]}" -gt 0 ] || [ "$PG_DUMPALL" = "true" ]; then
  read -r -a PREFLIGHT_EXEC <<< "$(runtime_exec)"
  require_command "${PREFLIGHT_EXEC[0]}"
fi
if [ "$PG_DUMPALL" = "true" ]; then
  require_command head
  require_command sed
fi

TS="$(date -u +%Y%m%dT%H%M%SZ)"
STAGE="$AXON_PERSONAL_ROOT/backup/staging/$CAP"
TARBALL="$AXON_PERSONAL_ROOT/backup/staging/$CAP-$TS.tar.gz"
REMOTE_DIR=""
if [ "$STREAM" -eq 0 ]; then REMOTE_DIR="$REMOTE_ROOT/$CAP"; fi
CAPABILITY_HELD=0

resume_capability() {
  [ "$CAPABILITY_HELD" -eq 1 ] || return 0
  if "$TOOLS_DIR/service-runner.sh" resume "$CAP" >/dev/null; then
    CAPABILITY_HELD=0
    return 0
  fi
  echo "backup.sh: CRITICAL: failed to resume '$CAP'; the maintenance hold remains active" >&2
  return 1
}

cleanup() { rm -rf "$STAGE" "$TARBALL"; }
on_exit() {
  status=$?
  trap - EXIT HUP INT TERM
  resume_failed=0
  resume_capability || resume_failed=1
  cleanup
  if declare -F close_master >/dev/null 2>&1; then close_master; fi
  if [ "$status" -eq 0 ] && [ "$resume_failed" -ne 0 ]; then status=1; fi
  exit "$status"
}
trap on_exit EXIT
interrupted() {
  signal="$1"
  trap - "$signal"
  case "$signal" in
    HUP)  exit 129 ;;
    INT)  exit 130 ;;
    TERM) exit 143 ;;
  esac
}
trap 'interrupted HUP' HUP
trap 'interrupted INT' INT
trap 'interrupted TERM' TERM

rm -rf "$STAGE"; mkdir -p "$STAGE"

stage_host_paths() {
  # Guard the expansion: under `set -u`, bash 3.2 treats "${EMPTY[@]}" as unbound.
  # A pg_dumpall-only capability legitimately declares zero paths.
  for p in "${PATHS[@]}"; do
    src="$AXON_PERSONAL_ROOT/$p"
    dest="$STAGE/$p"; mkdir -p "$(dirname "$dest")"
    rsync -a "$src/" "$dest/"
  done
}

echo "→ staging $CAP data"
# A SQLite capability is one stopped-state snapshot: acquire the hold BEFORE the first
# path copy, keep it through cold-copy integrity checking, then resume before compression
# or network I/O. This keeps attachments/config and their database from describing
# different moments.
if [ -n "$SQLITE_REL" ]; then
  echo "→ coherent cold snapshot: $CAP offline while declared paths and SQLite are staged"
  # Set before stop: resume is idempotent, so even a stop command that creates the hold
  # and then exits non-zero cannot strand the capability silently.
  CAPABILITY_HELD=1
  "$TOOLS_DIR/service-runner.sh" stop "$CAP"
fi
if [ "${#PATHS[@]}" -gt 0 ]; then
  stage_host_paths
fi

# Paths read from INSIDE the container, for data the host user cannot read.
#
# home-assistant runs as root and owns .storage that way — the auth store, the user
# database, core.config_entries. A host-side rsync as the invoking user hits Permission
# denied on exactly those files: rsync exits 23 and the run aborts, which is the correct
# outcome and also means the capability simply cannot be backed up from the host at all.
#
# Same reasoning as backup_pg_dumpall directly below: when the host cannot read the data
# correctly, read it where it lives instead. Staged as a tar rather than extracted, so
# ownership and modes survive the round trip — restoring a root-owned auth store as the
# invoking user is the kind of quiet degradation that only surfaces during a restore.
if [ "${#CPATHS[@]}" -gt 0 ]; then
  read -r -a EXEC <<< "$(runtime_exec)"
  for cp in "${CPATHS[@]}"; do
    safe="$(printf '%s' "${cp#/}" | tr '/' '_')"
    out="$STAGE/container-$safe.tar.gz"
    echo "  from container '$NAME': $cp"
    "${EXEC[@]}" tar czf - -C "$cp" . > "$out" || {
      echo "backup.sh: reading $cp from container '$NAME' failed — is it running?" >&2; exit 1; }
    [ -s "$out" ] || {
      echo "backup.sh: $cp produced an empty archive — refusing to ship it as a backup" >&2; exit 1; }
    echo "    $(wc -c < "$out" | tr -d ' ') bytes"
  done
fi

# Overwrite the raw-copied DB with a COLD copy taken while nothing has it open.
#
# The obvious version of this -- `sqlite3 <live-db> ".backup"` from the host -- is a
# trap, and it cost a live vaultwarden outage on 2026-07-25. SQLite's WAL mode
# coordinates readers and writers through a shared-memory index (the -shm file) and
# requires every connection to sit on the same host; a virtiofs bind mount between host
# and container is exactly the case SQLite excludes. The host-side open invalidated the
# container's -shm and every subsequent write came back as `disk I/O error` until the
# capability was stopped and the stale -shm/-wal cleared.
#
# So: hold the capability down, copy the file cold, resume. A clean shutdown
# checkpoints the WAL into the main DB, which is what makes a plain `cp` correct here --
# the -wal/-shm are copied too only if the runtime left them behind. The alternatives
# were weighed and rejected: no sqlite3 in the vaultwarden image (rules out the
# pg_dumpall-style in-container dump), and vaultwarden's admin backup endpoint would
# mean provisioning an ADMIN_TOKEN and exposing the admin panel just to take a copy.
# Cost of this approach: seconds of downtime per backup. Correctness beats uptime for
# the thing that holds every other credential.
if [ -n "$SQLITE_REL" ]; then
  echo "→ cold sqlite copy: $SQLITE_REL"
  db_src="$AXON_PERSONAL_ROOT/$SQLITE_REL"
  db_dst="$STAGE/$SQLITE_REL"; mkdir -p "$(dirname "$db_dst")"
  rm -f "$db_dst" "$db_dst-wal" "$db_dst-shm"

  cp "$db_src" "$db_dst"
  # `|| true`: a missing sidecar is the NORMAL case after a clean stop (the WAL is
  # checkpointed away), and under `set -e` a false test as the loop's last command
  # would abort the whole backup.
  for side in wal shm; do
    [ -f "$db_src-$side" ] && cp "$db_src-$side" "$db_dst-$side" || true
  done

  # A cold copy is only worth shipping if it opens. Verified on the COPY, never the
  # live file -- reading the original from here is the whole bug this replaced.
  ic="$(sqlite3 "$db_dst" 'pragma integrity_check;' 2>&1 | head -1)"
  [ "$ic" = "ok" ] || { echo "backup.sh: staged SQLite copy failed integrity_check: $ic" >&2; exit 1; }
  echo "  integrity_check: ok"

  # Explicit resume before compression/shipment minimizes downtime. The EXIT trap keeps
  # a second, loud attempt armed until this succeeds and clears CAPABILITY_HELD.
  resume_capability || exit 1
fi

# Postgres: a logical dump taken INSIDE the container is the only consistent
# option — the data dir is a managed volume with no host path (apple-container),
# and raw-copying a live cluster risks a torn backup either way. pg_dumpall (not
# pg_dump) so roles and every database land in one restorable file.
if [ "$PG_DUMPALL" = "true" ]; then
  ENV_FILE_REL="$(toml_get env_file "$MANIFEST")"
  PGUSER="postgres"
  if [ -n "$ENV_FILE_REL" ] && [ -f "$AXON_PERSONAL_ROOT/$ENV_FILE_REL" ]; then
    v="$(sed -n 's/^POSTGRES_USER=//p' "$AXON_PERSONAL_ROOT/$ENV_FILE_REL" | head -1)"
    [ -n "$v" ] && PGUSER="$v"
  fi

  read -r -a EXEC <<< "$(runtime_exec)"

  echo "→ pg_dumpall from container '$NAME' (user: $PGUSER)"
  # Fail loud on an unreachable/stopped container instead of shipping a 0-byte
  # dump that looks like a successful backup.
  "${EXEC[@]}" pg_isready -U "$PGUSER" >/dev/null 2>&1 || {
    echo "backup.sh: postgres container '$NAME' not ready — refusing to ship an empty dump" >&2; exit 1; }
  "${EXEC[@]}" pg_dumpall -U "$PGUSER" > "$STAGE/pg_dumpall.sql"
  [ -s "$STAGE/pg_dumpall.sql" ] || {
    echo "backup.sh: pg_dumpall produced an empty file — aborting" >&2; exit 1; }
  echo "  dump: $(wc -c < "$STAGE/pg_dumpall.sql" | tr -d ' ') bytes"

  # What the dump is supposed to contain, recorded now so restore can check that
  # it actually came back. A non-empty .sql file only proves pg_dumpall wrote
  # something; it says nothing about whether the rows survive a replay.
  PG_COUNT_SQL="SELECT count(*)::text || '|' || coalesce(sum((xpath('/row/c/text()', query_to_xml(format('select count(*) as c from %I.%I', n.nspname, c.relname), false, true, '')))[1]::text::bigint), 0)::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind = 'r' AND n.nspname NOT IN ('pg_catalog', 'information_schema');"
  PG_TABLES=0
  PG_ROWS=0
  for db in $("${EXEC[@]}" psql -U "$PGUSER" -d postgres -Atqc \
                "select datname from pg_database where datallowconn and not datistemplate order by 1;"); do
    pair="$("${EXEC[@]}" psql -U "$PGUSER" -d "$db" -Atqc "$PG_COUNT_SQL")"
    PG_TABLES=$((PG_TABLES + ${pair%%|*}))
    PG_ROWS=$((PG_ROWS + ${pair##*|}))
  done
  echo "  contents: $PG_TABLES user table(s), $PG_ROWS row(s)"
fi

# The archive describes the historical backup contract it was made from. Restore checks
# it against the tracked service.toml before using it, so a changed contract requires
# checking out the matching Axon revision instead of guessing. All values originate in
# tracked manifests and contain no host, user, target, receipt, or private path.
write_toml_array() { # <key> [values...]
  key="$1"; shift
  printf '%s = [' "$key"
  sep=""
  for value in "$@"; do
    case "$value" in
      *'"'*|*$'\n'*)
        echo "backup.sh: backup manifest value cannot contain quotes or newlines" >&2
        exit 1
        ;;
    esac
    printf '%s"%s"' "$sep" "$value"
    sep=", "
  done
  printf ']\n'
}

IMAGE="$(toml_get image "$MANIFEST")"
TAG="$(toml_get tag "$MANIFEST")"
{
  printf 'format = "1"\n'
  printf 'capability = "%s"\n' "$CAP"
  printf 'created_at = "%s"\n' "$TS"
  printf 'image = "%s"\n' "$IMAGE"
  printf 'tag = "%s"\n' "$TAG"
  printf 'sqlite = "%s"\n' "$SQLITE_REL"
  printf 'pg_dumpall = "%s"\n' "$PG_DUMPALL"
  # Optional and additive: absent on archives taken before 2026-08-02, which
  # restore.sh handles rather than rejecting. No format bump for that reason.
  if [ "$PG_DUMPALL" = "true" ]; then
    printf 'pg_user_tables = "%s"\n' "$PG_TABLES"
    printf 'pg_total_rows = "%s"\n' "$PG_ROWS"
  fi
  write_toml_array backup_paths ${PATHS[@]+"${PATHS[@]}"}
  write_toml_array backup_container_paths ${CPATHS[@]+"${CPATHS[@]}"}
} > "$STAGE/axon-backup.toml"

echo "→ tarball $CAP-$TS.tar.gz"
# COPYFILE_DISABLE stops macOS tar emitting AppleDouble ._ files / xattrs; a
# no-op on Linux tar, so the tool stays portable.
COPYFILE_DISABLE=1 tar czf "$TARBALL" -C "$STAGE" .

sha256_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    echo "backup.sh: need shasum or sha256sum to record archive identity" >&2
    exit 1
  fi
}
LOCAL_SHA256="$(sha256_file "$TARBALL")"

# `tar czf` exiting 0 is not the same as a complete archive. //:backup_test once observed a
# streamed archive arriving without its manifest member on a Linux runner -- same workflow, same
# runner image, same tree, green on a re-run -- and the producer exited 0 both times. A backup
# that is silently short is the one outcome a backup tool must never have, so completeness is the
# producer's own claim from here rather than something a consumer discovers later.
verify_archive() {  # <path>
  local members
  if ! members="$(tar -tzf "$1" 2>/dev/null)"; then
    echo "backup.sh: $1 is not a readable gzip archive — refusing to ship it" >&2
    return 1
  fi
  case "$members" in
    *./axon-backup.toml*) ;;
    *)
      echo "backup.sh: $1 is missing ./axon-backup.toml — incomplete, refusing to ship it" >&2
      return 1
      ;;
  esac
}
verify_archive "$TARBALL"

# Pull mode stops here. The source capability has already resumed, the archive is
# complete, and the EXIT trap removes all staging state after the caller consumes it.
# The caller must pipe stdout directly into an encrypted repository; redirecting it to
# a file would create the plaintext intermediate this mode exists to avoid.
if [ "$STREAM" -eq 1 ]; then
  echo "→ stream $CAP archive ($LOCAL_SHA256)"
  # cat's status is checked explicitly rather than left to `set -e`, so the failure carries a
  # sentence. A consumer that goes away mid-stream, or a far end that runs out of room, leaves
  # the caller holding a truncated archive; exiting 0 there is how a short backup becomes one
  # nobody notices until a restore.
  if ! cat "$TARBALL" >&3; then
    echo "backup.sh: streaming the $CAP archive failed part-way — the consumer's copy is incomplete" >&2
    exit 1
  fi
  exit 0
fi

# One shared ssh connection for every remote op (mkdir + ship + verify + prune) via
# ControlMaster — so the Bitwarden agent asks for approval ONCE, not per-command.
CTRL="/tmp/axon-backup-$CAP-$$.sock"
SSHM=(-o ControlMaster=auto -o "ControlPath=$CTRL" -o ControlPersist=30)
close_master() { ssh -O exit -o "ControlPath=$CTRL" "$SSH_USER@$HOST" 2>/dev/null || true; }

echo "→ ship → $SSH_USER@$HOST:$REMOTE_DIR/  (approve the Bitwarden agent prompt — ONCE)"
ssh "${SSHM[@]}" "$SSH_USER@$HOST" "mkdir -p '$REMOTE_DIR'"

# Shipped by streaming into `cat`, not with rsync or scp, and that is deliberate.
# A backup target is the one machine you want to grant as little as possible, and a
# hardened one grants exactly a shell. Synology DSM refuses `rsync --server` and the
# sftp subsystem to any account outside the administrators group -- key auth succeeds,
# then the exec request is denied (measured on a real backup target). The choices were
# to make the backup account an administrator or to stop needing a remote binary; the
# second one is not a workaround, because rsync buys nothing here anyway. Every run ships
# exactly one new tarball, so there is no delta to compute and nothing to resume.
#
# Written to a .part name and renamed only after the byte count matches. A truncated
# transfer that lands under the final name is worse than a failed one: it looks like a
# backup, and you find out otherwise on the day you need it.
REMOTE_NAME="$(basename "$TARBALL")"
REMOTE_PART="$REMOTE_DIR/.$REMOTE_NAME.part"
ssh "${SSHM[@]}" "$SSH_USER@$HOST" "cat > '$REMOTE_PART'" < "$TARBALL"

LOCAL_BYTES="$(wc -c < "$TARBALL" | tr -d ' ')"
REMOTE_BYTES="$(ssh "${SSHM[@]}" "$SSH_USER@$HOST" "wc -c < '$REMOTE_PART'" | tr -d ' ')"
if [ "$LOCAL_BYTES" != "$REMOTE_BYTES" ]; then
  ssh "${SSHM[@]}" "$SSH_USER@$HOST" "rm -f '$REMOTE_PART'"
  echo "backup.sh: short write — sent $LOCAL_BYTES bytes, target holds $REMOTE_BYTES. Nothing was kept." >&2
  exit 1
fi
ssh "${SSHM[@]}" "$SSH_USER@$HOST" "mv '$REMOTE_PART' '$REMOTE_DIR/$REMOTE_NAME'"
echo "  shipped $LOCAL_BYTES bytes, size verified on the target"

RETENTION_APPLIED=true
if [ "$NO_PRUNE" -eq 1 ]; then
  RETENTION_APPLIED=false
  echo "→ retain every prior archive (--no-prune)"
else
  echo "→ prune remote to last $RETAIN"
  ssh "${SSHM[@]}" "$SSH_USER@$HOST" "ls -1t '$REMOTE_DIR'/$CAP-*.tar.gz 2>/dev/null | tail -n +$((RETAIN + 1)) | xargs -r rm -f"
fi

# Written only here, after the remote byte count matched and the .part was moved into
# place. A receipt that exists therefore means a backup landed, not that a run started
# -- which is the whole point of having one: cleanup() deletes the staging dir and the
# tarball, so without this nothing local can answer "when was the last successful
# backup" and reading the answer off the target costs an ssh round trip plus an
# unlocked vault agent. Instance state, so it lives in the overlay, not in Axon.
# Deliberately NOT backup/local/.last-backup-timestamp: that path belongs to a retired
# ad-hoc script and reusing it would inherit its meaning.
# Hand-built JSON rather than jq: bash 3.2 everywhere, no new dependency for eight fields
# whose values are all either numeric or shell-controlled.
RECEIPT_DIR="$AXON_PERSONAL_ROOT/backup/receipts"
mkdir -p "$RECEIPT_DIR"
contents=""
[ "${#PATHS[@]}" -gt 0 ] && contents="paths"
[ "${#CPATHS[@]}" -gt 0 ] && contents="${contents:+$contents+}container_paths"
[ "$PG_DUMPALL" = "true" ] && contents="${contents:+$contents+}pg_dumpall"
cat > "$RECEIPT_DIR/$CAP.json" <<RECEIPT
{
  "capability": "$CAP",
  "completed_at": "$TS",
  "target": "$TARGET_ID",
  "tarball": "$CAP-$TS.tar.gz",
  "bytes": $LOCAL_BYTES,
  "sha256": "$LOCAL_SHA256",
  "contents": "$contents",
  "retention_applied": $RETENTION_APPLIED
}
RECEIPT
echo "  receipt → backup/receipts/$CAP.json"

echo "✓ backup complete: $CAP-$TS.tar.gz → $SSH_USER@$HOST:$REMOTE_DIR/"
