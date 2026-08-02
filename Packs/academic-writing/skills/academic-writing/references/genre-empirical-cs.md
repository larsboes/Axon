# Genre profile: empirical-CS / conference paper (IMRaD, CVPR/NeurIPS-style)

Use this profile when the target document is an empirical computer-science paper with a method,
baselines, and experiments — **not** for a DSR/qualitative thesis (use `genre-dsr-qualitative.md`
instead; the two are not interchangeable, see Gotchas in `SKILL.md`).

## Writing order
Sketch the pipeline figure → outline the Introduction's story + list the comparison/ablation
experiments it implies → outline Method, write it while running experiments → revise Intro/Method →
outline and write Experiments → polish pipeline/teaser figure → outline and write Related Work →
self-review and revise Intro/Method/Experiments → write Abstract → pick a title → repeat review/revise.

## Abstract — answer 4 questions, then pick a template
1. What technical problem, and why is there no well-established solution?
2. What's the contribution?
3. Why does it fundamentally work?
4. What's the technical advantage / new insight?

Templates: (a) **challenge → contribution**; (b) **challenge → insight → contribution**; (c)
**multi-contribution** (when the work has 2+ independently useful pieces).

## Introduction — "reverse then forward"
Reverse-answer the same 4 questions above internally, then forward-write in this order: task/
application → prior-method limitations → our contribution → technical advantage. Four opening
strategies (pick one, don't blend): task-first, application-first, general-to-specific, open-with-
challenge. Anti-pattern (house-style A2): never present your method as an incremental fix to a naive
baseline — frame the technical challenge first.

## Method — the "three-element" module template
Every subsection = **Motivation** (why this module, what gap it closes) + **Design** (structure, the
step-by-step forward pass) + **Technical Advantage** (why this design choice over the alternatives).
Figures should look visually distinct from prior work — they signal novelty, but the prose is what
makes the method understandable, not the figure alone.

## Experiments — answer 3 questions
1. Is the method stronger than baselines? (comparison table)
2. Are the modules individually useful? (ablations — one big table for core contributions, small
   tables per individual design choice)
3. What's the method's upper bound on harder data? (stress tests / scaling)
Include demos where applicable. Table formatting: booktabs style, no vertical rules, captions above
tables, metric-direction arrows (↑/↓) next to column headers.

## Related Work
Cite the closest papers first — reviewers reject on omissions of the nearest neighbors more than on
missing tangential citations — then organize the rest by topic, not chronologically.

## Conclusion / Limitations
State limitations as **scope/setting boundaries**, not confessed technical flaws — rule of thumb: if a
metric doesn't regress versus the state of the art, it's not a flaw, it's future work (house-style A4).
Per house-style F1, a paper delays limitations to the end; a thesis does not — check which document
type you're drafting for before applying this.

## Self-review checklist (adversarial pass before submission)
Interrogate the draft against 5 failure categories, in the voice of a hostile reviewer:
1. Insufficient contribution — is the nugget (house-style A7) actually novel and stated?
2. Unclear writing or missing implementation details.
3. Weak results — do the numbers actually support the claims?
4. Insufficient experiments — missing ablations, baselines, or metrics a reviewer would expect.
5. Flawed method design — is there a simpler baseline that would achieve the same result?

This checklist feeds `CriticReview`'s empirical-CS lens (see `SKILL.md` and `adversarial-redteam.md`).
