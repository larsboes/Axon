#!/usr/bin/env python3
"""Apply a batch of anchored edits to a document, atomically, loud on a miss.

Editing a long document by hand fails in one specific way: a replacement whose
anchor text no longer matches does nothing, silently, and you find out later when
a section is half-updated and you cannot tell which half. It happens constantly
because anchors go stale -- a previous edit reflowed a line, an editor
auto-formatted a table, a heading gained a word.

So the contract here is: **either every anchor matches and the file is written,
or none does and nothing is touched.** A miss names the anchor that failed and
shows the closest thing in the file, which is almost always enough to see what
changed.

Edits are JSON, so they can be generated, reviewed and re-run:

    [
      {"old": "exact text to find", "new": "replacement"},
      {"old": "another anchor",     "new": "", "why": "delete the stale claim"}
    ]

Usage:
    python3 docedit.py <file> <edits.json>
    python3 docedit.py <file> <edits.json> --dry-run     # report, write nothing
    python3 docedit.py <file> <edits.json> --stdin       # edits on stdin
    python3 docedit.py <file> --check "some anchor"      # is this anchor unique?

Every anchor must match exactly once. An anchor matching twice is ambiguous, and
picking the first occurrence is how the wrong paragraph gets rewritten -- so that
is an error too, not a silent choice.
"""

import argparse
import difflib
import json
import sys


def closest(text, anchor, n=1):
    """The line in `text` most like the anchor's first line.

    A stale anchor is usually one word off, and seeing the real line beside it
    turns a confusing failure into an obvious one.
    """
    probe = anchor.strip().split("\n")[0][:90]
    if not probe:
        return []
    return difflib.get_close_matches(probe, text.split("\n"), n=n, cutoff=0.55)


def apply_edits(text, edits):
    """Returns (new_text, problems). Nothing is applied if problems is non-empty."""
    problems = []
    for i, e in enumerate(edits):
        old = e.get("old")
        if old is None:
            problems.append((i, "edit has no 'old' key", []))
            continue
        count = text.count(old)
        if count == 0:
            problems.append((i, "anchor not found", closest(text, old, 2)))
        elif count > 1:
            problems.append(
                (i, f"anchor matches {count} times — ambiguous, make it longer", [])
            )
    if problems:
        return text, problems

    for e in edits:
        text = text.replace(e["old"], e["new"], 1)
    return text, []


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("file")
    ap.add_argument("edits", nargs="?", help="JSON file of edits")
    ap.add_argument("--stdin", action="store_true", help="read edits from stdin")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--check", help="report whether one anchor matches exactly once")
    a = ap.parse_args()

    try:
        text = open(a.file, encoding="utf-8").read()
    except OSError as e:
        sys.exit(f"cannot read {a.file}: {e}")

    if a.check:
        n = text.count(a.check)
        print(f"matches: {n}")
        if n == 0:
            for c in closest(text, a.check, 3):
                print(f"  closest: {c}")
        sys.exit(0 if n == 1 else 1)

    if a.stdin:
        edits = json.load(sys.stdin)
    elif a.edits:
        edits = json.load(open(a.edits, encoding="utf-8"))
    else:
        sys.exit("give an edits file or --stdin")
    if isinstance(edits, dict):
        edits = [edits]

    new_text, problems = apply_edits(text, edits)

    if problems:
        print(f"REFUSED — {len(problems)} of {len(edits)} anchors failed. Nothing written.\n")
        for i, msg, near in problems:
            head = str(edits[i].get("old", ""))[:80].replace("\n", "\\n")
            print(f"  [{i}] {msg}")
            print(f"      anchor: {head}")
            for c in near:
                print(f"      nearest in file: {c[:100]}")
        sys.exit(1)

    added = new_text.count("\n") - text.count("\n")
    if a.dry_run:
        print(f"OK — {len(edits)} anchors matched. {added:+d} lines. Nothing written (--dry-run).")
        return

    open(a.file, "w", encoding="utf-8").write(new_text)
    print(f"applied {len(edits)} edits to {a.file} ({added:+d} lines)")


if __name__ == "__main__":
    main()
