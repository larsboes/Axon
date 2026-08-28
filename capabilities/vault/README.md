# vault

Reads an Obsidian vault as data. Three CLI verbs and one HTTP surface, all
read-only.

```
vault links [--root PATH] [--json] [--dead] [--inbound FOLDER]
vault lint  [--root PATH] [--json] [--carrying KEY]
vault names [--root PATH] [--json] [--folder Atlas/People]
```

The root is a personal fact and never lives in this repo. It comes from the
overlay's `config/knowledge.toml` (`vault_root = "..."`), or from `--root`. The
server takes the same path from the same file and has no `--root`: a service
resolving its own root from an argument would be a second declaration of where
the vault is.

## The server

`vault-server` on `8094`, loopback. Four routes:

| Route | Answers |
|---|---|
| `GET /health` | Liveness. A literal — it cannot see the vault. |
| `GET /ready` | Readiness: the vault root and its `Projects/` folder resolve. |
| `GET /routes` | This manifest, as data. |
| `GET /api/tasks?status=open\|done` | Every action note under `Projects/`, read live. |

One task is `{id, title, done, due, priority, summary, projects, uri}`. `id` is
the vault-relative path; `uri` is the `obsidian://open` address of the note.

It exists because PRD **Q48** (2026-08-27) retired the `tasks` capability and
returned the Action kind to `Projects/**/Tasks/`, where the vault contract
§5.1b had assigned it all along. The dashboard's decision ladder needed an HTTP
source for band 620 and the data had moved here, so the reader that already
existed grew a second front end.

**There is no write route, and that is the ruling rather than an omission.** A
task is created, edited and marked done in Obsidian, in a note a human owns.
Adding a `PATCH` would make Axon a second writer of files a human is editing —
the conflict §5.5 states as "Axon reads the vault and does not write to it". The
ladder links to the note; it does not close it.

**Which frontmatter keys are served, and why not all of them.** A task note
carries eleven keys; five are served, because the ladder reads them: `summary`
renders the row (beside `title`, which is the filename, not a key), `due` and
`priority` rank it, `projects` labels it, `done` decides whether it is a
decision at all. The other six — `scheduled`, `context`, `energy`, `focus`,
`events` and `blocked_by` — have no reader, and a served field with no reader
is a contract nothing checks.

**What counts as a task** is `capabilities/vault/src/tasks.rs`'s module doc: the
vault's own `Resources/Bases/Tasks.base` filter, scoped to `Projects/`, minus
archived folders, minus notes with no `done` key. Each divergence is measured
against the live vault and named there. Tracking the operator's own Base rather
than inventing a second definition is the point — two surfaces disagreeing about
one folder is exactly what §5.1b's no-doubling law forbids.

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
