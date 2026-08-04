#!/bin/bash
# Planted-repo regression tests for the LifeOS mirror pre-commit hook.
#
# The defect the hook shipped with: it ended with `git add -- resources/backups/lifeos`, so
# a path-scoped `git add` of N files produced a commit of more than N. The content it added
# was always legitimate, which is why it went unnoticed for so long — nothing here checks
# the mirror's bytes, only the commit boundary.
#
# Each case builds a throwaway git repo plus a fake refresh tool, so the assertions are
# about staging behaviour and never about one machine's real LifeOS tree.
set -uo pipefail

if [ -n "${TEST_SRCDIR:-}" ]; then
  HOOK="$TEST_SRCDIR/$TEST_WORKSPACE/tools/templates/hooks/pre-commit-lifeos-mirror.tmpl"
else
  HOOK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/templates/hooks/pre-commit-lifeos-mirror.tmpl"
fi

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

fails=0

# plant <case> — a repo with one committed fixture and one committed mirror file, plus a
# fake refresh tool that dirties the mirror exactly the way the real one would.
plant() {
  local root="$SCRATCH/$1"
  mkdir -p "$root/resources/backups/lifeos/USER"
  cd "$root" || return 1
  git init -q .
  git config user.email t@example.com
  git config user.name Test
  git config commit.gpgsign false
  printf 'original\n' > fixture.txt
  printf 'mirrored v1\n' > resources/backups/lifeos/USER/IDENTITY.md
  git add -A && git commit -q -m init

  # Stands in for tools/mirror-lifeos-user.sh --apply: writes the mirror, prints, exits 0.
  cat > "$root/fake-mirror.sh" <<'TOOL'
#!/bin/bash
printf 'mirrored v2 refreshed\n' > resources/backups/lifeos/USER/IDENTITY.md
echo "1 path(s) updated"
exit 0
TOOL
  chmod +x "$root/fake-mirror.sh"
  mkdir -p .git/hooks
  # Substitute the placeholder the way `mirror-lifeos-user.sh install-hook` does, so these
  # cases run the hook as installed rather than one steered by an env var the real thing
  # never sets. AXON_MIRROR_TOOL still overrides, and case 5 uses it to point at nothing.
  sed -e "s|__MIRROR_TOOL__|$root/fake-mirror.sh|" "$HOOK" > .git/hooks/pre-commit
  chmod +x .git/hooks/pre-commit
  printf '%s' "$root"
}

check() { # check <label> <condition-result> <detail>
  if [ "$2" = "0" ]; then
    echo "  ok   $1"
  else
    echo "  FAIL $1 — $3"
    fails=$((fails + 1))
  fi
}

# ── 1. A path-scoped commit stays path-scoped ───────────────────────────────────────────
root="$(plant scoped)"
cd "$root" || exit 1
printf 'edited\n' > fixture.txt
git add fixture.txt
git commit -q -m "focused" 2>/dev/null
committed="$(git show --stat --format='' --name-only HEAD | grep -c '^')"
[ "$committed" = "1" ]; check "a commit of one staged file contains exactly that file" $? "committed $committed file(s), expected 1"
git show --name-only --format='' HEAD | grep -q '^fixture.txt$'; check "the staged file is the one that landed" $? "fixture.txt absent from the commit"
git show --name-only --format='' HEAD | grep -q 'resources/backups'; [ $? -ne 0 ]; check "no mirror path was added behind the caller" $? "the hook staged a mirror path the caller did not"

# ── 2. The refresh still happens, and is reported ───────────────────────────────────────
grep -q 'refreshed' resources/backups/lifeos/USER/IDENTITY.md; check "the working tree was still refreshed" $? "the hook skipped the refresh entirely"
git status --porcelain -- resources/backups/lifeos | grep -q .; check "the refreshed mirror is left dirty for an explicit commit" $? "mirror drift vanished — it was staged or reverted"

# ── 3. Opting in re-stages, because that is completing the request, not widening it ──────
root2="$(plant optin)"
cd "$root2" || exit 1
printf 'edited\n' > fixture.txt
# Opting in means staging a mirror path that actually carries a change — the shape of a
# real "refresh the mirror and commit it" run. A path staged while byte-identical to HEAD
# is indistinguishable from one never named, since git records no intent beyond the index.
printf 'mirrored v1 hand-edited\n' > resources/backups/lifeos/USER/IDENTITY.md
git add fixture.txt resources/backups/lifeos/USER/IDENTITY.md
git commit -q -m "explicit mirror" 2>/dev/null
git show --name-only --format='' HEAD | grep -q 'resources/backups/lifeos/USER/IDENTITY.md'; check "a caller who staged a mirror path gets it committed" $? "the opted-in mirror path did not land"
grep -q 'refreshed' <(git show HEAD:resources/backups/lifeos/USER/IDENTITY.md); check "and it lands at its refreshed content, not the stale copy" $? "committed the pre-refresh bytes"

# ── 4. A failing refresh warns and never blocks the commit ──────────────────────────────
root3="$(plant failing)"
cd "$root3" || exit 1
printf '#!/bin/bash\necho "boom" >&2\nexit 1\n' > "$root3/fake-mirror.sh"
chmod +x "$root3/fake-mirror.sh"
printf 'edited\n' > fixture.txt
git add fixture.txt
git commit -q -m "refresh fails" 2>/dev/null
[ "$(git rev-list --count HEAD)" = "2" ]; check "a failed refresh does not cost the commit" $? "the commit was blocked"

# ── 5. A missing tool is survivable ─────────────────────────────────────────────────────
root4="$(plant missing)"
cd "$root4" || exit 1
printf 'edited\n' > fixture.txt
git add fixture.txt
AXON_MIRROR_TOOL="$root4/definitely-not-here.sh" git commit -q -m "no tool" 2>/dev/null
[ "$(git rev-list --count HEAD)" = "2" ]; check "a missing refresh tool does not cost the commit" $? "the commit was blocked"

cd /
if [ "$fails" -eq 0 ]; then
  echo "pre-commit-lifeos-mirror: all checks passed"
  exit 0
fi
echo "pre-commit-lifeos-mirror: $fails check(s) failed" >&2
exit 1
