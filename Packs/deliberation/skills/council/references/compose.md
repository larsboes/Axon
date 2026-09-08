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

The four types are the four angles. Each one carries the read-only contract and the evidence rule;
the brief you write carries everything that makes this council different from the last one. Write
the brief first and pick the type from the angle it landed on — not the other way round, which
produces four members who each argue their job title.

Add a fifth member when one option has nobody who would defend it. An option with no advocate
loses for the wrong reason. A fifth member has no matching type: launch it as `general-purpose`
and paste the contract into its prompt — read only, never edit, cite or mark, return the round text
alone.

## Rules

- Every brief holds a different position. Two members that agree waste a round.
- A member argues its stance and drops it when the evidence goes the other way. Round 2 asks each
  member which of its own claims it withdraws.
- Do not write a member whose role is to summarize or to moderate. The orchestrator does that.
- Do not exceed five members. A sixth member adds text, not friction: rounds 2 and 3 grow with
  the square of the member count, and every member reads every other member.
- State each member's demands in its brief. A member with no evidence bar produces prose.
