# Skill evaluation

## Contents
- Three test tiers
- Iterate on one hard case first
- Under/over-triggering: signal → fix

## Three test tiers
Write these **before** padding out instructions — evaluations are how you find the actual gap, instead of documenting an imagined one.

| Tier | Goal | Example assertion |
|------|------|--------------------|
| 1. Triggering | Loads when it should, stays silent when it shouldn't | Should trigger: paraphrases of the obvious ask. Should NOT trigger: adjacent-but-different tasks, unrelated topics. |
| 2. Functional | Produces correct output | Given inputs → expect: specific state changes, zero unexpected failures, edge cases handled. |
| 3. Performance comparison | Skill beats no-skill baseline | Without: N back-and-forths, M failed calls, T tokens. With: fewer of each. If it isn't better on all three, the skill isn't earning its context cost. |

## Iterate on one hard case first
Don't start with broad coverage. Pick the single hardest realistic task, run it with no skill, capture the actual failure verbatim, then write only enough instruction to fix that failure. Expand to more test cases only after that one passes — this gives faster signal than authoring against imagined requirements.

## Under/over-triggering: signal → fix
| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| Skill never loads when it should | Description too generic, missing the keywords a user would actually say | Add concrete trigger phrases + relevant file types/terms |
| Skill loads for unrelated queries | Description too broad | Add a negative trigger ("Do not use for X — use `sibling-skill` instead") |
| User manually enables it every time | Same as under-triggering | Same fix — re-run tier-1 tests after editing |
| Skill "stops working" after a few turns | Content is still in context; the model is choosing something else | Sharpen instructions/description, or re-invoke after compaction if it's large |

Debug fast: ask Claude directly "when would you use the `<name>` skill?" — it quotes its own understanding of the description back, which shows you exactly what's missing without a full test run.
