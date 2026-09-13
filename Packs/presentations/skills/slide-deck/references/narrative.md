# Narrative

A deck is an argument with pictures, not a document with slides. This file is about
the argument: what goes in, in what order, and what each slide owes the next one.

This file is the craft **every** talk shares. Two things sit under it for an academic
talk, and neither replaces it:

- `academic-narrative.md`: the objects an academic argument is built from: the
  research question, the contribution and its type, the bounding register, the
  construct column, the four validities.
- a **genre profile**, `genre-dsr-defence.md` or `genre-empirical-cs-talk.md`, which
  fixes the spine for the talk in front of you.

For a project readout, a client report or a lecture, this file is enough. For a
defence or a conference talk, read this, then `academic-narrative.md`, then the genre
profile. The rest of this file applies either way.

## Contents

- [The spine](#the-spine)
- [Assertion titles](#assertion-titles)
- [Claim, evidence, bound](#claim-evidence-bound)
- [The time budget](#the-time-budget)
- [Speaker notes](#speaker-notes)
- [Backup slides](#backup-slides)
- [Ambiguity: what to do when the brief is thin](#ambiguity-what-to-do-when-the-brief-is-thin)

## The spine

Write the spine before writing a slide. Six to nine lines, one per section, each
naming the claim the section earns:

```
1  The case is a knowledge problem, not a programming problem
2  Nine studies leave one dimension unmeasured, and that is the gap
3  Four cumulative configurations, 15 runs, three separate instruments
4  The final configuration reaches O1 and O2; O3 stays open
5  Claim first, then the bound; the next step closes exactly that gap
```

If a section's claim cannot be written in one line, the section is two sections, or
it is not yet an argument. It is a topic. `Methode` is a topic. `Four cumulative
configurations, 15 runs` is a claim.

**Read the assertions alone.** A reader who sees only the slide assertions should be
able to reconstruct the argument. The assertion is the title, or — in a claim-first
deck — the claim bar at the foot of the slide. If neither exists, the deck is a set
of notes.

## Assertion titles

Every content slide's title is a sentence that could be false. A claim-first deck
puts that sentence in the claim bar instead and drops the title; both are the same
rule, and a slide with neither is the defect the rule exists to prevent.

| not this | this |
|---|---|
| Ergebnisse | Im Endstand erreichen beide Zielgrößen alle Läufe |
| Limitations | Analytische, nicht statistische Generalisierung |
| Methodik | Das Artefakt ist das Setup, nicht der generierte Code |
| Fazit | Wartbarkeit blieb offen, und das ist die nächste Aufgabe |

This is the single highest-leverage change available to a weak deck. It costs
nothing, it forces the author to know what each slide is for, and it makes the deck
readable at a glance from the back of the room.

Two-line titles are fine and often better. `header()` positions the rule from the
title height, so nothing is cramped. Put the newline where the sentence breaks, not
where the line runs out.

## Claim, evidence, bound

The rhythm inside a slide, and across the deck:

1. **Claim.** The affirmative sentence. First, unhedged, and short.
2. **Evidence.** The specific fact, number or source that carries it.
3. **Bound.** What it does *not* show. In the next breath, never instead of step 1.

A speaker who opens with the bound has conceded before being asked. A speaker who
never gives the bound is not trusted. Half a slide of evidence and one line of bound
is the usual proportion, and for a defence or a viva, the bound is what makes the
claim credible, not what weakens it.

The same rhythm at deck scale: the results section claims, the discussion bounds, and
the closing slide answers the opening question rather than summarising.

**Do not end on a summary.** `Zusammenfassung` slides are read as filler. End on the
answer, in the same words as the question, so the audience hears the loop close.

## The time budget

Count words, then divide. A deliberate speaker delivers **110–130 words per minute**;
a nervous one does 150 and runs out of slides. Budget:

| | per minute | for 10 min | for 20 min | for 45 min |
|---|---|---|---|---|
| slides (including dividers) | 1.6–2.0 | 18–22 | 35–42 | 70–85 |
| words of script | 110–130 | 1,100–1,300 | 2,200–2,600 | 5,000–5,800 |

Give dividers 5 seconds. Give a title slide 20. Everything else is content, and the
content must be weighted by what the audience cannot infer: the gap, the method, the
result, the bound. **The literature review gets less time than the author wants.**

Write the budget into the notes slide by slide, with a running clock, `ZEIT 4:20–5:10
(50 s)`. A budget that is only in the speaker's head gets spent in the first third.

Then name the two slides that are cuttable if the talk runs long, and say so in the
deck README. A talk that is 30 seconds over is a talk that skipped its own conclusion.

## Speaker notes

Every slide carries notes. The notes are a script to rehearse rather than a script to
read, plus the things the slide deliberately omits.

```python
s.notes("""
ZEIT 4:20–5:10 (50 s)

The one thing this slide must land: <it>.

Sprechtext: „<the paragraph, in the talk's language, including the pauses>“

If asked <the likely question>: <the answer, with the evidence for it>.
""")
```

Three parts, and the third is the one people skip:

- **The time and the target.** What this slide is *for*.
- **The spoken text.** Written out. Improvising from bullets produces a different
  talk every rehearsal, and the rehearsal never converges.
- **Contingency.** The question this slide will provoke, and the answer. This is
  where a defence is actually won, the notes are the place where the deck and the
  thing being defended meet.

**The first two parts are machine-readable.** `deck timing` reads the `ZEIT h:mm–h:mm (N s)`
line for the slot, sums the budget against `Deck(minutes=…)`, and counts the words in the
`Sprechtext: „…“` block against the 110–130 wpm band. Keep the markers exactly as above or
the rehearsal report goes blind: a slide with no `ZEIT` line is reported as not budgeted,
and a slide whose script is not quoted after `Sprechtext` has no word count. `deck handout`
puts the same notes beside each slide's image, one page per slide, for rehearsal.

`check` fails a slide with no notes. That is on purpose: a slide with no notes is a
slide nobody has decided how to present.

## Backup slides

Eight to twelve, after a `backup_divider`. Each answers **one** question the talk is
likely to raise, and the note names that question:

```python
s = deck.open("B4 · Das O3-Review im Detail", "Backup")
s.notes("Erwartete Frage: „O3 ist offen — warum nicht nachgeholt?“ — Antwort: …")
```

Two rules:

1. **Never show a backup slide unasked.** Opening one to fill time reads as padding.
2. **Never put a detail in the talk that only a backup slide supports.** The backup
   is for questions, not for the load-bearing argument.

The highest-value backup slides for a defence are: the related-work matrix in full,
the numbers to have cold, the two or three weakest points with the answer to each,
and a slide mapping every criticism received to where the reply lives. That last one
is for the speaker, not the examiners, and it should not be opened unless asked.

## Ambiguity: what to do when the brief is thin

A deck request often arrives as "make me a deck for X". Four facts change the whole
design, so ask for them in **one** batched round, recommended option first:

1. **Audience and their prior knowledge**: do they know the domain, or does the
   first third of the talk have to teach it?
2. **Duration and whether questions are included**: 10 minutes plus 20 of questions
   is a different deck from 30 minutes straight.
3. **Language**: if the request and the source material disagree, ask; do not infer
   from the source. A German defence of an English thesis is normal.
4. **What the deck must achieve**: persuade a jury, report to a client, teach a
   class, pass a viva. The same content arranged for those four reads completely
   differently, and the difference is in the bound, not the claim.

If the answer is "just do it", proceed on named defaults and **print them**:

```
defaults: language=de (source is de) · 10 min + questions · figures reused
from the source · backup slides for the eight expected questions
```

Then the user can correct one line instead of rejecting the deck.

Once the four facts are fixed, the **genre profile** follows from them: a "defend" goal
is a defence (`genre-dsr-defence.md`); a "report to a scientific audience" goal with a
method and baselines is a paper talk (`genre-empirical-cs-talk.md`). Read it before
writing the spine, because it decides what the spine is.
