---
name: diverge
description: Produces candidate options when there are none yet, because every other decision capability in this pack assumes somebody already thought of them. Assigns each candidate a different generator so the list is not one idea in five outfits, winnows the pile with the `narrow` tool, and hands the survivors to `council` or `crystallize`. Use when the request is "what could we do about X", "give me options", "brainstorm this", "I don't know where to start", or when a council cannot convene because there is no second option to weigh. Do not use for choosing between options that already exist (use council), for attacking one proposal (use red-team), or for turning notes into a document (use crystallize).
license: MIT
---

# diverge

Every other capability in this pack needs options to already exist. `council` needs two or more,
`red-team` needs one proposal, `crystallize` needs notes, `root-cause` needs a failure. Nothing
produces candidates. This is that missing half, and it is deliberately the only skill here that
generates rather than judges.

Generating is the easy half and the model does it freely. The work is **breadth that is real** and
then **winnowing**, which is why this skill spends most of its words on the second.

## Procedure

```
- [ ] 1. State what the candidates are candidates FOR
- [ ] 2. Name the generators, one per candidate
- [ ] 3. Produce the pile, one candidate per generator
- [ ] 4. Winnow with `narrow`
- [ ] 5. Hand the survivors on
```

**1 — State the target.** A pile without a question is a list. Write the decision or the goal the
candidates are meant to serve, and if the caller gave a topic rather than a question, confirm the
restatement with `ask` first — `council`'s `references/compose.md` says how, and the same reasoning
applies: a pile generated against the wrong target is entirely wasted work.

**2 — Name the generators before producing anything.** Pick five to eight and write them down. The
list is what makes the breadth checkable afterwards:

| Generator | The question it asks |
|---|---|
| Analogy | What solved a structurally similar problem somewhere else? |
| Inversion | What if we did the opposite, or optimised for the thing we are avoiding? |
| Constraint removal | Which constraint, if it vanished, would make this easy? |
| Extreme scale | What would this look like at 100x, or at 1/100th? |
| Recombination | Which two things we already have, combined, answer this? |
| Elimination | What if we removed the part everyone assumes is required? |
| Precedent | What did this repository already try and abandon, and why? |

**3 — Produce one candidate per generator.** One. Not five from analogy and one from everything
else, which is what happens when the first idea is good. If a generator yields nothing, say so and
name it — an empty generator is a finding, and it is often the most interesting line in the pile.

**4 — Winnow with `narrow`.** Pass every candidate: a one-line statement, its generator, and where
it is cheap to say so, its cost and the cheapest test that would show whether it works. `narrow`
takes one keystroke per candidate and returns the kept set, the rejected set with reasons, and
anything left undecided. If the tool is unavailable — a harness without this pack's pi `questions`
extension, or a non-interactive run — list the candidates as numbered text with their generators and
ask which to keep, in the same three-outcome form: kept, rejected with a reason, undecided.

The rejected list is the artifact. It is the same shape as `suggest-skills`' rejection table, for the
same reason: it is the only thing that shows the pass actually happened rather than being described.

**5 — Hand the survivors on.** Kept candidates are options, so they go to `council` — which is what
weighs options and already knows how to argue them out. If the pile converged on one answer and there
is nothing left to weigh, say that; a council convened on a settled question is theatre.

If nothing was kept, report that. A divergence round that produced nothing worth deciding about is a
real result, and pushing an empty pile forward to look productive is the failure mode this skill
exists to prevent.

## Rules

- **One candidate per generator, and no two candidates sharing a first-order rationale.** Five ways
  to say "add a cache" is one candidate. This is the only quality bar that matters here.
- **Do not rank while generating.** Ranking during divergence kills the weak-but-novel candidates,
  which are the ones worth having. Selection happens at step 4 and not before.
- **Never re-propose a rejected candidate** in a later round unless its reason no longer holds, and
  say which reason changed. `narrow` returns the reasons so this is checkable rather than a promise.
- **Do not converge by yourself.** Judging is `council`'s job and it does it better, with advocates
  and a clerk. Producing options and then quietly picking one is the one way to make this skill
  worse than a single-pass reply.
- **The anti-goal, stated plainly:** twenty plausible interchangeable ideas. If the pile reads as one
  idea in twenty outfits, the generators were labels rather than lenses, and the fix is step 2, not
  a longer pile.

## Related

`council` weighs what this produces. `crystallize` turns a decided outcome into a document.
`red-team` attacks one proposal once this has narrowed to one.
