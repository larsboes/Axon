# Academic narrative

The objects an academic talk is built from, and the moves a defence or a conference talk is
judged on. `narrative.md` covers the generic craft: assertion titles, claim/evidence/bound,
the time budget, the notes. Read that first. Read this file next. Then read the genre
profile for the talk in front of you.

`narrative.md` teaches how to deliver an argument. This file covers what the argument is
made of when the audience is examiners.

## Contents

- [The research question covers a class](#the-research-question-covers-a-class)
- [The contribution: type, then claim, then bound](#the-contribution-type-then-claim-then-bound)
- [The bounding register](#the-bounding-register)
- [Positioning: the construct column](#positioning-the-construct-column)
- [Limitations: four validities](#limitations-four-validities)
- [What the examiner is testing](#what-the-examiner-is-testing)

## The research question covers a class

A generic research question asks about a **class** of problems, and it names the case as
the *setting* where the study examines that class. The failure names the case as the
*subject*.

| | |
|---|---|
| case as subject (weak) | "What role does context engineering play in the AI-assisted migration of a highly customized enterprise data-engineering pipeline?" |
| class as subject, case as setting (strong) | "What role does context engineering, through repository structure, workflow instructions and feedback available during the run, play in supporting behavior preservation, process traceability and maintainability in cross-runtime migrations whose behavioral requirements reside outside the source code?", examined in one enterprise pipeline. |

**The delete test.** Remove the case from the question. When the question still asks
something a field would want answered, the question is generic. When it becomes empty, the
case was the subject.

This deduction is common and expensive. A single-case study whose question only concerns
the case appears to generalise nothing, and the examiner marks the abstraction as "only
partly achieved". The fix usually sits in the thesis already, where the introduction states
the general condition. It simply never made it into the question.

The question occupies **one slide**, using `headline`, with the setting named on the slide
or in the next breath. One question, never two.

## The contribution: type, then claim, then bound

**Name the type before you claim it.** Gregor & Hevner give the options: an instantiation
(a working artifact), a construct, method or model (design knowledge more abstract than the
artifact), or a theory. Most bachelor and master design work lands at an instantiation plus
design knowledge. Say which. Contribution-type ambiguity is a standard attack.

**Then claim unhedged, in one sentence, and bound in the next breath.** This is
`narrative.md`'s claim/evidence/bound at deck scale, and it is the highest-value sentence
in a defence.

> "Ich zeige an einem realen Enterprise-Fall, dass Context Engineering als
> Designgegenstand evaluierbar ist: Repository-Kontext, Workflow-Instruktionen und
> In-Run-Feedback haben unterscheidbare Rollen, und Verhaltenserhalt ist kein Beleg für
> Wartbarkeit. Die Grenze: gebündelte Inkremente, ein Fall, keine Kausalaussagen über
> Einzelkomponenten."

Claim first. Bound second. Never the bound alone.

For a design-science talk, know the answer to *why is this research and not consulting*.
Routine design applies known solutions to an organisational problem. Design research
contributes to a knowledge base. Three markers carry the answer: objectives and measures
fixed **before** the demonstration, implementation-independent evidence that pre-existed
the artifact, and reported failures rather than the one configuration that worked.

## The bounding register

Every sentence in the bounding register is individually correct and permanently hedged, and
the reader finishes unable to say what the field gained.

The worked example is the colloquium deck this skill was extracted from. Its contribution
section ran four sentences, two of them limitations. Its discussion ran on *"remain
unresolved"*, *"prevent attribution"* and *"does not establish"*. Its conclusion ended on
*"remain unresolved"*. The first assessment praised the methodology as "extensive,
well-derived and argued" and deducted ten of fifteen lost points from the contribution and
the positioning. One discipline produced both the praise and the deduction.

The correction is not to hedge less everywhere. The correction is to claim first and bound
second, every time. A defence is the one place where the claim leads.

When the deck cannot state the contribution in one unhedged sentence, the work has no
stated contribution yet. No layout fixes that.

## Positioning: the construct column

A related-work matrix that columns *task, context supplied, validation, outcomes examined*
is an **engineering table**. It reads the literature for design input. It does not position
the work, and the standard criticism follows: "the positioning in the literature is
predominantly practical in nature."

A theory-facing review names the **construct each study operationalises**, and states where
this work sits against it. Keep the engineering columns, because they carry the design
contribution. Add the construct column. The concession is cheap, the criticism is standard,
and the fix is one column.

**Test:** can a reader place this work against the literature from the matrix alone?

## Limitations: four validities

`narrative.md` says "bound". For an academic talk, the bound has a taxonomy. Wohlin et al.
give four: **conclusion, internal, construct, external**. Each one needs at least one named
threat and, where possible, its mitigation.

| validity | the threat sounds like | a design case's usual instance |
|---|---|---|
| conclusion | too few runs to infer anything | three runs per configuration expose variation, they carry no inference |
| internal | two things changed at once | a bundled increment, or a limit changed alongside the thing under test |
| construct | the instrument measures something narrower than the claim | one Likert item per file, or a test suite covering eleven pairs |
| external | the setting does not transfer | one case, one language pair, one model, one session |

A slide that says "small sample" is not a validity analysis. **Name the confound before you
are asked.** A limitation the speaker volunteers reads as rigour. The same limitation
extracted by an examiner reads as a defect.

## What the examiner is testing

A defence does not summarise the thesis. It is a live test of five things, and they are the
five an examiner's written critique usually turns on: whether the literature is positioned
theoretically, whether the generalisation is honestly scoped, whether the question abstracts
beyond the case, whether the contribution is claimed, and whether the thread runs straight.
`references/defence-readiness.md` turns those into a reviewer pass over the built deck.

The underlying constructs (contribution types, the validity taxonomy, evaluation-strategy
fit, the rigor and relevance cycles) belong to the **`academic-writing`** skill's
`references/genre-dsr-qualitative.md` and `references/genre-empirical-cs.md`. This file is
the talk rendering of them. Read those for the source.
