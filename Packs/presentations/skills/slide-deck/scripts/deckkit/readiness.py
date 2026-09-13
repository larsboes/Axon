"""The argument skeleton — what `deck readiness` prints, and why it is not a check.

`check` sees structure and `render` sees layout. Neither sees the **argument**. A deck
can pass both and still answer the wrong question, which is exactly the finding a
defence turns on: the abstraction the examiner asked for, the contribution that was
never claimed, the positioning that read the literature as design input.

This module extracts the argument — the ordered assertions, the sentences slides exist
to deliver, the closing verdict, the question each backup slide answers — so a reviewer
can read it in one screen and answer the five questions in
`references/defence-readiness.md`.

It is deliberately **not** a checker. Whether an argument is any good is not a rule, and
a script that guessed would be trusted. It extracts; the review is the reviewer's.
"""

from __future__ import annotations

from .deck import Deck

#: Column width for the slide kind (`title`, `content`, `backup`, ...).
_KIND = 8
_PAD = " " * (3 + 2 + _KIND + 1)


def report(deck: Deck) -> str:
    entries = deck.argument()
    talk = sum(1 for e in entries if e["kind"] not in ("backup", "backup_divider"))
    lines = [
        f"argument  {deck.name}   "
        f"({len(entries)} slides: {talk} talk, {len(entries) - talk} backup)",
        "",
    ]
    for entry in entries:
        title = " ".join(entry["title"].split())
        claims = list(entry["claims"])
        # A claim-first slide has no title, and the assertion it exists to deliver is
        # its first claim. Promoting it here keeps the "read the titles alone" test
        # usable on a deck that decided to carry no titles at all.
        if not title and claims:
            title = "\u25b8 " + claims.pop(0)
        lines.append(f"{str(entry['number']):>3}  {entry['kind']:<{_KIND}} {title}")
        for claim in claims:
            lines.append(f"{_PAD}> {claim}")
        if entry["kind"] == "backup":
            question = _first_line(entry["notes"])
            if question:
                lines.append(f"{_PAD}? {question}")
    lines += [
        "",
        "Read the titles alone first — they must reconstruct the argument. A slide\n"
        "whose line starts with \u25b8 carries no title; that line is its claim bar.\n"
        "Then answer the five questions in references/defence-readiness.md (abstraction, contribution,\n"
        "positioning, scope, thread) and report findings keyed to the slide numbers above.",
    ]
    return "\n".join(lines)


def _first_line(notes: str) -> str:
    for line in notes.splitlines():
        line = line.strip()
        if line:
            return line
    return ""
