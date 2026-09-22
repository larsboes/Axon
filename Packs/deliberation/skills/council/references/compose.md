# Composing a council

Read this only when no preset in `presets/` fits the topic.

## The brief

Write four to five members. Each brief has five fields and stays under 60 words:

- **Name** — one word. It is used in every round to address the member.
- **Role** — the expertise the member argues from.
- **Stance** — the position the member starts from. Write a position, not a topic.
- **Pushes on** — the two or three things this member attacks in the other members.
- **Demands** — the evidence this member refuses to argue without.

Worked example:

> **Mara — real-time systems engineer.** Holds that a bidirectional transport is the honest
> default. Pushes on reconnection behaviour, connection limits and what happens on a flaky
> network. Demands the measured connection count and the observed reconnect rate.

## Choosing the roles

Design the roles around this decision, not from a list of job titles. Four angles cover most
decisions, and each one has to be filled by somebody who would really hold it:

| Angle | The member argues from | `subagent_type` |
|---|---|---|
| Owner | Having built or run this kind of thing: what actually ships | `council-owner` |
| Sceptic | The failure mode: what breaks and who notices | `council-skeptic` |
| Cost realist | What it costs to build, run and reverse | `council-cost` |
| Outside evidence | Precedent, measurement, and what other people found | `council-evidence` |
| Advocate | The strongest case for one assigned option, held whether or not the member agrees | `council-advocate` |

The four member types are the four angles. Each one carries the read-only contract and the evidence
rule; the brief you write carries everything that makes this council different from the last one.
Write the brief first and pick the type from the angle it landed on — not the other way round, which
produces four members who each argue their job title.

The advocate is the fifth type and the only one whose position is *assigned* rather than held: its
brief names the option, not the stance. Use it when an option has nobody else defending it, because
an option with no advocate loses for the wrong reason — nobody states its strongest case, so the
council reports the absence of a case as the absence of one, and still returns something shaped like
a decision. That is worse than a weak member.

Step 4 of the skill checks this before round 1 and prints the option-to-advocate table in the
header, so a missing advocate is visible before the clerk is paid to read a round that could not
settle anything. The check belongs there and not in a question to the caller: you wrote the briefs,
so which option each member defends is readable off them. The first real run of the skill skipped
it, composed four members who all leaned one way, and recommended the option nobody had argued
against.

## Worked examples

Two decisions, and the four angles each topic produced. No two members hold the same angle, and
no name carries any meaning outside its own council.

**"Should we use WebSockets or SSE for the live feed?"**

| Member | Angle | Stance | Type |
|---|---|---|---|
| Mara | Owner | A bidirectional transport is the honest default | `council-owner` |
| Ilya | Sceptic | The long-lived connection is the part that fails at 3am | `council-skeptic` |
| Başak | Cost realist | Every reconnect is a bill and a support ticket | `council-cost` |
| Ruth | Outside evidence | Two production feeds already went one way | `council-evidence` |

**"Is AI overhyped?"** — as asked, this is a topic with no second option, so the first move is to
restate it as a decision: *do we ship on a hosted model API or keep the pipeline local?*

| Member | Angle | Stance | Type |
|---|---|---|---|
| Tomo | Owner | Ship on the hosted API and stop defending the local path | `council-owner` |
| Nadia | Sceptic | The provider's terms change faster than the code | `council-skeptic` |
| Piet | Cost realist | Per-token spend is unbounded; the local box is already paid for | `council-cost` |
| Wren | Outside evidence | Both local attempts in this repository stalled at evaluation, not at inference | `council-evidence` |

The second example is the one to copy. It restates a topic as a decision, which is the move that
turns four essays into a recommendation. A council convened on a topic produces essays.

### Confirm the restatement with `ask` when the caller gave a topic

Restating a topic as a decision is the one judgement in a council run that the orchestrator cannot
check for itself, and getting it wrong is the most expensive mistake available: every member then
writes against the wrong options and the clerk reads every one of their pointers. On the first real
run, the clerk alone spent 204 seconds on a round whose framing nobody had confirmed.

So when the caller hands over a topic rather than a decision, use the `ask` tool once, before
composing members:

- `prompt`: the decision as you read it, and the options you found
- 2 to 4 mutually exclusive options, each a different reading of what is being decided, with yours
  marked `recommendedIndex: 0` so Enter accepts it
- `context`: the caller's own words, so the correction has something to correct against
- leave `allowNote` on: the correction usually arrives as a note explaining what you misread

Skip this when the caller already named the decision and its options. A question whose answer is
plainly in the request is the kind of `ask` that teaches a reader to stop reading them.

Never use `ask` for the advocate check in step 4. That one is counting, and you have the briefs.

## Rules

- Every brief holds a different position. Two members that agree waste a round.
- A member argues its stance and drops it when the evidence goes the other way. Round 2 asks each
  member which of its own claims it withdraws.
- Do not write a member whose role is to summarize or to moderate. The orchestrator does that.
- Do not exceed five members. A sixth member adds text, not friction: rounds 2 and 3 grow with
  the square of the member count, and every member reads every other member.
- State each member's demands in its brief. A member with no evidence bar produces prose.
