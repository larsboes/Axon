#!/bin/bash
# check-store-transactions.sh — the write-transaction gate (CI: repo gates).
#
# A capability must not begin a transaction with rusqlite's deferred default.
# `conn.transaction()` defers the writer lock, so a transaction that reads
# before it writes has to upgrade into it, and SQLite answers a failed upgrade
# with SQLITE_BUSY at once. `busy_timeout` does not retry that case. Eleven
# services share one file on the deployed host, so the losing side of that race
# is a 400 the operator sees: `POST /feed/<id>/status → {"error":"database is
# locked"}`, which is what failed the Pages build on 2026-09-07.
#
# libs/axon-store/src/lib.rs has taken the lock up front in `migrate_once`
# since it was written, and states the reason above it. The reasoning was never
# migration-specific; `write_transaction` carries it to every writer, and this
# gate is what stops the deferred form coming back by omission — which is how
# it arrived: 29 deferred sites and 10 immediate ones, two of them in the same
# file.
#
# WHAT IS ALLOWED, and why the exemption is a path and not a comment marker:
# libs/axon-store owns both spellings — it defines `write_transaction` and it
# calls `transaction_with_behavior` inside it and inside `migrate_once`. An
# opt-out comment would let any call site declare itself exempt, which is the
# property this gate exists to remove.
#
# A read-only transaction legitimately wants the deferred form. None exists in
# the workspace today. When one does, it belongs behind a named helper in
# libs/axon-store beside this one, so the choice is made once and reviewed once,
# rather than by a call site that looks identical to a writer.
#
# Pure file-based check, same contract as the sibling gates: no git, no network,
# no build. It walks from its working directory rather than from the repository
# root, which is what lets check-store-transactions.test.sh plant a tree and
# prove the red path instead of asserting it.
set -e

# The owner of the primitive. Everything else is a caller.
OWNER="libs/axon-store/"

fail=0
scanned=0
hits=0

while IFS= read -r f; do
  rel="${f#./}"
  case "$rel" in
    "$OWNER"*) continue ;;       # the library that defines the safe form
    */target/*|target/*) continue ;;
  esac

  # A file in the list the tree cannot read means the two disagree. Say so
  # rather than letting grep's exit 2 read as "no match".
  if [ ! -r "$f" ]; then
    echo "FAIL [$rel]: listed but not readable — the file list is stale" >&2
    fail=1
    continue
  fi
  scanned=$((scanned + 1))

  # `.transaction()` with no argument. `transaction_with_behavior(...)` does not
  # match, because the `(` here must be immediately followed by `)`.
  if grep -qE '\.transaction\(\)' "$f"; then
    echo "FAIL [$rel]: begins a deferred transaction — use axon_store::write_transaction:" >&2
    grep -nE '\.transaction\(\)' "$f" | head -5 >&2
    hits=$((hits + 1))
    fail=1
  fi
done < <(find . -name '*.rs' -not -path './target/*' | sort)

if [ "$scanned" -eq 0 ]; then
  echo "FAIL: no *.rs files scanned — the find is broken, not the tree" >&2
  exit 1
fi

if [ "$fail" -ne 0 ]; then
  echo "store transaction check FAILED ($hits file(s))." >&2
  exit 1
fi

echo "store transaction check passed ($scanned Rust file(s) outside $OWNER, none begins a deferred transaction)."
