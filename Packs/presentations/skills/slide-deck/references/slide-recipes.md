# Slide recipes

The archetypes, when to reach for each, and what to do when none of them fit.

## Contents

- [Choosing a layout](#choosing-a-layout)
- [The archetypes](#the-archetypes)
- [The primitives](#the-primitives)
- [Composition patterns that recur](#composition-patterns-that-recur)
- [When nothing fits](#when-nothing-fits)

## Choosing a layout

| the slide's job | use | reads as |
|---|---|---|
| set up the talk | `deck.title` | front matter |
| show the route | `deck.agenda` | orientation |
| turn a corner | `deck.section` | a beat |
| three parallel things | `s.three_columns` | the default content slide |
| compare two things | `s.two_columns` | a labelled pair |
| one number per category | `s.kpi_row` | progress or outcome |
| a figure is the argument | `s.figure_beside` | result |
| dense facts, many rows | `s.table` | evidence |
| one sentence matters | `s.statement` | the takeaway |
| one sentence *is* the slide | `s.headline` | pull-quote |
| close | `deck.closing` | the answer |
| anticipate a question | `deck.backup_divider` + `deck.open` | reference |

A talk of ten minutes is roughly: 1 title, 1 agenda, 4–5 sections, 10–13 content
slides, 1 close, 8–12 backup slides. If the content count is above 15, the talk is
being written rather than designed.

## The archetypes

### `deck.title`
```python
deck.title(
    kicker="Kolloquium zur Bachelorarbeit",
    title="An assertion or a full title, newline for the break",
    date="FHDW · 18. September 2026",
    meta=["Author · Matrikelnummer · Studiengang", "Examiners or host"],
    tagline="10 Minuten Vortrag",
)
```
The title renders italic — it is a title, not a claim. `meta` is the block nobody
reads and everybody expects.

### `deck.agenda` / sections
```python
deck.agenda(["Problem", "Methode", "Ergebnisse", "Backup"])
deck.section(1, "Der Fall und warum er\nein Forschungsproblem ist",
             "Der Code ist nicht die Spezifikation.")
```
The section divider carries **one line** — the claim that section will earn. Not a
list of what is inside it. `deck.backup_divider()` is the same shape for the
appendix, and signals clearly that the talk is over.

### `s.three_columns` — the default
```python
s = deck.open("An assertion, not a topic", "2 · Methode")
s.three_columns([
    ("Heading A", "accent",    ["Point.", "Point."]),
    ("Heading B", "secondary", ["Point.", "Point."]),
    ("Heading C", "caution",   ["The limitation."]),
], reserve=s.statement_reserve)
s.statement("The sentence the slide exists to deliver.")
```
Three is the sweet spot. Two works. Four is a table wearing a costume — use
`s.table`. The `voice` drives the top rule, the heading colour and the emphasis
colour together, so a card is consistent by construction.

### `s.two_columns`
For a labelled pair: before/after, what-works/what-does-not, claim/bound.
```python
s.two_columns(("Was sich überträgt", [...]), ("Grenzen", [...]), reserve=0)
```

### `s.kpi_row` — progress at a glance
```python
s.kpi_row([
    ("O1", "Verhaltenserhalt",   "0/3 → 1/3 → 1/3 → 6/6", "accent"),
    ("O2", "Nachvollziehbarkeit","0/3 → 2/3 → 3/3 → 6/6", "secondary"),
    ("O3", "Wartbarkeit",        "—   —   0/3  →  offen",  "caution"),
])
```
The value is the number, the caption names it, the trail shows the path. The trail
is where the story is — a bare `6/6` says nothing, `0/3 → 1/3 → 1/3 → 6/6` says the
configuration took four attempts.

### `s.figure_beside` — a result
```python
s.figure_beside(FIG / "plot.png", [
    "What the figure shows, in one line.",
    "The number worth remembering.",
], note="Source: <where it came from, and what it does not show>.")
```
Figure left, two or three interpreting lines right. The `note` sits under the figure
at caption size; use it for provenance, which a technical audience will ask for.

**Never re-draw a figure that already exists.** If the source document has plots,
use those files. Redrawing produces a second visual language on the same slide, and
it will disagree with the paper the audience may have read.

### `s.table` — evidence
```python
s.table([("Study", 3.3), ("Task", 2.85), ("Validated", 3.3)],
        [["Ziftci 2025", "Integer width", "Build, tests, review"]],
        reserve=s.statement_reserve, aligns=["left", "left", "left"])
```
Widths are absolute inches and must sum to ≤ 11.89. Pass `reserve` to let `row_h`
compute itself; that is the whole reason the parameter exists. Above eight rows,
either the font (`size=10`) or the row count has to give.

### `s.statement` and `s.headline`
`s.statement` is a filled bar at the bottom of the slide carrying the one sentence.
`s.headline` is the same thing at the top, larger, and centred — reserve it for a
research question, a contribution or a verdict, where the sentence *is* the slide.

`width=` and `x=` narrow either one, which is how a statement is confined to the
left column when a figure occupies the right.

## The primitives

When an archetype does not fit, compose. Everything takes the same `(x, y, w, h)`
in inches.

```python
s.panel(x, y, w, h, items, voice="accent", size=13, bullet=True, fill=None)
s.write(x, y, w, h, ["line", "line"], size=13, bold=False, anchor="top")
s.table(cols, rows, reserve=0)
s.picture(path, x, y, w, h, note=None)
s.kpi(x, y, w, h, value, caption, trail=None, voice="accent")
s.card(x, y, w, h, title, items, voice="accent")
s.rule(x, y, w, colour=None, thickness=None)
s.rect(x, y, w, h, fill=..., line=..., deco=False)
```
`align` and `anchor` accept strings (`"left"`, `"centre"`, `"middle"`), so a content
file needs no python-pptx import. `anchor=2` is not a thing and will raise.

`s.rect(..., deco=True)` marks a shape as decoration so `check` skips it when looking
for collisions. Use it for anything you draw that a reader would not call a block —
an arrow, a connector, a custom underline.

## Composition patterns that recur

**Figure + rail + narrow statement** — a result slide with a conclusion.
```python
s.picture(FIG / "plot.png", s.x, s.top, 6.3, 2.8)
s.bullets(s.x + 6.65, s.top, s.w - 6.65, [...])
s.statement("The conclusion.", x=s.x, width=5.55)
```

**The chip row** — three small labelled boxes, no bullets.
```python
for i, (code, name, text) in enumerate(chips):
    s.panel(s.columns(3, 0.34)[i], s.top + 3.0, s.span(3, 0.34), 1.45,
            [f"**{code}**   {name}", text], voice="accent", size=11.5)
```

**The numbered step card** — a plan, or a "next steps" slide.
```python
s.panel(x, y, w, h, ["**1**", "**Title**", body, "Ergebnis: <what it buys>"],
        fill=s.theme.hex("muted_neutral"), voice="accent")
```
The fourth line is the one that earns the card. A step with no outcome is a task.

**A horizontal chain** — four iterations, a pipeline. Native shapes, and mark the
arrows `deco`:
```python
for i, label in enumerate(steps):
    x = s.x + i * (2.72 + 0.34)
    s.panel(x, s.top, 2.72, 1.45, [f"**{label}**", detail], fill=...)
    if i < len(steps) - 1:
        s.rect(x + 2.765, s.top + 0.645, 0.25, 0.16,
               fill=s.theme.hex("accent_mid"), shape=MSO_SHAPE.RIGHT_ARROW, deco=True)
```

## When nothing fits

Compose from primitives. That is the escape hatch, and using it is not a failure —
the archetypes cover the shapes that recur, not every slide a real talk needs.

Two constraints on a custom slide:

1. **Start at `s.top` and stop by `s.bottom`.** Hard-coding 1.7 or 7.2 will be wrong
   the moment the title wraps to two lines.
2. **Run `deck check` after it.** A hand-composed slide is exactly where an overlap
   or a footer collision gets introduced, and `check` catches both without eyes.

If the same custom composition appears on a third slide, it is an archetype. Add it
to `slide.py` and note it here — that is how this list grew.
