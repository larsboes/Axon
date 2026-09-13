# Defence readiness

`deck check` sees structure. `deck render` sees layout. Neither sees the **argument**, and
a defence is marked on the argument.

This is the third pass. `deck readiness` extracts the deck's argument in one screen. You
read it and answer the five questions below. The pass catches a deck that is structurally
clean, visually clean, and answers the wrong question.

## Procedure

```
- [ ] 1. Run `scripts/deck readiness <deck.py>`. Read the argument, not the slides.
- [ ] 2. Answer the five questions against that text, not from memory of the thesis.
- [ ] 3. Report each finding with its slide number and a severity.
- [ ] 4. Fix, rebuild, and run the readiness pass again with the render pass.
```

**Read the assertions alone first.** `narrative.md` puts it this way: a reader who sees only
the slide titles — or, on a claim-first deck, only the claim bars — should reconstruct the
argument. When the assertions do not answer the five questions, the talk will not either.

**Answer from the extracted argument, not from intent.** The pass exists to catch the
deck whose author knows the answer and whose slides do not state it. When the contribution
sentence lives only in the speaker's head, the skeleton shows it missing.

Key each finding. `K3 · slide 8 · the question names the case as its subject` is
actionable. "The abstraction could be stronger" is not.

## The five questions

These are the five an examiner's written assessment usually turns on. They are rendered
here from a real first assessment of a DSR thesis, the one this skill was extracted from.
Ten of fifteen lost points sat in four of them.

### 1. Does the question abstract beyond the case?

*The examiner asks:* is this about a class of problems, or about one company?

A passing deck states the research question over a **class**, and names the case as the
setting. Apply the delete test from `academic-narrative.md`: remove the case, and the
question must still ask something.

**Catches:** a single-case study that appears to generalise nothing, because its question
only named the case. The usual fix already sits in the thesis, and it never made it into
the question.

### 2. Is the contribution claimed before it is bounded?

*The examiner asks:* what did the field gain?

A passing deck shows a contribution slide with **one unhedged sentence**, the contribution
**type** named (instantiation, construct, method or theory), and the bound in the next
breath.

**Catches:** the bounding register. Every sentence is correct and hedged, and the reader
cannot state the contribution. Check the closing slide. Does it end on the answer, or on
"remains unresolved"?

### 3. Is the literature positioned theoretically?

*The examiner asks:* where does this sit in the literature?

A passing deck shows a positioning slide that names the **construct** each compared work
operationalises, and the column that is empty. A matrix of task, context, validation and
outcomes is an engineering table. It answers "what did they do", and it leaves the reader
unable to place the work.

**Catches:** "the positioning is predominantly practical in nature". The concession is
cheap. The missing column is the finding.

### 4. Is the generalisation scoped, and does the title match the evidence?

*The examiner asks:* how far does this carry?

A passing deck claims **analytical, not statistical** generalisation, names the two or
three findings that transfer, and concedes an over-broad title in one sentence.

**Catches:** an overclaiming title, and the hedge that hides it. When the title names a
class and the evidence covers one case, the deck says so before the examiner does.

### 5. Does the thread run straight?

*The examiner asks:* motivation, problem, gap, question. In that order?

A passing deck's titles reconstruct that line with **no re-entry into case detail after the
artifact is announced**, and the research question stated **exactly once**. Look for the
doubled case-detail block and the restated question. Both are what a "red thread"
criticism describes.

**Catches:** an introduction doing nine jobs in eight hundred words, and a second chapter
that restates the question instead of deriving it.

## What a readiness finding is not

- It is not a layout finding. That belongs to `render`.
- It is not a structural finding. That belongs to `check`.
- It does not replace the speaker's own stance on each point of the written assessment.
  `genre-dsr-defence.md` section The Kritikpunkte → Antwort map holds each stance and its
  evidence. This pass checks that the deck carries them.

Report what you ran and what you read. "Built and checked", "rendered and inspected", and
"read the argument and answered the five" are three different claims.
