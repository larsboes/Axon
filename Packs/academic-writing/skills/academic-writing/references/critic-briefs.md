# Critic briefs — job-specific agent prompts for CriticReview

Five distinct diagnostic lenses, adapted from a third-party multi-agent review plugin (static personas
stripped — these are prompt content for a `general-purpose` agent dispatch, per the "job-specific brief
beats a static persona" pattern). Each brief below is read-only: the agent reads and reports findings,
it does not edit. Dispatch each as an independent agent (parallel, not sequential) so findings aren't
anchored on each other; a single agent asked to do all five in one pass tends to under-report the
narrower ones. Every finding must cite file:line or a direct quote — a finding with neither is not
actionable and should be rejected before it's reported to the author.

## 1. Technical reviewer
Brief: "Review this document's technical claims. For each: (a) does the stated result actually follow
from the stated method/data — not just plausible-sounding, actually entailed? (b) is notation used
consistently throughout, and does it match its first definition? (c) does every citation supporting a
definitive claim actually say what the claim needs (open the source, don't infer from the citation
key)? (d) is the methodology itself sound given the claimed contribution — would a specialist in this
subfield object to a specific step? Report each finding as: claim (quote) → problem → what evidence
would resolve it. No 'looks fine' entries — if you found nothing wrong in a section, say so explicitly
and move on, don't pad."

## 2. Logic reviewer
Brief: "Review this document's argument structure, independent of whether individual sentences are
well-written. For each section: does it follow from what came before, or is there an unstated logical
leap? Are paragraph-to-paragraph transitions doing real argumentative work, or just juxtaposing
adjacent facts? Is anything redundant — the same point made twice without adding evidence the second
time? Does the narrative arc (problem → approach → result → implication) hold across the whole
document, or does it wander? Report each finding as: location → the specific logical gap or redundancy
→ what's missing to close it."

## 3. Consistency checker
Brief: "Check this document for internal consistency, not correctness. Terminology: does every term
mean the same thing everywhere it's used, or does meaning drift (e.g. a word used loosely early,
precisely later)? Cross-references: does every figure/table/section reference resolve to something
that actually exists and says what the reference claims? Figure-text-caption match: does the caption
describe what the figure actually shows, and does the body text's description of the figure match
both? Definition order: is every technical term defined before its first substantive use? Report each
finding with file:line for both the inconsistent instances."

## 4. Bibliography auditor
Brief: "Audit this document's citations. For each citation key used in the text: (a) does it resolve
in the bibliography file? (b) does the source it points to actually support the specific claim it's
attached to — open the source's note/abstract, don't assume from the title? For each bibliography
entry: (c) is it cited anywhere in the document, or is it dead weight? (d) for entries with a DOI/
identifier, does title/author/year match what the identifier resolves to (use the citation scripts in
`scripts/` if available)? Report unresolved keys, unsupported citations, and unused bib entries as
three separate lists."

## 5. Layout auditor (compiled-output level)
Brief: "Given the compiled output (rendered PDF, not the source), check float placement and page-level
layout: do figures/tables land near their first textual reference, or drift several pages away? Are
any figures/tables cut off, overlapping other content, or sized inconsistently with their neighbors?
Are captions positioned consistently (above tables, below figures, or whatever the document's own
established convention is — check for drift from that convention, not against a universal rule)? This
lens only applies if a compiled artifact exists — skip it and say so explicitly if only source is
available."

## Dispatch pattern
```
parallel([
  () => agent(technical-reviewer brief, {label: "critic:technical"}),
  () => agent(logic-reviewer brief, {label: "critic:logic"}),
  () => agent(consistency-checker brief, {label: "critic:consistency"}),
  () => agent(bibliography-auditor brief, {label: "critic:bibliography"}),
  () => agent(layout-auditor brief, {label: "critic:layout"}),  // skip if no compiled PDF
])
```
Each agent's brief should be prefixed with the target document's genre profile (see
`genre-empirical-cs.md` or `genre-dsr-qualitative.md`) so findings are calibrated to the right document
type — a technical reviewer checking a DSR thesis for "missing ablations" is asking the wrong question.
