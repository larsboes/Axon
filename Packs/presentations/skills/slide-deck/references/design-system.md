# Design system

The rules that make a deck look designed rather than assembled. They are the reason
a deckkit deck reads as one document, and the reason a theme swap is a one-file
change. Break them deliberately, not by accident.

## Contents

- [The four non-negotiables](#the-four-non-negotiables)
- [Role-based colour](#role-based-colour)
- [The type scale](#the-type-scale)
- [The grid](#the-grid)
- [Vertical rhythm](#vertical-rhythm)
- [Emphasis](#emphasis)
- [Density](#density)
- [What the system deliberately does not do](#what-the-system-deliberately-does-not-do)

## The four non-negotiables

1. **One idea per slide, stated as an assertion in the title.** `Wartbarkeit bleibt
   offen` is a title. `Ergebnisse` is a filing label. A reader who reads only the
   titles of a deck should get the whole argument.
2. **Claim, then evidence, then bound — in that order.** A slide that opens with its
   limitation has already lost. The bound belongs in the same breath as the claim,
   never instead of it.
3. **The accent colour is a voice, not decoration.** One accent for the spine, one
   secondary, one caution. A slide that uses all three for three equal points is
   three slides wearing a trenchcoat.
4. **Nothing is centred except a pull-quote.** Left-align body text. Centred panels
   read as a poster, not as a slide, and they make a two-line difference look like a
   mistake.

## Role-based colour

The palette is named by job, never by hue. `accent`, `secondary`, `caution`,
`emphasis_on_dark`. That is what allows `Theme.emphasis_on(fill)` to pick the right
emphasis colour without the content knowing which surface it landed on.

| role | used for | not used for |
|---|---|---|
| `accent` | spine: the header rule, headings, statement bars, the badge | a third category of content |
| `secondary` | a second distinct voice: a comparison column, a method row | emphasis |
| `caution` | limits, failures, open items, the thing that went wrong | anything neutral |
| `*_light` | the tint of a `kpi` card | panel fills on a text-heavy slide |
| `muted_neutral` | zebra striping, inert panels | text backgrounds |
| `hairline` | table rules, thin dividers | anything a reader must see |
| `emphasis_on_dark` | `~accent~` on a filled surface | text on white |

Three hues maximum per slide, and only two of them for content: the third is the
caution voice and should appear on maybe one slide in five.

## The type scale

Nine sizes, ~1.3 apart. `section` 38 · `title` 24 · `kpi` 31 · `subtitle` 16.5 ·
`statement` 15.5 · `body` 13 · `small` 11 · `tiny` 9.5 · `footer` 10.5.

If a slide needs a tenth size, the slide has two slides in it. The one deliberate
exception is a `kpi` value, which is a graphic element rather than text.

Font: **Arial**. Not because it is good, but because the figures a technical deck
embeds are almost always rendered in it, and a deck whose body text disagrees with
its own diagrams looks borrowed. If the source figures use something else, match
*them* — that is a `theming.md` decision, not a taste decision.

## The grid

16:9, 13.333 × 7.5 inches. Margin 0.72. Content width 11.893.

```
0.00 ┌─────────────────────────────────────────────┐
     │  title (0.50, up to 2 lines at 0.50 each)    │
     │  subtitle                                    │
1.4  │  ─────────────────────── accent rule ────────│
1.7  │  ↕ content starts here (computed)            │
     │                                              │
6.68 │  ↕ body floor — nothing ends past this       │
6.83 │  [badge]  footnote            label ·  page  │
7.50 └─────────────────────────────────────────────┘
```

The rule position is **computed from the title height**, not fixed. A two-line
title pushes everything down by 0.50in. That is deliberate: a two-line title with a
fixed rule under it looks cramped, and the reference decks in the wild do the same.

Columns come from `s.columns(n, gap)` and `s.span(n, gap)`; never compute a column
width by hand, because the answer changes with the theme's margin.

## Vertical rhythm

Three numbers matter, and the archetypes know them:

- `s.top` — where content starts. Read it, never hard-code 1.7.
- `s.body()` — usable height, minus anything you reserve below.
- `s.statement_reserve` — pass this as `reserve=` when a statement bar follows.

The failure this prevents: columns sized to the body floor, then a statement bar
drawn at the bottom, landing on the columns. `deck check` reports it as an overlap,
but the fix is to reserve the space rather than to shrink something afterwards.

Leave at least 0.15in between the last block and the body floor. A deck that runs
every slide to the footer looks anxious.

**Aim for a deliberate bottom margin, not a full slide.** The most common amateur
tell in a generated deck is content stretched to fill every slide exactly. Two
thirds of the body height is a comfortable slide; a wall-to-wall slide is a
document.

## Emphasis

Two mechanisms, and they are not interchangeable:

- `**bold**` — the load-bearing words in a sentence. Two or three per bullet, at most.
- `~accent~` — *one* phrase per block, the phrase a reader should remember if they
  read nothing else.

A block with four accented phrases has none. If everything is emphasised, the
emphasis is the new baseline and the slide has to be read linearly — which is what
the speaker is for.

Emphasis never carries meaning alone. If the bold words are deleted the sentence
must still say the same thing.

## Density

The working numbers, in words per slide:

| slide kind | target | hard ceiling |
|---|---|---|
| assertion + three columns | 90–130 | 150 |
| figure + bullet rail | 60–90 | 120 |
| statement or pull-quote | ≤ 40 | 60 |
| table | 80–140 | 180 |
| reference / backup panel | 120–200 | 220 |

`check` warns above `Deck(max_words=…)`, default 120. A deck whose audience *reads*
the slide while listening — a limits panel, a backup slide — legitimately raises it;
say so in the deck file rather than lowering the number globally, or the warning
stops meaning anything.

A bullet longer than two lines is a paragraph. Split it or cut it.

## What the system deliberately does not do

- **No gradients, no shadows, no glow.** `layout._flat()` strips PowerPoint's theme
  preset style from every shape for exactly this reason. A deck with shadows on
  every box looks like a template, and everyone has seen the template.
- **No decorative shapes.** Every rectangle in a deckkit deck either holds text or
  is a hairline. A block with nothing in it is a bug, not a design.
- **No slide transitions and no animation.** They cannot be verified by a contact
  sheet, they fail differently in every renderer, and they cost the audience
  attention that belongs to the argument.
- **No icon set.** A deck that needs icons to be read has a title problem.
