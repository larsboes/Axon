# The bar

A candidate becomes a proposal only by passing all five tests. Each one has a cheap check against
the corpus, so a proposal carries evidence rather than enthusiasm.

## 1 — It recurred across sessions

**Check:** `sessions >= 3` in the cluster, spread over more than a few days.

One session that returned to a topic eleven times is one task. A skill built from it fires once
and then sits in the trigger budget forever. Three sessions on three days is the floor, and the
`spanDays` field is what separates "a hard week" from "a standing problem".

## 2 — The work has a stable procedure

**Check:** write the procedure in five steps. If you cannot, it fails.

A skill is instructions that are the same next time. "Help me think about the architecture" has no
procedure; "read the tree, name the owning module, check the call sites, propose the placement,
record the ruling" does. A topic with no procedure is a preference, and preferences belong in the
always-on context file, not in a skill.

## 3 — The model gets it wrong without help

**Check:** name the specific failure the skill prevents, and find it in the corpus.

This is the test that kills the most candidates and the one most often skipped. A model that
already does the task correctly does not need a skill telling it to; the skill just costs tokens
and a trigger slot. Look for the evidence in the friction pairs: a correction the user had to type
twice is the failure the skill would remove. No evidence of failure means the honest verdict is
"the default behaviour is fine", and that is a result worth reporting.

## 4 — The cost repeats

**Check:** how long does the work take, times how often did it happen in the window?

A rare, cheap task does not repay a skill. A frequent, cheap one does — that is where the whole
value of automation lives. A rare, expensive, high-consequence one can also qualify, but only when
test 3 is strong: the case for it is that a mistake is costly, not that the work is long.

## 5 — Nothing already owns it

**Check:** read the bodies of every skill in `registry` whose description touches the topic.

Coverage means three things at once, and a candidate that fails any of them is *not* covered:

1. **It addresses the failure class**, not the subject area. A vault skill that never mentions
   this failure does not cover it.
2. **It is installed where the work happens.** A skill in the Pack source but not deployed to the
   harness the user is actually typing into covers nothing today.
3. **It would trigger.** Compare the skill's description against the actual words in the cluster
   samples. A skill that only fires on vocabulary the user never uses is invisible.

The third case has a cheaper fix than a new skill: change the description. Always report it that
way when that is what the evidence says.

## Rejection is the useful half

Every rejected candidate is written down with the test it failed and the count that failed it.
Without that record, the same idea returns next month with the same enthusiasm and no memory of
why it was dropped. A run that rejects five candidates and proposes none has done its job, and it
should say so plainly rather than promoting the least-bad option.

## The ranking

Rank survivors by `(sessions × friction pairs) ÷ estimated effort`. Treat the number as an ordering
device, not a score to defend: it exists to force a comparison, and the prose underneath is where
the argument actually lives. State the effort estimate as an estimate — nothing in the corpus
measures it.
