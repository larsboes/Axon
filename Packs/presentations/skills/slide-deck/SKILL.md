---
name: slide-deck
description: Builds and maintains slide decks as .pptx files from a Python module. Carries talk-genre profiles (DSR/qualitative thesis defence, empirical-CS conference talk, academic job talk), a role-based theme, layout archetypes, speaker notes on every slide, and a build-check-render-readiness-timing-handout loop. Use when the user wants a deck, slides, a talk or a presentation for a thesis defence or colloquium, a conference or seminar talk, a job talk, a project readout or a lecture; to rehearse a talk's timing or prepare a defence's Kritikpunkte map; or to rebuild, re-theme or check a deck a previous session started. Do not use for prose documents, papers or thesis chapters (academic-writing), for README or user-facing docs (human-writing), or for hand-editing an existing .pptx in a slide editor.
allowed-tools: Read, Write, Edit, Bash
---

# Slide deck

A deck states an argument with pictures. This skill gives you the mechanism and the
judgment. The mechanism is a theme, layout archetypes, a deterministic build, a render
pass and a rehearsal pass. The judgment is what each slide owes the next one, and what
an academic talk is judged on.

All content lives in the user's own deck directory. This skill holds mechanism only.

The tool surface is `<skill>/scripts/deck build|check|render|readiness|timing|handout|themes|init`.
Python runs through `uv`. The launcher pins the dependencies.

## How this skill works with the others

Four skills share this ground. Each one owns a different thing, and the boundaries are
worth stating.

| skill | owns |
|---|---|
| `academic-writing` | The argument in the paper or thesis, and the constructs a talk must render: contribution types, the validity taxonomy, the citation discipline. |
| `human-writing` | The prose a person reads. Notes, README text and any explanatory writing in the deck should pass its scanner. |
| `asd-ste100` | Text that a machine parses without a person to resolve ambiguity. Error messages, tool descriptions, and the instruction files of this skill. |
| `unslop` | Source code and web interfaces. |
| **this skill** | The deck module, the theme, the layout, the speaker notes and the rehearsal passes. |

Two consequences follow.

The instruction files in this skill follow the `asd-ste100` structural rules, because an
agent parses them. The sentences are short and active, each sentence carries one
instruction, and the files use no semicolons.

The talk's own words belong to the user. When a deck reads as machine-written, hand the
notes and the slide text to `human-writing`. When the argument is unclear, hand the
source document to `academic-writing`. This skill does not rewrite the user's argument.

## Genre profile

Pick the genre before you write a spine. The genre fixes the spine, the slides that must
land, the backup set, and the questions the room will ask.

| genre | reference |
|---|---|
| DSR or qualitative thesis defence (colloquium, viva, Disputation) | `references/genre-dsr-defence.md` |
| Empirical-CS or conference paper talk (method, baselines, ablations) | `references/genre-empirical-cs-talk.md` |
| Academic job talk (faculty or postdoc research presentation) | `references/genre-academic-job-talk.md` |

The three are not interchangeable. A defence is judged on abstraction, positioning and
contribution. A paper talk is judged on beating a baseline. A job talk is judged on a
research programme. If the genre is unclear, ask. A wrong genre produces a
confidently wrong deck.

## The academic layer

Read `narrative.md` first. It carries the craft that every talk shares. Then read the
files that sit under it for an academic talk.

| file | what it fixes |
|---|---|
| `academic-narrative.md` | The objects: a question at class level, the contribution type, the bounding register, the construct column, the four validities. |
| `evidence-on-slides.md` | How to show a number or a figure without overstating it. Access and colour. |
| `citing-on-slides.md` | Inline attribution, the single references slide, reproduced material. |
| `academic-register.md` | Person, tense and register in a spoken sentence. |
| `defence-readiness.md` | The five questions a reviewer answers against the argument. |

The constructs behind these files belong to `academic-writing`. Read its genre
references when the talk needs the underlying source. This skill renders them for a
spoken talk.

## Workflow

Copy this checklist and track progress.

