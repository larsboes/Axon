# Genre profile: DSR / qualitative thesis (Design Science Research)

Use this profile for a Design Science Research thesis or a qualitative single-case study — **not**
for an empirical-CS paper with baselines/ablations (use `genre-empirical-cs.md` instead). This profile
did not exist in any third-party source analyzed when this skill was built (2026-07) — it is authored
from general DSR-methodology knowledge, not distilled from a specific paper. **Verify every citation
key below against the actual source before using it** (per this skill's no-uncited-claims posture,
`SKILL.md` Gotchas) — the names are pointers to go read, not pre-verified citations.

## Why this profile is separate, not a variant of the empirical-CS one
A DSR thesis argues a design contribution against a knowledge base, not a method against baselines.
There is no "beat the state of the art" table; the equivalent move is positioning the artifact's
contribution type and demonstrating the relevance/rigor cycles closed. Porting the empirical-CS
Method/Experiments templates onto a DSR Discussion produces a confidently wrong-shaped chapter — this
was the central risk this skill's design was built to avoid.

## Core DSR moves (Hevner et al. 2004; Peffers et al. 2007 DSRM; Gregor & Hevner 2013)
- **Relevance cycle**: the problem comes from and returns value to a real application context — state
  this explicitly, don't assume it's implied by the case description.
- **Rigor cycle**: the design draws on and contributes back to a knowledge base (theories, prior
  artifacts, methods) — name what was drawn from and what is newly contributed.
- **Design cycle**: build → evaluate → refine, iterated until the artifact meets its objectives or the
  study's scope closes.
- **Contribution types (Gregor & Hevner 2013)**: a DSR contribution is typically one of — a novel
  **instantiation** (working artifact), a novel **construct/method/model** (design knowledge more
  abstract than the artifact), or a novel **theory**. Most bachelor/master DSR theses land at
  instantiation-plus-design-knowledge; state which type explicitly rather than leaving it implicit —
  examiners look for this.

## Discussion-chapter structure (generic DSR shape — check against your own confirmed outline first)
A DSR Discussion chapter typically closes the loop the Introduction opened, in this order:
1. **Results vs. objectives** — read the results against what the study set out to measure, not a
   restatement of results. This is the "did we meet what we set out to do" anchor.
2. **Design knowledge contributed** — what generalizes beyond this one artifact/case (construct/
   method/model-level knowledge, per the contribution-type framing above), separated from what is
   specific to this instantiation.
3. **Limitations, as ceilings not confessions** — each limitation should read as "here is the honest
   boundary of what this design/study establishes, here is what a follow-up study could do about it,"
   scoped to a single-case/qualitative design (no generalization claims beyond what repeated
   application would support).
4. **Practical relevance** — closes the relevance cycle: what this means for practice, as a
   consequence of 1–3, not a bolted-on closing paragraph.
(If your project has its own confirmed chapter outline — e.g. from a supervisor meeting — that outline
is the authority; this is the generic shape to fall back on when no such outline exists yet.)

## Common examiner attack surfaces for DSR/qualitative work (use in `CriticReview`)
- **n=1 / single-case generalization creep** — does any sentence imply the finding generalizes beyond
  the one case/artifact studied? Flag it; DSR single-case work supports "design learning," not
  population claims.
- **Contribution-type ambiguity** — can the examiner tell, from one paragraph, whether the claimed
  contribution is an instantiation, a method, or a theory? If not, that's a structural gap, not a
  wording one.
- **Rigor-cycle name-check** — does the thesis actually name what body of knowledge the design draws
  on and contributes back to, or is the "knowledge base" grounding only implicit?
- **Evaluation-strategy fit** — does the chosen evaluation approach (e.g. a FEDS-style strategy
  selection — Venable, Pries-Heje & Baskerville) match the claim being evaluated, or is a naturalistic/
  qualitative evaluation being used to support a claim that would need a summative/quantitative one?
- **Threats-to-validity completeness** — construct, internal, conclusion, and external validity
  (Cook & Campbell taxonomy, as commonly applied in software-engineering empirical work) each need at
  least one named threat + mitigation; a Threats section with an empty subsection is a visible gap to
  an examiner, not a neutral omission.

## What NOT to import from the empirical-CS profile
Do not apply: the "beat baselines" experiments framing, the three-element Method-module template, or
the "naive baseline" anti-pattern framing (A2 in house-style.md is CS-paper-specific — a DSR thesis's
Method/Design chapter is not defending against a naive-baseline reading, it's justifying design
choices against requirements). House-style principles A1, A7, B1–B8, F1 still apply generically; A2–A4
and the empirical-CS Method/Experiments templates do not.
