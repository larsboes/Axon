# Method

The steps of steps 1 to 5 of the skill, in detail.

## 1 — Decompose into atomic claims

Rewrite the proposal as a numbered list. Each entry states one thing that is true or false.

- Split a compound sentence into one claim per clause. "It is faster and cheaper" is two claims,
  and they fail separately.
- Separate the claim from its justification. Attack both, but never as one item.
- Write out the assumptions the proposal uses without stating them. These are where the fatal
  findings usually are, because nobody defended them.
- Move anything unfalsifiable to a short list headed "preferences, not claims". Do not attack it.

Quote the proposal's own words for each claim where possible. A paraphrase is where a strawman
enters.

## 2 — Steelman

Write the proposal as its author would if they had another hour.

- Repair its weak wording and supply the argument it left implicit.
- Use the strongest evidence available for it, including evidence the author did not cite.
- Drop the claims the author would drop under pressure. A steelman is not a summary.

Test: the author reads it and says "yes, that is my argument, better put". If the steelman is
weaker than the original, restart.

## 3 — Attack

Run every lens in `lenses.md` against the claim list, not against the summary. Each lens returns
findings in the form: the claim number, the mechanism by which it fails, the evidence, and the
observation that would settle it.

Attack the steelman, never the original. The original is already beaten.

When two lenses find the same thing independently, record the convergence. Convergence across
unrelated lenses is the strongest signal this method produces, and it is lost if the duplicate
is merged away silently.

## 4 — Rank by severity

| Severity | Meaning |
|---|---|
| `fatal` | The proposal fails outright if this holds. No version of it survives. |
| `structural` | The shape is wrong. The goal survives, the design does not. |
| `cost` | It works and costs more than stated, in money, time or maintenance. |
| `cosmetic` | Real, cheap to fix, and changes no decision. |

Rules:

- A finding with no mechanism is not a finding. Delete it.
- A finding that only restates the claim in a hostile tone is not a finding. Delete it.
- An `[unverified]` finding is ranked no higher than `cost`, whatever it would be if confirmed.
  Report what would confirm it.
- Report the count deleted. The reader should know how much noise the pass produced.

## 5 — The core issue

Name the one assumption whose failure takes the rest with it. It is usually one of four kinds:

- A hidden assumption that is false.
- A step that does not follow from the step before it.
- A category error: the proposal treats one thing as another and inherits rules that do not apply.
- A precedent that already went the other way and was never addressed.

If no single assumption carries the proposal, say so. A proposal with several independent legs is
harder to kill, and that is a finding in the proposal's favour.
