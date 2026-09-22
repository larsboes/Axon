# Adversarial red-team pass — the genuinely novel piece

Every third-party academic-writing tool checked when this skill was built (2026-07) is structurally
biased toward *finding issues to fix*, but none is briefed to actually argue the work should be
**rejected** — to actively hunt for the fatal flaw rather than a list of improvements. This file is
that missing piece. Use it as the final step of `CriticReview`, after the five lens-briefs in
`critic-briefs.md` have run — those find specific, scoped problems; this asks the harder question of
whether the document survives as a whole.

## Why this needs to be adversarial, not just thorough
A reviewer briefed to "find issues" tends toward a list of fixable nits, because nits are easy to spot
and safe to report. A reviewer briefed to **refute the core claim** has to engage with whether the
argument holds at all — a categorically different, harder question, and the one an actual examiner or
peer reviewer is implicitly asking. Multi-skeptic voting (independent agents, each defaulting to
"still flawed" under uncertainty) exists because a single adversarial pass can itself be too generous —
same failure mode as a single non-adversarial reviewer, one level up.

## The pattern
Spawn 3 independent skeptics in parallel (not sequential — sequential lets skeptic 2 anchor on skeptic
1's framing). Each is briefed with the SAME target document but a DIFFERENT attack axis, and
explicitly instructed to try to kill the work, not to improve it:

1. **Methodology skeptic**: "You are a hostile peer reviewer whose job is to argue this document's
   central method/design is unsound. Attack the methodology directly — is the evaluation strategy
   actually capable of supporting the claimed contribution? Is there a confound, an unaddressed
   alternative explanation, or a step where the design doesn't follow from the stated objectives? Find
   the strongest possible objection, not a list of minor ones. If you cannot find a genuine
   methodological flaw after genuinely trying, say so explicitly — do not manufacture one."

2. **Claim-evidence skeptic**: "You are a hostile peer reviewer whose job is to argue this document's
   claims outrun its evidence. For every definitive claim (a sentence asserting something is true, not
   hedged as a hypothesis), find the exact evidence it depends on and ask: does the evidence actually
   support the FULL claim as written, or a narrower one? Flag every claim that would need to be walked
   back if a skeptical reader pushed on it. If the claims are already appropriately calibrated to the
   evidence, say so explicitly."

3. **Structural skeptic**: "You are a hostile peer reviewer whose job is to argue this document's
   argument doesn't actually cohere end to end. If you removed the strongest single section, would the
   remaining argument still support the stated conclusion? Is there a version of the counter-argument
   the document never engages with? Would an examiner reasonably conclude the contribution is smaller
   than claimed once the framing is stripped away? If the argument genuinely holds together, say so
   explicitly."

## Verdict aggregation
Each skeptic returns: `{flaws: [...], verdict: "fatal" | "serious" | "survives"}`. Aggregate:
- Any skeptic returns `fatal` → surface it immediately, don't average it away. A fatal flaw found by
  one skeptic and missed by two others is still fatal.
- 2+ skeptics return `serious` on the same underlying issue (not just the same axis) → treat as
  load-bearing, not a nit.
- All three `survives` → the document is defensible at this pass; note explicitly which axes were
  checked so a later pass doesn't re-run the same three attacks.

## Calibration note
This is deliberately harder than the five lens-briefs in `critic-briefs.md`. Running it on a rough
first draft will produce a long list — that's expected and not a signal something went wrong with the
review; run the lens-briefs first, fix what they find, THEN run this pass on the result, not the other
way around. Running the adversarial pass on an unpolished draft wastes its signal on issues the
lens-briefs would have caught more cheaply.
