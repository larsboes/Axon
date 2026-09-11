---
name: slide-deck
description: Builds and maintains slide decks as .pptx files from a Python module, with a role-based theme, layout archetypes, speaker notes on every slide, and a build-check-render loop that produces a contact sheet for visual inspection. Use when the user wants a deck, slides or a presentation for a colloquium, a conference talk, a thesis defence, a project readout or a lecture; or to rebuild, re-theme or check a deck that a previous session started. Do not use for prose documents, papers or thesis chapters (academic-writing), for README or user-facing docs (human-writing), or for hand-editing an existing .pptx in a slide editor.
allowed-tools: Read, Write, Edit, Bash
---

# Slide deck

A deck is an argument with pictures. This skill supplies the mechanism — theme,
layout archetypes, deterministic build — and the judgment about what a slide owes
the one after it. Content lives in the user's own deck directory, never here.

`<skill>/scripts/deck build|check|render|themes|init` is the whole tool surface.
Python runs through `uv`; dependencies are pinned in the launcher.

## Workflow

Copy this checklist and track progress:

```
- [ ] 1. Brief fixed: audience, duration, language, what the deck must achieve
- [ ] 2. Theme derived from the source artifact, or `slate` if there is none
- [ ] 3. Spine written: sections, one assertion per slide, a time budget that sums
- [ ] 4. Deck module written, notes on every slide
- [ ] 5. build → clean
- [ ] 6. check → 0 errors; every warning either fixed or explained
- [ ] 7. render → LOOK at the contact sheet → fix → build again
- [ ] 8. Backup slides written for the questions the talk will raise
```

### Step 1 — Brief

Four facts change the whole design: audience and their prior knowledge, duration
and whether questions are included, language, and what the deck must achieve
(persuade / report / teach / defend). If the request leaves any of them open, ask
ONE batched round of numbered questions with concrete options, recommended first —
do not infer language from the source material, a German defence of an English
thesis is normal. If the user defers, proceed on named defaults and print them.
`references/narrative.md` §Ambiguity has the question set.

### Step 2 — Scaffold and theme

```bash
scripts/deck themes                                  # list built-in themes
scripts/deck init <name> --dir <where> [--theme x]   # deck.py + Assets/
```
If the source material has figures, derive the theme from their palette rather than
picking one — `references/theming.md` gives the order of preference and the contrast
rules. A deck that disagrees with its own embedded diagrams looks borrowed.

### Step 3 — Spine

Write the sections and the per-slide assertions before writing slides. Read
`references/narrative.md` for the spine shape, the assertion-title rule, the
claim/evidence/bound rhythm, and the words-per-minute budget. Titles alone should
reconstruct the argument; if they do not, the deck is a set of notes.

### Step 4 — Content

Edit `deck.py`. Reach for an archetype first — `references/slide-recipes.md` maps a
slide's job to a layout, and gives the primitives and the composition patterns when
none fits. Read `references/design-system.md` before the first slide and whenever a
slide looks wrong but nothing is broken: it states the colour roles, the type scale,
the grid and the emphasis rules.

Every slide gets notes with a running clock, the spoken text, and the answer to the
question the slide will provoke. `check` fails a slide without them.

### Step 5–7 — Build, check, render

```bash
scripts/deck build  <deck.py>    # -> out/<name>.pptx ; exit 2 on a broken module
scripts/deck check  <deck.py>    # exit code = error count
scripts/deck render <deck.py>    # -> out/render/  pdf + slide-NN.png + contact sheets
```

**Then look at the contact sheet.** This is the step that makes the deck good, and
the one a model skips. `check` cannot see a drop shadow, a centred bullet, an
imbalance or a dead bottom third. Read `references/verification.md` for the loop and
what to look at in what order, and `references/gotchas.md` when something renders
wrong for no visible reason.

Repeat build → render until the sheet is clean. Two passes is normal.

### Step 8 — Backup

Eight to twelve slides after `deck.backup_divider()`, each answering one question the
talk will raise, with the question named in its notes.

## Error handling

- `deck: theme not found` → the theme JSON was not copied; `init` copies one, or run
  `scripts/deck themes` and pass an explicit path to `Theme.load()`.
- `deck: figure not found` → figures resolve relative to the deck module. Use
  `HERE / "Assets/figures/..."` and check the file is really there.
- `check` reports `overlap` on a statement bar → pass
  `reserve=s.statement_reserve` to the archetype above it.
- `check` reports `shadow` → a shape was built without `deckkit.layout`. Use a
  primitive.
- `render` fails with "LibreOffice not found" → install it
  (`brew install --cask libreoffice`); there is no fallback, because an unrendered
  deck is an unverified deck.
- A change to `lib/` → rebuild and render a **real** deck, not the scaffold.

## Boundaries — Will / Will not

- Will: author a deck module, derive a theme, build/check/render, export `.pptx` and
  `.pdf`, write backup slides.
- Will not: edit a `.pptx` in place, animate or transition slides, produce anything
  other than a slide deck (that is `academic-writing` for papers and `human-writing`
  for prose), or store deck content inside this skill — decks live in the user's
  project, and this Pack carries mechanism only.

## Done when

- `scripts/deck check <deck.py>` exits 0.
- The contact sheet has been looked at, and the last build is the one that was looked at.
- Every slide carries notes with a time budget.
- The brief's four facts are either stated in the deck module or printed as defaults.
