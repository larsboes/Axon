# Output format

The transcript is the product. Print it as the council runs, not after.

## Header

Print this before round 1:

```markdown
## Council: [the decision, as a question]

**Options:** [option A] · [option B] · …
**Mode:** DEBATE (3 rounds) | QUICK (1 round)
**Members:** [name — role], [name — role], …
**Evidence supplied:** [what was gathered in step 4, listed]
```

## Rounds

Print each round under its own heading, in the order the members were launched:

```markdown
### Round 1 — Positions

**[Name] — [role]:**
[the member's text, unedited]
```

Print the member text as returned. Do not shorten it, do not fix its grammar, and do not remove
an `[unverified]` mark.

## Synthesis

Print this last:

```markdown
### Synthesis

**Agreed:** [claims no member contested, one per line]

**Contested:** [claims that stayed contested, with the members on each side]

**Unverified and load-bearing:** [each [unverified] claim that an argument rests on, with the
member who made it and what would settle it]

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

## QUICK summary

QUICK ends with this instead:

```markdown
### Summary

**Consensus:** [what they agree on, or "none"]
**Concerns:** [each concern, with the member who raised it]
**Verdict:** proceed | reconsider | escalate to DEBATE
```

Choose `escalate to DEBATE` when two members contradict each other on a fact, or when no option
can be named.
