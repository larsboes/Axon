#!/usr/bin/env python3
"""Measure a markdown corpus before forming an opinion about it.

The first move in crystallizing a document is replacing adjectives with numbers.
"My notes are a mess" is unarguable and unactionable; "42% of one folder is under
40 words while another is at 6%" names the problem and points at it.

What it reports, and why each one earns its place:

  per-folder health   note counts, stub rates, average length, links per note.
                      The comparison between folders is the finding -- a corpus
                      usually has one healthy area and one rotting one, and the
                      contrast tells you which conventions actually work.

  frontmatter keys    usage counts, plus how many carry an empty value. A key
                      present on 900 notes and filled on 3 is a template that was
                      never filled in, which is a different problem from a key
                      nobody uses.

  dialect drift       two spellings of one value (People/people, seedling/emoji).
                      The most common silent defect in a hand-maintained corpus,
                      because every query sees one spelling and misses the other.

  orphans             notes nothing links to. In a linked corpus an orphan is
                      either unfinished or unwanted, and both are worth knowing.

Usage:
    python3 census.py <root> [--frontmatter] [--stubs] [--links] [--all]
    python3 census.py <root> --key status      # one key's value distribution
    python3 census.py <root> --json            # machine-readable

Nothing here is corpus-specific: it assumes markdown files with optional YAML
frontmatter and `[[wikilinks]]`, and degrades quietly when either is absent.
"""

import argparse
import collections
import json
import os
import re
import sys

SKIP_DIRS = {".git", ".obsidian", ".trash", "node_modules", "__pycache__", ".venv"}
STUB_WORDS = 40
FM = re.compile(r"^---\n(.*?)\n---\n?(.*)$", re.S)
KEY = re.compile(r"^([A-Za-z_][\w-]*):(.*)$", re.M)
WIKILINK = re.compile(r"\[\[([^\]|#]+)")
EMPTY_VALUES = {"", '""', "''", "[]", "{}", "null", "~"}


def walk(root):
    """Every markdown file under root, skipping the usual noise."""
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS and not d.startswith(".")]
        for f in filenames:
            if f.endswith(".md"):
                yield os.path.join(dirpath, f)


def parse(path, root):
    rel = os.path.relpath(path, root)
    try:
        text = open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        return None
    m = FM.match(text)
    fm, body = (m.group(1), m.group(2)) if m else ("", text)
    parts = rel.split(os.sep)
    return {
        "rel": rel,
        "basename": os.path.basename(path)[:-3],
        "folder": parts[0] if len(parts) > 1 else "(root)",
        "fm": fm,
        "body": body,
        "words": len(body.split()),
        "links": WIKILINK.findall(body) + WIKILINK.findall(fm),
        "has_fm": bool(m),
    }


def collect(root):
    notes = [n for n in (parse(p, root) for p in walk(root)) if n]
    if not notes:
        sys.exit(f"no markdown found under {root}")
    return notes


def folder_health(notes):
    agg = collections.defaultdict(
        lambda: {"notes": 0, "stubs": 0, "words": 0, "links": 0, "no_fm": 0}
    )
    for n in notes:
        a = agg[n["folder"]]
        a["notes"] += 1
        a["words"] += n["words"]
        a["links"] += len(n["links"])
        if n["words"] < STUB_WORDS:
            a["stubs"] += 1
        if not n["has_fm"]:
            a["no_fm"] += 1
    rows = []
    for folder, a in sorted(agg.items(), key=lambda kv: -kv[1]["notes"]):
        rows.append(
            {
                "folder": folder,
                "notes": a["notes"],
                "stubs": a["stubs"],
                "stub_pct": round(100 * a["stubs"] / a["notes"]),
                "avg_words": a["words"] // a["notes"],
                "links_per_note": round(a["links"] / a["notes"], 1),
                "no_frontmatter": a["no_fm"],
            }
        )
    return rows


def key_usage(notes):
    present, filled = collections.Counter(), collections.Counter()
    for n in notes:
        for k, v in KEY.findall(n["fm"]):
            present[k] += 1
            if v.strip() not in EMPTY_VALUES:
                filled[k] += 1
    return [
        {"key": k, "present": c, "filled": filled[k], "empty": c - filled[k]}
        for k, c in present.most_common()
    ]


