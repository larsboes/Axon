"""The time budget, read back out of the notes.

`narrative.md` sets the craft: 110-130 words per minute, a running clock in the notes,
and a talk that runs 30 seconds over is a talk that skipped its conclusion. None of that
was enforceable. The notes already carry it in a fixed form:

    ZEIT 4:20-5:10 (50 s)
    ...
    Sprechtext: "..."

so this module reads it back — the sum of the noted budget against the allotted talk,
and the spoken script's word count against what its slot can hold at 125 wpm. It flags
the slides whose script cannot be said in the time the speaker gave them, which is the
drift a rehearsal finds too late.

It is a rehearsal aid, not a check. A long script can be deliberate, and a slide may
carry no clock on purpose; the report says so rather than failing a build.
"""

from __future__ import annotations

import re

from .deck import Deck

#: `ZEIT 4:20-5:10 (50 s)`, `ZEIT 10:00 (Schluss)`, `ZEIT 0:35-0:40 (5 s).`
_CLOCK = re.compile(
    r"ZEIT\s+(\d+):(\d+)(?:\s*[-\u2013\u2014]\s*(\d+):(\d+))?(?:\s*\((\d+)\s*s\))?")
#: The spoken script, in German or straight quotes.
_SCRIPT = (re.compile(r"\u201e(.+?)[\u201c\u201d\"]", re.S),
           re.compile(r"\"(.+?)\"", re.S))

#: Where the spoken script starts. Everything before it is clock, target and notes.
_MARKER = re.compile(r"(?:Sprechtext|Spoken script|Script)\s*:?\s*", re.I)

#: Words per minute the report assumes when judging a slot.
WPM = 125
#: The band from `narrative.md`; outside it, a note.
SLOW, FAST = 90, 140


def report(deck: Deck) -> str:
    rows, budgeted, sparse, long_, no_script = [], 0, 0, 0, 0
    total = 0
    total_words = 0
    end_of_talk = None
    backup = 0

    for entry in deck.argument():
        start, end, seconds, words = parse(entry["notes"])
        if entry["kind"] in ("backup", "backup_divider"):
            backup += 1
            continue
        rows.append((entry, start, end, seconds, words))
        if seconds:
            budgeted += 1
            total += seconds
        if end:
            end_of_talk = end
        if seconds and words is not None:
            total_words += words
            wpm = words / (seconds / 60)
            if wpm > FAST:
                long_ += 1
            elif wpm < SLOW:
                sparse += 1
        elif seconds:
            no_script += 1

    lines = [f"timing  {deck.name}   "
             f"noted {_mmss(total)} across {budgeted} slides   "
             f"declared {_declared(deck)}", ""]
    lines.append(f"{'slide':>5}  {'slot':<13} {'s':>4} {'words':>6} {'wpm':>5}   note")
    for entry, start, end, seconds, words in rows:
        slot = f"{start}\u2013{end}" if start and end else (start or "\u2014")
        secs = str(seconds) if seconds else "\u2014"
        word_cell = str(words) if words is not None else "\u2014"
        wpm = ""
        note = ""
        if seconds and words is not None:
            rate = words / (seconds / 60)
            wpm = f"{rate:.0f}"
            if rate > FAST:
                note = f"{words} words > {int(FAST * seconds / 60)} at {FAST} wpm"
            elif rate < SLOW:
                note = "sparse for the slot"
        elif seconds:
            note = "no quoted script in the notes"
        elif entry["kind"] == "closing":
            note = "not budgeted (closing)"
        number = str(entry["number"])
        lines.append(f"{number:>5}  {slot:<13} {secs:>4} {word_cell:>6} {wpm:>5}   {note}")

    lines.append("")
    if total_words:
        implied = round(total_words / WPM * 60)
        against = f", against a noted {_mmss(total)}" if total else ""
        lines.append(f"spoken script {total_words} words \u2014 {_mmss(implied)} at {WPM} wpm"
                     f"{against}")
    if deck.minutes is not None:
        allowed = int(deck.minutes * 60)
        drift = total - allowed
        verdict = "on budget" if abs(drift) <= 30 else (
            f"{abs(drift)} s {'over' if drift > 0 else 'under'} the allotted "
            f"{deck.minutes:g} min")
        lines.append(f"noted total {_mmss(total)} vs allotted {_mmss(allowed)} \u2014 {verdict}")
    else:
        lines.append(f"noted total {_mmss(total)}"
                     + (f", ending {end_of_talk}" if end_of_talk else "")
                     + " \u2014 no `minutes=` on the Deck, so nothing was compared")
    if backup:
        lines.append(f"{backup} backup slide(s), not budgeted by design")
    flags = []
    if long_:
        flags.append(f"{long_} slide(s) with a script longer than the slot holds")
    if sparse:
        flags.append(f"{sparse} sparse for the slot")
    if no_script:
        flags.append(f"{no_script} with no quoted script")
    lines.append("; ".join(flags) if flags else "every budgeted slide fits its slot")
    return "\n".join(lines)


def parse(notes: str):
    """Return `(start, end, seconds, words)` for one slide's notes. Any may be None."""
    match = _CLOCK.search(notes or "")
    start = end = seconds = None
    if match:
        start = f"{int(match.group(1))}:{int(match.group(2)):02d}"
        if match.group(3) is not None:
            end = f"{int(match.group(3))}:{int(match.group(4)):02d}"
        if match.group(5) is not None:
            seconds = int(match.group(5))
        elif match.group(3) is not None:
            seconds = (_minutes(match.group(3), match.group(4))
                       - _minutes(match.group(1), match.group(2)))
    words = _script_words(notes)
    return start, end, seconds, words


def _script_words(notes: str) -> int | None:
    """Words in the spoken script only.

    The notes also quote the question a contingency answers, and counting every quote
    would inflate the script and turn a useful number into noise. The contract is the one
    `narrative.md` states: a `Sprechtext:` marker, then the script as one quoted block.
    Without the marker, the first quoted block anywhere is taken as the script.
    """
    notes = notes or ""
    marker = _MARKER.search(notes)
    region = notes[marker.end():] if marker else notes
    for pattern in _SCRIPT:
        blocks = pattern.findall(region)
        if blocks:
            return len(blocks[0].split())
    return None


def _minutes(mm: str, ss: str) -> int:
    return int(mm) * 60 + int(ss)


def _mmss(seconds: int) -> str:
    return f"{seconds // 60}:{seconds % 60:02d}"


def _declared(deck: Deck) -> str:
    return f"{deck.minutes:g} min" if deck.minutes is not None else "\u2014"