```
- [ ]  1. Brief fixed: audience, duration, language, and what the deck must achieve
- [ ]  2. Genre chosen and read
- [ ]  3. Theme derived from the source artifact, or `slate` when there is none
- [ ]  4. Spine written: sections, one assertion per slide, a time budget that sums
- [ ]  5. Deck module written, notes on every slide
- [ ]  6. build returns clean
- [ ]  7. check returns 0 errors. Fix or explain every warning.
- [ ]  8. render. Look at the contact sheet. Fix and build again.
- [ ]  9. timing. The budget sums, and every script fits its slot.
- [ ] 10. readiness. Answer the five questions. Fix and rebuild. (a defence)
- [ ] 11. handout. Rehearse from it, or give it to the examiners.
- [ ] 12. Backup slides written for the questions the talk will raise
```

### Step 1. Fix the brief

Four facts change the whole design: the audience and their prior knowledge, the duration
and whether questions are included, the language, and what the deck must achieve. If the
request leaves a fact open, ask one batched round of numbered questions. Put the
recommended option first. Do not infer the language from the source material. A German
defence of an English thesis is normal.

When the user defers, proceed on named defaults and print them. `references/narrative.md`
section Ambiguity holds the question set. Record the duration as `Deck(minutes=...)`.
The `timing` verb compares against it.

### Step 2. Choose the genre

Read the genre profile. It fixes the spine, the slides that must land, the backup set and
the questions the room will ask. A defence profile also carries the Kritikpunkte map.

### Step 3. Scaffold and theme

```bash
scripts/deck themes                                  # list built-in themes
scripts/deck init <name> --dir <where> [--theme x]   # deck.py + Assets/
```

When the source material has figures, derive the theme from their palette. Read
`references/theming.md` for the order of preference and the contrast rules. A deck that
disagrees with its own embedded diagrams looks borrowed.

### Step 4. Write the spine

Write the sections and the per-slide assertions before you write any slide. Read
`references/narrative.md` for the spine shape, the assertion-title rule, the
claim/evidence/bound rhythm and the words-per-minute budget. Read
`references/academic-narrative.md` for the academic objects. The assertions alone should
reconstruct the argument — the slide title, or the claim bar on a claim-first deck. If
they do not, the deck is a set of notes.

### Step 5. Write the content

Edit `deck.py`. Reach for an archetype first. `references/slide-recipes.md` maps a
slide's job to a layout. It also gives you the primitives and the composition patterns
for the cases that no archetype fits, and the academic compositions (research question,
contribution, positioning matrix, objectives against evidence, threats to validity,
references). Read `references/design-system.md` before the first slide, and again
whenever a slide looks wrong but nothing is broken.

Follow `evidence-on-slides.md` for the evidence. Follow `citing-on-slides.md` for
attribution. Follow `academic-register.md` for the voice.

Every slide needs notes. The notes carry a running clock, the spoken text and the answer
to the question the slide will provoke. `check` fails a slide with no notes.

### Steps 6 to 8. Build, check and render

```bash
scripts/deck build  <deck.py>    # -> out/<name>.pptx, and the PDF when export_pdf=True
scripts/deck check  <deck.py>    # Exit code equals the error count.
scripts/deck render <deck.py>    # -> out/render/  pdf + slide-NN.png + contact sheets
```

`Deck(export_pdf=True)` makes `build` write the PDF beside the `.pptx` as well, so the
deck folder holds both files and the export is not a step to remember. It needs
LibreOffice, and it does not need the rasteriser. `render` writes the PDF beside the
`.pptx` too, and keeps its working files in `out/render/`.

Look at the contact sheet. This step makes the deck good, and a model tends to skip it.
`check` cannot see a drop shadow, a centred bullet, an imbalance or a dead bottom third.
Read `references/verification.md` for the loop and the order of inspection. Read
`references/gotchas.md` when something renders wrong and no visible cause exists.

Repeat build and render until the sheet is clean. Two passes is normal.

### Step 9. Check the timing

```bash
scripts/deck timing <deck.py>
```

The verb reads the `ZEIT` budget from the notes. It sums the budget against
`Deck(minutes=...)`, and it reports each slide's spoken words against its slot at 125
words per minute. A script that does not fit its slot is the defect that a rehearsal
finds too late.

