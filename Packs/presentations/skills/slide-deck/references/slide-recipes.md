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
| a uniform grid of panels | `s.panel_grid` | a chip row, a step grid, a Q&A block |
| compare two things | `s.two_columns` | a labelled pair |
| one number per category | `s.kpi_row` | progress or outcome |
| a figure is the argument | `s.figure_beside` | result |
| dense facts, many rows | `s.table` | evidence |
| one sentence matters | `s.statement` | the takeaway |
| one sentence *is* the slide | `s.headline` | pull-quote |
| the one question | `s.headline` + a setting line | the research question |
| what the field gained | `s.headline` + `s.panel_grid` | claim, then bound |
| where this sits in the literature | `s.table` with a construct column | the empty column |
| objectives against evidence | `s.panel_grid` / `s.table` | method |
| the limits, by type | `s.panel_grid(columns=2)` | four validities |
| the points you must answer | `s.table`, backup | speaker-only |
| the sources the talk cited | `s.panel_grid`, `size=9.5` | references |
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
The title renders italic. It is a title, not a claim. `meta` is the block nobody
reads and everybody expects.

### `deck.agenda` / sections
```python
deck.agenda(["Problem", "Methode", "Ergebnisse", "Backup"])
deck.section(1, "Der Fall und warum er\nein Forschungsproblem ist",
             "Der Code ist nicht die Spezifikation.")
```
The section divider carries **one line**: the claim that section will earn. Not a
list of what is inside it. `deck.backup_divider()` is the same shape for the
appendix, and signals clearly that the talk is over.

The running list is **not framed by rules**. The active item carries a small accent
bar and the ink colour, and that is the only state a reader has to see; full-width
lines above and below a list of six words read as a table border and compete with
the slide's claim bar.

### `s.three_columns`: the default
```python
s = deck.open("An assertion, not a topic", "2 · Methode")
s.three_columns([
    ("Heading A", "accent",    ["Point.", "Point."]),
    ("Heading B", "secondary", ["Point.", "Point."]),
    ("Heading C", "caution",   ["The limitation."]),
], reserve=s.statement_reserve)
s.statement("The sentence the slide exists to deliver.")
```
Three is the sweet spot. Two works. Four is a table wearing a costume. Use
`s.table`. The `voice` drives the top rule, the heading colour and the emphasis
colour together, so a card is consistent by construction.

### `s.two_columns`
For a labelled pair: before/after, what-works/what-does-not, claim/bound.
```python
s.two_columns(("Was sich überträgt", [...]), ("Grenzen", [...]), reserve=0)
```

### `s.panel_grid`: a uniform grid of plain panels
When a row or block of equal cells carries the slide, and each cell is a bare panel
rather than a `card` with a header rule. Styling is grid-level, with one exception.
`voices` gives a two-tone pair for a claim-against-bound comparison.
```python
# a chip row: no labels, three across
s.panel_grid([
    [f"**O1**   {name}", text],
    [f"**O2**   {name}", text],
    [f"**O3**   {name}", text],
], columns=3, height=1.45, size=11.5, pad=0.22, voice="accent")

# a labelled Q&A block: 2 x 2, a question above each answer
s.panel_grid([
    (question, answer),
    ...
], columns=2, row_gap=0.34, height=1.78, size=12, pad=0.20,
   outline=s.theme.hex("hairline"), label_colour=s.theme.hex("accent"))

# a two-tone pair: what transfers against what limits it
s.panel_grid([
    ["Was sich überträgt", ...],
    ["Grenzen", ...],
], columns=2, voices=("accent", "secondary"), size=12.5)
```
A card is `[items]` (plain panel) or `(label, [items])` (a raised label above the
panel; the label band is added automatically). `gap` is the **column** gap, as in
`three_columns`; a panel's paragraph spacing is `para_gap`, since the name `gap` is
already spoken for. Everything else (`voice`, `fill`, `outline`, `size`, `pad`,
`bullet`, `line_spacing`) forwards to `panel`. Leave `height` out and the grid
splits the body evenly, so it can never run past the body floor, pass it when the
cells carry a fixed amount of text and you want the slack at the bottom.

### `s.kpi_row`: progress at a glance
```python
s.kpi_row([
    ("O1", "Verhaltenserhalt",   "0/3 → 1/3 → 1/3 → 6/6", "accent"),
    ("O2", "Nachvollziehbarkeit","0/3 → 2/3 → 3/3 → 6/6", "secondary"),
    ("O3", "Wartbarkeit",        "—   —   0/3  →  offen",  "caution"),
])
```
The value is the number, the caption names it, the trail shows the path. The trail
is where the story is, a bare `6/6` says nothing, `0/3 → 1/3 → 1/3 → 6/6` says the
configuration took four attempts.

### `s.figure_beside`: a result
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

### `s.table`: evidence
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
`s.headline` is the same thing at the top, larger, and centred, reserve it for a
research question, a contribution or a verdict, where the sentence *is* the slide.

`width=` and `x=` narrow either one, which is how a statement is confined to the
left column when a figure occupies the right.

### Academic slides

These are compositions, not new archetypes, the mechanism already carries them. What
is new is the content rule. Read `academic-narrative.md` before any of them.

**The research question.** `headline` carries the question verbatim; a panel names the
case as the *setting*. One question, at class level.
```python
s.headline("Welche Rolle spielt Context Engineering bei Sprach- und "
           "Laufzeitmigrationen, deren ~Verhaltensanforderungen außerhalb des "
           "Quellcodes~ liegen?", height=1.85, size=20)
s.panel(s.x, s.top + 2.07, s.w, 0.72,
        "~Untersuchungssetting:~ eine hochgradig kundenspezifische Pipeline ...",
        voice="caution", size=13.5, pad=0.24, anchor="middle")
```
Apply the delete test before you build it: remove the case and the question must still
ask something.

