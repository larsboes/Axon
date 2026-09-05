# Output format

One report. Print it in this order.

```markdown
## Red team: [the proposal, in one line]

### Claims
1. [claim, quoting the proposal where possible]
2. …

Preferences, not claims: [anything unfalsifiable that was moved out]

### Steelman
1. [12 to 20 words]
2. …
[6 to 8 points. The version the author would sign.]

### Counter-argument
1. [12 to 20 words]
2. …
[6 to 8 points, attacking the steelman above, strongest first.]

### Findings

| # | Claim | Severity | Mechanism | Evidence | Test that settles it |
|---|---|---|---|---|---|
| 1 | 3 | fatal | … | `path:line` | … |

Discarded: [n] findings with no mechanism or no evidence.
Convergent: [findings that two or more lenses reached independently]

### The core issue
[One paragraph. The single assumption whose failure takes the rest with it, or the statement that
the proposal has no single such assumption.]

### Remediation
- Finding 1 → [what would fix it, and what the fixed proposal then claims]
- …

### If the answer is a choice
[Name the options that the findings leave open, and hand off to `council`. Omit this section when
the findings point at one repair.]
```

## Rules

- The counter-argument attacks the steelman printed above it. A counter-argument that attacks a
  weaker version is a defect in the report, not in the proposal.
- Every finding row has a mechanism and a test. A row with neither does not get printed.
- An `[unverified]` finding keeps the mark in its Evidence cell and is ranked no higher than
  `cost`.
- Severity order in the table is `fatal`, `structural`, `cost`, `cosmetic`. Do not reorder for
  effect.
- Report the discarded count. It tells the reader how much of the pass was noise.
- Findings are stated flat. No verdict on the author, and no praise for the attack.
