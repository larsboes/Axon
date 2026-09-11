# presentations pack

Turns an argument into a deck. One skill, **`slide-deck`**, which owns the mechanism —
a role-based theme, a set of layout archetypes, a deterministic build, and a
verification loop that renders the result so it can actually be looked at.

The pack exists because "make me a deck" produces two different failures. The first is
a deck that reads correctly in source and is visibly wrong on screen: drop shadows on
every box, bullets silently centred, a title overlapping its own rule, a statement bar
drawn on top of the columns above it. None of those are visible in code and all of them
are obvious in a render. The second is a deck that is technically clean and says
nothing — sections where slides should be, topics where assertions should be.

So the skill does two things. It puts a deterministic tool in front of the model
(`build`, `check`, `render`), and it carries the judgment about what a slide owes the
one after it (`references/narrative.md`). The tool catches the first failure class
without eyes; the reference catches the second.

## The mechanism

```
skills/slide-deck/
  SKILL.md              router: the eight-step workflow
  scripts/deck          bash + uv launcher, delegates to lib/deckkit
  lib/deckkit/          theme, layout, markup, slide, deck, check, render, cli
  references/           design-system, slide-recipes, narrative,
                        verification, theming, gotchas
  assets/deck.template.py   what `deck init` scaffolds
  assets/themes/        warm-scientific-teal, slate
```

A deck is a Python module that builds one `Deck`. A module rather than a YAML file
because a deck's structure is a sequence of decisions with numbers in it — a time
budget, a run count, a matrix row — and Python reads those better than a data format
does.

```python
s = deck.open("Verhaltenserhalt ist kein Beleg für Wartbarkeit", "4 · Ergebnisse")
s.panel(s.x, s.top, 5.55, 0.92, "Iteration 3, Lauf 3: **11 von 11** …",
        voice="caution")
s.figure_beside(FIG / "eval-reviewer-ratings.png", [...], note="36 Bewertungen.")
s.statement("~Konsequenz:~ Auf den Verhaltensvergleich muss eine getrennte "
            "Wartbarkeitsprüfung folgen.", x=s.x, width=5.55)
```

## Why `deck check` is a separate verb

`render` answers *does it look right* and needs eyes. `check` answers *is anything
mechanically broken* and needs none, so it runs in a loop, costs no tokens, and gates
a commit. It covers eleven findings, including shape-on-shape overlap — which is what
catches a statement bar landing on the columns above it, a defect that is invisible in
source and spans two slides in a real deck.

Exit code is the error count, so `deck check && deck render` is a usable gate.

## Why `deck render` emits a contact sheet

Because a deck is a sequence, and a sequence cannot be judged one slide at a time. The
contact sheet is a 3×3 grid of the whole deck in one image — balance, rhythm, where the
eye snags, whether the dividers punctuate or interrupt. Individual full-resolution
slides come out beside it for reading one slide closely.

Every entry in `references/gotchas.md` was found by looking at a contact sheet and by
nothing else.

## The first deck

The skill was extracted from a 32-slide bachelor colloquium deck (21 presentation
slides, 11 backup), which is the worked example that keeps the references honest. It
built clean and passed `check` with 0 errors and 0 warnings after the port — and the
port is what found four of the defects the library now prevents: a duplicated footer
from a second `footer()` call, vertical padding scaled from horizontal padding, a
figure whose caption overran the body floor by 0.03in, and a density threshold that
needed to be a deck-level decision rather than a global constant.

It is also where the awkward parts of the API come from. `reserve=s.statement_reserve`
exists because a statement bar was drawn on top of three columns. `deco=True` exists
because the overlap check needs to know a hairline is not a block.

## Activate

```bash
"$AXON_ROOT/tools/harnesses" status presentations
"$AXON_ROOT/tools/packs-claude" deploy presentations    # -> ~/.claude/skills/slide-deck
"$AXON_ROOT/tools/packs-codex"  deploy presentations    # -> ~/.agents/skills/slide-deck
```

Requires `uv` on PATH (already a `toolchain.toml` requirement) and **LibreOffice** for
`render` (`brew install --cask libreoffice`). LibreOffice is a host binary the Pack
assumes; it is not yet an entry in `toolchain.toml`, so `tools/doctor` will not warn
about a machine that cannot render.

## Attribution

Original work, no upstream. The design system is transcribed from a reference deck the
user supplied and re-coloured from that project's own figure palette
(`assets/themes/warm-scientific-teal.json`, provenance recorded in the file). The
`lib/deckkit` code is written against `python-pptx` and `pymupdf`, both pinned in
`scripts/deck`.
