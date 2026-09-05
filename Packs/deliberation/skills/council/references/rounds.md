# Rounds

How a council run is executed. The transcript shape is in `output-format.md`.

## Launch mechanics

- One member is one Agent call with `subagent_type: "general-purpose"`.
- Every member of one round goes out in a single message, so the round runs in parallel.
- Rounds are sequential. Round 2 needs round 1 in full.
- A subagent keeps no memory between rounds. Paste the whole transcript so far into every
  round 2 and round 3 prompt.
- Member names stay fixed for the whole run. A renamed member breaks the reply chain.
- Do not add or drop a member after round 1.

Every member prompt is built from four parts, in this order: the member brief, the decision and
its options, the evidence collected in step 4 of the skill, and the round instruction below.

## QUICK — one round

Round instruction:

```
QUICK COUNCIL — SINGLE ROUND

Give your position on the decision from your role.
- Write 40 to 60 words.
- State one concern or one recommendation, not both.
- Cite a file, a line, a measurement or a quoted figure for every factual claim.
- Write [unverified] after any claim you cannot cite.
```

Then write the QUICK summary from `output-format.md`. Escalate to DEBATE when two members
contradict each other on a fact, or when the summary cannot name a recommendation.

## DEBATE — three rounds

### Round 1 — positions

```
COUNCIL DEBATE — ROUND 1: POSITIONS

Give your opening position on the decision from your role.
- Write 120 to 180 words.
- Name the option you favour and the single strongest reason for it.
- Name the evidence you demanded and whether you received it.
- Cite a file, a line, a measurement or a quoted figure for every factual claim.
- Write [unverified] after any claim you cannot cite.
- Do not address the other members. You have not read them yet.
```

### Round 2 — challenges

```
COUNCIL DEBATE — ROUND 2: CHALLENGES

Here is round 1 in full:
[round 1 transcript]

Respond to the other members.
- Write 120 to 180 words.
- Quote at least one other member by name and say why their claim fails or holds.
- Attack the strongest version of the position you disagree with, not the weakest.
- Say which of your own round 1 claims you now withdraw, if any.
- Cite a file, a line, a measurement or a quoted figure for every factual claim.
- Write [unverified] after any claim you cannot cite.
```

### Round 3 — closing

```
COUNCIL DEBATE — ROUND 3: CLOSING

Here is the debate so far:
[round 1 and round 2 transcripts]

Close your case.
- Write 100 to 150 words.
- State what the council agrees on.
- State what you still disagree with, and name the member you disagree with.
- State your final recommendation and the one condition that would reverse it.
- Do not manufacture agreement. An unresolved disagreement is a result.
```

## After the rounds

The orchestrator writes the synthesis. Do not spawn a subagent for it: the orchestrator holds
every round, and a synthesis agent would only re-read what is already in context.
