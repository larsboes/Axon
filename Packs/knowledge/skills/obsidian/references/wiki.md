# Wiki layer — ingest, synthesize, lint

The vault *is* the wiki. There is no separate wiki tree, no `index.md`, no `log.md`, no
`sources/` + `entities/` + `concepts/` folders. Those belong to the older llm-wiki layout, which
built a parallel structure beside the vault's own nine Knowledge domains and contradicted the
contract's placement table and no-doubling law on every point. `Knowledge/llm-wiki/` is what is
left of it: two files, no owner. Do not extend it.

What replaces it is the contract, used as-is.

## Where an ingested source lands

One source produces up to three notes, each asserting a different kind of statement:

| Part of the source | Goes to | Type |
|---|---|---|
| The **object** — the book, paper, article, video | `Atlas/Media/<Title>.md` | `type: source` + `medium:` |
| The **file itself** (PDF, epub) | `Atlas/Media/Assets/` — gitignored | — |
| **What you learned** — the transferable idea | `Knowledge/<domain>/<Concept>.md` | `type: note` |
| A **person, org, place, product** named in it | `Atlas/People|Places|Items/<Name>.md` | entity |

The ideas never live on the Media note; the Media note is the object. The Knowledge concept links
back with `sources:`. That split *is* the no-doubling law applied to reading.

The nine live domains: `Business · Computing · Cybersecurity · Engineering · Homelab ·
Mathematics · Meta · Sciences · Telecommunications`. Primary domain = the folder; every other
membership is a `categories:` link, so one note appears in several MOCs without being copied.

Every folder under `Knowledge/` has exactly one MOC (`type: moc`, `maturity: moc`) named for the
concept, not the folder basename — `Intelligence/` → `Artificial Intelligence.md`. The MOC embeds
the shared `📂 In Category` base view; it is never a hand-maintained list.

## Ingest

1. Resolve the vault and read the contract (`scripts/vault root` / `contract`).
2. **Grep first.** Almost every concept already has a note — the job is usually to deepen an owner,
   not to create a sibling. `maturity: seedling → growing` is what a second source should produce.
3. Read the source. For a URL, use the `defuddle` skill to get clean markdown; the Obsidian Web
   Clipper drops its own captures in `Clippings/`, which is the same input by another route.
4. Decide the split per the table above, and **present the planned notes before writing** — which
   are new, which are updates, which links get added.
5. Write. Type at birth, link both directions, fix stale `maturity` on anything you touch.

## Synthesize (query)

Grep the vault, read the owning notes, answer with `[[links]]` to the notes that carried each
claim. File the answer back **only** if it is a durable concept in its own right — then it is a
Knowledge note like any other, not a `queries/` artifact. A synthesis that is really about one
project belongs in that project's note.

## Lint

Walk `Knowledge/` and report, without fixing anything unasked:

- notes with no `type:` / `maturity:` / `summary:` / `categories:` (untyped at birth)
- orphans: no inbound and no outbound `[[links]]`
- doubling: the same claim asserted in two owners, or the same note under two folders
- domains whose MOC is missing, or whose MOC hand-lists notes instead of embedding the base view
- legacy `maturity: evergreen` on notes that were never actually developed — the 2025 import
- contradictions and stale claims between notes that link each other

Apply only approved fixes, forward-only, on-touch. Never bulk-migrate.
