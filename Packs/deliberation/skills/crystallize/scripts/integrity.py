#!/usr/bin/env python3
"""Check that a long decision document is internally honest.

Four things rot as a document grows past a few hundred lines, and all four are
invisible while you are writing:

  dangling section refs   "see §7.4" pointing at a section that never existed or
                          got renumbered. The reader follows it, finds nothing,
                          and stops trusting the other cross-references.

  question bookkeeping    a question referenced but never recorded, recorded
                          twice, or still open while the prose says it is settled.
                          A document that miscounts its own questions has stopped
                          being a source of truth in a way readers notice fast.

  stale counts            hand-typed "22 of 26 answered" is wrong two edits later.
                          Derive it or do not state it.

  section balance         700 lines on mechanism and 48 on purpose. Invisible in
                          prose, obvious in a table -- and it usually means the
                          thin sections were never interrogated.

Usage:
    python3 integrity.py <file>
    python3 integrity.py <file> --sections     # line counts per section
    python3 integrity.py <file> --json

Conventions assumed (they are the ones the crystallize skill writes):
    headings   `## 5. Name`  /  `### 5.1b Name`
    refs       `§5.1b`
    records    `> [!done] Q13 — ...`   answered
               `> [!question] Q31 — ...` open

Exit code is 1 when something is inconsistent, so this works as a pre-commit gate.
"""

import argparse
import json
import re
import sys

HEADING = re.compile(r"^#{2,4}\s+(\d+(?:\.\d+)?[a-z]?)\.?\s+(.*)$", re.M)
SECTION_REF = re.compile(r"§(\d+(?:\.\d+)?[a-z]?)")
DONE = re.compile(r"\[!done\]\s+(Q\d+[a-z]?)\b")
OPEN = re.compile(r"\[!question\]\s+(Q\d+[a-z]?)\b")
ANY_Q = re.compile(r"\b(Q\d+[a-z]?)\b")
# A count claim the document makes about itself, e.g. "Twenty-two of twenty-six".
WORD_COUNT_CLAIM = re.compile(
    r"\b(one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|"
    r"fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|twenty|twenty-\w+|thirty|"
    r"thirty-\w+)\s+(?:of|are|remain|carry)\b",
    re.I,
)


def body_of(text):
    """Everything before an appendix, since verbatim appendices are not the document."""
    for marker in ("\n## Appendix", "\n## APPENDIX"):
        i = text.find(marker)
        if i > 0:
            return text[:i]
    return text


def check(text):
    body = body_of(text)
    headings = HEADING.findall(body)
    ids = {h[0] for h in headings}

    # A §ref is external when it cites another document rather than this one --
    # "Vault-Contract §1b", "[[Boundary|Boundary]] §V7". Those are not dangling,
    # and flagging them trains the reader to ignore the check. The test is what
    # sits immediately to the left: a wikilink close, a backtick, or a capitalised
    # word that is not this document's own prose lead-in.
    external, dangling = set(), []
    for m in SECTION_REF.finditer(body):
        sid = m.group(1)
        if sid in ids:
            continue
        left = body[max(0, m.start() - 40):m.start()]
        if re.search(r"(\]\]|`|\.md|\bContract\b|\bBoundary\b|\bREADME\b)\s*$", left):
            external.add(sid)
        else:
            dangling.append(sid)
    # A comma-run of refs to one external document ("§1, §1b, §2") only carries the
    # citation on the first, so an id proven external anywhere is external throughout.
    dangling = sorted(set(dangling) - external)

    done = DONE.findall(body)
    open_q = OPEN.findall(body)
    mentioned = set(ANY_Q.findall(body))

    dupes = sorted({q for q in done if done.count(q) > 1})
    both = sorted(set(done) & set(open_q))
    orphan_refs = sorted(mentioned - set(done) - set(open_q))

    claims = [m.group(0) for m in WORD_COUNT_CLAIM.finditer(body)]

    problems = []
    if dangling:
        problems.append(f"{len(dangling)} section ref(s) resolve to no heading: {dangling}")
    if dupes:
        problems.append(f"question(s) recorded twice: {dupes}")
    if both:
        problems.append(f"question(s) both open and answered: {both}")
    if orphan_refs:
        problems.append(
            f"{len(orphan_refs)} question id(s) referenced with no record: {orphan_refs}"
        )

    return {
        "headings": len(headings),
        "external_refs": sorted(external),
        "answered": sorted(set(done)),
        "open": sorted(set(open_q)),
        "dangling_refs": dangling,
        "duplicate_records": dupes,
        "both_states": both,
        "referenced_without_record": orphan_refs,
        "self_count_claims": claims,
        "problems": problems,
    }


def sections(text):
    body = body_of(text)
    marks = [(m.start(), m.group(0).strip()) for m in re.finditer(r"^##\s+.*$", body, re.M)]
    out = []
    for i, (pos, title) in enumerate(marks):
        end = marks[i + 1][0] if i + 1 < len(marks) else len(body)
        seg = body[pos:end]
        out.append(
            {
                "title": title.lstrip("# ").strip(),
                "lines": seg.count("\n"),
                "answered": len(DONE.findall(seg)),
                "open": len(OPEN.findall(seg)),
            }
        )
    return out


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("file")
    ap.add_argument("--sections", action="store_true")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()

    try:
        text = open(a.file, encoding="utf-8").read()
    except OSError as e:
        sys.exit(f"cannot read {a.file}: {e}")

    r = check(text)
    if a.sections:
        r["sections"] = sections(text)

    if a.json:
        print(json.dumps(r, indent=2, ensure_ascii=False))
        sys.exit(1 if r["problems"] else 0)

    total = len(r["answered"]) + len(r["open"])
    print(f"{a.file}")
    print(f"  headings   {r['headings']}")
    print(f"  questions  {total} distinct — {len(r['answered'])} answered, {len(r['open'])} open")
    if r["open"]:
        print(f"             open: {', '.join(r['open'])}")

    if r["problems"]:
        print("\nPROBLEMS")
        for p in r["problems"]:
            print(f"  - {p}")
    else:
        print("\n  no dangling refs, no duplicate or contradictory question records")
    if r["external_refs"]:
        print(f"  section refs citing another document (not errors): {', '.join(r['external_refs'])}")

    if r["self_count_claims"]:
        # Not an error: only a place where the document counts itself in prose and
        # will be wrong two edits later unless it is derived.
        print("\n  the document states counts in prose — check these against the numbers above:")
        for c in r["self_count_claims"][:6]:
            print(f"    “{c}…”")

    if a.sections:
        print(f"\n{'section':52}{'lines':>7}{'done':>6}{'open':>6}")
        for s in r["sections"]:
            print(f"{s['title'][:52]:52}{s['lines']:>7}{s['answered']:>6}{s['open']:>6}")
        longest = max(r["sections"], key=lambda s: s["lines"], default=None)
        thin = [s for s in r["sections"] if s["lines"] < 25]
        if longest and thin:
            print(
                f"\n  balance: '{longest['title'][:40]}' is {longest['lines']} lines while "
                f"{len(thin)} section(s) are under 25. A thin section is usually one nobody "
                f"asked questions about."
            )

    sys.exit(1 if r["problems"] else 0)


if __name__ == "__main__":
    main()
