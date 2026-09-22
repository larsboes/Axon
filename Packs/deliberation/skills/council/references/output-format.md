# Output format

The transcript is the product. Print it as the council runs, not after.

## Header

Print this before round 1:

```markdown
## Council: [the decision, as a question]

**Options:** [option A] · [option B] · …
**Mode:** DEBATE (3 rounds) | QUICK (1 round)
**Members:** [name — role, type], [name — role, type], …
**Advocates:** [option A] — [name] · [option B] — [name] · …
**Evidence supplied:** [what was gathered in step 5, listed]
```

Every option gets a name beside it. An option with nothing beside it means the council is not ready:
fix it per step 4 of the skill before round 1 runs. The table is printed here rather than checked
silently because this is the last moment the fault is cheap — one member's brief, against a clerk
pass that reads every pointer in every round.

## Rounds

Print each round under its own heading, in the order the members were launched:

```markdown
### Round 1 — Positions

**[Name] — [role]:**
[the member's text, unedited]

**Clerk — round 1:** checked [N] claims, [M] resolve.
[one line per claim that did not resolve: member — claim — verdict — what the source says]
```

Print the member text as returned. Do not shorten it, do not fix its grammar, and do not remove
an `[unverified]` mark. The clerk line goes under the round, never inside a member's text: an
annotation the member did not write must never look like something it did.

A round where every citation resolves still prints the clerk line. "Checked 14 claims, 14 resolve"
is the receipt that the check ran, and a missing line reads as a skipped step.

## Synthesis

Print this last:

```markdown
### Synthesis

**Agreed:** [claims no member contested, one per line]

**Contested:** [claims that stayed contested, with the members on each side]

**Unverified and load-bearing:** [each [unverified] claim that an argument rests on, with the
member who made it and what would settle it]

**Corrected by the clerk:** [each claim the clerk returned as narrower, contradicted or missing,
with the member, the pointer, and what the source actually says. Write "none" when every citation
resolved.]

**Recommendation:** [one option, and the reason it wins over the runner-up]

**Minority position:** [the strongest position that lost, attributed by name, in its own words,
plus the fact that would make it the recommendation]
```

Rules for the synthesis:

- One recommendation. "It depends" is not a recommendation. If the decision genuinely depends on
  an unknown, name the unknown and the measurement that resolves it.
- The minority position is never omitted. If every member converged, write "none — the council
  converged in round [n]" and treat the converged run as the weaker result it is.
- The synthesis adds no claim that no member made.
- **A load-bearing `[unverified]` claim gets settled or it gets named.** When the recommendation
turns on one claim nobody could check, the synthesis is not finished: use the `ask` tool once and
offer to settle it now — with the exact command or lookup that would do it — or to leave it open.
If the caller leaves it open, the synthesis states it as open under a heading that says so, and
names what would settle it. Do not present a recommendation resting on an unchecked claim as
decided. A run's unverified claims are the only part of a council whose value survives the round
that produced it, and on the first real run three of them were left in a transcript that nothing
kept.

## QUICK summary

QUICK ends with this instead:

```markdown
### Summary

**Consensus:** [what they agree on, or "none"]
**Concerns:** [each concern, with the member who raised it]
**Verdict:** proceed | reconsider | escalate to DEBATE | recompose
```

Choose `escalate to DEBATE` when two members contradict each other on a fact, when no option can be
named, or when the clerk returns `contradicted` on a claim an argument rests on — QUICK has no second
round, so nothing corrects that claim, and a one-round verdict resting on it is not a result.

Choose `recompose` when the council did not test what it claims to: an option had no advocate, or
every member converged in round 1. Both are faults in the council rather than evidence about the
decision, and a recommendation from either is a receipt for what the caller already thought. This is
the verdict to reach for when the run felt smooth.
