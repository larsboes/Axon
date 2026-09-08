# Output format

```markdown
## Skill gaps — [window], [N] prompts across [M] sessions

**Sources:** [history path, skill roots, pack roots]
**Warnings:** [every line from the script's warnings array, or "none"]

### Proposed

**1. [name] — [one line: what it would do]**
- **Evidence:** [cluster tokens], [sessions] sessions over [spanDays] days, [N] friction pairs
- **The failure it removes:** [the specific thing that went wrong, quoted from the corpus]
- **The procedure:** [five steps, or it does not qualify]
- **Not covered by:** [each near-miss skill, and why its body does not cover this]
- **Effort:** [estimate, marked as an estimate]
- **What would change this ranking:** [the evidence that would move it up or down]

### Rejected

| Candidate | Failed test | The number that failed it |
|---|---|---|
| [candidate] | recurrence · procedure · model-gets-it-wrong · cost · already owned | [count, or the skill that owns it] |

### Description fixes instead of new skills

| Existing skill | Words the corpus uses that its description lacks |
|---|---|

### What this run could not see
[the limits from the SKILL.md, instantiated for this corpus — the window, the missing sources]
```

## Rules

- **Never propose more than three.** A shortlist of eight is a list, and a list moves the judgment
  back onto the reader, which is the work this skill was supposed to do.
- **Every proposal quotes the corpus.** A proposal with no quoted prompt behind it is an idea about
  skills in general, and those are free and worthless.
- **Report zero proposals when zero pass.** Write it as the finding it is: "nothing recurred enough
  to justify a skill in this window", with the counts that show it.
- **Never write the skill.** Not even a draft SKILL.md, not even "to save a step". Hand over the
  shortlist and stop.
