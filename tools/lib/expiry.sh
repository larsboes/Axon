# tools/lib/expiry.sh — reading the dates on Axon's accepted-finding policies.
#
# Every accepted finding in this repo is accepted UNTIL a date. Today that means
# osv-scanner.toml's `ignoreUntil`; trivy-ignore/*.txt carried `exp:YYYY-MM-DD` per entry
# until that directory emptied on 2026-08-28 (52aa8c5). The scanner honours its own dates
# and never announces one. So a policy lapses mid-week, findings that were a reviewed
# decision yesterday are a build failure today, and nothing connects the two. On 2026-08-05
# two of the three image policies were ten days from exactly that, with no upstream fix
# available to reach for.
#
# Axon does not re-implement the format's matching rules. The scanner stays the
# authority on what an expired entry MEANS, and if the two readings ever disagree its is
# the one that decides the scan. This reads only the dates already written in the policy
# file, so a date can arrive as notice beforehand instead of a red build after.
#
# Extracted from tools/audit for two reasons. First, the classification can then be tested
# directly, without standing up a fixture root and mocking a scanner onto PATH for every
# edge case. Second,
# the inline version declared nothing `local` and clobbered `d`, `label` and `total` --
# names tools/audit uses elsewhere. Nothing broke, purely because of the order the calls
# happened to run in, which is precisely the trap this repo already warns about leaving for
# whoever adds the next check.
#
# Portable shell, bash 3.2 compatible (README.md#portable-shell).

# day_epoch <YYYY-MM-DD> — that date at midnight UTC, in epoch seconds. GNU date first,
# BSD/macOS fallback, the shape tools/upstream-checker's epoch_of also used before that
# script was retired on 2026-08-28. Both forms are
# handed an explicit 00:00:00 because BSD `date -j -f %Y-%m-%d` fills the missing time from
# the current clock, which moves the answer by a day depending on when the audit ran.
day_epoch() {
  if date --version >/dev/null 2>&1; then
    date -u -d "$1 00:00:00" +%s 2>/dev/null
  else
    date -j -u -f "%Y-%m-%d %H:%M:%S" "$1 00:00:00" +%s 2>/dev/null
  fi
}

# days_until <YYYY-MM-DD> — whole days from today to that date; negative once past.
# Non-zero when the date cannot be parsed, so a malformed entry is reported as undated
# rather than silently becoming a number.
#
# Today is floored to midnight UTC for the same reason day_epoch pins the time: a policy
# expiring today must read as 0 days regardless of whether the audit runs at 09:00 or 23:00.
days_until() {
  local target today
  target="$(day_epoch "$1")"
  today="$(day_epoch "$(date -u +%Y-%m-%d)")"
  { [ -n "$target" ] && [ -n "$today" ]; } || return 1
  echo $(( (target - today) / 86400 ))
}

# expiry_dates_osv <file> / osv_ignored_count <file> — the two facts about the
# [[IgnoredVulns]] blocks in osv-scanner.toml, where ignoreUntil is a bare unquoted date.
#
# expiry_dates_trivy read the `exp:` dates out of trivy-ignore/*.txt beside these until
# 2026-08-28. That directory emptied in 52aa8c5, so the function had no file to read and
# no caller; it is deleted rather than kept warm for a format nothing writes.
#
# Undated entries are deliberately NOT emitted. The caller knows the total and derives the
# undated count from the difference, which is how "how many of these never expire at all"
# stays answerable instead of quietly reading as zero.
expiry_dates_osv() {
  grep -E '^[[:space:]]*ignoreUntil[[:space:]]*=' "$1" \
    | grep -oE '[0-9]{4}-[0-9]{2}-[0-9]{2}'
}
osv_ignored_count() { grep -cE '^[[:space:]]*\[\[IgnoredVulns\]\]' "$1" | tr -d ' '; }

# expiry_note <label> <total entries> <warn days> <dates…> — one status line for a policy.
#   Exit 0 = nothing owed · 1 = needs attention · 2 = already lapsed.
#
# The window arrives as a parameter rather than being read from axon.toml in here, so this
# file decides only what the status IS while the caller owns both the configuration and
# what a status costs. That is also what lets the tests drive every boundary without
# planting a manifest. lib/version.sh's drift_note was built to the same shape until PRD
# Q41 deleted it on 2026-08-28; this is the surviving instance of the pattern.
#
# Dates arrive word-split into argv rather than on stdin because a `while read` on the
# right of a pipe runs in a subshell under bash 3.2, and every counter incremented inside
# it would be discarded at the loop's end -- reporting zero over a list that had some.
expiry_note() {
  local label="$1" total="$2" warn="$3"
  shift 3
  local d days nearest="" nearest_days=0 expired=0 dated=0 undated

  for d in "$@"; do
    days="$(days_until "$d")" || continue
    dated=$((dated + 1))
    [ "$days" -lt 0 ] && expired=$((expired + 1))
    if [ -z "$nearest" ] || [ "$days" -lt "$nearest_days" ]; then
      nearest="$d"; nearest_days="$days"
    fi
  done
  undated=$((total - dated))

  # An undated exception is the worst state rather than the quiet one: nothing will ever
  # prompt a re-decision on it, so it outranks a date that is merely close.
  if [ -z "$nearest" ]; then
    printf '%s — %s ID(s), no entry carries a date — nothing will ever prompt a re-decision\n' \
      "$label" "$total"
    return 1
  fi
  if [ "$expired" -gt 0 ]; then
    printf '%s — %s ID(s), ✗ EXPIRED %s (%sd ago; %s of %s dated entries) — the scanner has stopped honouring these\n' \
      "$label" "$total" "$nearest" "$((0 - nearest_days))" "$expired" "$dated"
    return 2
  fi
  if [ "$undated" -gt 0 ]; then
    printf '%s — %s ID(s), nearest expiry %s (%sd left), ⚠ %s undated and therefore permanent\n' \
      "$label" "$total" "$nearest" "$nearest_days" "$undated"
    return 1
  fi
  if [ "$nearest_days" -le "$warn" ]; then
    printf '%s — %s ID(s), ⚠ expires %s (%sd left, inside the %sd re-decision window)\n' \
      "$label" "$total" "$nearest" "$nearest_days" "$warn"
    return 1
  fi
  printf '%s — %s ID(s), expires %s (%sd left)\n' "$label" "$total" "$nearest" "$nearest_days"
  return 0
}
