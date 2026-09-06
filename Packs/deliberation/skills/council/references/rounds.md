# Rounds

How a council run is executed. The transcript shape is in `output-format.md`.

## Launch mechanics

- One member is one Agent call. Pick the `subagent_type` from the member's angle:

| The member argues from | `subagent_type` | Runs on |
|---|---|---|
| Having built or run this kind of thing | `council-owner` | sonnet |
| The failure mode: what breaks and who notices | `council-skeptic` | opus |
| What it costs to build, run and reverse | `council-cost` | sonnet |
| Precedent, measurement, what other people found | `council-evidence` | sonnet |
| A fifth angle none of the four covers | `general-purpose`, with the read-only contract pasted into the prompt | inherited |

  These four agent types carry the contract, not the character: read-only tools, the evidence rule,
  the `[unverified]` mark, and "return the round text alone". The member's name, stance, what it
  pushes on and what it demands all still come from the brief in the prompt, so two councils on two
  topics share the type and share nothing else. The model per type is set in the agent file
  (`agents/council-*.md` in this Pack) — change it there, not per run.

- A harness with no native subagents has none of these types. Launch every member as
  `general-purpose` and paste the contract into the prompt: read only, never edit, cite or mark,
  return the round text alone.
- Every member of one round goes out in a single message, so the round runs in parallel.
- Rounds are sequential. Round 2 needs round 1 in full.
- A subagent keeps no memory between rounds. Paste the whole transcript so far into every
  round 2 and round 3 prompt.
- Member names stay fixed for the whole run. A renamed member breaks the reply chain.
- Do not add or drop a member after round 1.

Every member prompt is built from four parts, in this order: the member brief, the decision and
its options, the evidence collected in step 4 of the skill, and the round instruction below.

A member cannot run a command and cannot fetch a page. It reads files. Everything else it is
allowed to cite has to arrive in the prompt, which is what step 4 of the skill is for. A member
that needs a measurement nobody took names the command and marks the claim `[unverified]`.

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

## The clerk — after every round

Print the round, then launch one `council-clerk` agent on it before the next round starts. One
clerk per round, not one per member: it needs to see the members contradict each other.

```
COUNCIL CLERK — ROUND [n]

Root directory for every claim: [absolute path, or "none — this decision is about the world"]

Here is round [n] in full:
[round transcript]

Here is the evidence pack the members were given:
[the same evidence, so you can tell a supplied quotation from an invented one]

Check every factual claim against its pointer and return your list.
```

Apply the clerk's verdicts to the transcript before the next round runs:

| Verdict | What the orchestrator does |
|---|---|
| `resolves` | Nothing |
| `narrower` | Annotate the claim in the transcript with what the source actually says |
| `contradicted` | Annotate, and carry the contradiction into the next round's prompt |
| `missing` | Strike the claim and name the member and the path in the synthesis |
| `uncited` | Add or keep the `[unverified]` mark at the claim |

Round 2 and round 3 read the annotated transcript, never the raw one. A member that argues against
a struck claim wastes its round.

## After the rounds

The orchestrator writes the synthesis. Do not spawn a subagent for it: the orchestrator holds
every round, and a synthesis agent would only re-read what is already in context.