def dialects(notes):
    """Keys whose spelling varies, and values that vary in form.

    Case-insensitive collision on the key name catches People/people. Distinct
    raw values for a low-cardinality key catches seedling vs an emoji.
    """
    by_lower = collections.defaultdict(collections.Counter)
    values = collections.defaultdict(collections.Counter)
    for n in notes:
        for k, v in KEY.findall(n["fm"]):
            by_lower[k.lower()][k] += 1
            v = v.strip().strip("\"'")
            if v and v not in EMPTY_VALUES:
                values[k.lower()][v] += 1
    key_drift = [
        {"key": low, "spellings": dict(forms)}
        for low, forms in sorted(by_lower.items())
        if len(forms) > 1
    ]
    value_drift = []
    for low, vals in sorted(values.items()):
        # Only low-cardinality fields are enums; a summary field varying is normal.
        if 1 < len(vals) <= 12:
            value_drift.append({"key": low, "values": dict(vals.most_common())})
    return key_drift, value_drift


def orphans(notes):
    targets = collections.Counter()
    for n in notes:
        for l in n["links"]:
            targets[os.path.basename(l.strip())] += 1
    return [n["rel"] for n in notes if targets[n["basename"]] == 0]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("root")
    ap.add_argument("--frontmatter", action="store_true", help="key usage, present vs filled")
    ap.add_argument("--stubs", action="store_true", help="list the shortest notes per folder")
    ap.add_argument("--links", action="store_true", help="orphan count and worst offenders")
    ap.add_argument("--dialects", action="store_true", help="two spellings of one key or value")
    ap.add_argument("--key", help="value distribution for one key")
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()
    if a.all:
        a.frontmatter = a.stubs = a.links = a.dialects = True

    notes = collect(a.root)
    out = {"total": len(notes), "folders": folder_health(notes)}

    if a.key:
        vals = collections.Counter()
        for n in notes:
            for k, v in KEY.findall(n["fm"]):
                if k.lower() == a.key.lower():
                    vals[v.strip().strip("\"'") or "(empty)"] += 1
        out["key"] = {"name": a.key, "values": dict(vals.most_common())}
    if a.frontmatter:
        out["keys"] = key_usage(notes)
    if a.dialects:
        kd, vd = dialects(notes)
        out["key_drift"], out["value_drift"] = kd, vd
    if a.links:
        o = orphans(notes)
        out["orphans"] = {"count": len(o), "sample": o[:20]}
    if a.stubs:
        short = sorted(notes, key=lambda n: n["words"])[:20]
        out["shortest"] = [{"rel": n["rel"], "words": n["words"]} for n in short]

    if a.json:
        print(json.dumps(out, indent=2, ensure_ascii=False))
        return

    print(f"{out['total']} notes under {a.root}\n")
    print(f"{'folder':22}{'notes':>7}{'stubs':>7}{'stub%':>7}{'avg words':>11}{'links/note':>12}")
    for r in out["folders"]:
        print(
            f"{r['folder'][:22]:22}{r['notes']:>7}{r['stubs']:>7}{r['stub_pct']:>6}%"
            f"{r['avg_words']:>11}{r['links_per_note']:>12}"
        )
    # The contrast between the best and worst folder is usually the whole finding.
    if len(out["folders"]) > 1:
        best = min(out["folders"], key=lambda r: r["stub_pct"])
        worst = max(out["folders"], key=lambda r: r["stub_pct"])
        if worst["stub_pct"] - best["stub_pct"] >= 15:
            print(
                f"\n  contrast: {worst['folder']} is {worst['stub_pct']}% stubs "
                f"against {best['folder']} at {best['stub_pct']}% — ask what differs"
            )

    if "key" in out:
        print(f"\n{out['key']['name']}: values")
        for v, c in out["key"]["values"].items():
            print(f"  {c:>6}  {v}")
    if "keys" in out:
        print("\nfrontmatter keys (present / filled):")
        for k in out["keys"][:30]:
            flag = "   <- mostly empty" if k["filled"] and k["empty"] > k["filled"] * 3 else ""
            print(f"  {k['present']:>6} / {k['filled']:<6} {k['key']}{flag}")
    if out.get("key_drift"):
        print("\nsame key, two spellings (a query sees one and misses the other):")
        for d in out["key_drift"]:
            print(f"  {d['key']}: {d['spellings']}")
    if out.get("value_drift"):
        print("\nenum-like keys with varied values:")
        for d in out["value_drift"][:10]:
            print(f"  {d['key']}: {d['values']}")
    if "orphans" in out:
        print(f"\norphans (nothing links to them): {out['orphans']['count']}")
        for o in out["orphans"]["sample"][:10]:
            print(f"  {o}")
    if "shortest" in out:
        print("\nshortest notes:")
        for s in out["shortest"][:10]:
            print(f"  {s['words']:>4}w  {s['rel']}")


if __name__ == "__main__":
    main()
