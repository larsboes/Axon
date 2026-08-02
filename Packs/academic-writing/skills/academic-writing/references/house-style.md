# House style (sections A/B/D/F)

Distilled from a third-party academic-writing-agents review-principles doc. Opinionated and specific
— apply these before generic "be clear" advice. Cite a letter+number (e.g. "B7") when flagging a
violation so the writer can find the rule.

## A — Contribution & framing
- **A1** State the paper/thesis's contribution in one sentence early; if you can't, the contribution
  isn't clear yet — fix that before polishing prose.
- **A2** Don't frame your method as an incremental fix to a "naive baseline" — it reads as low-value
  even when it isn't. Present the technical challenge first, then the solution.
- **A3** Every claim of novelty needs a "why didn't prior work already do this" answer, stated or
  implied.
- **A4** Match the ambition of the claim to the evidence. Don't claim "we solve X" when the evidence
  supports "we improve X under condition Y."
- **A7 (the nugget)** The work must have one stateable insight a reader could repeat back after
  closing the document. If reviewers can't name it, neither can readers — find it, then organize
  around it.

## B — Prose mechanics
- **B1** One message per paragraph, stated in full; state the point first (pyramid principle:
  conclusion, then supporting logic), not last.
- **B2** Ban AI-writing tells: "delve," "leverage," "landscape," "robust," "seamless," "comprehensive,"
  "crucial," "it is worth noting that," negation-contrast constructions ("not X but Y") used
  repeatedly, serial "Moreover/Furthermore" chains. These read as filler regardless of correctness.
- **B6** Calibrate confidence to claim type: assertive for verified facts, hedged for causal or
  interpretive claims. Don't mix registers within one paragraph — a sentence that hedges a fact or
  asserts a hypothesis undermines the reader's calibration.
- **B7** Ruthless conciseness — cut words that don't carry information. Substitution table:
  "utilize" → "use"; "in order to" → "to"; "a large number of" → "many"; "due to the fact that" →
  "because"; "it is important to note that" → delete the frame entirely.
- **B8** Watch for AI-rhythm tells independent of word choice: over-long multi-clause sentences,
  em-dash pile-ups, comma-before-and splices, serial parallel-structure paragraphs. Flag, don't
  silently rewrite — voice belongs to the author.

## D — Technical/structural
- **D5** Calibrate statistical language to sample size — no "rate," "% more reliable," "significant"
  language at small n; qualitative/inductive framing instead.
- **D6** LaTeX/Quarto subfigure alignment: keep captions outside float boundaries where the template
  allows; verify float placement against the rendered PDF, not the source.

## F — Placement
- **F1** Strategic limitation placement differs by document type: a **thesis** should surface
  limitations early (they're part of the argument — the examiner is grading the reasoning, not just
  the result); a **conference/journal paper** should delay limitations to avoid front-loading doubt
  before the contribution lands. Pick per document type, not by habit.

## How to use this file
When running `CriticReview` (see `SKILL.md`), check drafts against A/B/D/F above and cite the letter+
number in every finding. When running `Draft`, apply B1/B6/B7 to any skeleton language generated —
skeletons are meant to be extended by the author, but the skeleton language itself should already be
clean.