**The contribution.** Claim unhedged in one sentence, then the bound and the type in the
cards below. Never the other order.
```python
s.headline("Ich zeige an einem realen Enterprise-Fall, dass ~Context Engineering "
           "als Designgegenstand evaluierbar~ ist: ...",
           height=1.92, size=18, align="left")
s.panel_grid([
    ("Zwei Ebenen des Beitrags", [...]),
    ("Warum das Forschung ist und keine Beratung", [...]),
], columns=2, gap=0.40, top_pad=2.22, height=1.98, voice="accent",
   size=12.5, para_gap=9, bullet=True)
```

**The positioning matrix.** A `s.table` whose last column is the **construct** each
study operationalises. Without it the matrix is an engineering table and the standard
criticism follows; with it, it positions.
```python
table([("Study", 2.7), ("Task", 2.4), ("Context supplied", 3.0),
       ("Construct operationalised", 2.4), ("Outcomes", 1.4)], rows, size=10)
```

**Objectives against evidence.** One card per objective; the measure and the evidence
source in the card. It is the method slide and the evaluation-design slide at once.
```python
s.panel_grid([
    ["**O1** Functional correctness", measure, evidence],
    ["**O2** Process traceability", measure, evidence],
    ["**O3** Maintainability", measure, evidence],
], columns=3, height=2.6, size=11.5, voice="accent")
```

**The 1–3–3: question, objectives, metrics.** The same slide with the derivation made
visible: the research question in a full-width tinted band, one panel per objective
below it, and one panel per metric below that, column-aligned. It answers "are the
constructs derived, or set?" by construction, because every objective carries the
locator of its source. Use `~Source:~` for the citation line so it reads as
provenance rather than as body text.
```python
s.panel(s.x, s.top, s.w, 0.92, question, fill=s.theme.hex("accent_light"),
        size=12.5, anchor="middle")
s.panel_grid([
    ["**Objective 1 · Behavior preservation**", definition, "~Source:~ ISO/IEC 25010:2023, §3.1.2."],
    # … one per objective
], columns=3, top_pad=1.08, height=1.72, voice="accent", size=11.5, para_gap=7)
s.panel_grid([
    ["**Metric 1 · Automated output comparison**", measure, "**Met:** all pairs agree."],
    # … one per objective, in the same column order
], columns=3, top_pad=2.98, height=1.72, voice="secondary", size=11, para_gap=7)
```
The two grids must use the same `columns` and `gap`, or the tiers stop lining up, and
`top_pad` is measured from `s.top`, so the second grid's value is the band height plus
the gap plus the first grid's height.

**Threats to validity.** One cell per Wohlin type, conclusion, internal, construct,
external, each with its named threat. Evidence of rigour, not a confession.
```python
s.panel_grid([
    ("Conclusion validity", [threat]),
    ("Internal validity", [threat]),
    ("Construct validity", [threat]),
    ("External validity", [threat]),
], columns=2, height=1.85, size=11.5, voice="caution", para_gap=6)
```

**The points you must answer.** A defence backup slide: one row per point in the
written assessment, the stance, and where the evidence lives. Speaker-only, never
opened unasked.
```python
s.table([("Kritikpunkt", 4.05), ("Antwort", 4.35), ("Beleg", 3.49)], rows, size=10.5)
```

**The references slide.** One slide, before the appendix, listing only what the talk
cited. Two columns, `small` or `tiny`, for the room to photograph, never read aloud.
See `citing-on-slides.md`.
```python
s = deck.open("Literatur", "Referenzen")
s.panel_grid([
    [source_a, source_b, source_c],
    [source_d, source_e, source_f],
], columns=2, height=4.6, size=9.5, voice=None, bullet=False, para_gap=5)
```

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
for collisions. Use it for an arrow, a connector or a custom underline. Anything you draw
that a reader would not call a block belongs here.

## Composition patterns that recur

**Figure + rail + narrow statement**, a result slide with a conclusion.
```python
s.picture(FIG / "plot.png", s.x, s.top, 6.3, 2.8)
s.bullets(s.x + 6.65, s.top, s.w - 6.65, [...])
s.statement("The conclusion.", x=s.x, width=5.55)
```

**The chip row**, three small labelled boxes, no bullets. This is `s.panel_grid`
with `columns=3` and no labels; the loop below is what it replaces.
```python
for i, (code, name, text) in enumerate(chips):
    s.panel(s.columns(3, 0.34)[i], s.top + 3.0, s.span(3, 0.34), 1.45,
            [f"**{code}**   {name}", text], voice="accent", size=11.5)
```

**The numbered step card**, a plan, or a "next steps" slide. `s.panel_grid` with a
four-line card, or `three_columns` when the cell wants a header rule.
```python
s.panel(x, y, w, h, ["**1**", "**Title**", body, "Ergebnis: <what it buys>"],
        fill=s.theme.hex("muted_neutral"), voice="accent")
```
The fourth line is the one that earns the card. A step with no outcome is a task.

**A horizontal chain**, four iterations, a pipeline. Native shapes, and mark the
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

Compose from primitives. That is the escape hatch, and using it is not a failure. The
archetypes cover the shapes that recur, not every slide a real talk needs.

Two constraints on a custom slide:

1. **Start at `s.top` and stop by `s.bottom`.** Hard-coding 1.7 or 7.2 will be wrong
   the moment the title wraps to two lines.
2. **Run `deck check` after it.** A hand-composed slide is exactly where an overlap
   or a footer collision gets introduced, and `check` catches both without eyes.

If the same custom composition appears on a third slide, it is an archetype. Add it
to `slide.py` and note it here, that is how this list grew.
