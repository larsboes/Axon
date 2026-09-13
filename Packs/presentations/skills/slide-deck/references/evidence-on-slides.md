# Evidence on slides

A slide that shows a number can overstate it. This file covers the rules that keep a
result honest. `academic-narrative.md` covers the argument around the result. This file
covers the evidence inside it.

A defence or a paper talk is most often lost here. The failure is rarely a wrong result.
The failure is a result shown as more than it is.

## The result slide

A quantitative slide states five things on the slide itself, not in the notes.

- **n**. The runs, cases, subjects or repositories behind the number. A count without an
  n has no scope.
- **The metric direction and its units**, placed next to the column or the value. An
  arrow is cheap. `CC ↑` tells the reader that a higher value is worse.
- **The uncertainty**. Give the interval, the observed range, or the error bars. When
  none exists, say so. Three runs carry no uncertainty estimate.
- **The effect, before any test statistic.** Significance is not size. A p-value on a
  slide is a number nobody can use.
- **What the result does not cover.** One line, in the next breath.

The worked example uses one caption for this: *"Dargestellt sind beobachtete Läufe, keine
Wahrscheinlichkeiten"* (observed runs, not probabilities). The caption costs one line, and
it forecloses the standard question.

## What not to do

- A **p-value alone**, or the word *significant* on its own.
- A **percentage with no n**.
- A **mean with no spread**, when the spread is the finding.
- **"The results show"** for a descriptive count. Write "observed", or "in these runs".
- An **efficiency claim with no baseline**. Duration and cost describe resource demand.
  They become savings only against a measured alternative.
- **Ranking runs by a mean** when the ratings are ordinal and nested. Report the pattern
  across raters.

## Figures

- **Reproduce the original. Do not redraw it.** When a figure already exists in the paper,
  the related work or the source document, put that figure on the slide with its source
  line. A redrawn version creates a second visual language on the same slide, and it will
  disagree with the document the audience read. This rule covers a standard model figure:
  use the published one, credited.
- **State the source and the bound.** `s.figure_beside(..., note="...")` places provenance
  under the figure. A reproduced figure needs a visible `Quelle:` or `Source:` line.
- **Credit the origin.** `s.picture(..., credit="Reproduced from Peffers et al. (2007),
  p. 54")`. When a licence restricts reuse, the caption says so.
- **Do not truncate a y-axis** to exaggerate a difference. Do not use a **dual axis** to
  imply a relationship. A reviewer reads both as manipulation.
- **Label the baseline, the units and the sample size** on the figure itself.
- **Do not encode a category by hue alone.** See the next section.

## Access, colour and the projector

Some readers cannot separate the palette, and a projector washes out mid-tones.

- **Do not put meaning in hue alone.** `accent`, `secondary` and `caution` are three hues,
  and a reader with a colour-vision deficiency may not separate them. Pair every colour
  with position, a label, or a shape. This is the design system's rule that emphasis never
  carries meaning alone, applied to data.
- **Test the deck at 40 % brightness.** When the tints collapse into grey rectangles, too
  much of the deck is tint.
- **Keep readable text at `small` (11 pt) or above.** Reserve `tiny` (9.5 pt) for
  footnotes and captions.
- **Give every picture an `alt=`**, and every reproduced figure a `credit=`. Both live in
  the shape description. `deck check` warns on a picture with neither.

## Evidence and the readiness pass

A results slide must state its n, its uncertainty and its bound when `deck readiness`
prints it as a bare title. When it cannot, the slide carries less than its author
believes. The readiness questions in `defence-readiness.md` ask whether the claim is
bounded. This file keeps the evidence inside it honest.
