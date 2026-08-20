# vault

Reads an Obsidian vault as data. Two verbs, both read-only.

```
vault links [--root PATH] [--json] [--dead] [--inbound FOLDER]
vault lint  [--root PATH] [--json] [--carrying KEY]
```

The root is a personal fact and never lives in this repo. It comes from the
overlay's `config/knowledge.toml` (`vault_root = "..."`), or from `--root`.

## Why a binary

Every vault operation worth doing starts by asking the same two questions: what
is in here, and what links to what. A skill that describes how to answer them
answers differently each run. A binary with tests answers the same way twice,
which is the only reason a migration can be gated on it.

## Why the counts are the acceptance test

These figures were measured a first time by `find`, `rg` and hand
classification, before this crate existed. Those numbers are the fixture, and
the run below is the check:

| Measure | Fixture | `vault` | |
|---|---|---|---|
| Notes under `Knowledge/` | 1,138 | 1,138 | exact |
| Notes carrying a `knowledge:` key | 996 | 996 | exact |
| Path-form wikilinks | 1,486 | 1,486 | exact |
| Ambiguous basenames | 14 | 14 | exact |
| Wikilinks total | 18,332 | 18,084 | −1.4% |
| Dead wikilinks | 5,397 | 5,491 | +1.7% |
| Notes linked into `Knowledge/` from outside | 133 | 136 | tool is right |

The two percentage gaps are bracket pairs inside fenced code blocks, which the
shell probe counted as links and this one does not. The last row is the
interesting one: the shell probe scanned four folders and never looked at the
notes sitting at the vault root, so it undercounted. Where the tool and the
fixture disagree, the reason gets written down and the loser gets named. A
number quietly adjusted to match is not a check.

## What it found that the fixture could not

Roughly **11,000 of the vault's 18,000 wikilinks live in frontmatter**, not in
prose — in `categories:`, `related:` and `sources:`. That is the membership
graph every MOC is fed by and every provenance edge the knowledge model rests
on. This crate was written body-only first and reported a vault 60% smaller
than it is; the fixture caught it. The two counts stay separate because they
break differently: a folder move rewrites a prose link, an editor rewrites a
`categories:` entry, and one number cannot tell you which repair you owe.

## Dialect drift

`lint` scans the raw frontmatter rather than the parsed map, because the drift
this vault has is invisible after parsing: `knowledge: reference` and
`knowledge: "reference"` are the same value and two different conventions, and
`maturity: evergreen` versus `maturity: 🌲` is why every Base in the vault
carries a hand-written compatibility shim.

## Related

- `libs/markdown-root` — containment-checked vault access, recursive walk, and
  the byte-addressable frontmatter this crate reads. The offsets exist so a
  future writer can re-serialise frontmatter and concatenate the original body
  bytes rather than round-tripping prose through a parser that would reformat
  Bases embeds and Mermaid fences.
