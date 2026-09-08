# Grounding: verify before you claim

The discipline is one sentence — *never assert a fact you have not measured* — and it is hard to
follow because plausible inference feels like knowledge. This file is the evidence that it is not,
drawn from a single long session on a real system, plus the check that would have caught each case.

Read it once. The point is not the specific mistakes; it is that they were all **confidently
stated, individually reasonable, and wrong**.

## The catalogue

### 1. Measuring the wrong thing and believing the result

**Claimed:** "Postgres is down."
**Actually:** it had run continuously for two days.
**Cause:** `pg_isready` was not installed, so the command failed and the failure was read as
"the service is down" rather than "the check did not run."

> A tool that is absent and a service that is dead produce the same silence. Check that your
> instrument exists before trusting what it reports.

### 2. Inferring a cause from a coincidence

**Claimed:** "Backups stopped when the old system was removed."
**Actually:** backups had never been scheduled at all. There was no agent to stop.
**Cause:** two facts — backups are stale, a removal happened recently — arranged into a story.

> Two true facts do not make a causal claim. The check was one directory listing: *is there a
> scheduled job for this, and was there ever?*

### 3. Counting with the wrong resolution

**Claimed:** "74 embeds point at files that no longer exist."
**Actually:** 83 pointed at missing files, and a further 57 pointed at *missing views inside files
that existed* — a second defect class entirely, invisible to the first count.
**Cause:** the measurement answered a narrower question than the one being asked.

> When a count feels surprising, ask what it *cannot* see. Here the checker resolved filenames and
> never looked inside them.

### 4. Estimating impact instead of measuring it

**Claimed:** "Moving these files will break 257 links."
**Actually:** 33 broke. Most references still resolved.
**Also:** 92 silently retargeted to *different* files with the same basename — which nobody had
asked about and which is worse, because a dead link announces itself and a wrong one does not.

> The estimate was 8× too pessimistic and missed the only dangerous failure mode. The project's
> own link checker answered both questions in one run.

### 5. A parser that quietly saw half the data

**Claimed:** "These frontmatter keys hold no values — safe to delete."
**Actually:** 27 held real values in YAML block-list form, which the inline-only parser could not
see. They were deleted. A verification pass caught it and it was reverted.
**Cause:** the parser handled one of two dialects present in the same corpus.

> The lesson is not "write a better parser". It is **verify the operation, not the plan**: diff
> the before and after and assert nothing of value disappeared. That check found it in seconds.

### 6. A document that contradicted the code it described

Five separate claims in a specification were disproven by reading the implementation — including
one that ruled out a technique the codebase already shipped, tested and pinned.

> A document is a *claim* about a system. Documents drift; code runs. When they disagree, find out
> which is lying before you propose anything, because a plan built on a stale document is stale
> on arrival.

### 7. Same value, two spellings

Three times in one corpus: capitalised versus lowercase keys, emoji versus word values, and
identical-looking emoji differing by an invisible variation selector. Every case had the same
effect — a query saw one spelling and silently missed the other.

> `sort | uniq -c` over the raw values of any field you are about to rely on. Two spellings of one
> value is the most common silent defect in a hand-maintained corpus.

### 8. Success reported for work not done

Two tools reported success while doing nothing: one installed a scheduled job and left it disabled,
another ran a shell fragment that never executed while reporting `running` with empty stderr.

> **Check the artifact, not the exit code.** Did the file appear? Is the port open? Did the row
> count change? An exit status is a claim, and this file is about claims.

## The checks, as a habit

Before asserting anything about a system:

| Claim shape | The check |
|---|---|
| "X is running / not running" | Query the thing itself — a port, a process, a health endpoint. Confirm your instrument exists |
| "X caused Y" | Find the mechanism. Absent a mechanism, say the two facts and not the arrow |
| "There are N of X" | Run the count. State the method so it can be redone |
| "This change will break N things" | Use the project's own checker. Then ask what it cannot see |
| "This field is empty / unused" | Check both dialects, then diff before and after the operation |
| "The docs say…" | Read the code. Note which one is stale |
| "The command worked" | Check the artifact it should have produced |

## Why this earns its cost

The measured version is not merely more accurate. It is **more persuasive and more useful**:

- "42% of this folder is under 40 words, against 6% in the healthy one" starts a design
  conversation. "Your notes are messy" starts an argument.
- Numbers survive the session. A claim in a document with its method attached can be re-run in a
  year; an adjective cannot.
- The gaps between claim and measurement are where the real findings live. Every genuinely
  surprising result in that session came from a number disagreeing with a sentence.

## Recording what you measured

When a number goes into a document, record how to get it again:

```markdown
**Triage, 2026-08-23.** Measured from the repo, not from the names. Method, so it can be redone:
file counts under `capabilities/<name>/`, `git log --oneline -- capabilities/<name>` for commit
counts, `git log -1 --format=%ad` for last touch. No document holds this data. The measurement is
the source.
```

A number without a method is an assertion wearing a number's clothes.

## When you cannot measure

Say so, in the document, at the point of the claim:

```markdown
| Claim | Where | Status |
|---|---|---|
| Competitor X protects data only with local models | §1, §6, §11 | **My impression.** No source. Tracked as Q25 |
```

An unverified claim marked as unverified is honest and useful. The same claim unmarked is a
liability, and it is worse when the document is otherwise rigorous — the surrounding precision
lends it credibility it has not earned.
