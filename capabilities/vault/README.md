# vault

Reads an Obsidian vault as data. Six CLI verbs and one HTTP surface, all
read-only.

```
vault links  [--root PATH] [--json] [--dead] [--inbound FOLDER]
vault lint   [--root PATH] [--json] [--carrying KEY]
vault names  [--root PATH] [--json] [--folder Atlas/People]
vault class  [--root PATH] [--json] [--only c2] [--list]
vault people [--root PATH] [--json]
vault bases  [--root PATH] [--json] [--strict]
```

The root is a personal fact and never lives in this repo. It comes from the
overlay's `config/knowledge.toml` (`vault_root = "..."`), or from `--root`. The
server takes the same path from the same file and has no `--root`: a service
resolving its own root from an argument would be a second declaration of where
the vault is.

## `class` — which notes hold whose facts

PRD **Q9a** (2026-08-23): the folder sets the default, a note's frontmatter
`class:` key overrides it in either direction. `Atlas/People/`,
`Atlas/Documents/` and `Atlas/Finance/` are **C2 Others**; a health folder is
C2 wherever it sits; everything else is **C1 Mine**.

The rule is not here. `content_item::DataClass::classify_vault_note` holds it,
beside the mail rules and the C0–C3 vocabulary they share, because PRD §6.1
forbids a second definition of what `c2` means by name. This verb is the walk
and the report.

**Measured against this vault, 2026-09-07** — the acceptance figures, in the
same spirit as the link counts below:

| Class | Notes | From |
|---|---|---|
| C0 Public | 0 | Unreachable from a location. Publishing is an act (§15), not a folder |
| C1 Mine | 2,587 | Everything Q9a's table does not name |
| C2 Others | 170 | `Atlas/People` 89 · `Atlas/Documents` 74 · `Atlas/Finance` 7 |
| C3 Secret | 0 | Only reachable by declaring it |

Two things the measurement says that the ruling could not. **No note in this
vault carries a `class:` key**, so every one of the 2,757 rows above is a folder
default and the override path has no production evidence yet — it is tested, not
exercised. And **the health rule fires on nothing**: the only health folder here
is `Atlas/Documents/Gesundheit/`, which the `Atlas/Documents` rule already claims
one line earlier. It stays because Q9a names health as a rule rather than a
folder, and the day that folder moves out of `Atlas/Documents/` is the day it
starts earning its place.

Three lists, not one total: `--list` prints every note with its class,
`--only c2` prints one class with the reason each note landed there, and the
default report names the **refused declarations** — notes whose frontmatter set
a class outside the vocabulary. A refusal is the interesting row. The folder
default answers for it, so nothing fails; what it means is that somebody
believes that note is classified and it is not.

## `bases` — the Bases, checked against the vault they query

PRD **D5** says the Bases are unverified in Obsidian and that a CLI cannot
confirm Base *rendering*. Both are still true. What a CLI can confirm is what a
Base states about the vault before rendering starts: a Base is a query, it names
folders and it names the frontmatter keys it will draw as columns, and each of
those is checkable against the notes on disk.

**Measured 2026-09-08** — 28 Bases, 37 folder references:

| | |
|---|---|
| Folder references naming a folder that holds no note | **11**, across 11 Bases and 10 distinct folders |
| Declared columns no note in scope carries | **106** |

Those are two different failures and both look identical in Obsidian. A Base
whose folder moved renders an empty table; a Base whose folder is fine and whose
`maturity:` became `status:` renders a table of blank columns. Neither throws.

A missing folder gets **candidates, never a rewrite**. `.base` files live in the
vault and §5.5 is one-way, so this verb names where the folder probably went and
stops. The rule is narrow — a folder elsewhere in the vault with the same final
segment, holding at least one note — and each candidate is weighed by how many of
that Base's own declared columns its notes carry, because a matching name is not
a destination:

| Base | Names | Candidate | Columns it fills |
|---|---|---|---|
| `Focus.base` | `TELOS/Focus` | `Atlas/Focus` | 3 of 3 |
| `Reflections.base` | `TELOS/Reflections` | `Atlas/Reflections` | 4 of 5 |
| `Soma.base` | `Projects/Soma/Domains` | `Projects/Axon/Knowledge-Base/Domains` | 4 of 4 |
| `Investments.base` | `Atlas/Finance/Investments` | `Projects/Archive/Ledger/Notability/Investments` | **0 of 8** |
| `Tasks.base`, `Calendar.base` | `Projects/Tasks` | nine of them | 7 of 10 at best |

Four references have exactly one candidate and only three of them survive the
column check. The fourth is an archived Notability import that shares a word.
`Projects/Tasks` is the opposite shape: Q48 spread it across `Projects/**/Tasks/`
on purpose, so nine candidates is the correct answer and none of them is a
proposal. The remaining five — `Atlas/Places`, `Atlas/Identity`,
`Resources/Spots`, `Atlas/Finance/Income`, `Atlas/Finance/Purchases` — have no
candidate at all, which means the folder was never created rather than moved.

`--strict` exits non-zero when a positive folder reference resolves to nothing.
Without it the verb answers `0` for a vault where every Base is broken and `0`
for one where none is, which is an instrument that cannot be wrong.

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