The total is the number that matters. A talk 30 seconds over is a talk that skipped its
conclusion. Fix the script or the budget. Do not trust speed.

### Step 10. Check the readiness (a defence or a viva)

`check` sees the structure. `render` sees the layout. Neither sees the argument, and a
defence is marked on the argument.

```bash
scripts/deck readiness <deck.py>   # argument skeleton: titles, statements, closing verdict, backup questions
```

Read the skeleton instead of the slides. Answer the five questions in
`references/defence-readiness.md`: abstraction, contribution, positioning, scope and
thread. Report each finding with its slide number. Fix, rebuild, and run the pass again
after the next render.

This is a reviewer pass. The verb extracts the argument. You supply the review.

### Step 11. Build the handout

```bash
scripts/deck handout <deck.py>     # -> out/render/<name>-handout.pdf
```

One page per slide: the slide image, its notes and its clock. The speaker rehearses from
it, and an examiner sometimes asks for it. Cut the script here when the timing report
says the script runs over budget.

### Step 12. Write the backup

Write eight to twelve slides after `deck.backup_divider()`. Each slide answers one
question that the talk will raise, and its notes name that question. The genre profile
lists the highest-value set for a defence, and that set includes the Kritikpunkte map.

## Error handling

- `deck: theme not found`. The theme JSON was not copied. `init` copies one. You can
  also run `scripts/deck themes` and pass an explicit path to `Theme.load()`.
- `deck: figure not found`. Figures resolve relative to the deck module. Use
  `HERE / "Assets/figures/..."` and check that the file exists.
- `check` reports `overlap` on a statement bar. Pass `reserve=s.statement_reserve` to
  the archetype above it.
- `check` reports `shadow`. A shape was built outside `deckkit.layout`. Build it with a
  primitive.
- `check` reports `figure`. A picture carries no `alt=` and no `credit=`. Add one. A
  reproduced figure needs its source. Every figure needs a description.
- `render` reports "LibreOffice not found". Install it with
  `brew install --cask libreoffice`. No fallback exists, because an unrendered deck is an
  unverified deck.
- `timing` reports a slide as "not budgeted". The notes have no
  `ZEIT h:mm–h:mm (N s)` line, so the slide is invisible to the budget. Fix the notes.
- `handout` embeds 100 MB of images, or the notes show `?`. The images are not being
  downscaled, or the machine has no Unicode font for the PDF. `deckkit/handout.py`
  handles both cases. A machine with neither Arial nor a DejaVu or Liberation face
  transliterates the quotes and dashes.
- `readiness` prints a title but no statement. The slide's claim lives only in the
  notes. The skeleton shows what the audience hears. Move the claim onto the slide, or
  accept that the slide carries none.
- You changed `deckkit/`. Rebuild and render a real deck, not the scaffold.

## Boundaries

**Will:**
- Author a deck module, choose a talk genre, derive a theme.
- Run build, check, render, readiness, timing and handout.
- Export `.pptx` and `.pdf`.
- Write backup slides and a Kritikpunkte map.
- Run the defence-readiness reviewer pass.

**Will not:**
- Edit a `.pptx` in place.
- Animate or transition slides.
- Write the underlying argument of a thesis. That work belongs to `academic-writing`.
- Produce anything other than a slide deck. Prose belongs to `human-writing`.
- Store deck content inside this skill.

## Done when

- `scripts/deck check <deck.py>` exits 0.
- You looked at the contact sheet, and the last build is the one you looked at.
- `scripts/deck timing` shows the noted budget meeting `minutes=`, and no slide's script
  runs long for its slot without a recorded decision.
- An academic talk has the genre profile's "what must land" slides, evidence that follows
  `evidence-on-slides.md`, and attribution that follows `citing-on-slides.md`.
- A defence has a readiness pass with the five questions answered and findings keyed to
  slide numbers.
- Every slide carries notes with a time budget and a quoted `Sprechtext`.
- The brief's four facts appear either in the deck module or in the printed defaults.
