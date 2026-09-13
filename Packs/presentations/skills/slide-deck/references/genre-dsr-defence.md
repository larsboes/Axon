# Genre profile: DSR / qualitative thesis defence

Use this profile for the defence (viva, colloquium, Disputation) of a Design Science
Research thesis or a qualitative single-case study. Do not use it for a conference paper
talk (`genre-empirical-cs-talk.md`) or a project readout. The talk genres are not
interchangeable. A defence is judged on abstraction and contribution. A paper talk is
judged on beating a baseline.

The argument content (contribution types, the validity taxonomy, the rigor and relevance
cycles) belongs to the **`academic-writing`** skill's `references/genre-dsr-qualitative.md`.
This file covers the talk shape. Read `academic-narrative.md` before this one.

## The room

The room differs from every other talk genre in one way that changes the deck. **The
examiners wrote the critique, and the critique is the question list.** A written assessment
of the work already exists. Its points are where the questions come from, and the candidate
must hold a *stance* on each one. A stance is not an apology, and it is not a denial.

- The frame is usually 30 minutes, of which the talk takes 10 to 15. The rest is questions.
- Questions may lie "links und rechts vom Weg" (to either side of the path), including
  cross-disciplinary ground, under a rule such as *fachübergreifend und problembezogen auf
  wissenschaftlicher Grundlage*.
- Admission sometimes depends on more than one examiner's grade. The colloquium carries its
  own assessment, so it is a place to earn points rather than to defend the written grade.
- The examiners have read the thesis. **Do not summarise it.** The talk exists to display
  the argument that the written work may not have made clearly enough.

## The spine

```
motivation          the real problem, and why it matters operationally
problem             why the obvious answer does not settle it
gap                 positioning against the literature, with the construct column
question            the research question at class level, with the case as the setting
method              the approach (DSRM or equivalent) and how the study evaluates it
artifact            what was built, and what the artifact is not
results             per objective, numbers then the limit
contribution        the claim in one sentence, then the bound
limitations         the four validities, with the confounds named
outlook             the work that closes the open objective
```

Five sections is the usual shape. Divider slides make the spine audible. The order runs
motivation to question, with **no detour back into case detail after the artifact is
announced**. That re-entry is what a "red thread" criticism describes.

## What must land

One slide each. These are the load-bearing ones.

1. **The gap**: taken from the matrix, with the construct column. The nine rows are not the
   point. The empty column is.
2. **The research question at class level**: with the case as the setting. One question.
3. **The artifact boundary.** Write "the artifact is the setup, not the generated code". A
   DSR talk that lets the examiner hear the generated output as the contribution has handed
   away its research claim.
4. **The result that is also a finding.** Use the one where the numbers and the human
   judgement disagree, or the objective that stayed open. It is the best material in a
   defence.
5. **The contribution, claimed before it is bounded.**
6. **The open objective**: presented as the next task.

## The Kritikpunkte → Antwort map

This is the highest-value backup slide in a defence. It carries one row per point in the
written assessment, the stance taken, and the location of the evidence.

```python
s.table([("Kritikpunkt", 4.05), ("Antwort", 4.35), ("Beleg", 3.49)], [
    ["positioning predominantly practical",
     "Partly concede. The matrix is by task and validation. Hold: the constructs are derived, not set.",
     "Ch. 2; B1"],
    ...
])
```

**It is for the speaker, not the examiners.** It is a rehearsal instrument, and opening it
unasked reads as defensive. Its row count is the number of points you must have a stance
on.

## Q&A: the answer shape

**claim, evidence, boundary.** The written critique has already told you that hedging cost
points. The room is where you stop.

The standard set, and where each answer comes from:

| the question | the shape of the answer |
|---|---|
| What is the scientific contribution? | the one-sentence claim, then the type (instantiation plus design knowledge), then the bound |
| Why is this research, not consulting? | objectives and measures fixed before the demonstration, implementation-independent evidence that pre-existed the artifact, and failures reported |
| How generalisable is it? | *analytical, not statistical*. Then the findings that transfer, and the sentence "three runs per configuration carry no inference" |
| Why is the artifact not the generated code? | the setup is the artifact, the output is an object of evaluation, and DSR contributions are constructs, models, methods or instantiations |
| Why a cumulative design rather than a controlled one? | the question was what to build, and nothing could be ablated before a working configuration existed. Concede that no component-level attribution follows |
| Why is an objective still open? | the design constraint (the evidence had to be fixed before the outcome was known) and the time constraint. The alternative would have been worse science |
| Is the instrument weak? | concede its scope, then the pattern that survives it (the same rating split across independent reviewers), and that it is reported as indicative |
| You built on a diagnosis that turned out wrong? | concede, and say the iteration evaluates the decision made with the information available at the time |
| What would you do differently? | four items, ranked. Ten reads as panic or as an absence of judgement |

## Backup slides

Eight to twelve, each answering one expected question, with the question named in its
notes. The highest-value set for this genre:

- the related-work matrix in full, with the construct column
- the numbers to have cold
- the two or three weakest points, each with its answer
- the Kritikpunkte → Antwort map
- the artifact's components, for "what exactly did you build?"
- the evaluation design, for "how do you know it worked?"

## Traps

- **Do not claim efficiency** without a human-effort baseline. Duration and cost describe
  resource demand, not savings.
- **Do not say the work "succeeded"** when the evidence covers a test suite rather than
  production behaviour. "Preserved test-covered behaviour" is the honest phrase.
- **Do not defend the title.** When the title overclaims relative to the evidence, concede
  it in one sentence. When the topic was locked at registration, say so once and move on.
  Thirty seconds on the rule reads as evasive.
- **Do not argue the grade or the weighting.** It is fixed.
- **Do not over-hedge.** The critique already told you what hedging costs.
