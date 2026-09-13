# Verification

Three tools, and they answer different questions.

`deck check` reads the `.pptx` and reports what is structurally wrong. No eyes
needed, so it runs in a loop.

`deck render` produces a PDF, one PNG per slide, and a **contact sheet**, a 3×3
grid of the whole deck. That is the one that matters: a deck is a sequence, and a
sequence cannot be judged one slide at a time.

`deck readiness` prints the **argument**, the ordered assertions, the sentences
slides exist to deliver, the closing verdict, the question each backup slide answers.
It needs eyes too, but a different kind: read it and answer the five questions in
`defence-readiness.md`. `check` cannot see a shadow; `render` cannot see whether the
argument answers the question.

Neither of the first two replaces the other, and for an academic talk neither replaces
the third. `check` cannot see a shadow, a centred bullet, an
imbalance, or a dead bottom third. Rendering cannot tell you *why* something looks
wrong, and does not scale to 60 slides in one pass.

## Contents

- [The loop](#the-loop)
- [What `check` covers](#what-check-covers)
- [Overlap detection, and why decoration is marked](#overlap-detection-and-why-decoration-is-marked)
- [Recovering from a failed check](#recovering-from-a-failed-check)
- [Proportionate verification](#proportionate-verification)

## The loop

```
build  →  check  →  render  →  LOOK  →  fix  →  build
```

**The look is not optional.** Every defect in `gotchas.md` was found by looking at a
rendered contact sheet and by nothing else. The failure mode this skill exists to
prevent is a deck that reads correctly in source and is visibly wrong on screen.

When you look, in this order:

1. **The contact sheet, whole.** Balance, rhythm, where the eye snags. Is any slide
   much denser than its neighbours? Do the dividers punctuate, or interrupt?
2. **The bottom third of every slide.** Dead space, or a footer collision.
3. **The two or three slides whose layout is new.** Full resolution, `render/slide-NN.png`.
4. **The figures.** Do they sit flush, or is there a white box on a tinted background?

Then fix, rebuild, and render again. Two passes is normal; three is common on a
first deck.

## What `check` covers

| check | severity | what it means |
|---|---|---|
| `notes` | error | the slide has no speaker notes |
| `markup` | error | a stray `*` or `~` will print literally |
| `newline` | error | a literal newline inside a run, PowerPoint renders it as a space |
| `shadow` | error | a shape kept its theme preset style, so it has a drop shadow |
| `overlap` | error | two content blocks are drawn on top of each other |
| `overflow` | error | a block or a text run reaches past the body floor |
| `bounds` | error | a shape leaves the canvas |
| `alignment` | warn | a paragraph has no explicit alignment and inherits centring |
| `font` | warn | a run has no font name and falls back to the viewer's default |
| `textfit` | warn | a panel is too short for the text in it |
| `density` | warn | more words than the deck's `max_words` |
| `figure` | warn | a picture carries no alt text and no credit |

Exit code is the error count, so `deck check && deck render` is a usable gate.

Warnings are nudges, not rules. `textfit` deliberately over-reports, because a
missed overflow costs a slide and a false warning costs one glance. `density` is the
one to take seriously, a wall of text is the most common way a technically correct
deck fails its audience.

## Overlap detection, and why decoration is marked

`check` compares **content blocks**, filled or outlined rectangles that hold
something a reader would call a box. Three things are excluded, each for a measured
reason:

- **Decoration.** `layout.rect(..., deco=True)` prefixes the shape name, and `check`
  skips it. Hairlines and table outlines are *meant* to sit on an edge; counting them
  would bury the real finding. If you draw a custom arrow or connector, mark it.
- **Pure textboxes.** These declare a generous height and use what they need, a
  bullet rail claims 6in and may occupy 2, so their box is not a footprint.
  `_check_geometry` measures their estimated *text* extent instead.
- **Pictures.** An image is never the accidental collision this check exists for.

Everything else is compared pairwise. Adjacent table cells share an edge exactly, so
the tolerance is 0.04in on both axes before a pair counts as overlapping.

## Recovering from a failed check

| finding | fix |
|---|---|
| `overlap` on a statement bar | pass `reserve=s.statement_reserve` to the archetype above it |
| `overlap` between two hand-placed blocks | `s.body(reserve=…)` to compute the height, or move one block |
| `overflow` at the footer | reduce `reserve` targets, shrink the text, or shorten it, never move the body floor |
| `textfit` | shorten the text. Growing the box is usually the wrong answer, because it pushes the box into something else |
| `shadow` | the shape was not made by `deckkit.layout`. Build it with a primitive |
| `markup` | an unpaired `*` or `~`. Pair it or delete it |
| `notes` | write them; an undocumented slide is an undesigned slide |
| `density` | split the slide, or raise `Deck(max_words=…)` and say why in a comment |
| `figure` | pass `alt=` to `s.picture`, and `credit=` for a reproduced figure. `figure_beside`'s `note` becomes the credit |

Never lower the body floor or the footer position to silence a finding. Both are
theme constants, and both are where they are because the badge is 0.36in tall.

## Proportionate verification

Match the check to the change:

- **Text edit only** → `build` + `check`. No render.
- **A layout or a new slide** → `build` + `check` + `render` + look at the sheet.
- **A title, a question slide, or the contribution** → `build` + `check` + `readiness`,
  and answer the five in `defence-readiness.md`. A text edit that changes what a slide
  claims is an argument change, not a layout change.
- **A theme or `deckkit/` change** → rebuild a *real* deck, not a scaffold, and render all
  of it. Library changes break archetypes in ways a two-slide test cannot show.
- **A `check` change** → prove it fires. Break a deck on purpose, confirm the finding,
  then unbreak it. A check that never fails is worse than no check, because it is
  trusted.

Report what you ran and what you looked at. "Built and checked" and "rendered and
inspected" are different claims, and only the second one covers layout.
