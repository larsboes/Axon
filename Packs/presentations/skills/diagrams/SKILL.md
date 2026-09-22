---
name: diagrams
description: Renders Mermaid diagram sources to themed PNG figures, deriving the renderer theme from the artifact's own palette so a figure and the slides or pages around it cannot drift onto different colours. Use when a diagram should become a figure — a tree, a process, a decision path, a matrix — when a .mmd source needs a themed render, when a deck or paper has to embed a diagram, or when someone asks for mermaid-cli, a flowchart PNG, or a figure that matches the deck's theme. Do not use for data plots (that is a plotting library and the paper's own tooling), for editing a .pptx or a document in place, or for drawing a diagram by hand in a slide editor.
allowed-tools: Read, Write, Edit, Bash
---

# Diagrams

A diagram source is text. A figure is the PNG it renders to. This skill turns the
first into the second with the colours of the artifact that will embed it.

The tool surface is `<skill>/scripts/diagrams theme|render|doctor`. Python runs
through `uv`, which pins the interpreter. The renderer is pinned in the code, not
floating.

## The one idea

**The palette is the source, and the renderer theme is derived.** A deck already
carries a palette in `theme.json`. A paper carries one in whatever file its plot
tooling reads. A diagram must not have a third one, because a diagram authored in
its own colours is the figure that makes the slides around it look borrowed.

So the Mermaid theme is never hand-written. `diagrams theme` reads a palette and
writes a renderer config. Change `accent` in `theme.json` and every diagram
follows on the next render.

| what Mermaid needs | where it comes from |
|---|---|
| node fill, node border, text | the theme's `diagram.node_fill`, `accent`, `ink` |
| edges and arrows | `accent` |
| cluster background and border | `paper`, `diagram.cluster_edge` |
| secondary and tertiary surfaces | `secondary_light`, `paper` |
| stroke width, wrapping width | layout defaults, overridable from `diagram` |

The full table is in `references/mermaid.md`, with the layout defaults and the
measurement behind each one.

**Which roles a palette must carry is this Pack's contract, and it is held as a GATE rather
than a shared file.** Each skill names its own required roles — this skill's `NEEDED` in
`theme.py`, `slide-deck`'s `REQUIRED_ROLES` in its own — because a skill has to be able to run
from its own directory. What keeps them honest is
`tools/check-presentations-theme-contract.sh`, which fails when a role required here is not one
a deck-valid theme defines. That is the property that matters; a file both skills read was
tried and removed, because it made the source skill incomplete and was copied into all four
skills of the Pack when two read it.

`paper` is in both lists for exactly this reason. It is here because diagrams need it, and it
was diagram-only until 2026-09-17, which let a theme build a deck and then be unrenderable as a
diagram. The gate now catches that shape at the source rather than at render time.

## Workflow

```
- [ ] 1. A .mmd source exists, or you write one
- [ ] 2. diagrams render <mmd-dir>   → the theme is derived, the figures are written
- [ ] 3. Read the aspect ratio the tool prints
- [ ] 4. Place the figure, with alt= and — for reproduced material — credit=
- [ ] 5. Render the artifact and look at it
```

### 1. Write the source

A `.mmd` holds the structure, never a colour. Structure is `flowchart TB` for a
tree, `flowchart LR` for a sequence. Node text is `**heavy**` and `<i>italic</i>`
inside the label, because Mermaid renders HTML when `htmlLabels` is on. A `<br/>`
is the line break; a literal newline is not.

Node text should be labels, not sentences. A node that needs four lines of prose
is a slide panel, not a diagram node.

### 2. Render

```bash
scripts/diagrams render <mmd-dir>              # finds theme.json above the sources
scripts/diagrams render <mmd-dir> --only fig-objectives
scripts/diagrams theme <theme.json>            # derive only; no renderer needed
scripts/diagrams doctor                        # what is installed, what is pinned
```

`render` writes the derived theme beside the sources as `mermaid-theme.json`, then
renders every `*.mmd` in the directory. The output directory defaults to
`<mmd-dir>/../Assets/figures/` when that exists, which is the deck convention.

The first render downloads a headless Chromium (about 150 MB). `doctor` reports
whether it is there.

### 3. Read the aspect

The tool prints `3136x1348  aspect 2.33` for each figure. That number is what
decides where the figure can sit: a slide body is roughly 11.9in by 5in, so an
aspect near 2.4 fills it, and an aspect near 1 will not. The aspect is a property
of the node text, so it is measured, not predicted.

### 4. Place it

```python
s.picture(FIG / "fig-objectives.png", s.x, s.top, s.w, 4.72,
          alt="What the diagram shows, in one sentence",
          credit="Eigene Darstellung; Quellen im Diagramm")
```

`alt` is an obligation. `credit` is the academic one: a reproduced figure needs
its source, and an own figure needs `Eigene Darstellung`. `deck check` warns when
both are missing.

## Boundaries

**Will:**
- Derive a Mermaid theme from a palette, and render `.mmd` sources to PNG.
- Report the aspect ratio of every figure it writes.
- Report what is missing when the renderer cannot run.

**Will not:**
- Draw a diagram the source does not describe. The `.mmd` is the author's.
- Choose the palette. It reads what the artifact already declares.
- Plot data. A chart of numbers is a plotting library's job, not Mermaid's.
- Render anything but Mermaid. One renderer, done well, beats a framework.

## Done when

- `scripts/diagrams render <mmd-dir>` exits 0 and names every figure it wrote.
- The derived `mermaid-theme.json` differs from the artifact's palette only where
  the `diagram` block says so.
- Every figure has a measured aspect, and the slide that embeds it was built and
  rendered afterwards.
- The figure carries `alt`, and `credit` when it reproduces someone else's work.
