#!/bin/bash
# Planted-tree regression tests for check-doc-self-references.sh. The gate walks
# `find . -name '*.md'` from its working directory, so each case is a throwaway
# tree the test cds into — no repo docs, and the red path is proven rather than
# assumed. Both bugs below were live on 2026-08-02 and reproduced here first.
set -uo pipefail

if [ -n "${TEST_SRCDIR:-}" ]; then
  CHECK="$TEST_SRCDIR/$TEST_WORKSPACE/tools/check-doc-self-references.sh"
else
  CHECK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-doc-self-references.sh"
fi

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

fails=0

# Each case gets its own tree, named so a failure says which property broke.
tree() { # tree <case-name> -> echoes the fresh tree root
  local root="$SCRATCH/$1"
  rm -rf "$root"
  mkdir -p "$root"
  printf '%s' "$root"
}

run() { # run <tree-root> -> gate output on stdout+stderr, exit status in $status
  local root="$1"
  out=$(cd "$root" && "$CHECK" 2>&1)
  status=$?
}

expect_pass() { # expect_pass <description> <tree-root>
  run "$2"
  if [ "$status" -ne 0 ]; then
    echo "FAIL: $1 should pass, got exit $status:"
    printf '%s\n' "$out" | sed 's/^/    /'
    fails=$((fails + 1))
  fi
}

expect_fail_with() { # expect_fail_with <description> <tree-root> <substring>
  run "$2"
  if [ "$status" -eq 0 ] || ! printf '%s' "$out" | grep -qF "$3"; then
    echo "FAIL: $1 should fail and name $3, got exit $status:"
    printf '%s\n' "$out" | sed 's/^/    /'
    fails=$((fails + 1))
  fi
}

# The class the gate exists for: a doc citing its own path as a cross-reference.
root=$(tree cites-self)
mkdir -p "$root/capabilities/x"
printf 'See `capabilities/x/README.md` for the manifest.\n' > "$root/capabilities/x/README.md"
expect_fail_with "a doc citing its own path" "$root" "capabilities/x/README.md"

# The ordinary green path, so a gate that fails everything cannot pass this file.
root=$(tree clean)
mkdir -p "$root/capabilities/x"
printf 'The manifest lives beside this file.\n' > "$root/capabilities/x/README.md"
expect_pass "a doc that cites nothing" "$root"

# Bug 1 (silent green): the unquoted `find` substitution word-split any path
# containing a space, so this file was never checked and the run still exited 0.
root=$(tree whitespace-path)
mkdir -p "$root/a dir"
printf 'See `a dir/notes.md` for the rest.\n' > "$root/a dir/notes.md"
expect_fail_with "a self-citing doc whose path contains a space" "$root" "a dir/notes.md"

# ...and the same path with nothing to find must still come out green, so the
# fix is "the file is read", not "any spaced path fails".
root=$(tree whitespace-path-clean)
mkdir -p "$root/a dir"
printf 'Nothing self-referential here.\n' > "$root/a dir/notes.md"
expect_pass "a clean doc whose path contains a space" "$root"

# Bug 2 (false positive): a fixed-string match is a substring match, so a doc
# mentioning a longer path that happens to start with its own was reported.
root=$(tree superstring-suffix)
mkdir -p "$root/a"
printf 'The pre-rewrite copy is a/b.md.bak, kept for the diff.\n' > "$root/a/b.md"
expect_pass "a doc mentioning a longer path that extends its own" "$root"

# Same class, other direction: its own path as the tail of a sibling's path.
root=$(tree superstring-prefix)
mkdir -p "$root/a"
printf 'Not to be confused with docs/a/b.md, which is generated.\n' > "$root/a/b.md"
expect_pass "a doc mentioning a longer path that ends with its own" "$root"

# The documented skip: a root-level file's repo-relative path is its basename,
# which appears in prose all the time.
root=$(tree root-level)
printf 'ARCHITECTURE.md is generated from the manifests.\n' > "$root/ARCHITECTURE.md"
mkdir -p "$root/a"
printf 'nothing here\n' > "$root/a/keep.md"
expect_pass "a root-level doc naming its own basename" "$root"

# A broken data list must be loud, not an empty green run.
root=$(tree no-docs)
expect_fail_with "an empty tree" "$root" "no *.md files materialized"

if [ "$fails" -gt 0 ]; then
  echo "doc self-reference gate: $fails check(s) failed"
  exit 1
fi
echo "doc self-reference gate: all checks passed"
