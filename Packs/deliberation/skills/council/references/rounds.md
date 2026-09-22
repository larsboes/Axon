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
| One assigned option, held whether or not the member agrees | `council-advocate` | opus |

  These agent types carry the contract, not the character: read-only tools, the evidence rule,
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
its options, the evidence collected in step 5 of the skill, and the round instruction below.

**Assemble the evidence pack for this run. Do not reuse one from an earlier run.** Its line numbers
are citations, and one `edit` between two runs moves every one of them: re-running the same decision
after editing a cited file left line 72 pointing at an unrelated sentence, and the clerk returned
`missing` on three claims that were true and merely mis-numbered. The clerk caught it, which is the
control working — but a pack that costs nothing to rebuild is not worth defending.

A member cannot run a command and cannot fetch a page. It reads files. Everything else it is
allowed to cite has to arrive in the prompt, which is what step 5 of the skill is for. A member
that needs a measurement nobody took names the command and marks the claim `[unverified]`.

Two pointers exist that are not file paths, and every member prompt has to say so, because a clerk
that cannot resolve them reports honest arguments as uncited:

- `[brief]` — a claim about the decision, the options or another member's brief. That text is in
  the prompt and nowhere on disk.
- `[evidence]` — a claim quoting the evidence pack the orchestrator supplied.

The clerk is given both, so it can check a `[brief]` or `[evidence]` claim against what was
actually supplied rather than searching the repository for a sentence that was never written to a
file. The first real run of this council lost one otherwise-sound argument to exactly that gap.

## QUICK — one round

Round instruction:

```
QUICK COUNCIL — SINGLE ROUND

Give your position on the decision from your role.
- Write 40 to 60 words.
- State one concern or one recommendation, not both.
- Cite a file, a line, a measurement or a quoted figure for every factual claim.
- Write [brief] or [evidence] when the claim is about the decision or the supplied evidence.
- Write [unverified] after any claim you cannot cite.
```

Then write the QUICK summary from `output-format.md`.

**Narrow the clerk on QUICK.** The synthesis here rests on a handful of claims, so send the clerk
the claims the recommendation and the minority position actually depend on, and have it name the
remainder as unchecked rather than resolving them. A full pass is what a three-round DEBATE needs.
Two measured runs of the same decision, 2026-09-16: a full pass over 18 claims took 204 seconds and
66k tokens; a narrowed pass over 19 claims took 68 seconds and 44k tokens, and it caught more,
because the claims it did check were the ones that mattered. Say in the clerk line that the pass was
narrowed, so a later reader can tell a cheap check from a complete one.

Escalate to DEBATE when two members contradict each other on a fact, when the clerk returns
`contradicted` on a claim the recommendation rests on, or when the summary cannot name a
recommendation. Only the last of those is visible without the clerk — the other two are why a QUICK
run with a real decision behind it still pays for one.

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

## Wall clock

Measured on the first real pi run, 2026-09-16, QUICK mode with four members and one clerk.

| Phase | Wall clock | Parallelism |
|---|---|---|
| Round 1 | 9 to 18 s | every member at once (4 members) |
| Rounds 2 and 3 | same as round 1 | every member at once |
| Clerk | **1 to 4 minutes** | one clerk, after the round it checks |
| Synthesis | inline | the orchestrator, no subagent |

The clerk, not the members, is the cost. That run's four members returned in 9 to 18 seconds; the one
clerk pass took 204 seconds and 66k tokens, because it reads every pointer itself and on a live round
it has fifteen or more to resolve. So QUICK is one round plus one clerk, 1.5 to 4 minutes, and DEBATE
is three of each, 4 to 13 minutes — not the 45 to 90 seconds this table claimed before it had been
run. Both figures assume the round was sent as a single message; a round launched one member at a time
costs the sum of its members rather than the maximum, which is the whole reason the launch is batched.

Two consequences worth knowing before convening:

- **Budget the clerk, not the debate.** A member writes 40 to 150 words from evidence already in its
  prompt. The clerk opens files. If a run feels expensive, the fix is fewer claims to check, not fewer
  members.
- **Skip the clerk on a rehearsal.** While composing members and tuning briefs, run the round without
  it — the verdicts only matter on a round whose result you intend to act on.

## After the rounds

The orchestrator writes the synthesis. Do not spawn a subagent for it: the orchestrator holds
every round, and a synthesis agent would only re-read what is already in context.
