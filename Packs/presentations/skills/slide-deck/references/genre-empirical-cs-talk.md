# Genre profile: empirical-CS / conference paper talk

Use this profile for a conference, workshop or seminar talk about an empirical
computer-science paper. That paper has a method, baselines and an evaluation. Do not use
it for a DSR or qualitative thesis defence (`genre-dsr-defence.md`).

The paper-side reference, with the constructs, belongs to `academic-writing`
(`references/genre-empirical-cs.md`). Read `academic-narrative.md` first for the
claim/bound and positioning rules. This file covers the talk shape.

## The room

- Reviewers may sit in the audience. The live question is not "is this good?" but "how
  does this differ from the paper I reviewed?" Cite the nearest neighbour on a slide,
  before the question arrives.
- The session chair enforces the time. A talk that runs long is a talk that skipped its
  conclusion.
- The audience is not the reader. Nobody reads the method section. The figure and the
  headline number carry it.

## The spine

```
task or application     the problem and the setting
prior limitations       why existing solutions leave it open
insight                 the one idea that makes the approach work
contribution            the claim, then the bound
method                  the pipeline and the modules that matter
experiments             comparison, then ablations, then the stress case
related work            the nearest neighbour first
conclusion              the answer, then the one honest limitation
```

The assertion-title rule does most of the work here. An experiment slide's title states
the finding, not the table's name.

## What must land

1. **The technical challenge, framed before the naive-baseline reading.** State the
   challenge first. Never present the method as an incremental fix to an obvious baseline.
2. **The insight.** Say why the approach fundamentally works. This is the slide the
   audience remembers, and the one a reviewer probes.
3. **The headline comparison number**: with the metric direction visible, and the
   **ablation** that shows a module earns its place.
4. **The nearest-neighbour distinction.** One slide, explicit, before the question.
5. **The bound.** What the evaluation does not cover.

## Q&A

Answer shape: claim, evidence, boundary.

| the question | the shape of the answer |
|---|---|
| How is this different from *X*? | the nearest neighbour, the axis of difference, then the evidence |
| What about the ablation? | which module, which table, what it buys |
| Does it generalise to harder data? | the stress test, or an honest "that is the upper-bound question we did not test" |
| Why these baselines? | the strongest available, named, and what a weaker one would have flattered |
| Is the gain within noise? | the variance, and why the comparison still means something |

## Backup slides

Eight to twelve. The highest-value set for this genre: the full comparison table, the
per-module ablations the talk compressed, the failure cases, the hyperparameter
sensitivity, the dataset details, and the extra related-work rows.

## Traps

- Do not read the table aloud. Show the headline, then interpret it.
- Do not apply the DSR framing (relevance and rigor cycles, contribution types) to a
  paper with baselines. It asks and answers the wrong questions.
- Do not end on a summary slide. End on the answer, then the limitation.
- Do not order the related-work slide chronologically. Organise it by topic, nearest
  neighbours first.
