# Theming

A theme is one JSON file: a role-based palette, a type scale and the grid. Swapping
it re-themes every slide without touching content.

## Contents

- [Derive it from an artifact, not from taste](#derive-it-from-an-artifact-not-from-taste)
- [The file](#the-file)
- [Choosing the colours](#choosing-the-colours)
- [Contrast, and the projector tax](#contrast-and-the-projector-tax)
- [Type](#type)
- [Grid](#grid)
- [Verify a theme by rendering, not by reading](#verify-a-theme-by-rendering-not-by-reading)

## Derive it from an artifact, not from taste

The best theme for a deck is usually already sitting in the source material. A
colloquium deck built from a thesis should use the thesis's figure palette, because
those figures will be embedded in the slides and a deck that disagrees with its own
diagrams looks assembled.

Where to look, in order:

1. **The figures.** Their plotting palette. If the source repo has a
   `visual-palette.json`, a `theme.py`, or a matplotlib style, that is the answer.
2. **The document template.** Its heading colour, if any.
3. **The institution's brand.** Last, because brand colours are chosen for letterheads
   and are usually too saturated for a full-bleed statement bar.
4. **`slate`.** A neutral default, for a deck with no source artifact to match.

The worked example is `assets/themes/warm-scientific-teal.json`, derived from a
thesis whose figures share one plotting palette. Its provenance line says so.

## The file

```json
{
  "name": "warm-scientific-teal",
  "provenance": "one line: where this came from, so the next person knows it is not arbitrary",
  "palette": {
    "ink": "1F2727", "white": "FFFFFF", "paper": "FBFAF7",
    "accent": "4A6A69", "accent_mid": "85A09F", "accent_light": "E6EFED",
    "secondary": "527B8B", "secondary_light": "E7EFF1",
    "caution": "8A6653", "caution_light": "F3E9E4",
    "emphasis_on_dark": "DBAF26",
    "muted_neutral": "F3F5F3", "hairline": "DDD6CF"
  },
  "type": { "font": "Arial", "title": 24.0, "body": 13.0 },
  "grid": { "title_rule": 0.0, "corner_radius": 0.09, "flourish": 1.0 }
}
```

Hex without `#`. Nine roles are required (`REQUIRED_ROLES` in `theme.py`); the three
optional ones, `accent_mid`, `muted_neutral`, `hairline`, default to a sane
derivation if omitted. `type` and `grid` merge over the defaults, so a theme only
states what it changes.

## Choosing the colours

| role | pick it by | watch for |
|---|---|---|
| `ink` | near-black, slightly warm or cool to match the accent | pure `000000` is harsh at 24pt |
| `accent` | the source palette's primary | must reach ~4.5:1 against white for the `small` size |
| `accent_mid` | `accent` mixed 55% toward white | it carries footnotes, if it is unreadable, the deck looks grey |
| `accent_light` | `accent` mixed 90% toward white | must be distinguishable from white on a projector |
| `secondary` | the source palette's second hue | needs the same contrast discipline as `accent` |
| `caution` | a warm, muted tone | never a bright red, a limit is not an alarm |
| `emphasis_on_dark` | a light warm tone | it must read on a **filled** `accent`, not on white |
| `hairline` | `ink` mixed 85% toward white | if visible from three metres it is too strong |

The one role people get wrong is `emphasis_on_dark`. On a filled accent bar, `accent`
is invisible against itself, so `Theme.emphasis_on(fill)` switches emphasis to this
role. Pick it by putting it on a filled `accent` rectangle and checking it reads.

## Contrast, and the projector tax

Projectors wash out mid-tones, and a conference room is often brighter than the
monitor the deck was built on. Rules that survive:

- Body text and anything at `small` or below is `ink` on white, or white on `accent`.
  Nothing else. Coloured small text fails.
- `accent_mid` is for footnotes and captions only, things that are allowed to be
  hard to read.
- A `caution` panel is `caution_light` with `ink` text. Never white on `caution_light`.
- Test the palette at 40% brightness once. If the deck turns into grey rectangles,
  too much of it is tints.

## Type

Set `font` to whatever the source figures use. If there are no figures, Arial, it is
the lowest-common-denominator sans and renders identically everywhere, which matters
more for a deck than character does.

The scale is nine sizes and is not meant to be re-tuned per deck. If a theme needs a
tenth size, the deck has a layout problem.

**Typeface is not a theme decision that travels.** A theme naming a font the
presenting machine lacks will substitute silently, and the substitution changes line
wrapping, which changes every layout that was verified. Prefer a font the whole world
has. If the deck must use a specific face, embed it in the `.pptx` or accept that the
verified layout is only verified on this machine.

## Grid

Leave `grid` alone unless the deck is not 16:9. The numbers encode the relationship
between the badge height, the body floor and the footer, and changing one without the
others puts every slide 0.15in out.

Four values are meant to be set per deck:

| key | default | what it does |
|---|---|---|
| `title_rule` | `0.0` | thickness of the full-width rule under a slide heading. `0` draws none and gives the claim-first shape. |
| `corner_radius` | `0.09` | corner radius of panels, statement bars and KPI cards, in inches. `0` is square. Tables are always square. |
| `flourish` | `0.0` | `1` draws the background motif (`layout.flourish`). Off by default, because a motif is a per-deck decision. |
| `content_y` | `0.92` | where content starts on a slide that carries no title. |

The third value worth touching is `margin` (0.72), and only to make a deck feel
denser or more generous. If the margin changes, re-render: every column width is
derived from it.

## Verify a theme by rendering, not by reading

A palette that looks right in JSON can be wrong in three ways that only a render
shows: an emphasis colour that disappears on a filled bar, a tint that is
indistinguishable from white, and a `caution` that reads as an error. Build a deck
with all three voices on one slide, render it, and look.

```bash
scripts/deck themes                      # list what ships
scripts/deck init probe --theme slate    # scaffold
scripts/deck build probe/deck.py && scripts/deck render probe/deck.py
```
