# Citing on slides

A talk cites differently from a paper. A deck that cites like a paper is unreadable. A
deck that does not cite at all is not academic. The rule is small and absolute:
**attribute every borrowed claim on the slide, and carry one references slide.**

The paper-side discipline belongs to `academic-writing` (`references/citation-workflows.md`
and `references/house-style.md`). It covers what a source must support, when a claim needs
a citation, and why you do not cite what you have not read. This file is the slide
rendering of it.

## The inline form

Short form, on the slide, next to the claim it carries: **author and year**.

- Write `Ziftci et al. (2025)`, `(Mei et al., 2025)`, or `ISO/IEC 25010 §3.1.2` for a
  standard.
- Use brackets and numbers only when the venue's style demands it. Keep one style
  throughout.
- A source that carries a whole row of a matrix appears in the matrix, not in a footnote.
- A claim that is yours carries **no** citation. Empty attribution is not modesty. It is
  noise.

## The references slide

One slide, at the end of the talk, before the appendix. It lists **only** what the talk
cited.

- Use `small` or `tiny`, and two columns when the list is long. The room photographs it.
  Nobody reads it aloud.
- Do not print a bibliography. The thesis has one. A thirty-item list on a slide says the
  speaker did not choose.
- Put the backup slides after it, so a question that reaches for a source has the slide.

```python
s = deck.open("Literatur", "Referenzen")
s.panel_grid([
    [source_a, source_b, source_c],
    [source_d, source_e, source_f],
], columns=2, height=4.6, size=9.5, voice=None, bullet=False, para_gap=5)
s.notes("Nicht vorlesen. Zeigen und weiter.")
```

## The nearest neighbour

In a paper talk and a defence alike, the question is not "did you cite enough". The
question is "did you place this against the work it is closest to". Name the nearest
neighbour on a slide, before it is asked, and state the axis of difference. A matrix whose
construct column is empty (`academic-narrative.md` section Positioning) is a citation
list in the shape of positioning.

## Reproduced material

- A reproduced figure gets a **visible source line** on the slide, written as `Quelle:` or
  `Source:`, and a `credit=` on the call. Use both. The reader sees one, and the file
  carries the other.
- When a licence restricts reuse, the caption says so. The talk says it again if asked.
  This is the one place where a slide is also a legal artefact.
- Treat data you did not collect like a figure you did not draw.

## Rules of thumb

- **Cite the claim, not the topic.** A slide titled "Related work" with five authors
  beneath it attributes nothing.
- **One source per claim is enough.** Three sources on one bullet reads as insecurity.
- **A standard is a source.** Cite `ISO/IEC 25010 §3.7`. Do not write "the literature".
- **Treat vendor material as a design document, not as evidence.** Name it that way. A
  sharp thesis chapter writes: "Anthropic's engineering publications describe the
  practice. Provider-reported evaluation is not independent evidence."
